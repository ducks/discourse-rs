//! Admin::BackupsController: only its read-only toggle so far.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::session::current::AuthGuardian;
use crate::{AppError, AppState};

/// PUT /admin/backups/readonly(.json)?enable= (BackupsController#readonly,
/// AdminConstraint): the user read-only key on or off, logged as
/// change_readonly_mode, an empty 200.
pub async fn readonly(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    let pairs: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !super::session::csrf_ok(&state, &headers, &pairs, uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    // check_xhr: not skipped for this action.
    let xhr = headers
        .get("x-requested-with")
        .is_some_and(|v| v == "XMLHttpRequest");
    if !xhr && !uri.path().ends_with(".json") && !super::wants_json(uri.query(), &headers) {
        return Ok(super::session::render_empty());
    }
    // params.fetch(:enable): present, even blank.
    let Some(enable) = p.get("enable") else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "errors": ["param is missing or the value is empty or invalid: enable"]
            })),
        )
            .into_response());
    };
    let enable = crate::params::scalar(enable).as_deref() == Some("true");
    let mut tx = state.pool.begin().await?;
    if enable {
        crate::readonly::enable(&mut tx, &state.bus, crate::readonly::USER_READONLY_MODE_KEY)
            .await?;
    } else {
        crate::readonly::disable(&mut tx, &state.bus, crate::readonly::USER_READONLY_MODE_KEY)
            .await?;
    }
    // StaffActionLogger#log_change_readonly_mode: previous_value !state,
    // new_value state, as the text column stores booleans.
    let as_text = |b: bool| if b { "t" } else { "f" };
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, previous_value, new_value, admin_only, \
                                     created_at, updated_at) \
         VALUES ($1, $2, $3, $4, TRUE, now(), now())",
    )
    .bind(crate::staff_action_logs::action_id("change_readonly_mode").unwrap_or(44))
    .bind(guardian.user_id())
    .bind(as_text(!enable))
    .bind(as_text(enable))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::OK.into_response())
}
