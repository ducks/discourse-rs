//! The color definitions stylesheets (Stylesheet::Manager's
//! `color_definitions_*`): the default theme's color scheme, or the base
//! light palette, and its dark scheme when it has one.

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::color_scheme::ColorScheme;
use crate::site_settings::SiteSettings;
use crate::stylesheet::color_definitions::{SchemeColors, css};
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

fn stylesheet(body: Option<String>) -> Response {
    match body {
        Some(body) => ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], body).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The light scheme: the default theme's, else the base palette.
pub async fn light(State(state): State<AppState>) -> Result<Response, AppError> {
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
    Ok(stylesheet(scheme_css(&mut conn, scheme_id).await?))
}

/// The default theme's dark scheme (`dark_scheme_id`), if it has one.
pub async fn dark(State(state): State<AppState>) -> Result<Response, AppError> {
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
        Some(id) => Ok(stylesheet(scheme_css(&mut conn, Some(id)).await?)),
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}
