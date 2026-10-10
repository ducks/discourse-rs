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

/// GET /assets/emoji-data.json: what pretty-text's emoji module carries
/// in Ember's bundle and chat's "+emoji" shortcut reads (normalizeEmoji's
/// names and aliasMap, and the unicode replacements), from the vendored
/// emoji data.
pub async fn client_data() -> axum::response::Response {
    use axum::response::IntoResponse;
    static DATA: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let emoji = &*discourse_markdown::emoji::DATA;
        let mut names: Vec<&String> = emoji.names.iter().collect();
        names.sort();
        let aliases: std::collections::BTreeMap<&String, &String> = emoji.aliases.iter().collect();
        let unicode: std::collections::BTreeMap<&String, &String> = emoji.unicode.iter().collect();
        serde_json::json!({ "names": names, "aliases": aliases, "unicode": unicode }).to_string()
    });
    (
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (axum::http::header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        DATA.as_str(),
    )
        .into_response()
}
