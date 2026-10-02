//! The `cooked` column of a post: `Post#cook` (PrettyText.cook with the
//! post's own options), then what `Jobs::ProcessPost` writes after
//! `CookedPostProcessor#post_process` (lib/cooked_post_processor.rb).
//!
//! Only the steps that change the html and need nothing outside the
//! database are ported: quotes marked as missing or modified, local urls
//! made absolute (`optimize_urls`), the `u` parameter taken off links to
//! the site (`remove_user_ids`), and `enforce_nofollow`. The processor
//! also writes (the post's and topic's image, badges, links to uploads,
//! thumbnails) and those are not part of the column.
//!
//! Refused: oneboxes (fetched from the network), images other than emoji
//! (sized from the upload or fetched, optimized, given lightboxes), video
//! placeholders of uploads (optimized videos), posts cooked from email,
//! secure uploads.

use sqlx::PgConnection;

use super::cleanup::{
    add_rel_attributes, all_elements, attr, dom_text, element_name, has_class, parse, rel_settings,
    set_attr, text, to_html, uri_host,
};
use super::{CookError, Host, MarkdownOptions, cook};
use crate::Unsupported;
use crate::config::Config;
use crate::site_settings::SiteSettings;
use crate::url::Urls;

/// `Post.cook_methods`
const COOK_REGULAR: i32 = 1;
const COOK_RAW_HTML: i32 = 2;

/// `QuoteComparer.whitespace`
fn without_whitespace(s: &str) -> String {
    s.chars().filter(|c| !" \t\r\n".contains(*c)).collect()
}

/// `post_process_quotes`: a quote of a post that is gone, or whose text
/// that post no longer has, is marked.
async fn mark_quotes(
    conn: &mut PgConnection,
    html: &str,
) -> Result<Vec<(usize, &'static str)>, CookError> {
    // Collected first: the document cannot be held across a query.
    let quotes: Vec<(usize, i32, i32, String)> = {
        let dom = parse(html);
        all_elements(&dom)
            .iter()
            .filter(|e| element_name(e) == Some("aside") && has_class(e, "quote"))
            .enumerate()
            .filter_map(|(i, q)| {
                let topic = attr(q, "data-topic")?;
                let post = attr(q, "data-post")?;
                // `css("blockquote").text`: every blockquote inside.
                let mut quoted = String::new();
                fn blockquotes(node: &markup5ever_rcdom::Handle, out: &mut String) {
                    for child in node.children.borrow().iter() {
                        if element_name(child) == Some("blockquote") {
                            out.push_str(&text(child));
                        }
                        blockquotes(child, out);
                    }
                }
                blockquotes(q, &mut quoted);
                Some((
                    i,
                    crate::ruby::to_i(&topic) as i32,
                    crate::ruby::to_i(&post) as i32,
                    quoted,
                ))
            })
            .collect()
    };
    let mut marks = Vec::new();
    for (index, topic_id, post_number, quoted) in quotes {
        let parent: Option<String> = sqlx::query_scalar(
            "SELECT cooked FROM posts WHERE topic_id = $1 AND post_number = $2 \
             AND deleted_at IS NULL ORDER BY id LIMIT 1",
        )
        .bind(topic_id)
        .bind(post_number)
        .fetch_optional(&mut *conn)
        .await?;
        match parent {
            None => marks.push((index, "quote-post-not-found")),
            Some(cooked) => {
                let quoted = without_whitespace(&quoted);
                let parent_text = without_whitespace(&dom_text(&parse(&cooked)));
                if quoted.is_empty() || !parent_text.contains(&quoted) {
                    marks.push((index, "quote-modified"));
                }
            }
        }
    }
    Ok(marks)
}

/// `UrlHelper.cook_url` without a CDN or secure uploads: a local url made
/// absolute and schemaless, any other left alone.
fn cook_url(url: &str, urls: &Urls<'_>, base_path: &str) -> Result<String, CookError> {
    let base_no_prefix = urls.base_url_no_prefix()?;
    // FileStore::LocalStore#has_been_uploaded?
    let uploads = format!("{base_path}/uploads/default");
    let scheme = urls.scheme()?;
    let absolute_form = match url.strip_prefix("//") {
        Some(rest) => format!("{scheme}://{rest}"),
        None => url.to_string(),
    };
    let uploaded = url.starts_with(&uploads)
        || absolute_form.starts_with(&format!("{base_no_prefix}{uploads}"));
    let assets = ["assets", "plugins", "images"]
        .iter()
        .any(|dir| url.starts_with(&format!("{base_path}/{dir}/")));
    let local = !url.is_empty() && (uploaded || assets || url.starts_with(&base_no_prefix));
    if !local {
        return Ok(url.to_string());
    }
    // absolute_without_cdn, then schemaless.
    let absolute = if url.starts_with('/') && !url.starts_with("//") && url.len() > 1 {
        format!("{base_no_prefix}{url}")
    } else {
        url.to_string()
    };
    Ok(match absolute.get(..5) {
        Some(head) if head.eq_ignore_ascii_case("http:") => absolute[5..].to_string(),
        _ => absolute,
    })
}

/// `Rack::Utils.escape`
fn rack_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b' ' => out.push('+'),
            b if b.is_ascii_alphanumeric() || b"*-._".contains(&b) => out.push(b as char),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `remove_user_ids` for one href: the `u` parameter taken off a link to
/// the site, the query rebuilt as Rack does. None leaves the href alone.
fn without_user_id(href: &str, hostname: &str) -> Result<Option<String>, CookError> {
    let Ok(Some(host)) = uri_host(href) else {
        return Ok(None);
    };
    if host != hostname {
        return Ok(None);
    }
    let (before_fragment, fragment) = match href.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (href, None),
    };
    let Some((base, query)) = before_fragment.split_once('?') else {
        return Ok(None);
    };
    let pairs: Vec<(String, String)> = query
        .split(['&', ';'])
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            let decode = |s: &str| {
                mdurl::urlencode::decode(&s.replace('+', " "), mdurl::urlencode::AsciiSet::new())
                    .into_owned()
            };
            (decode(k), decode(v))
        })
        .collect();
    if !pairs.iter().any(|(k, _)| k == "u") {
        return Ok(None);
    }
    if pairs.iter().any(|(k, _)| k.contains(['[', ']'])) {
        return Err(Unsupported("nested query parameters in a link with a user id").into());
    }
    // parse_nested_query keeps the last value of a repeated key.
    let mut kept: Vec<(String, String)> = Vec::new();
    for (k, v) in pairs.into_iter().filter(|(k, _)| k != "u") {
        match kept.iter_mut().find(|(key, _)| *key == k) {
            Some(entry) => entry.1 = v,
            None => kept.push((k, v)),
        }
    }
    let query: Vec<String> = kept
        .iter()
        .map(|(k, v)| format!("{}={}", rack_escape(k), rack_escape(v)))
        .collect();
    let mut out = base.to_string();
    if !query.is_empty() {
        out.push('?');
        out.push_str(&query.join("&"));
    }
    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(fragment);
    }
    Ok(Some(out))
}

/// `post_process` on `Post#cook`'s html: what the job writes.
pub(crate) async fn post_process(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    config: &Config,
    html: &str,
    omit_nofollow: bool,
) -> Result<String, CookError> {
    if settings.get("secure_uploads")?.truthy() {
        return Err(Unsupported("secure uploads in the post processor").into());
    }
    if config.globals.cdn_url().is_some() || config.globals.s3_cdn_url().is_some() {
        return Err(Unsupported("a CDN in the post processor").into());
    }
    {
        let dom = parse(html);
        for e in all_elements(&dom) {
            let name = element_name(&e).unwrap_or_default();
            if has_class(&e, "onebox") || has_class(&e, "inline-onebox-loading") {
                return Err(Unsupported("oneboxes (fetched from the network)").into());
            }
            if name == "img"
                && attr(&e, "src").is_some_and(|s| !s.starts_with("data"))
                && !has_class(&e, "emoji")
            {
                return Err(Unsupported(
                    "images in the post processor (sizes, optimized images, lightboxes)",
                )
                .into());
            }
            if has_class(&e, "video-placeholder-container")
                && attr(&e, "data-video-src").is_some_and(|s| s != "/404")
            {
                return Err(Unsupported("videos in the post processor (optimized videos)").into());
            }
        }
    }
    let marks = mark_quotes(conn, html).await?;

    let urls = Urls { config, settings };
    let base_path = config.globals.relative_url_root();
    let hostname = urls.current_hostname()?;
    let add_nofollow = !omit_nofollow && settings.get("add_rel_nofollow_to_user_content")?.truthy();
    let (site_host, allowlist) = rel_settings(settings, config)?;

    let dom = parse(html);
    let quotes: Vec<_> = all_elements(&dom)
        .into_iter()
        .filter(|e| element_name(e) == Some("aside") && has_class(e, "quote"))
        .collect();
    for (index, mark) in marks {
        if let Some(q) = quotes.get(index) {
            let class = format!("{} {mark}", attr(q, "class").unwrap_or_default());
            set_attr(q, "class", class.trim());
        }
    }
    for e in all_elements(&dom) {
        match element_name(&e) {
            Some("a") => {
                for name in ["href", "data-download-href"] {
                    if let Some(value) = attr(&e, name) {
                        set_attr(&e, name, &cook_url(&value, &urls, base_path)?);
                    }
                }
            }
            Some("img") | Some("video") => {
                if let Some(src) = attr(&e, "src") {
                    set_attr(&e, "src", &cook_url(&src, &urls, base_path)?);
                }
            }
            _ => {}
        }
    }
    for a in all_elements(&dom) {
        if element_name(&a) != Some("a") {
            continue;
        }
        if let Some(href) = attr(&a, "href")
            && let Some(cleaned) = without_user_id(&href, &hostname)?
        {
            set_attr(&a, "href", &cleaned);
        }
    }
    add_rel_attributes(&dom, add_nofollow, &site_host, &allowlist);
    Ok(to_html(&dom))
}

/// The post as cooking reads it.
#[derive(sqlx::FromRow)]
struct PostRow {
    raw: String,
    topic_id: i32,
    last_editor_id: Option<i32>,
    cook_method: i32,
    /// The topic exists and is not deleted.
    topic_present: bool,
    /// `Post#add_nofollow?` turned around.
    omit_nofollow: bool,
}

/// The `cooked` column for a post, as creating it (or rebaking it) and the
/// post processor leave it.
pub async fn cooked_column(host: &Host, post_id: i64) -> Result<String, CookError> {
    let mut conn = host.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &host.site_setting_defs, &host.config.globals).await?;
    let tl3_no_follow = settings.get("tl3_links_no_follow")?.truthy();
    // add_nofollow?: false for staff, else true without a user, with
    // tl3_links_no_follow, or below trust level 3 (staged counts as 3).
    let post: Option<PostRow> = sqlx::query_as(
        "SELECT p.raw, p.topic_id, p.last_editor_id, p.cook_method, \
                (t.id IS NOT NULL) AS topic_present, \
                COALESCE(u.admin OR u.moderator \
                    OR (NOT $2 AND (u.staged OR u.trust_level >= 3)), FALSE) AS omit_nofollow \
         FROM posts p \
         LEFT JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL \
         LEFT JOIN users u ON u.id = p.user_id \
         WHERE p.id = $1",
    )
    .bind(post_id)
    .bind(tl3_no_follow)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(post) = post else {
        return Err(CookError::Db(sqlx::Error::RowNotFound));
    };
    match post.cook_method {
        COOK_RAW_HTML => return Ok(post.raw),
        COOK_REGULAR => {}
        _ => return Err(Unsupported("posts cooked from email").into()),
    }
    drop(conn);

    let opts = MarkdownOptions {
        topic_id: Some(i64::from(post.topic_id)),
        post_id: Some(post_id),
        user_id: post.last_editor_id.map(i64::from),
        force_quote_link: false,
        omit_nofollow: post.omit_nofollow,
    };
    let cooked = cook(host, &post.raw, &opts).await?;
    // Jobs::ProcessPost does nothing for a post whose topic is gone.
    if !post.topic_present {
        return Ok(cooked);
    }
    let mut conn = host.pool.acquire().await?;
    post_process(
        &mut conn,
        &settings,
        &host.config,
        &cooked,
        post.omit_nofollow,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_ids_come_off_links_to_the_site() {
        let strip = |href: &str| without_user_id(href, "localhost").unwrap();
        assert_eq!(
            strip("http://localhost:3000/t/x/1?u=bob"),
            Some("http://localhost:3000/t/x/1".to_string())
        );
        assert_eq!(
            strip("http://localhost/t/x/1?a=1&u=bob#p"),
            Some("http://localhost/t/x/1?a=1#p".to_string())
        );
        assert_eq!(strip("https://example.com/?u=bob"), None);
        assert_eq!(strip("/t/x/1?u=bob"), None);
    }
}
