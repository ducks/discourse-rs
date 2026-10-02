//! `TopicLink.extract_from(post)` (app/models/topic_link.rb) with
//! `PrettyText.extract_links`: a row per distinct link in the cooked post.
//! Links to topics (and the reflections they create), uploads and video
//! embeds are refused; Rails' router decides what counts as internal, and
//! only the site paths listed here are recognized.

use markup5ever_rcdom::Handle;
use sqlx::PgConnection;

use crate::pretty_text::cleanup::{attr, element_name, fragment_root, has_class, parse};
use crate::{AppError, Unsupported};

/// `TopicLink.max_url_length`
const MAX_URL_LENGTH: usize = 500;
/// `TopicLink.max_domain_length`
const MAX_DOMAIN_LENGTH: usize = 100;

/// The post whose links are extracted.
pub struct LinkPost<'a> {
    pub id: i32,
    pub user_id: i32,
    pub topic_id: i32,
    pub cooked: &'a str,
}

/// Where the site lives, for telling internal links apart.
pub struct Site<'a> {
    pub hostname: &'a str,
    pub base_path: &'a str,
}

/// `PrettyText.extract_links(html)`: hrefs outside quotes, oneboxes,
/// elided parts and image lightboxes.
fn extract(cooked: &str) -> Result<Vec<String>, Unsupported> {
    let dom = parse(cooked);
    let mut links = Vec::new();
    collect(&fragment_root(&dom), false, &mut links)?;
    Ok(links)
}

fn collect(node: &Handle, skip: bool, links: &mut Vec<String>) -> Result<(), Unsupported> {
    let name = element_name(node);
    let mut skip = skip;
    if let Some(name) = name {
        if name == "aside" && has_class(node, "quote") && attr(node, "data-topic").is_some() {
            return Err(Unsupported(
                "links from quotes (topic links and QuotedPost)",
            ));
        }
        if name == "aside" && has_class(node, "onebox") {
            return Err(Unsupported("links from oneboxes"));
        }
        if name == "div" && attr(node, "data-video-id").is_some() {
            return Err(Unsupported("links from video embeds"));
        }
        if (name == "aside" && has_class(node, "quote")) || has_class(node, "elided") {
            skip = true;
        }
        if name == "a" && !skip {
            let lightbox_image = (has_class(node, "lightbox") || has_class(node, "onebox"))
                && node
                    .children
                    .borrow()
                    .iter()
                    .any(|c| element_name(c) == Some("img"));
            if !lightbox_image
                && let Some(href) =
                    attr(node, "href").filter(|h| !h.is_empty() && !h.starts_with('#'))
            {
                links.push(href);
            }
        }
    }
    for child in node.children.borrow().iter() {
        collect(child, skip, links)?;
    }
    Ok(())
}

/// The parts of a URL TopicLink reads.
#[derive(Debug, PartialEq)]
struct Parsed<'a> {
    scheme: Option<&'a str>,
    host: Option<&'a str>,
    path: &'a str,
    query: Option<&'a str>,
}

/// A permissive split in place of `UrlHelper.relaxed_parse`.
fn parse_url(url: &str) -> Option<Parsed<'_>> {
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let (scheme, rest) = match url.find(':') {
        Some(i)
            if url[..i]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
                && url[..i]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic())
                && !url[..i].contains('/') =>
        {
            (Some(&url[..i]), &url[i + 1..])
        }
        _ => (None, url),
    };
    let rest = rest.split('#').next().unwrap_or("");
    let (rest, query) = match rest.split_once('?') {
        Some((r, q)) => (r, Some(q)),
        None => (rest, None),
    };
    let (host, path) = match rest.strip_prefix("//") {
        Some(authority_and_path) => {
            let (authority, path) = match authority_and_path.find('/') {
                Some(i) => (&authority_and_path[..i], &authority_and_path[i..]),
                None => (authority_and_path, ""),
            };
            let host = authority.rsplit('@').next().unwrap_or("");
            let host = host.split(':').next().unwrap_or("");
            (Some(host).filter(|h| !h.is_empty()), path)
        }
        None => (None, rest),
    };
    Some(Parsed {
        scheme,
        host,
        path,
        query,
    })
}

/// Site paths Rails' router recognizes, the users ones apart.
const RECOGNIZED: [&str; 16] = [
    "/c/",
    "/tag/",
    "/tags",
    "/latest",
    "/top",
    "/new",
    "/unread",
    "/categories",
    "/search",
    "/g/",
    "/badges",
    "/about",
    "/faq",
    "/guidelines",
    "/tos",
    "/privacy",
];

/// `File.extname(path)[1..10].downcase`, None without an extension.
fn extension(path: &str) -> Option<String> {
    let base = path.rsplit('/').next().unwrap_or("");
    let trimmed = base.trim_start_matches('.');
    let dot = trimmed.rfind('.')?;
    let ext: String = trimmed[dot + 1..].chars().take(10).collect();
    Some(ext.to_lowercase())
}

/// `TopicLink.extract_from(post)` followed by `cleanup_entries`.
pub async fn extract_from(
    conn: &mut PgConnection,
    site: &Site<'_>,
    post: &LinkPost<'_>,
) -> Result<(), AppError> {
    let mut seen: Vec<String> = Vec::new();
    let mut current_urls: Vec<String> = Vec::new();
    for url in extract(post.cooked)? {
        let Some(parsed) = parse_url(&url) else {
            continue;
        };
        if parsed.scheme == Some("mailto") || seen.contains(&url) {
            continue;
        }
        seen.push(url.clone());
        if url.contains("/uploads/") {
            return Err(Unsupported("links to uploads").into());
        }
        let bp = site.base_path;
        let on_site = parsed.host.is_none()
            || (parsed.host == Some(site.hostname) && parsed.path.starts_with(bp));
        let mut internal = false;
        if on_site {
            let path = parsed.path.strip_prefix(bp).unwrap_or(parsed.path);
            if parsed
                .query
                .is_some_and(|q| q.split('&').any(|p| p == "silent=true"))
            {
                continue;
            }
            if path.starts_with("/u/") || path.starts_with("/users/") || path == "/u" {
                continue;
            }
            if path.starts_with("/t/") {
                return Err(Unsupported("links to topics (link targets and reflections)").into());
            }
            if !RECOGNIZED.iter().any(|p| path.starts_with(p)) {
                return Err(Unsupported("links to site paths the router is not ported for").into());
            }
            internal = true;
        }
        if parsed.host.is_some_and(|h| h.len() > MAX_DOMAIN_LENGTH) {
            continue;
        }
        let url: String = url.chars().take(MAX_URL_LENGTH).collect();
        let domain = parsed.host.unwrap_or(site.hostname);
        sqlx::query(
            "INSERT INTO topic_links (post_id, user_id, topic_id, url, domain, internal, link_topic_id, \
                                      link_post_id, quote, extension, reflection, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, NULL, NULL, FALSE, $7, FALSE, clock_timestamp(), clock_timestamp()) \
             ON CONFLICT DO NOTHING",
        )
        .bind(post.id)
        .bind(post.user_id)
        .bind(post.topic_id)
        .bind(&url)
        .bind(domain)
        .bind(internal)
        .bind(extension(parsed.path))
        .execute(&mut *conn)
        .await?;
        current_urls.push(url);
    }
    // cleanup_entries: no reflections are made, so all of the post's go.
    if current_urls.is_empty() {
        sqlx::query(
            "DELETE FROM topic_links WHERE (post_id = $1 AND NOT reflection) \
             OR (link_post_id = $1 AND reflection)",
        )
        .bind(post.id)
        .execute(&mut *conn)
        .await?;
    } else {
        sqlx::query(
            "DELETE FROM topic_links WHERE NOT (url = ANY($2)) AND post_id = $1 AND NOT reflection",
        )
        .bind(post.id)
        .bind(&current_urls)
        .execute(&mut *conn)
        .await?;
        sqlx::query("DELETE FROM topic_links WHERE link_post_id = $1 AND reflection")
            .bind(post.id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_links() {
        let p = parse_url("https://example.com/page?a=1#x").unwrap();
        assert_eq!(p.host, Some("example.com"));
        assert_eq!(p.path, "/page");
        assert_eq!(p.query, Some("a=1"));
        let p = parse_url("/c/general/4").unwrap();
        assert_eq!(p.host, None);
        assert_eq!(p.path, "/c/general/4");
        assert_eq!(parse_url("mailto:a@b.c").unwrap().scheme, Some("mailto"));
    }

    #[test]
    fn extensions() {
        assert_eq!(extension("/page"), None);
        assert_eq!(extension("/a/file.PNG"), Some("png".into()));
        assert_eq!(extension("/a/file."), Some(String::new()));
    }
}
