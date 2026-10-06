//! Port of UserAvatarsController#show_proxy_letter
//! (app/controllers/user_avatars_controller.rb): the default
//! external_system_avatars_url points here, and each letter avatar is
//! fetched once from the avatar CDN and served from a disk cache after.

use std::path::{Path as FsPath, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sha1::{Digest, Sha1};

use crate::site_settings::SiteSettings;
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
    let mut response = Response::new(Body::from(bytes));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    headers.insert(
        header::LAST_MODIFIED,
        HeaderValue::from_static(LAST_MODIFIED),
    );
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
