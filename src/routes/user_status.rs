//! Port of UserStatusController: GET /user-status (the current user's
//! status, null without one), PUT /user-status (set it: description and
//! emoji required, an optional ends_at) and DELETE /user-status (clear
//! it). Each needs a login, then enable_user_status (a 404 without it).

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::users::status;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn wants_json(uri: &Uri, headers: &HeaderMap) -> bool {
    uri.path().ends_with(".json") || super::wants_json(uri.query(), headers)
}

/// `ensure_feature_enabled`: a RoutingError, Discourse's not found.
async fn enabled(state: &AppState) -> Result<Option<SiteSettings>, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    Ok(settings
        .get("enable_user_status")?
        .truthy()
        .then_some(settings))
}

/// `params.require(key)`: a string that isn't blank, else
/// ParameterMissing's 400.
fn require(p: &Map<String, Value>, key: &str) -> Result<Result<String, Response>, AppError> {
    match p.get(key) {
        Some(Value::String(s)) if !crate::ruby::is_blank(s) => Ok(Ok(s.clone())),
        None | Some(Value::Null) | Some(Value::String(_)) => {
            Ok(Err(super::accounts::param_missing(key)))
        }
        Some(_) => Err(Unsupported("a user status param that is not a string").into()),
    }
}

/// GET /user-status
pub async fn get(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    if enabled(&state).await?.is_none() {
        return Ok(super::topics::not_found_response(&state));
    }
    // respond_to format.json alone.
    if !wants_json(&uri, &headers) {
        return Err(Unsupported("GET /user-status in a format other than JSON").into());
    }
    let mut conn = state.pool.acquire().await?;
    let body = status::own(&mut conn, user_id)
        .await?
        .unwrap_or(Value::Null);
    Ok(Json(body).into_response())
}

/// PUT /user-status
pub async fn set(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let Some(settings) = enabled(&state).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let description = match require(&p, "description")? {
        Ok(d) => d,
        Err(response) => return Ok(response),
    };
    let emoji = match require(&p, "emoji")? {
        Ok(e) => e,
        Err(response) => return Ok(response),
    };
    let ends_at = match p.get("ends_at") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => status::cast_ends_at(Some(s))?,
        Some(_) => return Err(Unsupported("a user status ends_at that is not a string").into()),
    };
    let mut tx = state.pool.begin().await?;
    match status::set(
        &mut tx,
        &state.bus,
        &settings,
        &state.i18n,
        user_id,
        &description,
        &emoji,
        ends_at,
    )
    .await
    {
        Ok(()) => {}
        Err(e) => return invalid(e, &uri, &headers),
    }
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// The RecordInvalid rescue: a JSON request's 422, raised for another.
fn invalid(e: AppError, uri: &Uri, headers: &HeaderMap) -> Result<Response, AppError> {
    match e.downcast_ref::<status::RecordInvalid>() {
        Some(invalid) if wants_json(uri, headers) => {
            Ok(status::record_invalid_response(&invalid.0))
        }
        Some(_) => Err(Unsupported("an invalid user status outside a JSON request").into()),
        None => Err(e),
    }
}

/// DELETE /user-status
pub async fn clear(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "DELETE") {
        return Ok(bad_csrf());
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    if enabled(&state).await?.is_none() {
        return Ok(super::topics::not_found_response(&state));
    }
    let mut tx = state.pool.begin().await?;
    status::clear(&mut tx, &state.bus, user_id).await?;
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}
