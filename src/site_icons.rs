//! Upload-backed site settings and their URLs: the upload getter in
//! lib/site_setting_extension.rb, `SiteSetting.site_<name>_url`
//! (app/models/site_setting.rb) and lib/site_icon_manager.rb.

use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::url::{UrlError, Urls};

/// `Upload::SEEDED_ID_THRESHOLD`: an upload setting of 0 means unset.
const SEEDED_ID_THRESHOLD: i64 = 0;

/// `SiteIconManager::SKETCH_LOGO_ID`
const SKETCH_LOGO_ID: i64 = -6;

struct Icon {
    name: &'static str,
    size: Option<(i32, i32)>,
    settings: &'static [&'static str],
    fallback_to_sketch: bool,
    resize_required: bool,
}

/// `SiteIconManager::ICONS`
const ICONS: &[Icon] = &[
    Icon {
        name: "digest_logo",
        size: None,
        settings: &["digest_logo", "logo"],
        fallback_to_sketch: false,
        resize_required: false,
    },
    Icon {
        name: "mobile_logo",
        size: None,
        settings: &["mobile_logo", "logo"],
        fallback_to_sketch: false,
        resize_required: false,
    },
    Icon {
        name: "mobile_logo_dark",
        size: None,
        settings: &["mobile_logo_dark", "logo_dark"],
        fallback_to_sketch: false,
        resize_required: false,
    },
    Icon {
        name: "large_icon",
        size: None,
        settings: &["large_icon", "logo_small"],
        fallback_to_sketch: true,
        resize_required: false,
    },
    Icon {
        name: "manifest_icon",
        size: Some((512, 512)),
        settings: &["manifest_icon", "large_icon", "logo_small"],
        fallback_to_sketch: true,
        resize_required: true,
    },
    Icon {
        name: "favicon",
        size: Some((32, 32)),
        settings: &["favicon", "large_icon", "logo_small"],
        fallback_to_sketch: true,
        resize_required: false,
    },
    Icon {
        name: "apple_touch_icon",
        size: Some((180, 180)),
        settings: &["apple_touch_icon", "large_icon", "logo_small"],
        fallback_to_sketch: true,
        resize_required: false,
    },
    Icon {
        name: "opengraph_image",
        size: None,
        settings: &["opengraph_image", "large_icon", "logo_small", "logo"],
        fallback_to_sketch: true,
        resize_required: false,
    },
];

#[derive(Debug)]
pub enum IconError {
    Db(sqlx::Error),
    Url(UrlError),
}

impl std::fmt::Display for IconError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IconError::Db(e) => write!(f, "loading upload: {e}"),
            IconError::Url(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for IconError {}

impl From<sqlx::Error> for IconError {
    fn from(e: sqlx::Error) -> Self {
        IconError::Db(e)
    }
}

impl From<UrlError> for IconError {
    fn from(e: UrlError) -> Self {
        IconError::Url(e)
    }
}

impl From<crate::site_settings::SettingError> for IconError {
    fn from(e: crate::site_settings::SettingError) -> Self {
        IconError::Url(UrlError::Setting(e))
    }
}

/// The only upload column URL generation needs so far.
struct Upload {
    id: i32,
    url: String,
}

async fn find_upload(conn: &mut PgConnection, id: i64) -> Result<Option<Upload>, sqlx::Error> {
    let Ok(id) = i32::try_from(id) else {
        return Ok(None);
    };
    let row: Option<(i32, String)> = sqlx::query_as("SELECT id, url FROM uploads WHERE id = $1")
        .bind(id)
        .fetch_optional(conn)
        .await?;
    Ok(row.map(|(id, url)| Upload { id, url }))
}

/// The upload getter: `value.to_i`, nil for 0 or a missing upload.
async fn setting_upload(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    name: &str,
) -> Result<Option<Upload>, IconError> {
    let id = settings.get(name)?.to_i();
    if id == SEEDED_ID_THRESHOLD {
        return Ok(None);
    }
    Ok(find_upload(conn, id).await?)
}

/// `SiteIconManager.<name>`: the first configured upload in the chain (or
/// the sketch logo), preferring an optimized copy at the icon's size.
async fn resolve_icon(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    icon: &Icon,
) -> Result<Option<String>, IconError> {
    let mut original = None;
    for name in icon.settings {
        if let Some(upload) = setting_upload(conn, settings, name).await? {
            original = Some(upload);
            break;
        }
    }
    if original.is_none() && icon.fallback_to_sketch {
        original = find_upload(conn, SKETCH_LOGO_ID).await?;
    }

    if let (Some((width, height)), Some(upload)) = (icon.size, &original) {
        let optimized: Option<String> = sqlx::query_scalar(
            "SELECT url FROM optimized_images \
             WHERE upload_id = $1 AND width = $2 AND height = $3 ORDER BY id LIMIT 1",
        )
        .bind(upload.id)
        .bind(width)
        .bind(height)
        .fetch_optional(&mut *conn)
        .await?;
        if optimized.is_some() {
            return Ok(optimized);
        }
    }

    Ok(if icon.resize_required {
        None
    } else {
        original.map(|u| u.url)
    })
}

/// `SiteSetting.site_<name>_url`: SiteIconManager for managed icons, else
/// the setting's own upload. "" when nothing resolves.
pub async fn site_url(
    conn: &mut PgConnection,
    urls: &Urls<'_>,
    name: &str,
) -> Result<String, IconError> {
    let url = match ICONS.iter().find(|i| i.name == name) {
        Some(icon) => resolve_icon(conn, urls.settings, icon).await?,
        None => setting_upload(conn, urls.settings, name)
            .await?
            .map(|u| u.url),
    };
    match url {
        Some(url) => Ok(urls.full_cdn_url(&url)?),
        None => Ok(String::new()),
    }
}

/// `MiniMime.lookup_by_filename(url)&.content_type || "image/png"`.
pub fn mime_type(url: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    match path
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("ico") => "image/vnd.microsoft.icon",
        Some("svg") => "image/svg+xml",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "image/png",
    }
}

/// `SiteIconManager.ensure_optimized!`, as far as it matters here: Rails
/// makes each sized icon's optimized copy at boot, and the seed's rows
/// for them point at files only the reference has. A row whose file is
/// missing from the public directory gets it made again from the
/// original (looked up there, then in the Discourse checkout). Making a
/// copy that has no row is not ported. The number of files written.
pub async fn ensure_optimized(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    config: &crate::config::Config,
) -> Result<usize, crate::AppError> {
    let public = &config.public_dir;
    let base_path = config.globals.relative_url_root();
    let mut written = 0;
    for icon in ICONS {
        let Some((width, height)) = icon.size else {
            continue;
        };
        let mut original = None;
        for name in icon.settings {
            if let Some(upload) = setting_upload(conn, settings, name).await? {
                original = Some(upload);
                break;
            }
        }
        if original.is_none() && icon.fallback_to_sketch {
            original = find_upload(conn, SKETCH_LOGO_ID).await?;
        }
        let Some(original) = original else {
            continue;
        };
        let optimized: Option<String> = sqlx::query_scalar(
            "SELECT url FROM optimized_images \
             WHERE upload_id = $1 AND width = $2 AND height = $3 ORDER BY id LIMIT 1",
        )
        .bind(original.id)
        .bind(width)
        .bind(height)
        .fetch_optional(&mut *conn)
        .await?;
        let Some(url) = optimized else {
            continue;
        };
        let relative = |u: &str| -> Option<String> {
            let u = u.strip_prefix(base_path).unwrap_or(u);
            (u.starts_with('/') && !u.starts_with("//"))
                .then(|| u.trim_start_matches('/').to_string())
        };
        let Some(target) = relative(&url).map(|p| public.join(p)) else {
            continue;
        };
        if target.exists() {
            continue;
        }
        let Some(source) = relative(&original.url) else {
            continue;
        };
        let candidates = std::iter::once(public.join(&source)).chain(
            config
                .discourse_src
                .iter()
                .map(|src| src.join("public").join(&source)),
        );
        let Some(bytes) = candidates.filter_map(|p| std::fs::read(p).ok()).next() else {
            tracing::warn!(url = %original.url, "site icon original not found");
            continue;
        };
        let Some(format) = std::path::Path::new(&source)
            .extension()
            .and_then(|e| e.to_str())
            .and_then(crate::images::Format::from_extension)
        else {
            continue;
        };
        let quality = settings.get("image_quality")?.to_i().clamp(1, 100) as u8;
        let (w, h) = (width as u32, height as u32);
        let resized = tokio::task::spawn_blocking(move || {
            crate::images::thumbnail(&bytes, format, w, h, false, quality)
        })
        .await
        .map_err(std::io::Error::other)?;
        let Some(resized) = resized else {
            continue;
        };
        if let Some(dir) = target.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        tokio::fs::write(&target, resized).await?;
        written += 1;
    }
    Ok(written)
}
