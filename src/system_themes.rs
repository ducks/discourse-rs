//! The core themes' files that `SystemThemesManager.sync!`
//! (db/fixtures/600_themes.rb) stores at seed time. Rails imports each
//! themes/<name> directory with RemoteTheme.import_theme_from_directory,
//! and ThemeScreenshotsHandler stores every screenshot about.json lists
//! through UploadCreator: an upload row, its bytes unchanged in the local
//! store at original/1X/<sha1>.png, and a theme_screenshot_upload_var
//! field naming it. The seed carries the rows; the files are vendored
//! with the schema and written here at boot, as site_icons does for the
//! icons' optimized copies.

use sha1::{Digest, Sha1};
use sqlx::PgConnection;

/// `ThemeField.types[:theme_screenshot_upload_var]`
const THEME_SCREENSHOT_UPLOAD_VAR: i32 = 7;

/// A core theme's screenshot: `Theme::CORE_THEMES`' id, the field
/// ThemeScreenshotsHandler names after the file, and the file.
pub struct Screenshot {
    pub theme_id: i32,
    pub field: &'static str,
    pub bytes: &'static [u8],
}

/// The screenshots the core themes' about.json list, in their order.
pub const SCREENSHOTS: [Screenshot; 4] = [
    Screenshot {
        theme_id: -1,
        field: "screenshot_light",
        bytes: include_bytes!("../vendor/discourse/themes/foundation/screenshots/light.png"),
    },
    Screenshot {
        theme_id: -1,
        field: "screenshot_dark",
        bytes: include_bytes!("../vendor/discourse/themes/foundation/screenshots/dark.png"),
    },
    Screenshot {
        theme_id: -2,
        field: "screenshot_light",
        bytes: include_bytes!("../vendor/discourse/themes/horizon/screenshots/light.png"),
    },
    Screenshot {
        theme_id: -2,
        field: "screenshot_dark",
        bytes: include_bytes!("../vendor/discourse/themes/horizon/screenshots/dark.png"),
    },
];

/// Writes each core theme screenshot whose upload's file is missing from
/// the local store. Only an upload whose sha1 is the vendored file's gets
/// it (a site whose themes were imported from other files keeps its own),
/// and an upload on an external store is left alone. The number of files
/// written.
pub async fn ensure_screenshots(
    conn: &mut PgConnection,
    config: &crate::config::Config,
) -> Result<usize, crate::AppError> {
    let base_path = config.globals.relative_url_root();
    let mut written = 0;
    for shot in &SCREENSHOTS {
        let upload: Option<(String, String)> = sqlx::query_as(
            "SELECT u.url, u.sha1 FROM theme_fields f JOIN uploads u ON u.id = f.upload_id \
             WHERE f.theme_id = $1 AND f.name = $2 AND f.type_id = $3 \
             ORDER BY f.id LIMIT 1",
        )
        .bind(shot.theme_id)
        .bind(shot.field)
        .bind(THEME_SCREENSHOT_UPLOAD_VAR)
        .fetch_optional(&mut *conn)
        .await?;
        let Some((url, sha1)) = upload else {
            continue;
        };
        if sha1 != format!("{:x}", Sha1::digest(shot.bytes)) {
            continue;
        }
        let path = url.strip_prefix(base_path).unwrap_or(&url);
        if !path.starts_with("/uploads/") {
            continue;
        }
        let target = config.public_dir.join(path.trim_start_matches('/'));
        if tokio::fs::try_exists(&target).await? {
            continue;
        }
        if let Some(dir) = target.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        tokio::fs::write(&target, shot.bytes).await?;
        written += 1;
    }
    Ok(written)
}
