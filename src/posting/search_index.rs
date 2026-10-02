//! Port of app/services/search_indexer.rb for posts and topics: the
//! weighted tsvector Postgres builds, Discourse's extra lexemes for dotted
//! words and its cap on repeated positions, and HtmlScrubber, which turns
//! cooked HTML into the indexed text.

use std::sync::LazyLock;

use markup5ever_rcdom::{Handle, NodeData};
use regex::Regex;
use sqlx::PgConnection;

use crate::AppError;
use crate::Unsupported;
use crate::pretty_text::cleanup::{attr, element_name, has_class, parse_document};
use crate::search::{clean_term, ts_config};
use crate::site_settings::SiteSettings;

const POST_INDEX_VERSION: i32 = 5;
const TOPIC_INDEX_VERSION: i32 = 4;
/// `Topic::MAX_SIMILAR_BODY_LENGTH`
const MAX_SIMILAR_BODY_LENGTH: usize = 200;

/// What a post's index is built from.
pub struct PostIndex<'a> {
    pub post_id: i32,
    pub topic_id: i32,
    pub is_first_post: bool,
    pub topic_title: &'a str,
    pub category_name: Option<&'a str>,
    pub tag_names: Option<&'a str>,
    pub cooked: &'a str,
    pub private_message: bool,
}

/// `SearchIndexer.index(post)` after the post's cooked changed: the post's
/// row, and the topic's for a first post.
pub async fn index_post(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    base: &BaseUrls<'_>,
    post: &PostIndex<'_>,
) -> Result<(), AppError> {
    check_unported(settings)?;
    let default_locale = settings.get("default_locale")?.to_s();
    let text = scrub(post.cooked, base);
    let d: String = text.chars().take(600_001).collect();
    let weights = [
        Some(post.topic_title),
        post.category_name,
        post.tag_names,
        Some(d.as_str()),
    ];
    let (search_data, prepared) = tsvector(conn, settings, None, &weights).await?;
    let raw_data = clean_post_raw_data(&prepared[3])?;
    sqlx::query(
        "INSERT INTO post_search_data (post_id, raw_data, locale, version, search_data, private_message) \
         VALUES ($1, $2, $3, $4, $5::tsvector, $6) \
         ON CONFLICT (post_id) DO UPDATE SET raw_data = EXCLUDED.raw_data, locale = EXCLUDED.locale, \
           version = EXCLUDED.version, search_data = EXCLUDED.search_data, \
           private_message = EXCLUDED.private_message",
    )
    .bind(post.post_id)
    .bind(&raw_data)
    .bind(&default_locale)
    .bind(POST_INDEX_VERSION)
    .bind(&search_data)
    .bind(post.private_message)
    .execute(&mut *conn)
    .await?;
    if post.is_first_post {
        let body: String = text.chars().take(MAX_SIMILAR_BODY_LENGTH).collect();
        let weights = [Some(post.topic_title), Some(body.as_str()), None, None];
        let (search_data, prepared) = tsvector(conn, settings, None, &weights).await?;
        let raw_data = prepared
            .iter()
            .filter(|d| !d.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        sqlx::query(
            "INSERT INTO topic_search_data (topic_id, raw_data, locale, version, search_data) \
             VALUES ($1, $2, $3, $4, $5::tsvector) \
             ON CONFLICT (topic_id) DO UPDATE SET raw_data = EXCLUDED.raw_data, locale = EXCLUDED.locale, \
               version = EXCLUDED.version, search_data = EXCLUDED.search_data",
        )
        .bind(post.topic_id)
        .bind(&raw_data)
        .bind(&default_locale)
        .bind(TOPIC_INDEX_VERSION)
        .bind(&search_data)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

const USER_INDEX_VERSION: i32 = 3;

/// `SearchIndexer.update_users_index`: username (A) and name (B),
/// lowercased, under the "simple" stemmer. Searchable user fields (C) are
/// not ported.
pub async fn index_user(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
    username_lower: &str,
    name: Option<&str>,
) -> Result<(), AppError> {
    check_unported(settings)?;
    let searchable: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_fields WHERE searchable)")
            .fetch_one(&mut *conn)
            .await?;
    if searchable {
        return Err(Unsupported("indexing searchable user fields").into());
    }
    let name = name.map(str::to_lowercase).unwrap_or_default();
    let weights = [Some(username_lower), Some(name.as_str()), Some(""), None];
    let (search_data, prepared) = tsvector(conn, settings, Some("simple"), &weights).await?;
    let raw_data = prepared
        .iter()
        .filter(|d| !d.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    sqlx::query(
        "INSERT INTO user_search_data (user_id, raw_data, locale, version, search_data) \
         VALUES ($1, $2, $3, $4, $5::tsvector) \
         ON CONFLICT (user_id) DO UPDATE SET raw_data = EXCLUDED.raw_data, locale = EXCLUDED.locale, \
           version = EXCLUDED.version, search_data = EXCLUDED.search_data",
    )
    .bind(user_id)
    .bind(&raw_data)
    .bind(settings.get("default_locale")?.to_s())
    .bind(USER_INDEX_VERSION)
    .bind(&search_data)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

fn check_unported(settings: &SiteSettings) -> Result<(), AppError> {
    if settings.get("search_ignore_accents")?.truthy() {
        return Err(Unsupported("search_ignore_accents (unaccent) when indexing").into());
    }
    let locale = settings.get("default_locale")?.to_s();
    if settings.get("search_tokenize_chinese")?.truthy()
        || settings.get("search_tokenize_japanese")?.truthy()
        || locale.starts_with("zh")
        || locale.starts_with("ja")
    {
        return Err(Unsupported("CJK segmentation when indexing").into());
    }
    Ok(())
}

static URL_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^"?((https?://)\S+?)"?$"#).unwrap());

/// `Search.prepare_data(data, :index)` for a non-CJK site: the term
/// cleaned and squished, URLs without their query.
fn prepare_data(data: &str) -> String {
    let squished = squish(&clean_term(data));
    squished
        .split(' ')
        .map(|word| match URL_TOKEN.captures(word) {
            Some(c) => strip_query(&c[1]).unwrap_or_else(|| word.to_string()),
            None => word.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `uri = URI.parse(url); uri.query = nil; uri.to_s`, or None where
/// URI.parse would raise.
fn strip_query(url: &str) -> Option<String> {
    let valid = url
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(c));
    if !valid {
        return None;
    }
    match url.find('?') {
        None => Some(url.to_string()),
        Some(q) => {
            let fragment = url[q..].find('#').map(|f| &url[q + f..]).unwrap_or("");
            Some(format!("{}{fragment}", &url[..q]))
        }
    }
}

/// ActiveSupport's `squish`.
pub fn squish(s: &str) -> String {
    s.split(|c: char| c.is_whitespace())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

static DOTTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"'(([a-zA-Z0-9]+\.)+[a-zA-Z0-9]+)':([A-Za-z0-9_+,]+)").unwrap());
static VERSION_LIKE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d+\.)?(\d+\.)*(\*|\d+)$").unwrap());
/// `TS_VECTOR_PARSE_REGEX`
static TS_VECTOR_PARSE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"('(?:[^']*|'')*':)((?:[0-9]+[A-D]?,?)+)").unwrap());

/// `update_index`'s tsvector for weights A to D, and the prepared data.
async fn tsvector(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    stemmer: Option<&str>,
    weights: &[Option<&str>; 4],
) -> Result<(String, Vec<String>), AppError> {
    let stemmer = match stemmer {
        Some(s) => s,
        None => ts_config(&settings.get("default_locale")?.to_s()),
    };
    let max_word = settings
        .get("search_max_indexed_word_length")?
        .to_i()
        .max(0) as usize;
    let prepared: Vec<String> = weights
        .iter()
        .map(|w| prepare_data(w.unwrap_or("")))
        .collect();
    let indexed: Vec<String> = prepared
        .iter()
        .map(|d| {
            d.split(' ')
                .map(|w| w.chars().take(max_word).collect::<String>())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    let sql = format!(
        "SELECT (setweight(to_tsvector('{stemmer}', coalesce($1,'')), 'A') || \
                 setweight(to_tsvector('{stemmer}', coalesce($2,'')), 'B') || \
                 setweight(to_tsvector('{stemmer}', coalesce($3,'')), 'C') || \
                 setweight(to_tsvector('{stemmer}', coalesce($4,'')), 'D'))::text"
    );
    let base: String = sqlx::query_scalar(&sql)
        .bind(&indexed[0])
        .bind(&indexed[1])
        .bind(&indexed[2])
        .bind(&indexed[3])
        .fetch_one(&mut *conn)
        .await?;

    let mut additional_lexemes = Vec::new();
    let mut additional_words: Vec<(String, String)> = Vec::new();
    for c in DOTTED.captures_iter(&base) {
        let mut lexeme = c[1].to_string();
        let positions = c[3].to_string();
        if VERSION_LIKE.is_match(&lexeme) {
            continue;
        }
        for _ in 0..9 {
            let Some((term, remaining)) = lexeme.split_once('.') else {
                break;
            };
            if remaining.is_empty() {
                break;
            }
            additional_words.push((term.to_string(), positions.clone()));
            additional_lexemes.push(format!("'{remaining}':{positions}"));
            lexeme = remaining.to_string();
        }
    }
    let extra = if additional_words.is_empty() {
        String::new()
    } else {
        let words = additional_words
            .iter()
            .map(|(t, _)| t.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let vector: String =
            sqlx::query_scalar(&format!("SELECT to_tsvector('{stemmer}', $1)::text"))
                .bind(&words)
                .fetch_one(&mut *conn)
                .await?;
        TS_VECTOR_PARSE
            .captures_iter(&vector)
            .map(|c| {
                let indexes = c[2]
                    .split(',')
                    .map(|index| {
                        let n: usize = index
                            .trim_end_matches(|ch: char| ch.is_ascii_alphabetic())
                            .parse()
                            .unwrap_or(0);
                        match n.checked_sub(1).and_then(|i| additional_words.get(i)) {
                            Some((_, positions)) => positions.clone(),
                            None => index.to_string(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                format!("{}{indexes}", &c[1])
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut vector = format!("{base} {} {extra}", additional_lexemes.join(" "));

    let max_dupes = settings.get("max_duplicate_search_index_terms")?.to_i();
    if max_dupes > 0 {
        let reduced: Vec<String> = TS_VECTOR_PARSE
            .captures_iter(&vector)
            .map(|c| {
                let mut counts: std::collections::HashMap<Option<char>, i64> = Default::default();
                let kept: Vec<&str> = c[2]
                    .split(',')
                    .filter(|index| !index.is_empty())
                    .filter(|index| {
                        let family = index.chars().last().filter(|ch| ('A'..='D').contains(ch));
                        let count = counts.entry(family).or_insert(0);
                        *count += 1;
                        *count <= if family == Some('A') { 1 } else { max_dupes }
                    })
                    .collect();
                format!("{}{}", c[1].trim(), kept.join(","))
            })
            .collect();
        vector = reduced.join(" ");
    }
    Ok((vector, prepared))
}

static MEDIA_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)https?://\S+\.(mov|mp4|webm|m4v|3gp|ogv|avi|mpeg|ogg|mp3|wav|m4a|oga|opus|aac|flac)\b").unwrap()
});

/// `clean_post_raw_data!`: links to video or audio files are indexed as a
/// word for the kind of media; not ported.
fn clean_post_raw_data(raw: &str) -> Result<String, AppError> {
    if MEDIA_URL.is_match(raw) {
        return Err(Unsupported("indexing posts linking to video or audio files").into());
    }
    Ok(raw.to_string())
}

/// What `UrlHelper.is_local` compares against.
pub struct BaseUrls<'a> {
    pub base_path: &'a str,
    pub base_url_no_prefix: String,
}

impl BaseUrls<'_> {
    /// `UrlHelper.is_local`: an upload, a site asset, or anything on the
    /// site's own origin.
    fn is_local(&self, url: &str) -> bool {
        let bp = self.base_path;
        !url.is_empty()
            && (url.starts_with(&format!("{bp}/uploads/default"))
                || url.starts_with(&format!("{}{bp}/uploads/default", self.base_url_no_prefix))
                || ["assets", "plugins", "images"]
                    .iter()
                    .any(|d| url.starts_with(&format!("{bp}/{d}/")))
                || url.starts_with(&self.base_url_no_prefix))
    }
}

const SCRUB_ATTRIBUTES: [&str; 4] = ["alt", "title", "href", "data-video-title"];

/// `SearchIndexer::HtmlScrubber.scrub(cooked)`: the text and the telling
/// attributes, in document order.
pub fn scrub(html: &str, base: &BaseUrls<'_>) -> String {
    if html.trim().is_empty() {
        return String::new();
    }
    let dom = parse_document(&format!("<div>{html}</div>"));
    let mut out = String::new();
    walk(&dom.document, base, false, &mut out);
    squish(&out)
}

fn walk(node: &Handle, base: &BaseUrls<'_>, in_lightbox: bool, out: &mut String) {
    match &node.data {
        NodeData::Text { contents } => {
            if !in_lightbox {
                out.push_str(&format!(" {} ", contents.borrow()));
            }
        }
        NodeData::Element { .. } => {
            let name = element_name(node).unwrap_or("");
            // Inside a lightbox wrapper only links (without their text
            // attributes) and images survive.
            if in_lightbox && name != "a" && name != "img" {
                return;
            }
            let class = attr(node, "class");
            for attribute in SCRUB_ATTRIBUTES {
                let Some(value) = attr(node, attribute).filter(|v| !v.is_empty()) else {
                    continue;
                };
                if name == "img" && attribute == "alt" && has_class(node, "emoji") {
                    continue;
                }
                if name == "a" && in_lightbox {
                    continue;
                }
                if name == "a" && attribute == "href" {
                    let text = crate::pretty_text::cleanup::text(node);
                    let mention =
                        matches!(class.as_deref(), Some("mention") | Some("mention-group"));
                    let anchor = class.as_deref() == Some("anchor") && value.starts_with('#');
                    if value == text || mention || anchor {
                        continue;
                    }
                }
                if attribute == "href" && base.is_local(&value) {
                    continue;
                }
                out.push_str(&format!(" {value} "));
            }
            let lightbox = in_lightbox || (name == "div" && has_class(node, "lightbox-wrapper"));
            for child in node.children.borrow().iter() {
                walk(child, base, lightbox, out);
            }
        }
        _ => {
            for child in node.children.borrow().iter() {
                walk(child, base, in_lightbox, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> BaseUrls<'static> {
        BaseUrls {
            base_path: "",
            base_url_no_prefix: "http://localhost:3000".into(),
        }
    }

    #[test]
    fn scrubs_like_the_recorded_reply() {
        let cooked = r##"<h2><a name="a-heading-1" class="anchor" href="#a-heading-1" aria-label="Heading link"></a>A heading</h2>
<p>With <strong>bold</strong>, a mention of <a class="mention" href="/u/user0">@user0</a>, <a class="hashtag-cooked" href="/c/general/4" data-type="category" data-slug="general" data-id="4" data-style-type="emoji" data-emoji="blue_book"><span class="hashtag-icon-placeholder"><svg class="fa d-icon d-icon-square-full svg-icon svg-node"><use href="#square-full"></use></svg></span><span>General</span></a> and <img src="/images/emoji/twitter/smile.png?v=15" title=":smile:" class="emoji" alt=":smile:" loading="lazy" width="20" height="20">, and a <a href="https://example.com/page" rel="noopener nofollow ugc">link</a>.</p>"##;
        assert_eq!(
            scrub(cooked, &base()),
            "A heading With bold , a mention of @user0 , /c/general/4 #square-full General and :smile: , and a https://example.com/page link ."
        );
    }

    #[test]
    fn urls_lose_their_query() {
        assert_eq!(
            prepare_data("see https://x.com/a?b=1#c now"),
            "see https://x.com/a#c now"
        );
    }
}
