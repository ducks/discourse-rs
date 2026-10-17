//! Port of UserAvatarsController (app/controllers/user_avatars_controller.rb).
//! show_proxy_letter: the default external_system_avatars_url points here,
//! and each letter avatar is fetched once from the avatar CDN and served
//! from a disk cache after. show_letter: the letter avatars drawn here
//! (src/letter_avatar.rs) when that setting is blank.

use std::path::{Path as FsPath, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use sha1::{Digest, Sha1};

use crate::letter_avatar;
use crate::site_settings::SiteSettings;
use crate::upload_references::percent_decode;
use crate::{AppError, AppState};

/// `max_file_size`
const MAX_FILE_SIZE: usize = 1024 * 1024;
/// `PROXY_CACHE_MAX_ENTRIES` and `PROXY_CACHE_EVICT_COUNT`
const PROXY_CACHE_MAX_ENTRIES: usize = 10_000;
const PROXY_CACHE_EVICT_COUNT: usize = 1000;
/// `Time.new(1990, 01, 01).httpdate`, the Last-Modified of every proxied
/// avatar and of the blank one.
const LAST_MODIFIED: &str = "Mon, 01 Jan 1990 00:00:00 GMT";

/// `FileHelper.download(follow_redirect: true, read_timeout: 10)`
static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(Duration::from_secs(10))
        .build()
        .expect("the avatar proxy's HTTP client builds")
});

/// `GET /letter_avatar_proxy/:version/letter/:letter/:color/:size.png`
pub async fn show_proxy_letter(
    State(state): State<AppState>,
    Path((version, letter, color, size)): Path<(String, String, String, String)>,
) -> Result<Response, AppError> {
    let Some(size) = size.strip_suffix(".png").filter(|s| !s.contains('.')) else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    drop(conn);
    if !settings
        .get("external_system_avatars_url")?
        .to_s()
        .starts_with("/letter_avatar_proxy")
    {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }

    let url = format!(
        "{}/{}/letter/{}/{}/{}.png",
        state.config.letter_avatar_cdn,
        segment(&version),
        segment(&letter),
        segment(&color),
        segment(size)
    );
    let dir = state.config.tmp_dir.join("avatar_proxy");
    let path = dir.join(format!("{:x}.png", Sha1::digest(url.as_bytes())));
    if !tokio::fs::try_exists(&path).await? {
        match download(&url).await {
            Ok(Some(bytes)) => store(&dir, &path, &bytes).await?,
            Ok(None) => return render_blank(&state).await,
            Err(e) => {
                tracing::warn!(url, error = %e, "letter avatar proxy download failed");
                return render_blank(&state).await;
            }
        }
    }
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        // Evicted by another request in between.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return render_blank(&state).await,
        Err(e) => return Err(e.into()),
    };
    Ok(png(bytes, "max-age=31556952, public, immutable"))
}

/// `GET /letter_avatar/:username/:size/:version.png`: the local letter
/// avatar (`LetterAvatar.generate`), for sites whose
/// external_system_avatars_url is blank; any other version is the blank
/// avatar.
pub async fn show_letter(State(state): State<AppState>, uri: Uri) -> Result<Response, AppError> {
    // The route's constraints, on the raw segments: username is
    // RouteFormat.username, size \d+, the version a segment before `.png`.
    let raw: Vec<&str> = uri.path().rsplitn(4, '/').collect();
    let [file, size, username, _] = raw[..] else {
        return Ok(super::topics::not_found_response(&state));
    };
    let username_ok = !username.is_empty()
        && username
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'%' | b'_' | b'.' | b'-'));
    let size_ok = !size.is_empty() && size.bytes().all(|b| b.is_ascii_digit());
    let version = file
        .strip_suffix(".png")
        .filter(|v| !v.is_empty() && !v.contains('.'));
    let (true, true, Some(version)) = (username_ok, size_ok, version) else {
        return Ok(super::topics::not_found_response(&state));
    };
    if percent_decode(version) != letter_avatar::version(&state.config.letter_avatar_font) {
        return render_blank(&state).await;
    }
    let username = percent_decode(username);
    // `params[:size].to_i`: digits past a u32 are past FULLSIZE too.
    let size = size.parse::<u32>().unwrap_or(u32::MAX);
    let tmp_dir = state.config.tmp_dir.clone();
    let font = state.config.letter_avatar_font.clone();
    let path = tokio::task::spawn_blocking(move || {
        letter_avatar::generate(&tmp_dir, &font, &username, size)
    })
    .await
    .map_err(std::io::Error::other)??;
    // A failed resize (an unsafe letter) leaves no file: File.ctime raises
    // and the request fails.
    let bytes = tokio::fs::read(&path).await?;
    Ok(png_modified(
        bytes,
        "max-age=31556952, public, immutable",
        HeaderValue::from_str(&ctime_httpdate(&path).await?)?,
    ))
}

/// `GET /user_avatar/:hostname/:username/:size/:version.png` (show): the
/// user's avatar upload at that size, made on demand, on the site the
/// hostname names (the dispatcher picks it, as `with_hostname` does).
pub async fn show(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    // The route's constraints: hostname [\w.-]+, username
    // RouteFormat.username, size \d+, the version a segment before `.png`.
    let raw: Vec<&str> = uri.path().rsplitn(5, '/').collect();
    let [file, size, username, hostname, _] = raw[..] else {
        return Ok(super::topics::not_found_response(&state));
    };
    let word = |s: &str, extra: &[u8]| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || extra.contains(&b))
    };
    let version = file
        .strip_suffix(".png")
        .filter(|v| !v.is_empty() && !v.contains('.'));
    let (true, true, true, Some(version)) = (
        word(hostname, b".-"),
        word(username, b"%.-"),
        !size.is_empty() && size.bytes().all(|b| b.is_ascii_digit()),
        version,
    ) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let username = percent_decode(username);
    let version = percent_decode(version);

    let mut conn = state.pool.acquire().await?;
    let user: Option<(String, Option<i32>)> = sqlx::query_as(
        "SELECT username_lower, uploaded_avatar_id FROM users WHERE username_lower = $1",
    )
    .bind(username.to_lowercase())
    .fetch_optional(&mut *conn)
    .await?;
    let Some((username_lower, uploaded_avatar_id)) = user else {
        return render_blank(&state).await;
    };
    // `upload_id, version = params[:version].split("_")`
    let mut parts = version.split('_');
    let upload_id = crate::ruby::to_i(parts.next().unwrap_or(""));
    let version = parts.next().map_or(
        i64::from(crate::optimized_images::VERSION),
        crate::ruby::to_i,
    );
    if version > i64::from(crate::optimized_images::VERSION) || upload_id <= 0 {
        return render_blank(&state).await;
    }
    let size = size.parse::<i64>().unwrap_or(i64::MAX);
    if !(8..=1000).contains(&size) {
        return render_blank(&state).await;
    }
    let (upload_id, size) = (i32::try_from(upload_id).unwrap_or(i32::MAX), size as i32);
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let store = crate::file_store::FileStore::for_site(&state.config, &settings)?;
    // An external store serves the avatar sizes only: another size is
    // sent to the closest.
    if store.external() {
        let sizes = crate::avatar::avatar_sizes(&settings)?;
        let wanted = i64::from(size);
        if !sizes.iter().any(|s| i64::from(*s) == wanted) {
            let mut closest: Option<u32> = None;
            for s in &sizes {
                if closest
                    .is_none_or(|c| (wanted - i64::from(*s)).abs() < (wanted - i64::from(c)).abs())
                {
                    closest = Some(*s);
                }
            }
            if let Some(closest) = closest {
                let location = format!(
                    "{}/user_avatar/{hostname}/{}/{closest}/{upload_id}_{}.png",
                    state.config.globals.relative_url_root(),
                    crate::avatar::encode_component(&username_lower),
                    crate::optimized_images::VERSION
                );
                return Ok(found(&headers, &location));
            }
        }
    }

    let in_user_avatar: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_avatars a JOIN users u ON u.id = a.user_id \
         WHERE u.username_lower = $1 AND (a.gravatar_upload_id = $2 OR a.custom_upload_id = $2))",
    )
    .bind(&username_lower)
    .bind(upload_id)
    .fetch_one(&mut *conn)
    .await?;
    let mut upload = if in_user_avatar {
        crate::optimized_images::Upload::find(&mut conn, upload_id).await?
    } else {
        None
    };
    let uploaded_avatar = match uploaded_avatar_id {
        Some(id) => crate::optimized_images::Upload::find(&mut conn, id).await?,
        None => None,
    };
    if upload.is_none() && uploaded_avatar_id == Some(upload_id) {
        upload = uploaded_avatar.clone();
    }
    let Some(upload) = upload else {
        if let Some(current) = uploaded_avatar {
            // UserAvatar.local_avatar_url for the current avatar.
            let location = format!(
                "{}/user_avatar/{hostname}/{}/{size}/{}_{}.png",
                state.config.globals.relative_url_root(),
                crate::avatar::encode_component(&username_lower),
                current.id,
                crate::optimized_images::VERSION
            );
            return Ok(found(&headers, &location));
        }
        return render_blank(&state).await;
    };
    // get_optimized_image: an SVG is served as it is.
    let (url, attachment) = if upload.extension.as_deref() == Some("svg") {
        (upload.url.clone(), true)
    } else {
        match crate::optimized_images::get_optimized_image(
            &mut conn, &settings, &store, &upload, size,
        )
        .await?
        {
            Some(optimized) => (optimized.url, false),
            None => return render_blank(&state).await,
        }
    };
    // An external store's file: redirect_s3_avatar, or proxy_avatar
    // (Rails downloads its url; the port reads the object).
    if store.path_for(&url).is_none() && store.external() {
        let cdn = store.cdn_url(&url);
        if state
            .config
            .globals
            .get("redirect_avatar_requests")
            .is_some_and(|v| v == "true")
        {
            let location = if cdn.starts_with("//") {
                format!(
                    "{}:{cdn}",
                    if settings.get("force_https")?.truthy() {
                        "https"
                    } else {
                        "http"
                    }
                )
            } else {
                cdn
            };
            let mut response = (StatusCode::FOUND, [(header::LOCATION, location)]).into_response();
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static(
                    "max-age=3600, public, immutable, stale-while-revalidate=86400",
                ),
            );
            return Ok(response);
        }
        let Some(bytes) = store.read(&url).await? else {
            return render_blank(&state).await;
        };
        let created_at: chrono::NaiveDateTime =
            sqlx::query_scalar("SELECT created_at FROM uploads WHERE id = $1")
                .bind(upload.id)
                .fetch_one(&mut *conn)
                .await?;
        let modified = created_at
            .and_utc()
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string();
        let len = bytes.len();
        let mut response = png_modified(
            bytes,
            "max-age=31556952, public, immutable",
            HeaderValue::from_str(&modified)?,
        );
        let h = response.headers_mut();
        h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
        if let Some(kind) = crate::file_store::mime_type(&url) {
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static(kind));
        }
        if attachment {
            h.insert(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("sandbox;"),
            );
            h.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("image/svg+xml"),
            );
            h.insert(
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&format!(
                    "attachment; filename=\"{}\"",
                    url.rsplit('/').next().unwrap_or_default()
                ))?,
            );
        }
        return Ok(response);
    }
    let Some(path) = store.path_for(&url) else {
        return render_blank(&state).await;
    };
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return render_blank(&state).await,
        Err(e) => return Err(e.into()),
    };
    let mut response = png_modified(
        bytes,
        "max-age=31556952, public, immutable",
        HeaderValue::from_str(&ctime_httpdate(&path).await?)?,
    );
    if attachment {
        // FileHelper.is_inline_safe? is false for SVG.
        let headers = response.headers_mut();
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("sandbox;"),
        );
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("image/svg+xml"),
        );
        headers.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&format!(
                "attachment; filename=\"{}\"",
                path.file_name().unwrap_or_default().to_string_lossy()
            ))?,
        );
    }
    Ok(response)
}

/// `redirect_to` a site path: made absolute with the request's host.
fn found(headers: &HeaderMap, location: &str) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let mut response = (
        StatusCode::FOUND,
        [(header::LOCATION, format!("http://{host}{location}"))],
    )
        .into_response();
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}

/// `File.ctime(path).httpdate`
async fn ctime_httpdate(path: &FsPath) -> std::io::Result<String> {
    use std::os::unix::fs::MetadataExt;
    let ctime = tokio::fs::metadata(path).await?.ctime();
    Ok(chrono::DateTime::from_timestamp(ctime, 0)
        .unwrap_or_default()
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string())
}

/// A path segment, percent-encoded so a decoded `/` or `..` stays inside
/// its segment of the CDN URL.
fn segment(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The body, or None for an HTTP error status or one over MAX_FILE_SIZE
/// (both render the blank avatar in Rails).
async fn download(url: &str) -> Result<Option<Vec<u8>>, reqwest::Error> {
    let mut response = CLIENT.get(url).send().await?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_FILE_SIZE {
            return Ok(None);
        }
    }
    Ok(Some(body))
}

/// Writes through a temporary name so a concurrent reader never sees a
/// partial file, then evicts as `DiskCacheEviction.evict` does.
async fn store(dir: &FsPath, path: &FsPath, bytes: &[u8]) -> Result<(), AppError> {
    tokio::fs::create_dir_all(dir).await?;
    let tmp = path.with_extension(format!("png.{}.tmp", std::process::id()));
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, path).await?;
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || evict(&dir)).await??;
    Ok(())
}

/// `DiskCacheEviction.evict`: past max_entries files, the oldest
/// evict_count by mtime go.
fn evict(dir: &FsPath) -> std::io::Result<()> {
    let files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    if files.len() <= PROXY_CACHE_MAX_ENTRIES {
        return Ok(());
    }
    let mut dated: Vec<(std::time::SystemTime, PathBuf)> = files
        .into_iter()
        .map(|f| {
            let mtime = std::fs::metadata(&f)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (mtime, f)
        })
        .collect();
    dated.sort();
    for (_, f) in dated.into_iter().take(PROXY_CACHE_EVICT_COUNT) {
        match std::fs::remove_file(&f) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// `render_blank`: public/images/avatar.png for ten minutes.
async fn render_blank(state: &AppState) -> Result<Response, AppError> {
    let mut candidates = vec![state.config.public_dir.join("images/avatar.png")];
    if let Some(src) = &state.config.discourse_src {
        candidates.push(src.join("public/images/avatar.png"));
    }
    for path in candidates {
        match tokio::fs::read(&path).await {
            Ok(bytes) => return Ok(png(bytes, "max-age=600, public")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(StatusCode::NOT_FOUND.into_response())
}

/// send_file of a png, with `apply_cdn_headers`.
fn png(bytes: Vec<u8>, cache_control: &'static str) -> Response {
    png_modified(
        bytes,
        cache_control,
        HeaderValue::from_static(LAST_MODIFIED),
    )
}

fn png_modified(bytes: Vec<u8>, cache_control: &'static str, modified: HeaderValue) -> Response {
    let mut response = Response::new(Body::from(bytes));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    headers.insert(header::LAST_MODIFIED, modified);
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_cannot_leave_their_place() {
        assert_eq!(segment("a"), "a");
        assert_eq!(segment("../x"), "%2E%2E%2Fx");
        assert_eq!(segment("é"), "%C3%A9");
    }

    #[test]
    fn eviction_removes_the_oldest_past_the_limit() {
        let dir = std::env::temp_dir().join(format!("avatar-evict-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..PROXY_CACHE_MAX_ENTRIES + 1 {
            std::fs::write(dir.join(format!("{i}.png")), b"").unwrap();
        }
        let oldest = dir.join("0.png");
        let old = std::time::SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&oldest)
            .unwrap()
            .set_modified(old)
            .unwrap();
        evict(&dir).unwrap();
        let left = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(left, PROXY_CACHE_MAX_ENTRIES + 1 - PROXY_CACHE_EVICT_COUNT);
        assert!(!oldest.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
