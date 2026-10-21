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
            .with_defined(&resolved)
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
    // theme.color_scheme: a scheme that is gone (or a remote copy) is the
    // base palette, as Stylesheet::Manager falls back to it.
    let scheme_id: Option<i32> = sqlx::query_scalar(
        "SELECT cs.id FROM themes t \
         JOIN color_schemes cs ON cs.id = t.color_scheme_id AND NOT cs.remote_copy \
         WHERE t.id = $1",
    )
    .bind(theme_id as i32)
    .fetch_optional(&mut *conn)
    .await?;
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

/// `/assets/color_definitions_scheme/:id.css`: any other scheme a page
/// links (a member's own light or dark palette, crate::color_mode).
pub async fn scheme(
    State(state): State<AppState>,
    Path(file): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(id) = file
        .strip_suffix(".css")
        .and_then(|id| id.parse::<i32>().ok())
    else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let body = scheme_css(&mut conn, Some(id)).await?;
    Ok(stylesheet(
        &headers,
        body.map(|css| css + &font_css(&state, &settings)),
    ))
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

/// `/color-scheme-stylesheet/:id(/:theme_id)` (StylesheetsController#
/// color_scheme): the stylesheet a palette preview links, as
/// `{color_scheme_id, new_href}`. A theme the viewer may use other than the
/// default one would serve its own color definitions, which the port does
/// not draw.
pub async fn color_scheme(
    State(state): State<AppState>,
    crate::session::current::AuthGuardian(guardian): crate::session::current::AuthGuardian,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
    Path(segments): Path<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let get = |name: &str| {
        segments
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    // The route's `.json` format; any other dot ends the id elsewhere.
    let id = get("id").unwrap_or_default();
    let id = id.strip_suffix(".json").unwrap_or(id);
    if id.contains('.') {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let theme_param = match get("theme_id") {
        Some(t) => {
            let t = t.strip_suffix(".json").unwrap_or(t);
            let digits = t.strip_prefix('-').unwrap_or(t);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return Ok(StatusCode::NOT_FOUND.into_response());
            }
            Some(t.to_string())
        }
        None => crate::params::parse_query(query.as_deref().unwrap_or_default())
            .get("theme_id")
            .and_then(crate::params::scalar),
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let default_theme = settings.get("default_theme_id")?.to_i();
    // `params[:theme_id]&.to_i`, nil unless guardian.allow_themes?([id],
    // include_preview: true).
    if let Some(theme_id) = theme_param.map(|t| crate::ruby::to_i(&t))
        && theme_id != default_theme
    {
        let staff_preview = guardian.is_staff()
            && sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM themes WHERE id = $1)")
                .bind(i32::try_from(theme_id).unwrap_or(0))
                .fetch_one(&mut *conn)
                .await?;
        let selectable = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM themes WHERE id = $1 AND user_selectable)",
        )
        .bind(i32::try_from(theme_id).unwrap_or(0))
        .fetch_one(&mut *conn)
        .await?;
        if staff_preview || selectable {
            return Err(crate::Unsupported(
                "color definitions of a theme other than the default (258a)",
            )
            .into());
        }
    }
    let base_path = state.config.globals.relative_url_root().to_string();
    let (scheme_id, href) = crate::color_mode::scheme_stylesheet(
        &mut conn,
        &settings,
        &base_path,
        // find_by(id:) casts as ActiveModel's integer type does (to_i).
        Some(crate::ruby::to_i(id)),
    )
    .await?;
    Ok(axum::Json(serde_json::json!({
        "color_scheme_id": scheme_id,
        "new_href": href,
    }))
    .into_response())
}
