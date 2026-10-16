//! The uploads' scheduled jobs: whether GIFs are animated
//! (Jobs::UpdateAnimatedUploads) and the dominant colour of uploads
//! without one (Jobs::BackfillDominantColors).

use crate::file_store::FileStore;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `Jobs::UpdateAnimatedUploads`, hourly.
pub const UPDATE_ANIMATED_UPLOADS: &str = "update_animated_uploads";
/// `Jobs::BackfillDominantColors`, every 15 minutes.
pub const BACKFILL_DOMINANT_COLORS: &str = "backfill_dominant_colors";

/// `UpdateAnimatedUploads::MAX_PROCESSED_GIF_IMAGES`
const MAX_PROCESSED_GIF_IMAGES: i64 = 200;
/// `Upload.backfill_dominant_colors!(count)`'s count.
const DOMINANT_COLORS_BATCH: i64 = 25;

/// A url with a scheme: fetched over HTTP by FastImage, which the port
/// does not do.
fn remote(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

async fn store(state: &AppState) -> Result<(SiteSettings, FileStore), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let store = FileStore::for_site(&state.config, &s)?;
    Ok((s, store))
}

/// `Jobs::UpdateAnimatedUploads`: GIFs not yet checked, each marked
/// animated or not by its content (FastImage.animated?); an animated one's
/// optimized images go. An upload whose file is missing, or whose type
/// has no animation check, stays unchecked.
pub async fn update_animated_uploads(state: &AppState) -> Result<(), AppError> {
    let (_, store) = store(state).await?;
    let rows: Vec<(i32, String)> = sqlx::query_as(
        "SELECT id, url FROM uploads \
         WHERE (extension = 'gif' OR (extension IS NULL AND original_filename LIKE '%.gif')) \
           AND uploads.animated IS NULL LIMIT $1",
    )
    .bind(MAX_PROCESSED_GIF_IMAGES)
    .fetch_all(&state.pool)
    .await?;
    for (id, url) in rows {
        if remote(&url) {
            return Err(Unsupported("checking remote uploads for animation").into());
        }
        // FastImage.animated?(path_for(upload) || url): a protocol-relative
        // url (an external store's) reads as a missing file, so nil, and
        // update!(animated: nil) changes nothing.
        if url.starts_with("//") {
            continue;
        }
        let Some(bytes) = store.read(&url).await? else {
            continue;
        };
        let animated = match crate::images::info(&bytes) {
            Some(info) if info.format != crate::images::Format::Jpeg => info.animated,
            _ => continue,
        };
        let mut tx = state.pool.begin().await?;
        sqlx::query(
            "UPDATE uploads SET animated = $2, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(id)
        .bind(animated)
        .execute(&mut *tx)
        .await?;
        if animated {
            // optimized_images.destroy_all: each file to the tombstone,
            // then the row.
            let optimized: Vec<(i32, String)> =
                sqlx::query_as("SELECT id, url FROM optimized_images WHERE upload_id = $1")
                    .bind(id)
                    .fetch_all(&mut *tx)
                    .await?;
            for (optimized_id, optimized_url) in optimized {
                store.remove_file(&optimized_url).await?;
                sqlx::query("DELETE FROM optimized_images WHERE id = $1")
                    .bind(optimized_id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;
    }
    Ok(())
}

/// `Upload.backfill_dominant_colors!(25)`: the newest uploads without a
/// colour, each given its file's (empty for a type ImageMagick is not
/// asked about, or a missing or undecodable file), without touching
/// updated_at.
pub async fn backfill_dominant_colors(state: &AppState) -> Result<(), AppError> {
    let (_, store) = store(state).await?;
    if state
        .config
        .globals
        .get("enable_vips_image_processing")
        .is_some_and(|v| v == "true")
    {
        return Err(Unsupported("dominant colours with libvips").into());
    }
    let rows: Vec<(i32, String, Option<String>)> = sqlx::query_as(
        "SELECT id, url, extension FROM uploads WHERE uploads.dominant_color IS NULL \
         ORDER BY id DESC LIMIT $1",
    )
    .bind(DOMINANT_COLORS_BATCH)
    .fetch_all(&state.pool)
    .await?;
    for (id, url, extension) in rows {
        let ext = extension.unwrap_or_default();
        // calculate_dominant_color!
        let supported = crate::uploads::is_supported_image(&format!("image.{ext}"))
            && !matches!(ext.as_str(), "svg" | "ico");
        let color = if !supported {
            String::new()
        } else {
            // path_for, or the external store's download (from the bucket,
            // where Rails fetches the public url).
            if remote(&url) {
                return Err(Unsupported("downloading remote uploads").into());
            }
            match store.read(&url).await? {
                None => String::new(),
                Some(bytes) => match crate::images::sniff(&bytes) {
                    Some(format) => tokio::task::spawn_blocking(move || {
                        crate::images::dominant_color(&bytes, format).unwrap_or_default()
                    })
                    .await
                    .map_err(std::io::Error::other)?,
                    // ImageMagick's other delegates decide these.
                    None if matches!(
                        ext.to_lowercase().as_str(),
                        "avif" | "heic" | "heif" | "jxl"
                    ) =>
                    {
                        return Err(
                            Unsupported("dominant colours of AVIF, HEIC and JPEG XL").into()
                        );
                    }
                    None => String::new(),
                },
            }
        };
        sqlx::query("UPDATE uploads SET dominant_color = $2 WHERE id = $1")
            .bind(id)
            .bind(&color)
            .execute(&state.pool)
            .await?;
    }
    Ok(())
}
