//! Port of HashtagsController: GET /hashtags/search(.json), what the
//! composers' `#` autocomplete asks, GET /hashtags, the slugs' lookup
//! the composer preview makes, and GET /hashtags/by-ids, the hashtags
//! the post decorations load colors for.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::hashtags::Search;
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

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

/// GET /hashtags (HashtagsController#lookup): `slugs` and `order`, both
/// arrays, the hashtags each type has for the slugs.
pub async fn lookup(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if guardian.user_id().is_none() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let p = params::parse(uri.query(), &headers, &body);
    let strings = |key: &str| match p.get(key) {
        Some(Value::Array(a)) => Some(
            a.iter()
                .map(|v| params::scalar(v).unwrap_or_default())
                .collect::<Vec<String>>(),
        ),
        _ => None,
    };
    let Some(slugs) = strings("slugs") else {
        return Ok(super::search::invalid_parameters(&state, "slugs"));
    };
    let Some(order) = strings("order") else {
        return Ok(super::search::invalid_parameters(&state, "order"));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let results = Search {
        conn: &mut conn,
        settings: &settings,
        guardian: &guardian,
        base_path: state.config.globals.relative_url_root(),
    }
    .lookup_slugs(&slugs, &order)
    .await?;
    Ok(Json(Value::Object(results)).into_response())
}

/// GET /hashtags/by-ids (HashtagsController#by_ids): each type's ids
/// (`category[]`, `tag[]`, `channel[]`), open to visitors unless login is
/// required.
pub async fn by_ids(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // redirect_to_login_if_required answers before the action's own
    // NotFound.
    if settings.get("login_required")?.truthy() && guardian.user_id().is_none() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let mut ids = Vec::new();
    for kind in ["category", "tag", "channel"] {
        match p.get(kind) {
            None | Some(Value::Null) => {}
            // params[type]&.map(&:to_i)
            Some(Value::Array(a)) => ids.push((
                kind.to_string(),
                a.iter()
                    .map(|v| crate::ruby::to_i(&params::scalar(v).unwrap_or_default()))
                    .collect(),
            )),
            Some(_) => return Err(Unsupported("hashtag ids not given as a list").into()),
        }
    }
    let results = Search {
        conn: &mut conn,
        settings: &settings,
        guardian: &guardian,
        base_path: state.config.globals.relative_url_root(),
    }
    .find_by_ids(&ids)
    .await?;
    Ok(Json(Value::Object(results)).into_response())
}
