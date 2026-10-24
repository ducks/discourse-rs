//! HasPostUploadReferences: the uploads a post's cooked html points at
//! (`each_upload_url`), found as `Upload.fetch_from` finds them and
//! recorded as the post's UploadReferences (`link_post_uploads`).
//!
//! Refused: video thumbnails (video_thumbnails_enabled with a video) and
//! secure uploads (the access control post); a multisite database other
//! than `default`.

use sqlx::PgConnection;

use crate::file_store::FileStore;
use crate::pretty_text::cleanup::{all_elements, attr, element_name, parse, uri_host};
use crate::pretty_text::helpers::{sha1_from_base62, sha1_from_short_url};
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `Upload::SHA1_LENGTH`
const SHA1_LENGTH: usize = 40;

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

pub(crate) fn percent_decode(s: &str) -> String {
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
pub fn each_upload_url(html: &str, hostname: &str, store: &FileStore) -> Vec<UploadUrl> {
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
            src.contains(&format!("/uploads/{}/", store.db())),
            src.contains("/original/"),
            src.contains("/optimized/"),
        ];
        if !patterns.contains(&true) {
            continue;
        }
        // has_been_uploaded? on the store, or a relative url
        // (include_local_upload).
        let relative = src.starts_with('/') && !src.starts_with("//");
        if !relative && !store.has_been_uploaded(&src) {
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
    store: &FileStore,
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
        for found in each_upload_url(&images, hostname, store) {
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
    store: &FileStore,
    post_id: i32,
    cooked: &str,
) -> Result<(), AppError> {
    if settings.get("secure_uploads")?.truthy() {
        return Err(Unsupported("secure uploads (access control posts)").into());
    }
    let video_thumbnails = settings.get("video_thumbnails_enabled")?.truthy();
    let mut upload_ids: Vec<i32> = Vec::new();
    for found in each_upload_url(cooked, hostname, store) {
        let Some(id) = fetch_from(&mut *conn, &found).await? else {
            continue;
        };
        if video_thumbnails {
            let (extension, sha1): (Option<String>, Option<String>) =
                sqlx::query_as("SELECT extension, sha1 FROM uploads WHERE id = $1")
                    .bind(id)
                    .fetch_one(&mut *conn)
                    .await?;
            let ext = extension.unwrap_or_default().to_lowercase();
            // A video's thumbnail is the upload named after the video's
            // sha1 (the composer uploads it before the post exists).
            if let Some(sha1) = sha1.filter(|s| !s.is_empty())
                && crate::file_store::SUPPORTED_VIDEO.contains(&ext.as_str())
            {
                let thumbnail: Option<i32> = sqlx::query_scalar(
                    "SELECT id FROM uploads WHERE original_filename LIKE $1 ORDER BY id DESC LIMIT 1",
                )
                .bind(format!("{sha1}.%"))
                .fetch_optional(&mut *conn)
                .await?;
                if let Some(thumbnail) = thumbnail {
                    upload_ids.push(thumbnail);
                    handle_video_thumbnail(&mut *conn, settings, store, post_id, thumbnail).await?;
                }
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

/// `Post#handle_video_thumbnail`: a first post's video thumbnail becomes
/// its topic's image, with the topic's thumbnails, unless the topic has
/// one.
async fn handle_video_thumbnail(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    store: &FileStore,
    post_id: i32,
    thumbnail_id: i32,
) -> Result<(), AppError> {
    let row: Option<(i32, Option<i32>, Option<i64>)> = sqlx::query_as(
        "SELECT p.post_number, t.id, t.image_upload_id FROM posts p \
         LEFT JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((post_number, topic_id, image_upload_id)) = row else {
        return Ok(());
    };
    if post_number != 1 {
        return Ok(());
    }
    // `topic.image_upload_id` on a post whose topic is gone raises.
    let topic_id = topic_id.ok_or(Unsupported("a video thumbnail on a post without its topic"))?;
    if image_upload_id.is_some() {
        return Ok(());
    }
    sqlx::query("UPDATE topics SET image_upload_id = $2 WHERE id = $1")
        .bind(topic_id)
        .bind(i64::from(thumbnail_id))
        .execute(&mut *conn)
        .await?;
    if let Some(upload) = crate::optimized_images::Upload::find(&mut *conn, thumbnail_id).await? {
        crate::optimized_images::generate_topic_thumbnails(conn, settings, store, &upload)
            .await
            .map_err(crate::optimized_images::app_error)?;
    }
    Ok(())
}

/// The sha1s `Upload.extract_upload_ids` looks for in a raw: each
/// `/(\h{40})`, then each `/([a-zA-Z0-9]+)` decoded from base62 (None
/// where it doesn't decode to a sha1), as Ruby's two scans find them.
fn extract_sha1s(raw: &str) -> Vec<Option<String>> {
    let bytes = raw.as_bytes();
    let slashes: Vec<usize> = raw.match_indices('/').map(|(i, _)| i).collect();
    let mut sha1s = Vec::new();
    for &i in &slashes {
        let hex = &bytes[i + 1..];
        if hex.len() >= SHA1_LENGTH && hex[..SHA1_LENGTH].iter().all(u8::is_ascii_hexdigit) {
            sha1s.push(Some(raw[i + 1..i + 1 + SHA1_LENGTH].to_string()));
        }
    }
    for &i in &slashes {
        let run = bytes[i + 1..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric())
            .count();
        if run > 0 {
            sha1s.push(sha1_from_base62(&raw[i + 1..i + 1 + run]));
        }
    }
    sha1s
}

/// `Upload.extract_upload_ids(raw)`: the uploads whose sha1 the raw names
/// (`Upload.where(sha1: sha1s.uniq).pluck(:id)`, so a sha1 that didn't
/// decode matches uploads without one).
pub async fn extract_upload_ids(
    conn: &mut PgConnection,
    raw: &str,
) -> Result<Vec<i64>, sqlx::Error> {
    if crate::ruby::is_blank(raw) {
        return Ok(Vec::new());
    }
    let sha1s = extract_sha1s(raw);
    if sha1s.is_empty() {
        return Ok(Vec::new());
    }
    let with_nil = sha1s.iter().any(Option::is_none);
    let mut found: Vec<String> = Vec::new();
    for sha1 in sha1s.into_iter().flatten() {
        if !found.contains(&sha1) {
            found.push(sha1);
        }
    }
    sqlx::query_scalar(
        "SELECT id::bigint FROM uploads WHERE sha1 = ANY($1) OR ($2 AND sha1 IS NULL)",
    )
    .bind(&found)
    .bind(with_nil)
    .fetch_all(conn)
    .await
}

/// `UploadReference.ensure_exist!(upload_ids: [upload_id], target:)` for
/// one upload: the target's other references deleted and the upload's
/// inserted when missing; with none, every reference of the target
/// deleted.
pub async fn ensure_exist(
    conn: &mut PgConnection,
    target_type: &str,
    target_id: i64,
    upload_id: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM upload_references WHERE target_type = $1 AND target_id = $2 \
         AND ($3::bigint IS NULL OR upload_id <> $3)",
    )
    .bind(target_type)
    .bind(target_id)
    .bind(upload_id)
    .execute(&mut *conn)
    .await?;
    if let Some(upload_id) = upload_id {
        // insert_all: the existing row skipped.
        sqlx::query(
            "INSERT INTO upload_references (upload_id, target_type, target_id, created_at, updated_at) \
             VALUES ($1, $2, $3, now(), now()) ON CONFLICT DO NOTHING",
        )
        .bind(upload_id)
        .bind(target_type)
        .bind(target_id)
        .execute(conn)
        .await?;
    }
    Ok(())
}

/// `UploadReference.ensure_exist!(upload_ids:, target:)` for several
/// uploads: the target's other references deleted, the missing ones
/// inserted; with none, every reference of the target deleted.
pub async fn ensure_exist_many(
    conn: &mut PgConnection,
    target_type: &str,
    target_id: i64,
    upload_ids: &[i64],
) -> Result<(), sqlx::Error> {
    let mut ids: Vec<i64> = Vec::new();
    for id in upload_ids {
        if !ids.contains(id) {
            ids.push(*id);
        }
    }
    sqlx::query(
        "DELETE FROM upload_references WHERE target_type = $1 AND target_id = $2 \
         AND NOT (upload_id = ANY($3))",
    )
    .bind(target_type)
    .bind(target_id)
    .bind(&ids)
    .execute(&mut *conn)
    .await?;
    for id in ids {
        // insert_all: the existing rows skipped.
        sqlx::query(
            "INSERT INTO upload_references (upload_id, target_type, target_id, created_at, updated_at) \
             VALUES ($1, $2, $3, now(), now()) ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(target_type)
        .bind(target_id)
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
    fn a_raw_names_uploads_by_sha1_and_short_url() {
        let raw = format!(
            "![a|1x1](upload://sLckxyrYcJI4X5RPt6GOaGETgux.png) [b](/uploads/default/original/1X/{SHA1}.png)"
        );
        let found = extract_sha1s(&raw);
        // The 40 hex digits first, then every path segment from base62:
        // `/`, `uploads`, `default`, ... decode to no upload or none.
        assert_eq!(found[0].as_deref(), Some(SHA1));
        assert!(found.contains(&Some("c992b9f1e934fb321456b9b2b1c6928ac81ad3ad".into())));
        // A segment longer than MAX_BASE62_SHA1_LENGTH is no sha1.
        assert!(found.contains(&None));
        assert!(extract_sha1s("no slashes at all").is_empty());
    }

    #[test]
    fn upload_urls_carry_their_sha1() {
        let html = format!(
            r#"<p><img src="/uploads/default/original/1X/{SHA1}.png" alt="a"></p>
<p><a class="attachment" href="/uploads/short-url/xyKj1g5mJy4zbB1fLRxTRBXpZhp.pdf">doc.pdf</a></p>
<p><img src="/images/transparent.png" data-orig-src="upload://xyKj1g5mJy4zbB1fLRxTRBXpZhp.png"></p>
<p><a href="https://example.com/uploads/default/original/1X/{SHA1}.png">elsewhere</a></p>
<p><img src="/uploads/default/optimized/1X/{SHA1}_2_690x388.png"></p>"#
        );
        let store = crate::file_store::FileStore::test_local("http://test.localhost");
        let found = each_upload_url(&html, "test.localhost", &store);
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
