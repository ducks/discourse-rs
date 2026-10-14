//! Port of Admin::ImpersonateController: POST /admin/impersonate starts
//! acting as another user on this session's token, DELETE stops it, GET
//! is Admin::AdminController#index (an empty body).

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::session::current::{AuthGuardian, Incoming};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `UserHistory.actions`
const IMPERSONATE: i32 = 19;
const STOP_IMPERSONATING: i32 = 121;

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// `render body: nil`
fn empty() -> Response {
    StatusCode::OK.into_response()
}

async fn log(
    conn: &mut sqlx::PgConnection,
    action: i32,
    acting_user_id: i32,
    target_user_id: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(action)
    .bind(acting_user_id)
    .bind(target_user_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// GET /admin/impersonate: Admin::AdminController#index.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    // AdminConstraint
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    Ok(empty())
}

/// POST /admin/impersonate: the log first, then the token now acting as
/// the user until experimental_impersonation_time_limit_minutes from now.
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    axum::Extension(incoming): axum::Extension<Incoming>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    // AdminConstraint: an impersonating admin is the puppet here, who is
    // not an admin, so a second start misses the route.
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let Some(session) = &incoming.session else {
        return Err(Unsupported("impersonating without a session token (API keys)").into());
    };
    let Some(who) = p
        .get("username_or_email")
        .and_then(params::scalar)
        .filter(|v| !v.is_empty())
    else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "errors": ["param is missing or the value is empty or invalid: username_or_email"]
            })),
        )
            .into_response());
    };
    let mut conn = state.pool.acquire().await?;
    let Some(target_id) = crate::pm_invite::find_by_username_or_email(&mut conn, &who).await?
    else {
        return Ok(super::topics::not_found_response(&state));
    };
    let target_admin: bool = sqlx::query_scalar("SELECT admin FROM users WHERE id = $1")
        .bind(target_id)
        .fetch_one(&mut *conn)
        .await?;
    let actor = &session.user;
    let development = state.config.rails_env == crate::config::RailsEnv::Development;
    if !crate::impersonation::can_impersonate(
        &mut conn,
        &state.config.globals,
        development,
        actor.id,
        actor.admin,
        target_admin,
    )
    .await?
    {
        return Ok(super::search::invalid_access(&state));
    }
    log(&mut conn, IMPERSONATE, actor.id, target_id).await?;
    if actor.is_impersonating() {
        return Ok(super::search::invalid_access(&state));
    }
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let minutes = settings
        .get("experimental_impersonation_time_limit_minutes")?
        .to_i();
    // start_impersonating_user: @user_token.update!
    sqlx::query(
        "UPDATE user_auth_tokens SET impersonated_user_id = $2, \
           impersonation_expires_at = clock_timestamp() + make_interval(mins => $3), \
           updated_at = clock_timestamp() \
         WHERE id = $1",
    )
    .bind(session.token.id)
    .bind(target_id)
    .bind(i32::try_from(minutes).unwrap_or(i32::MAX))
    .execute(&mut *conn)
    .await?;
    Ok(empty())
}

/// DELETE /admin/impersonate: back to the acting admin, logged by them.
/// ImpersonatorConstraint lets in an admin or an impersonating session.
pub async fn destroy(
    State(state): State<AppState>,
    axum::Extension(incoming): axum::Extension<Incoming>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let Some(session) = &incoming.session else {
        return Ok(super::topics::not_found_response(&state));
    };
    let user = &session.user;
    if !user.admin && !user.is_impersonating() {
        return Ok(super::topics::not_found_response(&state));
    }
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "DELETE") {
        return Ok(bad_csrf());
    }
    if user.is_impersonating() {
        let mut conn = state.pool.acquire().await?;
        sqlx::query(
            "UPDATE user_auth_tokens SET impersonated_user_id = NULL, impersonation_expires_at = NULL, \
               updated_at = clock_timestamp() \
             WHERE id = $1",
        )
        .bind(session.token.id)
        .execute(&mut *conn)
        .await?;
        // The current user looked up again: the token's own user.
        log(
            &mut conn,
            STOP_IMPERSONATING,
            session.token.user_id,
            user.id,
        )
        .await?;
    }
    Ok(empty())
}
