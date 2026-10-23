//! The checklist plugin's Checklist::CheckboxesController, mounted at
//! /checklist: PUT toggle (crate::plugins::checklist::toggle).

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::plugins::checklist::{self, Answer};
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::rate_limiter::{LimitUser, RateLimiter};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// `failed_json.merge(...)`
fn failed(status: StatusCode, extra: Value) -> Response {
    let mut body = Map::new();
    body.insert("failed".into(), json!("FAILED"));
    if let Value::Object(extra) = extra {
        body.extend(extra);
    }
    (status, Json(Value::Object(body))).into_response()
}

/// PUT /checklist/toggle
pub async fn toggle(
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
    // requires_login (ApplicationController's before_action), then
    // requires_plugin, then the toggle rate limit.
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let settings = {
        let mut conn = state.pool.acquire().await?;
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?
    };
    if !checklist::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    let limiter = RateLimiter::new(
        Some(LimitUser {
            id: user_id,
            staff: guardian.is_staff(),
        }),
        "checklist_toggle",
        checklist::MAX_TOGGLE_REQUESTS_PER_MINUTE,
        60,
    );
    if let Err(refused) = limiter
        .performed(&state.pool, state.config.rate_limits)
        .await?
    {
        return Ok(refused.into_response(&state.i18n));
    }
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    Ok(
        match checklist::toggle(&state.pool, &ctx, &guardian, &p).await? {
            Answer::Done(body) => (StatusCode::OK, Json(body)).into_response(),
            Answer::InvalidContract(errors) => {
                failed(StatusCode::BAD_REQUEST, json!({ "errors": errors }))
            }
            Answer::NotFound => super::topics::not_found_response(&state),
            Answer::Forbidden => super::search::invalid_access(&state),
            Answer::Conflict {
                raw,
                updated_at,
                retryable,
            } => failed(
                StatusCode::CONFLICT,
                json!({
                    "errors": [ctx.t("checklist.checkboxes_changed")],
                    "raw": raw,
                    "updated_at": updated_at,
                    "retryable": retryable,
                }),
            ),
            Answer::Unprocessable(error) => failed(
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({ "errors": [error] }),
            ),
            Answer::Failed => failed(StatusCode::UNPROCESSABLE_ENTITY, json!({})),
            Answer::RateLimited(refused) => refused.into_response(&state.i18n),
        },
    )
}
