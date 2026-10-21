//! `IncomingLink.add`: a topic page reached from another site (its
//! Referer) or through a member's share link (`?u=username`), which
//! TopicsController#track_visit_to_topic defers past the response of every
//! page (not JSON) request. The link, its referer's IncomingDomain and
//! IncomingReferer, and the post's and topic's incoming_link_count.

use axum::http::HeaderMap;
use sqlx::PgConnection;

use crate::{AppError, Unsupported};

/// What `track_visit_to_topic` hands IncomingLink.add.
pub struct Visit {
    /// `request.referer`
    pub referer: Option<String>,
    /// `request.host`
    pub host: String,
    pub current_user_id: Option<i32>,
    pub topic_id: i32,
    /// `@topic_view.current_post_number`
    pub post_number: Option<i64>,
    /// `request["u"]` when it is a string.
    pub username: Option<String>,
    /// `request.remote_ip`
    pub ip_address: String,
}

/// `request.host`: the last X-Forwarded-Host when one is given, else the
/// Host header (HTTP/2's authority), less a trailing port.
pub fn request_host(headers: &HeaderMap, uri: &axum::http::Uri) -> Result<String, AppError> {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let raw = match header("x-forwarded-host").filter(|h| !crate::ruby::is_blank(h)) {
        // `split(/,\s?/).last`: trailing empty fields dropped.
        Some(forwarded) => {
            let fields: Vec<&str> = forwarded
                .split(',')
                .enumerate()
                .map(|(i, f)| if i == 0 { f } else { strip_one_space(f) })
                .collect();
            match fields.iter().rev().find(|f| !f.is_empty()) {
                Some(last) => last.to_string(),
                None => return Err(Unsupported("an X-Forwarded-Host of separators only").into()),
            }
        }
        None => match header("host").or_else(|| uri.authority().map(|a| a.to_string())) {
            Some(h) => h,
            // "#{server_name}:#{port}", which the server sets.
            None => return Err(Unsupported("a request without a host").into()),
        },
    };
    // `sub(/:\d+$/, "")`
    Ok(match raw.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
            h.to_string()
        }
        _ => raw,
    })
}

/// The `\s?` after a comma.
fn strip_one_space(f: &str) -> &str {
    f.strip_prefix([' ', '\t', '\n', '\x0b', '\x0c', '\r'])
        .unwrap_or(f)
}

/// An http(s) referer as `URI.parse` reads it.
struct Referer {
    /// `uri.host`, its case kept.
    host: String,
    https: bool,
    /// `uri.port`, the scheme's default when none is given.
    port: i32,
    /// `uri.path`
    path: String,
}

/// `URI.parse(referer)`: None where it raises URI::Error; refused for
/// what it reads that isn't a plain http(s) URL with a host (other
/// schemes, relative references, user info, IP literals).
fn parse_referer(referer: &str) -> Result<Option<Referer>, AppError> {
    if !crate::final_destination::ruby_uri_valid(referer) {
        return Ok(None);
    }
    let unsupported = || -> AppError { Unsupported("a referer that is not an http(s) URL").into() };
    let (scheme, rest) = referer.split_once("://").ok_or_else(unsupported)?;
    let https = match scheme.to_ascii_lowercase().as_str() {
        "http" => false,
        "https" => true,
        _ => return Err(unsupported()),
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.contains('@') || authority.starts_with('[') {
        return Err(unsupported());
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, "")) => (h, None),
        Some((h, p)) => (h, Some(p.parse::<i32>().map_err(|_| unsupported())?)),
        None => (authority, None),
    };
    if host.is_empty() {
        return Err(unsupported());
    }
    let path = tail.split(['?', '#']).next().unwrap_or("");
    Ok(Some(Referer {
        host: host.to_string(),
        https,
        port: port.unwrap_or(if https { 443 } else { 80 }),
        path: path.to_string(),
    }))
}

/// `IncomingLink.add(opts)`
pub async fn add(conn: &mut PgConnection, visit: Visit) -> Result<(), AppError> {
    let mut user_id: Option<i32> = None;
    if let Some(username) = visit.username.filter(|u| !u.contains('\0')) {
        user_id = sqlx::query_scalar("SELECT id FROM users WHERE username_lower = $1 LIMIT 1")
            .bind(username.to_lowercase())
            .fetch_optional(&mut *conn)
            .await?;
    }
    let (host, referer) = match visit.referer.filter(|r| !crate::ruby::is_blank(r)) {
        Some(r) => match parse_referer(&r)? {
            // `opts[:referer][0..999]`
            Some(parsed) => (
                Some(parsed.host),
                Some(r.chars().take(1000).collect::<String>()),
            ),
            None => (None, None),
        },
        None => (None, None),
    };
    if host.as_deref() == Some(visit.host.as_str()) || (user_id.is_none() && referer.is_none()) {
        return Ok(());
    }
    let post_number = visit.post_number.unwrap_or(1);
    let post_id: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM posts WHERE topic_id = $1 AND post_number = $2 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(visit.topic_id)
    .bind(post_number)
    .fetch_optional(&mut *conn)
    .await?;
    let current_user_id = visit.current_user_id;
    let ip_address = current_user_id.is_none().then_some(visit.ip_address);
    if current_user_id.is_some() && current_user_id == user_id {
        return Ok(());
    }
    let Some(post_id) = post_id else {
        return Ok(());
    };

    // `referer=`: the domain and referer rows, made before validation.
    let mut incoming_referer: Option<(i32, String, usize)> = None;
    if let Some(referer) = referer.as_deref().filter(|r| !crate::ruby::is_blank(r)) {
        // The truncated referer is parsed again; URI::Error is ignored.
        if let Some(parsed) = parse_referer(referer)? {
            let domain_id = incoming_domain(&mut *conn, &parsed).await?;
            let referer_id = incoming_referer_id(&mut *conn, &parsed.path, domain_id).await?;
            // IncomingDomain#to_url + path
            let default_port = if parsed.https { 443 } else { 80 };
            let mut url = format!(
                "http{}://{}",
                if parsed.https { "s" } else { "" },
                parsed.host
            );
            if parsed.port != default_port {
                url.push_str(&format!(":{}", parsed.port));
            }
            url.push_str(&parsed.path);
            incoming_referer = Some((referer_id, url, parsed.host.chars().count()));
        }
    }
    // referer_valid
    if let Some((_, url, domain_length)) = &incoming_referer {
        let length = url.chars().count();
        if !(3..=100).contains(&length) || !(1..=100).contains(domain_length) {
            return Ok(());
        }
    }
    sqlx::query(
        "INSERT INTO incoming_links (created_at, user_id, ip_address, current_user_id, post_id, incoming_referer_id) \
         VALUES ($1, $2, $3::inet, $4, $5, $6)",
    )
    .bind(crate::clock::now_naive())
    .bind(user_id)
    .bind(ip_address)
    .bind(current_user_id)
    .bind(post_id)
    .bind(incoming_referer.map(|(id, _, _)| id))
    .execute(&mut *conn)
    .await?;
    // update_link_counts
    sqlx::query(
        "UPDATE topics SET incoming_link_count = incoming_link_count + 1 \
         WHERE id = (SELECT topic_id FROM posts WHERE id = $1)",
    )
    .bind(post_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query("UPDATE posts SET incoming_link_count = incoming_link_count + 1 WHERE id = $1")
        .bind(post_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `IncomingDomain.add!(uri)`: the domain's row, found or made.
async fn incoming_domain(conn: &mut PgConnection, uri: &Referer) -> Result<i32, AppError> {
    let find =
        "SELECT id FROM incoming_domains WHERE name = $1 AND https = $2 AND port = $3 LIMIT 1";
    let found: Option<i32> = sqlx::query_scalar(find)
        .bind(&uri.host)
        .bind(uri.https)
        .bind(uri.port)
        .fetch_optional(&mut *conn)
        .await?;
    if let Some(id) = found {
        return Ok(id);
    }
    // A duplicate made meanwhile is found instead (RecordNotUnique).
    let made: Option<i32> = sqlx::query_scalar(
        "INSERT INTO incoming_domains (name, https, port) VALUES ($1, $2, $3) \
         ON CONFLICT DO NOTHING RETURNING id",
    )
    .bind(&uri.host)
    .bind(uri.https)
    .bind(uri.port)
    .fetch_optional(&mut *conn)
    .await?;
    match made {
        Some(id) => Ok(id),
        None => Ok(sqlx::query_scalar(find)
            .bind(&uri.host)
            .bind(uri.https)
            .bind(uri.port)
            .fetch_one(&mut *conn)
            .await?),
    }
}

/// `IncomingReferer.add!(path:, incoming_domain:)`
async fn incoming_referer_id(
    conn: &mut PgConnection,
    path: &str,
    domain_id: i32,
) -> Result<i32, AppError> {
    let find =
        "SELECT id FROM incoming_referers WHERE path = $1 AND incoming_domain_id = $2 LIMIT 1";
    let found: Option<i32> = sqlx::query_scalar(find)
        .bind(path)
        .bind(domain_id)
        .fetch_optional(&mut *conn)
        .await?;
    if let Some(id) = found {
        return Ok(id);
    }
    let made: Option<i32> = sqlx::query_scalar(
        "INSERT INTO incoming_referers (path, incoming_domain_id) VALUES ($1, $2) \
         ON CONFLICT DO NOTHING RETURNING id",
    )
    .bind(path)
    .bind(domain_id)
    .fetch_optional(&mut *conn)
    .await?;
    match made {
        Some(id) => Ok(id),
        None => Ok(sqlx::query_scalar(find)
            .bind(path)
            .bind(domain_id)
            .fetch_one(&mut *conn)
            .await?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_host_like_action_dispatch() {
        let uri = axum::http::Uri::from_static("/t/1");
        let mut h = HeaderMap::new();
        h.insert("host", "forum.example:8080".parse().unwrap());
        assert_eq!(request_host(&h, &uri).unwrap(), "forum.example");
        h.insert(
            "x-forwarded-host",
            "a.example, b.example:443".parse().unwrap(),
        );
        assert_eq!(request_host(&h, &uri).unwrap(), "b.example");
        h.insert("x-forwarded-host", "a.example,".parse().unwrap());
        assert_eq!(request_host(&h, &uri).unwrap(), "a.example");
    }

    #[test]
    fn referers_as_uri_parses_them() {
        let r = parse_referer("https://Search.Example:8443/a/b?q=1#x")
            .unwrap()
            .unwrap();
        assert_eq!(
            (r.host.as_str(), r.https, r.port, r.path.as_str()),
            ("Search.Example", true, 8443, "/a/b")
        );
        let r = parse_referer("http://example.com").unwrap().unwrap();
        assert_eq!((r.port, r.path.as_str()), (80, ""));
        assert!(parse_referer("http://exa mple.com/").unwrap().is_none());
        assert!(parse_referer("android-app://com.example/").is_err());
    }
}
