//! OptimizedImage: an upload's thumbnails, made by `OptimizedImage.create_
//! for` (here with src/images.rs in place of ImageMagick) and stored by the
//! local FileStore under `optimized/`, and the Upload methods that find and
//! create them.

use crate::file_store::FileStore;

use sha1::{Digest, Sha1};
use sqlx::PgConnection;

use crate::Unsupported;
use crate::images::{self, Format};
use crate::pretty_text::CookError;
use crate::site_settings::SiteSettings;

/// `OptimizedImage::VERSION`
pub const VERSION: i32 = 2;

/// The upload columns thumbnails and the post processor read.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Upload {
    pub id: i32,
    pub url: String,
    pub sha1: Option<String>,
    pub extension: Option<String>,
    pub original_filename: String,
    pub filesize: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub animated: Option<bool>,
    pub dominant_color: Option<String>,
}

impl Upload {
    pub async fn find(conn: &mut PgConnection, id: i32) -> Result<Option<Upload>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, url, sha1, extension, original_filename, filesize, width, height, animated, \
                    dominant_color FROM uploads WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(conn)
        .await
    }
}

/// An optimized image's row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Optimized {
    pub id: i32,
    pub url: String,
    pub width: i32,
    pub height: i32,
    pub filesize: Option<i32>,
    pub version: Option<i32>,
}

/// `Upload#thumbnail(width, height)`: the optimized image of that size.
pub async fn thumbnail(
    conn: &mut PgConnection,
    upload_id: i32,
    width: i32,
    height: i32,
) -> Result<Option<Optimized>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, url, width, height, filesize, version FROM optimized_images \
         WHERE upload_id = $1 AND width = $2 AND height = $3 ORDER BY id LIMIT 1",
    )
    .bind(upload_id)
    .bind(width)
    .bind(height)
    .fetch_optional(conn)
    .await
}

/// `OptimizedImage::IM_DECODERS`
fn decodable(extension: &str) -> bool {
    matches!(
        extension.to_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "svg"
    )
}

/// `Upload#create_thumbnail!`: nothing unless create_thumbnails is on.
pub async fn create_thumbnail(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    store: &FileStore,
    upload: &Upload,
    width: i32,
    height: i32,
    crop: bool,
) -> Result<Option<Optimized>, CookError> {
    if !settings.get("create_thumbnails")?.truthy() {
        return Ok(None);
    }
    create_for(conn, settings, store, upload, width, height, crop).await
}

/// `Topic.share_thumbnail_size`, the one `Topic.thumbnail_sizes` has
/// without plugins.
const SHARE_THUMBNAIL_SIZE: (i32, i32) = (1024, 1024);

/// `Topic#generate_thumbnails!` for the topic's image upload: a
/// TopicThumbnail per size (`TopicThumbnail.find_or_create_for!`), with an
/// optimized image when the original is larger. Themes' extra thumbnail
/// sizes are not supported.
pub async fn generate_topic_thumbnails(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    store: &FileStore,
    upload: &Upload,
) -> Result<(), CookError> {
    if !settings.get("create_thumbnails")?.truthy() {
        return Ok(());
    }
    if upload.filesize >= settings.get("max_image_size_kb")?.to_i() * 1024 {
        return Ok(());
    }
    let (Some(width), Some(height)) = (upload.width, upload.height) else {
        return Ok(());
    };
    // ThemeModifierHelper#topic_thumbnail_sizes over user-selectable themes.
    let theme_sizes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM theme_modifier_sets m JOIN themes t ON t.id = m.theme_id \
         WHERE t.user_selectable AND m.topic_thumbnail_sizes IS NOT NULL \
         AND m.topic_thumbnail_sizes <> '{}'",
    )
    .fetch_one(&mut *conn)
    .await?;
    if theme_sizes > 0 {
        return Err(Unsupported("theme topic thumbnail sizes").into());
    }
    let (max_width, max_height) = SHARE_THUMBNAIL_SIZE;
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM topic_thumbnails WHERE upload_id = $1 AND max_width = $2 AND max_height = $3",
    )
    .bind(upload.id)
    .bind(max_width)
    .bind(max_height)
    .fetch_optional(&mut *conn)
    .await?;
    if existing.is_some() {
        return Ok(());
    }
    // ImageSizer.resize within the thumbnail's bounds.
    let (w, h) = (f64::from(width), f64::from(height));
    let (target_width, target_height) = if w <= f64::from(max_width) && h <= f64::from(max_height) {
        (width, height)
    } else {
        let ratio = (f64::from(max_width) / w).min(f64::from(max_height) / h);
        ((w * ratio).floor() as i32, (h * ratio).floor() as i32)
    };
    let needs_optimization = target_width < width && target_height < height;
    let skip_animated =
        settings.get("animated_topic_thumbnails")?.truthy() && upload.animated == Some(true);
    let optimized = if needs_optimization && !skip_animated {
        create_for(
            conn,
            settings,
            store,
            upload,
            target_width,
            target_height,
            false,
        )
        .await?
    } else {
        None
    };
    sqlx::query(
        "INSERT INTO topic_thumbnails (upload_id, max_width, max_height, optimized_image_id) \
         VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
    )
    .bind(upload.id)
    .bind(max_width)
    .bind(max_height)
    .bind(optimized.map(|o| o.id))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `OptimizedImage.create_for(upload, width, height, crop:)`: the existing
/// thumbnail of that size and extension, else one made from the stored
/// original. None when it cannot be made (Rails logs and moves on).
pub async fn create_for(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    store: &FileStore,
    upload: &Upload,
    width: i32,
    height: i32,
    crop: bool,
) -> Result<Option<Optimized>, CookError> {
    if width <= 0 || height <= 0 {
        return Ok(None);
    }
    let Some(sha1) = upload.sha1.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let Some(ext) = upload.extension.as_deref().filter(|e| !e.is_empty()) else {
        return Err(Unsupported("fixing an upload's missing extension").into());
    };
    // get_optimized_image raises on an extension ImageMagick cannot read,
    // and gives up when fixing it changes nothing.
    if !decodable(ext) {
        return Ok(None);
    }
    let extension = format!(".{ext}");
    let existing: Option<Optimized> = sqlx::query_as(
        "SELECT id, url, width, height, filesize, version FROM optimized_images \
         WHERE upload_id = $1 AND width = $2 AND height = $3 AND extension = $4",
    )
    .bind(upload.id)
    .bind(width)
    .bind(height)
    .bind(&extension)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(t) = existing {
        if t.url.is_empty() || t.version != Some(VERSION) {
            return Err(Unsupported("replacing an outdated optimized image").into());
        }
        return Ok(Some(t));
    }
    let Some(format) = Format::from_extension(ext) else {
        return Err(Unsupported("thumbnails of SVG or AVIF uploads").into());
    };
    // store.path_for(upload); a remote store would download it.
    let Some(original) = store.read(&upload.url).await? else {
        tracing::error!(url = %upload.url, "could not find the file in the store");
        return Ok(None);
    };
    // image_preview_jpg_quality, else image_quality.
    let quality = match settings.get("image_preview_jpg_quality")?.to_i() {
        0 => settings.get("image_quality")?.to_i(),
        q => q,
    }
    .clamp(1, 100) as u8;
    let (w, h) = (width as u32, height as u32);
    let bytes = tokio::task::spawn_blocking(move || {
        images::thumbnail(&original, format, w, h, crop, quality)
    })
    .await
    .map_err(std::io::Error::other)?;
    let Some(bytes) = bytes else {
        tracing::warn!(upload_id = upload.id, "failed to optimize image");
        return Ok(None);
    };
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO optimized_images (upload_id, sha1, extension, width, height, url, filesize, version, \
                                       created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, '', $6, $7, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(upload.id)
    .bind(format!("{:x}", Sha1::digest(&bytes)))
    .bind(&extension)
    .bind(width)
    .bind(height)
    .bind(bytes.len() as i32)
    .bind(VERSION)
    .fetch_one(&mut *conn)
    .await?;
    // store_optimized_image, then the url saved.
    let url = store
        .store_optimized_image(
            &bytes,
            upload.id,
            sha1,
            &format!("_{VERSION}_{width}x{height}{extension}"),
        )
        .await?;
    sqlx::query(
        "UPDATE optimized_images SET url = $2, updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(id)
    .bind(&url)
    .execute(&mut *conn)
    .await?;
    Ok(Some(Optimized {
        id,
        url,
        width,
        height,
        filesize: Some(bytes.len() as i32),
        version: Some(VERSION),
    }))
}
