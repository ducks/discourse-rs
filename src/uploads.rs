//! Uploads: UploadsController#create through UploadCreator, UploadValidator
//! and the local FileStore, and UploadSerializer.
//!
//! Attachments and GIFs are stored as sent. PNG, JPEG and WebP go through
//! UploadCreator's steps (conversion to JPEG, re-encoding, orientation,
//! optimization, downsizing) with the in-process tools of src/images.rs,
//! which make Rails' decisions but not always its bytes. Refused: SVG
//! cleaning, HEIF conversion, cropped upload types.

use std::path::Path;

use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use sqlx::PgConnection;

use crate::file_store::FileStore;
use crate::images;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// `FileHelper.supported_images`
const SUPPORTED_IMAGES: [&str; 11] = [
    "jpg", "jpeg", "png", "gif", "svg", "ico", "webp", "avif", "heic", "heif", "jxl",
];

/// `UploadCreator::TYPES_TO_CROP`
const TYPES_TO_CROP: [&str; 4] = [
    "avatar",
    "card_background",
    "custom_emoji",
    "profile_background",
];

/// `FileHelper.is_supported_image?(filename)`
pub fn is_supported_image(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    SUPPORTED_IMAGES
        .iter()
        .any(|ext| lower.ends_with(&format!(".{ext}")))
}

/// The image type FastImage reads from a file's first bytes.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ImageType {
    Gif,
    Png,
    Jpeg,
    Webp,
    Bmp,
    Ico,
    Tiff,
    Other,
}

impl ImageType {
    fn name(self) -> &'static str {
        match self {
            ImageType::Gif => "gif",
            ImageType::Png => "png",
            ImageType::Jpeg => "jpeg",
            ImageType::Webp => "webp",
            ImageType::Bmp => "bmp",
            ImageType::Ico => "ico",
            ImageType::Tiff => "tiff",
            ImageType::Other => "other",
        }
    }
}

/// `FastImage.new(file).type`, for the formats it tells apart by magic.
fn detect(bytes: &[u8]) -> Option<ImageType> {
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ImageType::Gif)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageType::Png)
    } else if bytes.starts_with(b"\xff\xd8") {
        Some(ImageType::Jpeg)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageType::Webp)
    } else if bytes.starts_with(b"BM") {
        Some(ImageType::Bmp)
    } else if bytes.starts_with(b"\x00\x00\x01\x00") {
        Some(ImageType::Ico)
    } else if bytes.starts_with(b"II*\x00") || bytes.starts_with(b"MM\x00*") {
        Some(ImageType::Tiff)
    } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        // HEIC, AVIF and other ISO media
        Some(ImageType::Other)
    } else {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_lowercase();
        head.contains("<svg").then_some(ImageType::Other)
    }
}

/// `ImageSizer.resize(w, h)`: within max_image_width and
/// max_image_height, keeping the ratio.
fn image_sizer_resize(s: &SiteSettings, w: u32, h: u32) -> Result<(i64, i64), AppError> {
    let max_w = s.get("max_image_width")?.to_i() as f64;
    let max_h = s.get("max_image_height")?.to_i() as f64;
    let (w, h) = (w as f64, h as f64);
    if w <= max_w && h <= max_h {
        return Ok((w.floor() as i64, h.floor() as i64));
    }
    let ratio = (max_w / w).min(max_h / h);
    Ok(((w * ratio).floor() as i64, (h * ratio).floor() as i64))
}

/// `ActiveSupport::NumberHelper.number_to_human_size`
pub fn human_size(bytes: i64) -> String {
    if bytes.abs() < 1024 {
        return format!("{bytes} {}", if bytes == 1 { "Byte" } else { "Bytes" });
    }
    let units = ["KB", "MB", "GB", "TB", "PB", "EB"];
    let mut exponent = ((bytes as f64).ln() / 1024f64.ln()).floor() as i32;
    exponent = exponent.clamp(1, units.len() as i32);
    let value = bytes as f64 / 1024f64.powi(exponent);
    // three significant digits, half up, insignificant zeros stripped
    let digits = value.abs().log10().floor() as i32 + 1;
    let decimals = (3 - digits).max(0);
    let factor = 10f64.powi(decimals);
    let rounded = (value * factor + 0.5).floor() / factor;
    let mut text = format!("{rounded:.*}", decimals as usize);
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    format!("{text} {}", units[(exponent - 1) as usize])
}

/// `UploadValidator`'s extension list (`extensions_to_set`).
fn extensions_to_set(value: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let cleaned: String = value
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '.')
        .collect::<String>()
        .to_lowercase();
    for ext in cleaned.split('|') {
        if !ext.contains('*') && !out.contains(&ext.to_string()) {
            out.push(ext.to_string());
        }
    }
    out
}

/// `UploadCreator::MIN_PIXELS_TO_CONVERT_TO_JPEG`, `MIN_CONVERT_TO_JPEG_
/// BYTES_SAVED` and `MIN_CONVERT_TO_JPEG_SAVING_RATIO`.
const MIN_PIXELS_TO_CONVERT_TO_JPEG: u64 = 1280 * 720;
const MIN_CONVERT_TO_JPEG_BYTES_SAVED: usize = 75_000;
const MIN_CONVERT_TO_JPEG_SAVING_RATIO: f64 = 0.70;

/// What the image steps read from the settings and the request.
#[derive(Clone, Copy)]
struct ImageOpts {
    pasted: bool,
    /// max_image_megapixels in pixels, 0 for no limit.
    max_pixels: u64,
    /// max_image_size_kb in bytes, 0 for no limit.
    max_size: usize,
    png_to_jpg_quality: u8,
    recompress_quality: u8,
}

/// Why an image upload is refused, as UploadCreator's errors.
enum ImageRefusal {
    Corrupted,
    SizeNotFound,
    TooManyPixels,
    TooLarge,
    OptimizeFailure,
}

/// `extract_image_info!`
fn extract_image_info(bytes: &[u8], opts: &ImageOpts) -> Result<images::Info, ImageRefusal> {
    let info = images::info(bytes).ok_or(ImageRefusal::Corrupted)?;
    if info.pixels() == 0 {
        return Err(ImageRefusal::SizeNotFound);
    }
    if opts.max_pixels > 0 && info.pixels() >= opts.max_pixels {
        return Err(ImageRefusal::TooManyPixels);
    }
    Ok(info)
}

/// `replace_with_jpeg_if_sufficiently_smaller!`
fn replace_with_jpeg(bytes: &mut Vec<u8>, info: &mut images::Info, quality: u8, opts: &ImageOpts) {
    if bytes.len() < MIN_CONVERT_TO_JPEG_BYTES_SAVED {
        return;
    }
    let Some(jpeg) = images::to_jpeg(bytes, info.format, quality) else {
        return;
    };
    let keep = (jpeg.len() as f64) < bytes.len() as f64 * MIN_CONVERT_TO_JPEG_SAVING_RATIO
        && bytes.len() - jpeg.len() > MIN_CONVERT_TO_JPEG_BYTES_SAVED;
    if keep && let Ok(new_info) = extract_image_info(&jpeg, opts) {
        *bytes = jpeg;
        *info = new_info;
    }
}

/// UploadCreator#create_for's image steps, in its order: PNG to JPEG
/// (large or pasted PNGs), JPEG re-encoding (over the recompress
/// quality), orientation, optimization and downsizing, then the size
/// check. GIFs are stored as sent.
fn process_image(
    mut bytes: Vec<u8>,
    opts: &ImageOpts,
) -> Result<(Vec<u8>, images::Info), ImageRefusal> {
    use images::Format;
    let mut info = extract_image_info(&bytes, opts)?;
    if info.format == Format::Gif {
        return Ok((bytes, info));
    }
    match info.format {
        // convert_png_to_jpeg?
        Format::Png
            if opts.png_to_jpg_quality != 100
                && (opts.pasted || info.pixels() > MIN_PIXELS_TO_CONVERT_TO_JPEG) =>
        {
            replace_with_jpeg(&mut bytes, &mut info, opts.png_to_jpg_quality, opts);
        }
        // should_alter_jpeg_quality?: target_jpeg_image_quality is present
        // when the file's quality is unknown or above the target.
        Format::Jpeg => {
            let quality = images::jpeg_quality(&bytes);
            if quality == 0 || quality > u32::from(opts.recompress_quality) {
                replace_with_jpeg(&mut bytes, &mut info, opts.recompress_quality, opts);
            }
        }
        _ => {}
    }
    // fix_orientation!
    if info.format == Format::Jpeg && info.orientation > 1 {
        let upright = images::fix_orientation(&bytes).ok_or(ImageRefusal::Corrupted)?;
        info = extract_image_info(&upright, opts)?;
        bytes = upright;
    }
    // should_optimize?: never GIF; PNG under two megapixels.
    if (info.format != Format::Png || info.pixels() < 2_000_000)
        && let Some(optimized) = images::optimize(&bytes, info.format, false)
    {
        info = extract_image_info(&optimized, opts)?;
        bytes = optimized;
    }
    // should_downsize? / downsize!: halved up to three times.
    let should_downsize = |bytes: &[u8], info: &images::Info| {
        opts.max_size > 0 && bytes.len() >= opts.max_size && !info.animated
    };
    if should_downsize(&bytes, &info) {
        for _ in 0..3 {
            let original_size = bytes.len();
            let smaller =
                images::downsize_half(&bytes, info.format).ok_or(ImageRefusal::OptimizeFailure)?;
            info = extract_image_info(&smaller, opts).map_err(|_| ImageRefusal::OptimizeFailure)?;
            bytes = smaller;
            if bytes.len() >= original_size || info.pixels() == 0 || !should_downsize(&bytes, &info)
            {
                break;
            }
        }
    }
    // is_still_too_big?
    if opts.max_pixels > 0 && info.pixels() >= opts.max_pixels {
        return Err(ImageRefusal::TooManyPixels);
    }
    if opts.max_size > 0 && bytes.len() >= opts.max_size {
        return Err(ImageRefusal::TooLarge);
    }
    Ok((bytes, info))
}

/// What an upload request carried.
pub struct NewUpload<'a> {
    pub user_id: i32,
    pub staff: bool,
    pub upload_type: &'a str,
    pub filename: &'a str,
    pub bytes: &'a [u8],
    /// `params[:pasted] == "true"`: pasted from the clipboard.
    pub pasted: bool,
}

/// How an upload ends: the serialized upload or its errors.
pub enum Outcome {
    Created(Value),
    Invalid(Vec<String>),
}

/// `UploadsController.create_upload` -> `UploadCreator#create_for`.
pub async fn create(
    conn: &mut PgConnection,
    s: &SiteSettings,
    i18n: &crate::i18n::I18n,
    urls: &Urls<'_>,
    store: &FileStore,
    up: &NewUpload<'_>,
) -> Result<Outcome, AppError> {
    let t = |key: &str| i18n.t(key).unwrap_or(key).to_string();
    if up.bytes.is_empty() {
        return Ok(Outcome::Invalid(vec![t("upload.empty")]));
    }
    if up.upload_type == "avatar" {
        return Err(Unsupported("avatar uploads").into());
    }
    if s.get("secure_uploads")?.truthy() {
        return Err(Unsupported("secure uploads").into());
    }
    let detected = detect(up.bytes);
    let is_image = is_supported_image(up.filename)
        || detected.is_some_and(|d| is_supported_image(&format!("test.{}", d.name())));
    let mut image: Option<(u32, u32, bool)> = None;
    let mut image_type: Option<&'static str> = None;
    let mut bytes: Vec<u8> = up.bytes.to_vec();
    if is_image {
        match detected {
            Some(ImageType::Gif | ImageType::Png | ImageType::Jpeg | ImageType::Webp) => {}
            Some(_) => return Err(Unsupported("uploading this image type").into()),
            None => {
                return Ok(Outcome::Invalid(vec![t(
                    "upload.images.not_supported_or_corrupted",
                )]));
            }
        }
        if TYPES_TO_CROP.contains(&up.upload_type) {
            return Err(Unsupported("cropped upload types").into());
        }
        if !s.get("strip_image_metadata")?.truthy() {
            return Err(Unsupported("keeping image metadata (strip_image_metadata off)").into());
        }
        let quality = |name: &str| -> Result<u8, AppError> {
            // SiteSetting::ImageQuality: the setting, else image_quality.
            let value = match s.get(name)?.to_i() {
                0 => s.get("image_quality")?.to_i(),
                q => q,
            };
            Ok(value.clamp(1, 100) as u8)
        };
        let opts = ImageOpts {
            pasted: up.pasted,
            max_pixels: (s.get("max_image_megapixels")?.to_f() * 1_000_000.0) as u64,
            max_size: (s.get("max_image_size_kb")?.to_i() * 1024).max(0) as usize,
            png_to_jpg_quality: quality("png_to_jpg_quality")?,
            recompress_quality: quality("recompress_original_jpg_quality")?,
        };
        let processed = tokio::task::spawn_blocking(move || process_image(bytes, &opts))
            .await
            .map_err(std::io::Error::other)?;
        let (processed, info) = match processed {
            Ok(done) => done,
            Err(error) => {
                let message = match error {
                    ImageRefusal::Corrupted => t("upload.images.not_supported_or_corrupted"),
                    ImageRefusal::SizeNotFound => t("upload.images.size_not_found"),
                    ImageRefusal::OptimizeFailure => t("upload.optimize_failure_message"),
                    ImageRefusal::TooManyPixels => i18n
                        .t_with(
                            "upload.images.larger_than_x_megapixels",
                            &[
                                (
                                    "max_image_megapixels",
                                    &s.get("max_image_megapixels")?.to_s(),
                                ),
                                ("original_filename", up.filename),
                            ],
                        )
                        .unwrap_or_default(),
                    ImageRefusal::TooLarge => i18n
                        .t_with(
                            "upload.images.too_large_humanized",
                            &[("max_size", &human_size(opts.max_size as i64))],
                        )
                        .unwrap_or_default(),
                };
                return Ok(Outcome::Invalid(vec![message]));
            }
        };
        bytes = processed;
        image = Some((info.width, info.height, info.animated));
        image_type = Some(info.format.name());
    }

    let sha1 = format!("{:x}", Sha1::digest(&bytes));
    // do we already have that upload?
    let existing: Option<(i32, String)> =
        sqlx::query_as("SELECT id, url FROM uploads WHERE sha1 = $1")
            .bind(&sha1)
            .fetch_optional(&mut *conn)
            .await?;
    if let Some((id, url)) = existing {
        if url.is_empty() {
            return Err(Unsupported("replacing a failed upload").into());
        }
        sqlx::query(
            "INSERT INTO user_uploads (user_id, upload_id, created_at) \
             SELECT $1, $2, clock_timestamp() \
             WHERE NOT EXISTS (SELECT 1 FROM user_uploads WHERE user_id = $1 AND upload_id = $2)",
        )
        .bind(up.user_id)
        .bind(id)
        .execute(&mut *conn)
        .await?;
        return Ok(Outcome::Created(serialize(conn, s, urls, id).await?));
    }

    // The original filename, its extension corrected for an image.
    let mut original_filename = up.filename.to_string();
    if let Some(kind) = image_type {
        let ext_of = |name: &str| {
            Path::new(name)
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
                .unwrap_or_default()
                .replace("jpeg", "jpg")
        };
        let current = ext_of(up.filename);
        let expected = format!(".{kind}").replace("jpeg", "jpg");
        if current != expected {
            let stem = up.filename.strip_suffix(&current).unwrap_or(up.filename);
            let stem = if stem.is_empty() { "image" } else { stem };
            original_filename = format!("{stem}{expected}");
        }
    }
    let file_ext = Path::new(up.filename)
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    let extension: String = match image_type {
        Some(kind) => kind.to_string(),
        None => file_ext.chars().take(10).collect(),
    };

    // UploadValidator on save.
    let errors = validate(s, i18n, up, &original_filename, bytes.len())?;
    if !errors.is_empty() {
        return Ok(Outcome::Invalid(errors));
    }

    let (width, height, thumb, animated) = match image {
        Some((w, h, animated)) => (
            Some(w as i64),
            Some(h as i64),
            Some(image_sizer_resize(s, w, h)?),
            Some(animated),
        ),
        None => (None, None, None, None),
    };

    let id: i32 = sqlx::query_scalar(
        "INSERT INTO uploads (user_id, original_filename, filesize, sha1, url, extension, width, height, \
                              thumbnail_width, thumbnail_height, animated, \
                              verification_status, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, '', $5, $6, $7, $8, $9, $10, 1, clock_timestamp(), clock_timestamp()) \
         RETURNING id",
    )
    .bind(up.user_id)
    .bind(&original_filename)
    .bind(bytes.len() as i32)
    .bind(&sha1)
    .bind(&extension)
    .bind(width.map(|v| v as i32))
    .bind(height.map(|v| v as i32))
    .bind(thumb.map(|t| t.0 as i32))
    .bind(thumb.map(|t| t.1 as i32))
    .bind(animated)
    .fetch_one(&mut *conn)
    .await?;

    // FileStore::LocalStore#store_upload, then the url saved.
    let ext = if extension.is_empty() {
        String::new()
    } else {
        format!(".{extension}")
    };
    let url = store.store_upload(&bytes, id, &sha1, &ext).await?;
    // calculate_dominant_color! on the processed file; one that does not
    // decode saves an empty colour.
    let color = match image_type.and_then(crate::images::Format::from_extension) {
        Some(format) => {
            let colour_bytes = bytes.clone();
            Some(
                tokio::task::spawn_blocking(move || {
                    crate::images::dominant_color(&colour_bytes, format).unwrap_or_default()
                })
                .await
                .map_err(std::io::Error::other)?,
            )
        }
        None => None,
    };
    sqlx::query(
        "UPDATE uploads SET url = $2, dominant_color = $3, updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(id)
    .bind(&url)
    .bind(&color)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO user_uploads (user_id, upload_id, created_at) \
         SELECT $1, $2, clock_timestamp() \
         WHERE NOT EXISTS (SELECT 1 FROM user_uploads WHERE user_id = $1 AND upload_id = $2)",
    )
    .bind(up.user_id)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    Ok(Outcome::Created(serialize(conn, s, urls, id).await?))
}

/// `UploadValidator#validate` for an upload by a user (not for a theme,
/// an export, a site setting, a message or a gravatar).
fn validate(
    s: &SiteSettings,
    i18n: &crate::i18n::I18n,
    up: &NewUpload<'_>,
    original_filename: &str,
    filesize: usize,
) -> Result<Vec<String>, AppError> {
    let extension = Path::new(original_filename)
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    let authorized_setting = s.get("authorized_extensions")?.to_s();
    let staff_setting = s.get("authorized_extensions_for_staff")?.to_s();
    let all = authorized_setting.contains('*') || (up.staff && staff_setting.contains('*'));
    let authorized = extensions_to_set(&authorized_setting);
    let staff_extensions = if up.staff {
        extensions_to_set(&staff_setting)
    } else {
        Vec::new()
    };
    let lower = extension.to_lowercase();
    // extension_authorized?(upload, extension, extensions)
    let check = |extensions: &[String]| -> Option<String> {
        if all || staff_extensions.contains(&lower) || extensions.contains(&lower) {
            return None;
        }
        let mut listed: Vec<String> = extensions.to_vec();
        for e in &staff_extensions {
            if !listed.contains(e) {
                listed.push(e.clone());
            }
        }
        i18n.t_with(
            "upload.unauthorized",
            &[("authorized_extensions", &listed.join(", "))],
        )
    };
    if let Some(message) = check(&authorized) {
        return Ok(vec![message]);
    }
    let image = is_supported_image(original_filename);
    let subset: Vec<String> = authorized
        .iter()
        .filter(|e| SUPPORTED_IMAGES.contains(&e.as_str()) == image)
        .cloned()
        .collect();
    let mut errors = Vec::new();
    if let Some(message) = check(&subset) {
        errors.push(message);
    }
    // maximum_file_size
    let kind = if image { "image" } else { "attachment" };
    let max_kb = if up.user_id == -1 && kind == "attachment" {
        s.get("system_user_max_attachment_size_kb")?
            .to_i()
            .max(s.get("max_attachment_size_kb")?.to_i())
    } else {
        s.get(&format!("max_{kind}_size_kb"))?.to_i()
    };
    let max_bytes = max_kb * 1024;
    if filesize as i64 > max_bytes {
        errors.push(
            i18n.t_with(
                &format!("upload.{kind}s.too_large_humanized"),
                &[("max_size", &human_size(max_bytes))],
            )
            .unwrap_or_default(),
        );
    }
    Ok(errors)
}

/// `UploadSerializer`
pub async fn serialize(
    conn: &mut PgConnection,
    s: &SiteSettings,
    urls: &Urls<'_>,
    id: i32,
) -> Result<Value, AppError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i32,
        url: String,
        original_filename: String,
        filesize: i64,
        width: Option<i32>,
        height: Option<i32>,
        thumbnail_width: Option<i32>,
        thumbnail_height: Option<i32>,
        extension: Option<String>,
        sha1: Option<String>,
        retain_hours: Option<i32>,
        dominant_color: Option<String>,
    }
    let u: Row = sqlx::query_as(
        "SELECT id, url, original_filename, filesize::int8 AS filesize, width, height, thumbnail_width, \
                thumbnail_height, extension, sha1, retain_hours, dominant_color \
         FROM uploads WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    let store = FileStore::for_site(urls.config, s)?;
    // for_site_setting uploads keep their url uncooked; they are refused.
    let url = crate::pretty_text::cooked_post_processor::cook_url(
        &u.url,
        urls,
        urls.config.globals.relative_url_root(),
        &store,
    )?;
    let sha1 = u.sha1.clone().unwrap_or_default();
    let base62 = crate::pretty_text::helpers::base62_sha1(&sha1).unwrap_or_default();
    let ext = u.extension.clone().filter(|e| !e.is_empty());
    let basename = match &ext {
        Some(e) => format!("{base62}.{e}"),
        None => base62.clone(),
    };
    let thumbnail: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM optimized_images WHERE upload_id = $1 AND width = $2 AND height = $3 LIMIT 1",
    )
    .bind(u.id)
    .bind(u.thumbnail_width)
    .bind(u.thumbnail_height)
    .fetch_optional(&mut *conn)
    .await?;
    if thumbnail.is_some() {
        return Err(Unsupported("serializing upload thumbnails").into());
    }
    let short_path = match &ext {
        Some(e) => format!(
            "{}/uploads/short-url/{base62}.{e}",
            urls.config.globals.relative_url_root()
        ),
        None => format!(
            "{}/uploads/short-url/{base62}",
            urls.config.globals.relative_url_root()
        ),
    };
    Ok(json!({
        "id": u.id,
        "url": url,
        "original_filename": u.original_filename,
        "filesize": u.filesize,
        "width": u.width,
        "height": u.height,
        "thumbnail_width": u.thumbnail_width,
        "thumbnail_height": u.thumbnail_height,
        "extension": u.extension,
        "short_url": format!("upload://{basename}"),
        "short_path": short_path,
        "retain_hours": u.retain_hours,
        "human_filesize": human_size(u.filesize),
        "dominant_color": u.dominant_color,
        "thumbnail": Value::Null,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_sizes_like_active_support() {
        assert_eq!(human_size(1), "1 Byte");
        assert_eq!(human_size(48), "48 Bytes");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(10240), "10 KB");
        assert_eq!(human_size(4096 * 1024), "4 MB");
        assert_eq!(human_size(123_456), "121 KB");
    }

    #[test]
    fn extension_sets_drop_wildcards_and_dots() {
        assert_eq!(extensions_to_set(" .JPG|png|*|png"), vec!["jpg", "png"]);
    }
}
