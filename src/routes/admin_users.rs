//! Port of Admin::UsersController's penalties: PUT
//! /admin/users/:user_id/{suspend,unsuspend,silence,unsilence}(.json).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::user_penalties::{self, Outcome, Penalty};
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// Which penalty a route applies.
#[derive(Clone, Copy)]
enum Action {
    Suspend,
    Unsuspend,
    Silence,
    Unsilence,
}

/// The params the services take that are not ported.
fn unported(p: &Map<String, Value>) -> Option<&'static str> {
    let present = |k: &str| {
        p.get(k)
            .is_some_and(|v| !v.is_null() && params::scalar(v).is_none_or(|s| !s.is_empty()))
    };
    if present("other_user_ids") {
        Some("penalizing several users at once (other_user_ids)")
    } else if present("post_id") || present("post_action") || present("post_edit") {
        Some("acting on a post with a penalty")
    } else if present("reviewable_id") {
        Some("penalties from the review queue")
    } else {
        None
    }
}

async fn handle(
    state: AppState,
    guardian: crate::guardian::Guardian,
    user_id: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    action: Action,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    // The admin namespace is routed for staff only.
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state, false));
    }
    let user_id = crate::ruby::to_i(user_id.strip_suffix(".json").unwrap_or(&user_id)) as i32;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    drop(conn);
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let text = |k: &str| p.get(k).and_then(params::scalar);
    let until_param = match action {
        Action::Silence => "silenced_till",
        _ => "suspend_until",
    };
    let (reason, message, until) = (text("reason"), text("message"), text(until_param));
    let penalty = Penalty {
        reason: reason.as_deref(),
        message: message.as_deref(),
        until: until.as_deref(),
        unported: unported(&p),
    };
    // Suspending is one transaction; silencing sends a system message,
    // which PostCreator commits on its own.
    let outcome = match action {
        Action::Suspend | Action::Unsuspend => {
            let mut tx = state.pool.begin().await?;
            let outcome = match action {
                Action::Suspend => {
                    user_penalties::suspend(&mut tx, &ctx, &guardian, user_id, &penalty).await?
                }
                _ => user_penalties::unsuspend(&mut tx, &guardian, user_id).await?,
            };
            if matches!(outcome, Outcome::Done(_)) {
                tx.commit().await?;
            }
            outcome
        }
        Action::Silence => {
            user_penalties::silence(&state.pool, &ctx, &guardian, user_id, &penalty).await?
        }
        Action::Unsilence => {
            user_penalties::unsilence(&state.pool, &ctx, &guardian, user_id).await?
        }
    };
    Ok(match outcome {
        Outcome::Done(body) => (StatusCode::OK, Json(body)).into_response(),
        Outcome::NotFound => super::topics::not_found_response(&state, false),
        Outcome::Forbidden => super::search::invalid_access(&state),
        Outcome::Invalid(errors) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "failed": "FAILED", "errors": errors })),
        )
            .into_response(),
    })
}

/// PUT /admin/users/:user_id/suspend
pub async fn suspend(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    handle(
        state,
        guardian,
        user_id,
        headers,
        uri,
        body,
        Action::Suspend,
    )
    .await
}

/// PUT /admin/users/:user_id/unsuspend
pub async fn unsuspend(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    handle(
        state,
        guardian,
        user_id,
        headers,
        uri,
        body,
        Action::Unsuspend,
    )
    .await
}

/// PUT /admin/users/:user_id/silence
pub async fn silence(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    handle(
        state,
        guardian,
        user_id,
        headers,
        uri,
        body,
        Action::Silence,
    )
    .await
}

/// PUT /admin/users/:user_id/unsilence
pub async fn unsilence(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    handle(
        state,
        guardian,
        user_id,
        headers,
        uri,
        body,
        Action::Unsilence,
    )
    .await
}
