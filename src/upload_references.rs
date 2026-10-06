//! HasPostUploadReferences: the uploads a post's cooked html points at
//! (`each_upload_url`), found as `Upload.fetch_from` finds them and
//! recorded as the post's UploadReferences (`link_post_uploads`).
//!
//! Refused: video thumbnails (video_thumbnails_enabled with a video) and
//! secure uploads (the access control post); a multisite database other
//! than `default`.

use sqlx::PgConnection;

use crate::pretty_text::cleanup::{all_elements, attr, element_name, parse, uri_host};
use crate::pretty_text::helpers::{sha1_from_base62, sha1_from_short_url};
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `Upload::SHA1_LENGTH`
const SHA1_LENGTH: usize = 40;

/// `RailsMultisite::ConnectionManagement.current_db`
const CURRENT_DB: &str = "default";

/// An upload url and the sha1 it carries, as each_upload_url yields them.
#[derive(Debug, PartialEq)]
pub struct UploadUrl {
    pub url: String,
    pub sha1: Option<String>,
}

/// `Upload.sha1_from_short_path`: `/uploads/short-url/<base62>`.
fn sha1_from_short_path(path: &str) -> Option<String> {
    let rest = &path[path.find("/uploads/short-url/")? + "/uploads/short-url/".len()..];
    let encoded: String = rest
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    if encoded.is_empty() {
        return None;
    }
    sha1_from_base62(&encoded)
}

/// `URL_REGEX` of Upload (`/original/\dX[/.\w]*/(\h+)[.\w]*`) or
/// OptimizedImage (`/optimized/...([a-zA-Z0-9]+)...`): the whole match
/// and the sha1 group.
fn extract_url<'a>(path: &'a str, segment: &str, hex_only: bool) -> Option<(&'a str, &'a str)> {
    let mut from = 0;
    while let Some(i) = path[from..].find(segment) {
        let start = from + i;
        let after = &path[start + segment.len()..];
        let mut chars = after.char_indices();
        // \dX
        if let (Some((_, d)), Some((_, 'X'))) = (chars.next(), chars.next())
            && d.is_ascii_digit()
        {
            let body = &after[2..];
            // [/.\w]* greedily, then backtrack to a `/` followed by the sha1.
            let run_len = body
                .find(|c: char| !(c == '/' || c == '.' || c == '_' || c.is_ascii_alphanumeric()))
                .unwrap_or(body.len());
            let run = &body[..run_len];
            let is_sha1_char = |c: char| {
                if hex_only {
                    c.is_ascii_hexdigit()
                } else {
                    c.is_ascii_alphanumeric()
                }
            };
            for (slash, _) in run.match_indices('/').rev() {
                let tail = &run[slash + 1..];
                let sha1_len = tail.find(|c: char| !is_sha1_char(c)).unwrap_or(tail.len());
                if sha1_len > 0 {
                    let sha1 = &tail[..sha1_len];
                    let end_rest = tail[sha1_len..]
                        .find(|c: char| !(c == '.' || c == '_' || c.is_ascii_alphanumeric()))
                        .unwrap_or(tail.len() - sha1_len);
                    let whole_end = start + segment.len() + 2 + slash + 1 + sha1_len + end_rest;
                    return Some((&path[start..whole_end], sha1));
                }
            }
        }
        from = start + segment.len();
    }
    None
}

/// `Upload.extract_sha1` / `OptimizedImage.extract_sha1`: a 40-character
/// sha1 from a long url.
fn extract_sha1(path: &str, optimized: bool) -> Option<String> {
    let (_, sha1) = if optimized {
        extract_url(path, "/optimized/", false)?
    } else {
        extract_url(path, "/original/", true)?
    };
    (sha1.len() == SHA1_LENGTH).then(|| sha1.to_string())
}

/// `UrlHelper.unencode`
fn unencode(url: &str) -> String {
    percent_decode(url)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The path of an absolute, protocol-relative or relative url.
fn url_path(url: &str) -> Option<String> {
    let rest = if let Some(i) = url.find("://") {
        &url[i + 3..]
    } else if let Some(r) = url.strip_prefix("//") {
        r
    } else {
        return Some(url.split(['?', '#']).next().unwrap_or("").to_string());
    };
    let path = rest.find('/').map(|i| &rest[i..]).unwrap_or("");
    Some(path.split(['?', '#']).next().unwrap_or("").to_string())
}

/// `each_upload_url(fragments:)` over cooked html, with the site's
/// hostname for the short urls and local store's urls.
pub fn each_upload_url(html: &str, hostname: &str) -> Vec<UploadUrl> {
    const SELECTORS: [(&str, &str); 7] = [
        ("a", "href"),
        ("img", "src"),
        ("source", "src"),
        ("track", "src"),
        ("video", "poster"),
        ("div", "data-video-src"),
        ("div", "data-original-video-src"),
    ];
    let dom = parse(html);
    let elements = all_elements(&dom);
    // fragments.css(...) yields each selector's matches in turn.
    let mut links: Vec<String> = Vec::new();
    for (tag, name) in SELECTORS {
        for e in &elements {
            if element_name(e) != Some(tag) {
                continue;
            }
            let Some(src) = attr(e, name).filter(|s| !s.trim().is_empty()) else {
                continue;
            };
            let src = match attr(e, "data-orig-src").filter(|s| !s.trim().is_empty()) {
                Some(orig) if src.ends_with("/images/transparent.png") => orig,
                _ => src,
            };
            if !links.contains(&src) {
                links.push(src);
            }
        }
    }

    let mut out = Vec::new();
    for src in links {
        let src = src.split('?').next().unwrap_or("").to_string();
        if src.starts_with("upload://") {
            let sha1 = sha1_from_short_url(&src);
            out.push(UploadUrl { url: src, sha1 });
            continue;
        }
        if src.contains("/uploads/short-url/") {
            if let Ok(Some(host)) = uri_host(&src)
                && host != hostname
            {
                continue;
            }
            let sha1 = sha1_from_short_path(&src);
            out.push(UploadUrl { url: src, sha1 });
            continue;
        }
        let patterns = [
            src.contains(&format!("/uploads/{CURRENT_DB}/")),
            src.contains("/original/"),
            src.contains("/optimized/"),
        ];
        if !patterns.contains(&true) {
            continue;
        }
        // has_been_uploaded? on the local store, or a relative url
        // (include_local_upload).
        let relative = src.starts_with('/') && !src.starts_with("//");
        let local = matches!(uri_host(&src), Ok(Some(ref host)) if host == hostname)
            && url_path(&src).is_some_and(|p| p.starts_with("/uploads/"));
        if !relative && !local {
            continue;
        }
        let Some(path) = url_path(&unencode(&src)).filter(|p| !p.is_empty()) else {
            continue;
        };
        let sha1 = if path.contains("optimized") {
            extract_sha1(&path, true)
        } else {
            extract_sha1(&path, false).or_else(|| sha1_from_short_path(&path))
        };
        out.push(UploadUrl { url: src, sha1 });
    }
    out
}

/// `Upload.fetch_from(sha1:, url:)`: by sha1, else `get_from_url`.
pub(crate) async fn fetch_from(
    conn: &mut PgConnection,
    found: &UploadUrl,
) -> Result<Option<i32>, AppError> {
    if let Some(sha1) = found.sha1.as_deref().filter(|s| !s.is_empty()) {
        let id: Option<i32> = sqlx::query_scalar("SELECT id FROM uploads WHERE sha1 = $1 LIMIT 1")
            .bind(sha1)
            .fetch_optional(&mut *conn)
            .await?;
        if id.is_some() {
            return Ok(id);
        }
    }
    Ok(get_from_url(&mut *conn, &found.url)
        .await?
        .map(|(id, _)| id))
}

/// `Upload.get_from_url`: the upload a long url names (its sha1, else the
/// end of its url), or whose url is the path; its id and url.
pub async fn get_from_url(
    conn: &mut PgConnection,
    url: &str,
) -> Result<Option<(i32, String)>, sqlx::Error> {
    let Some(path) = url_path(&unencode(url)).filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    let found: Option<(i32, String)> = match extract_url(&path, "/original/", true) {
        None => {
            sqlx::query_as("SELECT id, url FROM uploads WHERE url = $1 LIMIT 1")
                .bind(&path)
                .fetch_optional(&mut *conn)
                .await?
        }
        Some((whole, sha1)) => {
            let by_sha1 = if sha1.len() == SHA1_LENGTH {
                sqlx::query_as("SELECT id, url FROM uploads WHERE sha1 = $1 LIMIT 1")
                    .bind(sha1)
                    .fetch_optional(&mut *conn)
                    .await?
            } else {
                None
            };
            match by_sha1 {
                Some(found) => Some(found),
                None => {
                    let pattern = format!(
                        "%{}",
                        whole
                            .replace('\\', "\\\\")
                            .replace('%', "\\%")
                            .replace('_', "\\_")
                    );
                    sqlx::query_as("SELECT id, url FROM uploads WHERE url LIKE $1 LIMIT 1")
                        .bind(pattern)
                        .fetch_optional(&mut *conn)
                        .await?
                }
            }
        }
    };
    Ok(found)
}

/// `update_post_image`'s choice: the first upload among the post's images
/// marked `data-thumbnail`, else among the others (`extract_images_for_
/// post`: not emoji, quoted, onebox site icons or avatars, or GitHub
/// folder oneboxes).
pub async fn post_image_upload(
    conn: &mut PgConnection,
    hostname: &str,
    cooked: &str,
) -> Result<Option<i32>, AppError> {
    let (marked, others) = {
        let dom = parse(cooked);
        let mut marked = String::new();
        let mut others = String::new();
        for e in all_elements(&dom) {
            if element_name(&e) != Some("img") {
                continue;
            }
            let Some(src) = attr(&e, "src") else {
                continue;
            };
            let class = attr(&e, "class").unwrap_or_default();
            let classes: Vec<&str> = class.split_whitespace().collect();
            if [
                "emoji",
                "site-icon",
                "onebox-avatar",
                "onebox-avatar-inline",
            ]
            .iter()
            .any(|c| classes.contains(c))
            {
                continue;
            }
            let mut ancestors = Vec::new();
            let mut current = parent(&e);
            while let Some(p) = current {
                current = parent(&p);
                ancestors.push(p);
            }
            let has = |node: &markup5ever_rcdom::Handle, c: &str| {
                attr(node, "class").is_some_and(|v| v.split_whitespace().any(|x| x == c))
            };
            if ancestors.iter().any(|a| has(a, "quote"))
                || ancestors
                    .iter()
                    .any(|a| has(a, "onebox") && has(a, "githubfolder"))
            {
                continue;
            }
            // The image alone, for each_upload_url to read again.
            let escaped = |v: &str| v.replace('&', "&amp;").replace('"', "&quot;");
            let mut tag = format!("<img src=\"{}\"", escaped(&src));
            if let Some(orig) = attr(&e, "data-orig-src") {
                tag.push_str(&format!(" data-orig-src=\"{}\"", escaped(&orig)));
            }
            tag.push('>');
            if attr(&e, "data-thumbnail").is_some() {
                marked.push_str(&tag);
            } else {
                others.push_str(&tag);
            }
        }
        (marked, others)
    };
    for images in [marked, others] {
        for found in each_upload_url(&images, hostname) {
            if let Some(id) = fetch_from(&mut *conn, &found).await? {
                return Ok(Some(id));
            }
        }
    }
    Ok(None)
}

/// A node's parent, left in place.
fn parent(node: &markup5ever_rcdom::Handle) -> Option<markup5ever_rcdom::Handle> {
    let weak = node.parent.take();
    let up = weak.as_ref().and_then(|w| w.upgrade());
    node.parent.set(weak);
    up
}

/// `link_post_uploads(fragments:)` for the post `post_id` with this
/// cooked html: its UploadReferences replaced by the uploads it points at.
pub async fn link_post_uploads(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    hostname: &str,
    post_id: i32,
    cooked: &str,
) -> Result<(), AppError> {
    if settings.get("secure_uploads")?.truthy() {
        return Err(Unsupported("secure uploads (access control posts)").into());
    }
    let video_thumbnails = settings.get("video_thumbnails_enabled")?.truthy();
    let mut upload_ids: Vec<i32> = Vec::new();
    for found in each_upload_url(cooked, hostname) {
        let Some(id) = fetch_from(&mut *conn, &found).await? else {
            continue;
        };
        if video_thumbnails {
            let extension: Option<String> =
                sqlx::query_scalar("SELECT extension FROM uploads WHERE id = $1")
                    .bind(id)
                    .fetch_one(&mut *conn)
                    .await?;
            let ext = extension.unwrap_or_default().to_lowercase();
            // FileHelper.supported_video
            if ["mov", "mp4", "webm", "ogv", "m4v", "3gp", "avi", "mpeg"].contains(&ext.as_str()) {
                return Err(Unsupported("video thumbnails of a post's videos").into());
            }
        }
        upload_ids.push(id);
    }
    sqlx::query("DELETE FROM upload_references WHERE target_type = 'Post' AND target_id = $1")
        .bind(post_id)
        .execute(&mut *conn)
        .await?;
    // insert_all: the unique index keeps one reference per upload.
    for id in upload_ids {
        sqlx::query(
            "INSERT INTO upload_references (upload_id, target_type, target_id, created_at, updated_at) \
             VALUES ($1, 'Post', $2, now(), now()) ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(post_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA1: &str = "e9d71f5ee7c92d6dc9e92ffdad17b8bd49418f98";

    #[test]
    fn upload_urls_carry_their_sha1() {
        let html = format!(
            r#"<p><img src="/uploads/default/original/1X/{SHA1}.png" alt="a"></p>
<p><a class="attachment" href="/uploads/short-url/xyKj1g5mJy4zbB1fLRxTRBXpZhp.pdf">doc.pdf</a></p>
<p><img src="/images/transparent.png" data-orig-src="upload://xyKj1g5mJy4zbB1fLRxTRBXpZhp.png"></p>
<p><a href="https://example.com/uploads/default/original/1X/{SHA1}.png">elsewhere</a></p>
<p><img src="/uploads/default/optimized/1X/{SHA1}_2_690x388.png"></p>"#
        );
        let found = each_upload_url(&html, "test.localhost");
        let short = sha1_from_base62("xyKj1g5mJy4zbB1fLRxTRBXpZhp");
        assert_eq!(
            found,
            vec![
                UploadUrl {
                    url: "/uploads/short-url/xyKj1g5mJy4zbB1fLRxTRBXpZhp.pdf".into(),
                    sha1: short.clone()
                },
                UploadUrl {
                    url: format!("/uploads/default/original/1X/{SHA1}.png"),
                    sha1: Some(SHA1.into())
                },
                UploadUrl {
                    url: "upload://xyKj1g5mJy4zbB1fLRxTRBXpZhp.png".into(),
                    sha1: short
                },
                UploadUrl {
                    url: format!("/uploads/default/optimized/1X/{SHA1}_2_690x388.png"),
                    sha1: Some(SHA1.into())
                },
            ]
        );
    }
}
