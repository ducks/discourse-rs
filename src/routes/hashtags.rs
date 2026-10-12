//! Port of HashtagsController#search: GET /hashtags/search(.json), what
//! the composers' `#` autocomplete asks.

use axum::Json;
use axum::extract::State;
use axum::http::Uri;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::hashtags::Search;
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// GET /hashtags/search: `params.require(:order)`, an array of types in
/// priority order, and an optional `term`.
pub async fn search(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: Uri,
) -> Result<Response, AppError> {
    if guardian.user_id().is_none() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let p = params::parse_query(uri.query().unwrap_or_default());
    let order = match p.get("order") {
        None => return Ok(super::accounts::param_missing("order")),
        Some(Value::String(s)) if s.is_empty() => {
            return Ok(super::accounts::param_missing("order"));
        }
        Some(Value::Array(a)) if a.is_empty() => {
            return Ok(super::accounts::param_missing("order"));
        }
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Ok(super::search::invalid_parameters(&state, "order")),
    };
    let term = params::string(&p, "term");
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let results = Search {
        conn: &mut conn,
        settings: &settings,
        guardian: &guardian,
        base_path: state.config.globals.relative_url_root(),
    }
    .search(term.as_deref(), &order)
    .await?;
    let results: Vec<Value> = results.iter().map(|item| item.to_json()).collect();
    Ok(Json(json!({ "success": "OK", "results": results })).into_response())
}
