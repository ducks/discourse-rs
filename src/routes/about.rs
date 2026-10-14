//! GET /about.json (about#index): the about page's document.

use axum::Json;
use axum::extract::State;
use serde_json::Value;

use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

/// GET /about.json
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Json<Value>, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    Ok(Json(
        crate::about::document(&mut conn, &settings, &urls, &guardian).await?,
    ))
}
