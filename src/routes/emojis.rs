//! EmojisController: the emoji picker's data.

use axum::Json;
use axum::extract::State;
use serde_json::Value;

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// GET /emojis.json (`Emoji.grouped`)
pub async fn index(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let base_path = state.config.globals.relative_url_root();
    Ok(Json(
        crate::emojis::grouped(&mut conn, &settings, base_path).await?,
    ))
}

/// GET /emojis/search-aliases.json
pub async fn search_aliases() -> Json<Value> {
    Json(crate::emojis::search_aliases())
}
