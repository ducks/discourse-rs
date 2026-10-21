//! Jobs::PullHotlinkedImages: the media a post hotlinks from other sites
//! downloaded into uploads (download_remote_images_to_local), each
//! attempt recorded in post_hotlinked_media, and the post processed again
//! when a record was added. Media the site serves itself are skipped
//! unless their upload is missing.
//!
//! Refused: downloading (HotlinkedMedia.download), secure upload urls.

use std::collections::HashMap;

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::file_store::FileStore;
use crate::hotlinked_media::{download_src_for, extract_candidates, normalize_src};
use crate::onebox::uri::RubyUri;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// The job's Sidekiq name.
pub const PULL_HOTLINKED_IMAGES: &str = "pull_hotlinked_images";
/// `Jobs::UpdateHotlinkedRaw`, which the job schedules for a post with
/// hotlinked media records.
pub const UPDATE_HOTLINKED_RAW: &str = "update_hotlinked_raw";

async fn load_settings(
    state: &AppState,
    conn: &mut PgConnection,
) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// `Jobs::PullHotlinkedImages#execute`
pub async fn pull_hotlinked_images(state: &AppState, args: &Value) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let mut s = load_settings(state, &mut conn).await?;
    let store = FileStore::for_site(&state.config, &s)?;
    if disable_if_low_on_disk_space(state, &mut conn, &s, &store).await? {
        s = load_settings(state, &mut conn).await?;
    }
    let post_id = match args.get("post_id") {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(v)) if !crate::ruby::is_blank(v) => Some(crate::ruby::to_i(v)),
        _ => None,
    };
    let Some(post_id) = post_id else {
        return Err(
            Unsupported("pull_hotlinked_images without a post_id (InvalidParameters)").into(),
        );
    };
    // DistributedMutex "pull_hotlinked_images_<post_id>", held by a
    // transaction of its own while the job works on its connection.
    let mut mutex = state.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("pull_hotlinked_images_{post_id}"))
        .execute(&mut *mutex)
        .await?;
    pull(state, &mut conn, &s, &store, post_id).await?;
    mutex.commit().await?;
    Ok(())
}

/// `pull_hotlinked_images`
async fn pull(
    state: &AppState,
    conn: &mut PgConnection,
    s: &SiteSettings,
    store: &FileStore,
    post_id: i64,
) -> Result<(), AppError> {
    // Post.find_by(id:), and its topic (both without the trashed).
    let post: Option<(i32, String)> = sqlx::query_as(
        "SELECT p.id, p.cooked FROM posts p \
         JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL \
         WHERE p.id = $1 AND p.deleted_at IS NULL",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((post_id, cooked)) = post else {
        return Ok(());
    };
    // post.post_hotlinked_media.index_by(&:url)
    let records: Vec<(String, i64)> =
        sqlx::query_as("SELECT url, id FROM post_hotlinked_media WHERE post_id = $1")
            .bind(post_id)
            .fetch_all(&mut *conn)
            .await?;
    let hotlinked_map: HashMap<String, i64> = records.into_iter().collect();
    let local_bases = local_bases(state, s)?;
    let force_https = s.get("force_https")?.truthy();
    for original_src in extract_candidates(&cooked) {
        let download_src = download_src_for(&original_src, force_https)?;
        let normalized_src = normalize_src(&download_src, true)?;
        if !should_download_image(conn, s, store, &local_bases, &download_src).await? {
            continue;
        }
        if !hotlinked_map.contains_key(&normalized_src) {
            return Err(
                Unsupported("downloading hotlinked media (HotlinkedMedia.download)").into(),
            );
        }
    }
    if !hotlinked_map.is_empty() {
        let args = json!({"post_id": post_id});
        // Jobs.cancel_scheduled_job, then enqueue_in the editing grace
        // period and a second.
        sqlx::query(
            "DELETE FROM discourse_rs.jobs WHERE name = $1 AND failed_at IS NULL \
             AND locked_until IS NULL AND args = $2",
        )
        .bind(UPDATE_HOTLINKED_RAW)
        .bind(&args)
        .execute(&mut *conn)
        .await?;
        let delay = s.get("editing_grace_period")?.to_i() + 1;
        crate::jobs::enqueue_in(&mut *conn, delay, UPDATE_HOTLINKED_RAW, args).await?;
    }
    Ok(())
}

/// `Discourse.base_url`, `Discourse.asset_host` and external_emoji_url,
/// normalized: the prefixes of media this site serves.
fn local_bases(state: &AppState, s: &SiteSettings) -> Result<Vec<String>, AppError> {
    let urls = Urls {
        config: &state.config,
        settings: s,
    };
    let mut bases = vec![urls.base_url()?];
    if let Some(cdn) = state.config.globals.cdn_url() {
        bases.push(cdn.to_string());
    }
    let emoji = s.get("external_emoji_url")?.to_s();
    if !crate::ruby::is_blank(&emoji) {
        bases.push(emoji);
    }
    Ok(bases
        .iter()
        .map(|b| normalize_src(b, true))
        .collect::<Result<_, _>>()?)
}

/// `should_download_image?(src, post)`: a local url only when its upload
/// is missing; another site's when download_remote_images_to_local is on
/// and its host isn't in disabled_image_download_domains.
async fn should_download_image(
    conn: &mut PgConnection,
    s: &SiteSettings,
    store: &FileStore,
    local_bases: &[String],
    src: &str,
) -> Result<bool, AppError> {
    if crate::ruby::is_blank(src) {
        return Ok(false);
    }
    let normalized = normalize_src(src, true)?;
    let relative = src.starts_with('/') && !src.starts_with("//");
    if store.has_been_uploaded(src)
        || local_bases
            .iter()
            .any(|b| normalized.starts_with(b.as_str()))
        || relative
    {
        if secure_uploads_url(src) {
            return Err(Unsupported("secure upload urls in pull_hotlinked_images").into());
        }
        if !src.contains("/uploads/") {
            return Ok(false);
        }
        let upload = crate::upload_references::get_from_url(&mut *conn, src).await?;
        // Upload.consider_for_reuse(upload, post)
        if upload.is_some() && s.get("secure_uploads")?.truthy() {
            return Err(Unsupported("reusing uploads under secure uploads").into());
        }
        return Ok(upload.is_none());
    }
    if !s.get("download_remote_images_to_local")?.truthy() {
        return Ok(false);
    }
    let Some(uri) = RubyUri::parse(src) else {
        return Ok(false);
    };
    let Some(host) = uri.host else {
        return Ok(false);
    };
    // SiteSetting.should_download_images?(src)
    let disabled = s.get("disabled_image_download_domains")?.to_s();
    if crate::ruby::is_blank(&disabled) {
        return Ok(true);
    }
    Ok(!disabled.split('|').any(|d| d == host))
}

/// `Upload.secure_uploads_url?(url)`: a path the uploads#show_secure
/// route (or its deprecated secure-media-uploads form) takes, of a
/// supported media file.
fn secure_uploads_url(url: &str) -> bool {
    let Some(uri) = RubyUri::parse(url) else {
        return false;
    };
    (uri.path.starts_with("/secure-uploads/") || uri.path.starts_with("/secure-media-uploads/"))
        && crate::file_store::is_supported_media(url)
}

/// `disable_if_low_on_disk_space`: with a local store downloading remote
/// images, under download_remote_images_threshold percent of the
/// uploads' disk free turns the downloads off, logged by the system user,
/// and the site contact is told. Whether it did.
async fn disable_if_low_on_disk_space(
    state: &AppState,
    conn: &mut PgConnection,
    s: &SiteSettings,
    store: &FileStore,
) -> Result<bool, AppError> {
    if store.external() || !s.get("download_remote_images_to_local")?.truthy() {
        return Ok(false);
    }
    let threshold = s.get("download_remote_images_threshold")?.to_i();
    if available_disk_space(&state.config)? >= threshold {
        return Ok(false);
    }
    const NAME: &str = "download_remote_images_to_local";
    if let Err(message) = crate::site_setting_update::set(
        &mut *conn,
        &state.bus,
        s,
        &state.site_setting_defs,
        &state.i18n,
        NAME,
        &json!(false),
    )
    .await?
    {
        return Err(std::io::Error::other(message).into());
    }
    let reason = state
        .i18n
        .t("disable_remote_images_download_reason")
        .unwrap_or("disable_remote_images_download_reason")
        .to_string();
    crate::site_setting_update::log_site_setting_change_details(
        &mut *conn, -1, NAME, "true", "false", &reason,
    )
    .await?;
    let s = load_settings(state, &mut *conn).await?;
    let contact = site_contact_user(&mut *conn, &s).await?;
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    crate::system_message::create_from_system_user(
        &state.pool,
        &ctx,
        contact,
        "download_remote_images_disabled",
        &[],
    )
    .await?;
    Ok(true)
}

/// `Discourse.site_contact_user`: site_contact_username's user, else the
/// system user.
async fn site_contact_user(conn: &mut PgConnection, s: &SiteSettings) -> Result<i32, AppError> {
    let username = s.get("site_contact_username")?.to_s();
    if !crate::ruby::is_blank(&username) {
        let id: Option<i32> = sqlx::query_scalar("SELECT id FROM users WHERE username_lower = $1")
            .bind(username.to_lowercase())
            .fetch_optional(&mut *conn)
            .await?;
        if let Some(id) = id {
            return Ok(id);
        }
    }
    Ok(-1)
}

/// `available_disk_space`: 100 less `DiskSpace.percent_free` of
/// public/uploads, which reads `df -P`'s capacity column (the percent
/// used), so the percent free. The uploads directory is made on the
/// first upload here; until then its nearest existing parent is read.
fn available_disk_space(config: &crate::config::Config) -> Result<i64, AppError> {
    let uploads = config.public_dir.join("uploads");
    let existing = uploads
        .ancestors()
        .find(|p| p.exists())
        .unwrap_or(std::path::Path::new("/"));
    let out = std::process::Command::new("df")
        .arg("-P")
        .arg(existing)
        .output()?;
    if !out.status.success() {
        return Err(std::io::Error::other(format!(
            "df -P {}: {}",
            existing.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
        .into());
    }
    Ok(100 - df_capacity(&String::from_utf8_lossy(&out.stdout)))
}

/// `df -P`'s second line's fifth column, as `"92%".to_i` reads it.
fn df_capacity(output: &str) -> i64 {
    output
        .lines()
        .nth(1)
        .and_then(|l| l.split_whitespace().nth(4))
        .map_or(0, crate::ruby::to_i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_df_capacity_as_rails() {
        let out = "Filesystem     1024-blocks      Used Available Capacity Mounted on\n\
                   /dev/sda1        854645600 739815064  71343180      92% /\n";
        assert_eq!(df_capacity(out), 92);
        assert_eq!(df_capacity(""), 0);
    }

    #[test]
    fn recognizes_secure_upload_urls() {
        assert!(secure_uploads_url("/secure-uploads/original/1X/abc.png"));
        assert!(secure_uploads_url(
            "http://test.localhost/secure-media-uploads/a/b.jpg"
        ));
        assert!(!secure_uploads_url("/t/secure-uploads-are-cool/223452"));
        assert!(!secure_uploads_url("/uploads/default/original/1X/abc.png"));
    }
}
