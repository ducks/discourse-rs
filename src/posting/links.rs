//! `TopicLink.extract_from(post)` (app/models/topic_link.rb) with
//! `PrettyText.extract_links`: a row per distinct link in the cooked post,
//! links to topics with their target and the reflection they leave in the
//! linked topic. Uploads and video embeds are refused; Rails' router
//! decides what counts as internal, and only the site paths listed here
//! (and topic pages) are recognized.

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
    /// `Discourse.base_url_no_prefix`, for topic URLs and reflections.
    pub base_url_no_prefix: &'a str,
    /// For whether the author can see a linked topic.
    pub settings: &'a crate::site_settings::SiteSettings,
}

/// A topic page's route parts (`topics#show`).
struct TopicRoute {
    topic_id: Option<i32>,
    slug: Option<String>,
    post_number: Option<i32>,
}

fn topic_route(rest: &str) -> Result<TopicRoute, Unsupported> {
    let digits = |s: &str| {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<i32>().ok())
            .flatten()
    };
    let segments: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
    let (topic_id, slug, post_number) = match segments.as_slice() {
        // t/:id, a topic id or a slug.
        [id] => (digits(id), Some((*id).to_string()), None),
        // t/:slug/:topic_id
        [slug, id] if digits(slug).is_none() && digits(id).is_some() => {
            (digits(id), Some((*slug).to_string()), None)
        }
        // t/:slug/:topic_id/:post_number
        [slug, id, n] if digits(id).is_some() && digits(n).is_some() => {
            (digits(id), Some((*slug).to_string()), digits(n))
        }
        _ => {
            return Err(Unsupported(
                "links to topic paths the router is not ported for",
            ));
        }
    };
    Ok(TopicRoute {
        topic_id,
        slug,
        post_number,
    })
}

/// What `ensure_entry_for` finds behind a link to a topic.
struct TopicTarget {
    id: i32,
    /// The canonical URL, when the author can see the topic.
    url: Option<String>,
    post_id: Option<i32>,
    private_message: bool,
}

/// The topic a link points to, if it exists.
async fn topic_target(
    conn: &mut PgConnection,
    site: &Site<'_>,
    post: &LinkPost<'_>,
    rest: &str,
) -> Result<Option<TopicTarget>, AppError> {
    let TopicRoute {
        topic_id,
        slug,
        post_number,
    } = topic_route(rest)?;
    let post_number = post_number.unwrap_or(1);
    let mut topic: Option<(i32, Option<String>, String)> = match topic_id {
        Some(id) => {
            sqlx::query_as(
                "SELECT id, slug, archetype FROM topics WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?
        }
        None => None,
    };
    if topic.is_none()
        && let Some(slug) = slug.filter(|s| !s.is_empty())
    {
        topic = sqlx::query_as(
            "SELECT id, slug, archetype FROM topics WHERE slug = $1 AND deleted_at IS NULL \
             ORDER BY id LIMIT 1",
        )
        .bind(slug)
        .fetch_optional(&mut *conn)
        .await?;
    }
    let Some((id, slug, archetype)) = topic else {
        return Ok(None);
    };
    // guardian.can_see?(topic) for the post's acting user: its author.
    let user: crate::session::current::SessionUser = sqlx::query_as(&format!(
        "SELECT {} FROM users WHERE id = $1",
        crate::session::current::SESSION_USER_COLUMNS
    ))
    .bind(post.user_id)
    .fetch_one(&mut *conn)
    .await?;
    let guardian = crate::guardian::Guardian::for_user(&mut *conn, &user).await?;
    let visible =
        match crate::topic_guardian::TopicCtx::load(&mut *conn, site.settings, &guardian, id)
            .await?
        {
            Some(t) => {
                let secure = guardian
                    .secure_category_ids(&mut *conn, site.settings)
                    .await?;
                guardian.can_see_topic(site.settings, &t, true, &secure)?
            }
            None => false,
        };
    // topic.relative_url, with the post number past the first.
    let url = visible.then(|| {
        let mut url = format!("{}{}/t/", site.base_url_no_prefix, site.base_path);
        if let Some(slug) = slug.as_deref().filter(|s| !s.is_empty()) {
            url.push_str(slug);
            url.push('/');
        }
        url.push_str(&id.to_string());
        if post_number > 1 {
            url.push_str(&format!("/{post_number}"));
        }
        url
    });
    let post_id: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM posts WHERE topic_id = $1 AND post_number = $2 AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(post_number)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(Some(TopicTarget {
        id,
        url,
        post_id,
        private_message: archetype == "private_message",
    }))
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

/// A topic_links row as `safe_create_topic_link` takes it.
struct NewLink<'a> {
    post_id: Option<i32>,
    user_id: i32,
    topic_id: i32,
    url: &'a str,
    domain: &'a str,
    internal: bool,
    link_topic_id: Option<i32>,
    link_post_id: Option<i32>,
    extension: Option<String>,
    reflection: bool,
}

/// `TopicLink.safe_create_topic_link`: inserted unless it exists; the id
/// of the row either way, and whether it is new (its title then crawled
/// once the post commits).
async fn safe_create_topic_link(
    conn: &mut PgConnection,
    link: &NewLink<'_>,
) -> Result<(Option<i32>, bool), AppError> {
    let created: Option<i32> = sqlx::query_scalar(
        "INSERT INTO topic_links (post_id, user_id, topic_id, url, domain, internal, link_topic_id, \
                                  link_post_id, quote, extension, reflection, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, FALSE, $9, $10, clock_timestamp(), clock_timestamp()) \
         ON CONFLICT DO NOTHING RETURNING id",
    )
    .bind(link.post_id)
    .bind(link.user_id)
    .bind(link.topic_id)
    .bind(link.url)
    .bind(link.domain)
    .bind(link.internal)
    .bind(link.link_topic_id)
    .bind(link.link_post_id)
    .bind(&link.extension)
    .bind(link.reflection)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = created {
        crate::jobs::enqueue(
            &mut *conn,
            "crawl_topic_link",
            serde_json::json!({ "topic_link_id": id }),
        )
        .await?;
        return Ok((Some(id), true));
    }
    let existing: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM topic_links WHERE post_id IS NOT DISTINCT FROM $1 AND topic_id = $2 AND url = $3",
    )
    .bind(link.post_id)
    .bind(link.topic_id)
    .bind(link.url)
    .fetch_optional(&mut *conn)
    .await?;
    Ok((existing, false))
}

/// `TopicLink.extract_from(post)` followed by `cleanup_entries`.
pub async fn extract_from(
    conn: &mut PgConnection,
    site: &Site<'_>,
    post: &LinkPost<'_>,
) -> Result<(), AppError> {
    let mut seen: Vec<String> = Vec::new();
    let mut current_urls: Vec<String> = Vec::new();
    let mut reflected_ids: Vec<i32> = Vec::new();
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
        let mut target: Option<TopicTarget> = None;
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
            if let Some(rest) = path.strip_prefix("/t/") {
                target = topic_target(&mut *conn, site, post, rest).await?;
                // Skip linking to ourselves.
                if target.as_ref().is_some_and(|t| t.id == post.topic_id) {
                    continue;
                }
            } else if !RECOGNIZED.iter().any(|p| path.starts_with(p)) {
                return Err(Unsupported("links to site paths the router is not ported for").into());
            }
            internal = true;
        }
        if parsed.host.is_some_and(|h| h.len() > MAX_DOMAIN_LENGTH) {
            continue;
        }
        // A visible topic's link is stored as its canonical URL.
        let url = target
            .as_ref()
            .and_then(|t| t.url.clone())
            .unwrap_or_else(|| url.clone());
        let url: String = url.chars().take(MAX_URL_LENGTH).collect();
        let domain = parsed.host.unwrap_or(site.hostname);
        let (link_topic_id, link_post_id) = match &target {
            Some(t) => (Some(t.id), t.post_id),
            None => (None, None),
        };
        safe_create_topic_link(
            &mut *conn,
            &NewLink {
                post_id: Some(post.id),
                user_id: post.user_id,
                topic_id: post.topic_id,
                url: &url,
                domain,
                internal,
                link_topic_id,
                link_post_id,
                extension: extension(parsed.path),
                reflection: false,
            },
        )
        .await?;
        // The reflection in the linked topic, when the author can see it
        // and neither side is a message.
        if let Some(t) = target
            .as_ref()
            .filter(|t| t.url.is_some() && !t.private_message)
        {
            let (archetype, visible, post_number, slug): (String, bool, i32, Option<String>) =
                sqlx::query_as(
                    "SELECT t.archetype, t.visible, p.post_number, t.slug FROM posts p \
                     JOIN topics t ON t.id = p.topic_id WHERE p.id = $1",
                )
                .bind(post.id)
                .fetch_one(&mut *conn)
                .await?;
            if archetype != "private_message" && visible {
                let mut reflected_url = format!("{}{}/t/", site.base_url_no_prefix, site.base_path);
                if let Some(slug) = slug.as_deref().filter(|s| !s.is_empty()) {
                    reflected_url.push_str(slug);
                    reflected_url.push('/');
                }
                reflected_url.push_str(&post.topic_id.to_string());
                if post_number > 1 {
                    reflected_url.push_str(&format!("/{post_number}"));
                }
                let (id, _) = safe_create_topic_link(
                    &mut *conn,
                    &NewLink {
                        post_id: t.post_id,
                        user_id: post.user_id,
                        topic_id: t.id,
                        url: &reflected_url,
                        domain: site.hostname,
                        internal: true,
                        link_topic_id: Some(post.topic_id),
                        link_post_id: Some(post.id),
                        extension: None,
                        reflection: true,
                    },
                )
                .await?;
                if let Some(id) = id {
                    reflected_ids.push(id);
                }
            }
        }
        current_urls.push(url);
    }
    // cleanup_entries
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
        sqlx::query(
            "DELETE FROM topic_links WHERE link_post_id = $1 AND reflection AND NOT (id = ANY($2))",
        )
        .bind(post.id)
        .bind(&reflected_ids)
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
