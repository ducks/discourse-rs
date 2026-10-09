//! The color definitions stylesheets (Stylesheet::Manager's
//! `color_definitions_*`): the default theme's color scheme, or the base
//! light palette, and its dark scheme when it has one.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::color_scheme::ColorScheme;
use crate::site_settings::SiteSettings;
use crate::stylesheet::color_definitions::{SchemeColors, css};
use crate::stylesheet::fonts;
use crate::{AppError, AppState};

async fn scheme_css(
    conn: &mut sqlx::PgConnection,
    id: Option<i32>,
) -> Result<Option<String>, AppError> {
    let colors = match id {
        None => SchemeColors::from_hex(|_| None),
        Some(id) => {
            let Some(scheme) = ColorScheme::find(conn, id).await? else {
                return Ok(None);
            };
            let resolved = scheme.resolved_colors()?;
            SchemeColors::from_hex(|name| {
                resolved
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, hex)| hex.clone())
            })
        }
    };
    Ok(Some(css(&colors)))
}

/// The stylesheet, revalidated by its ETag on every use: it follows the
/// color scheme in the database, so it is never cached for good.
fn stylesheet(headers: &HeaderMap, body: Option<String>) -> Response {
    match body {
        Some(body) => {
            let digest = crate::assets::digest_of(body.as_bytes());
            crate::assets::cached(headers, "text/css; charset=utf-8", body, &digest, false)
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The light scheme: the default theme's, else the base palette.
pub async fn light(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let theme_id = settings.get("default_theme_id")?.to_i();
    let scheme_id: Option<i32> =
        sqlx::query_scalar("SELECT color_scheme_id FROM themes WHERE id = $1")
            .bind(theme_id as i32)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
    let body = scheme_css(&mut conn, scheme_id).await?;
    Ok(stylesheet(
        &headers,
        body.map(|css| css + &font_css(&state, &settings)),
    ))
}

/// The default theme's dark scheme (`dark_scheme_id`), if it has one.
pub async fn dark(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let theme_id = settings.get("default_theme_id")?.to_i();
    let id: Option<i32> =
        sqlx::query_scalar("SELECT dark_color_scheme_id FROM themes WHERE id = $1")
            .bind(theme_id as i32)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
    match id.filter(|id| *id > 0) {
        Some(id) => {
            let body = scheme_css(&mut conn, Some(id)).await?;
            Ok(stylesheet(
                &headers,
                body.map(|css| css + &font_css(&state, &settings)),
            ))
        }
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}

/// The font definitions Rails adds to every color definitions file.
fn font_css(state: &AppState, settings: &SiteSettings) -> String {
    let setting = |name| {
        settings
            .get(name)
            .map(|v| v.to_s().to_string())
            .unwrap_or_default()
    };
    let fonts_dir = format!("{}/fonts", state.config.globals.relative_url_root());
    fonts::css(&setting("base_font"), &setting("heading_font"), &fonts_dir)
}

/// The discourse-fonts files this port has (static/fonts/README).
const FONT_FILES: &[(&str, &[u8])] = &[
    (
        "InterVariable.woff2",
        include_bytes!("../../static/fonts/InterVariable.woff2"),
    ),
    (
        "JetBrainsMono-Regular.woff2",
        include_bytes!("../../static/fonts/JetBrainsMono-Regular.woff2"),
    ),
    (
        "JetBrainsMono-Bold.woff2",
        include_bytes!("../../static/fonts/JetBrainsMono-Bold.woff2"),
    ),
];

/// `/fonts/:file`
pub async fn font(Path(file): Path<String>) -> Response {
    match FONT_FILES.iter().find(|(name, _)| *name == file) {
        Some((_, bytes)) => (
            [
                (header::CONTENT_TYPE, "font/woff2"),
                (header::CACHE_CONTROL, "max-age=31556952, public, immutable"),
            ],
            *bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
