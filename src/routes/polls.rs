//! discourse-poll's PollsController, mounted at /polls: PUT and DELETE
//! vote, PUT toggle_status, GET voters and grouped_poll_results.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::guardian::Guardian;
use crate::params;
use crate::plugins::poll::vote::{self, Outcome};
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// `params.require(key)`: a present scalar.
fn required(p: &Map<String, Value>, key: &str) -> Option<String> {
    p.get(key)
        .and_then(params::scalar)
        .filter(|v| !v.trim().is_empty())
}

fn respond(state: &AppState, outcome: Outcome) -> Response {
    match outcome {
        Outcome::Done(body) => (StatusCode::OK, Json(body)).into_response(),
        Outcome::Error(message) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "errors": [message] })),
        )
            .into_response(),
        Outcome::InvalidParameters(name) => super::search::invalid_parameters(state, name),
        Outcome::Forbidden => super::search::invalid_access(state),
        Outcome::NotFound(error) => {
            (StatusCode::NOT_FOUND, Json(json!({ "error": error }))).into_response()
        }
        Outcome::Unprocessable(error) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": error })),
        )
            .into_response(),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Vote,
    RemoveVote,
    ToggleStatus,
    Voters,
    GroupedResults,
}

async fn run(
    state: AppState,
    guardian: Guardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    method: &str,
    action: Action,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if method != "GET" && !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), method) {
        return Ok(bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // requires_plugin
    if !crate::plugins::poll::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    // ensure_logged_in, except voters and grouped_poll_results.
    let open = matches!(action, Action::Voters | Action::GroupedResults);
    if !open && guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    // ensure_can_see_post
    let Some(post_id) = required(&p, "post_id") else {
        return Ok(super::accounts::param_missing("post_id"));
    };
    let post_id = crate::ruby::to_i(&post_id);
    if !vote::ensure_can_see_post(&mut tx, &ctx, &guardian, post_id).await? {
        return Ok(super::search::invalid_access(&state));
    }
    let Some(poll_name) = required(&p, "poll_name") else {
        return Ok(super::accounts::param_missing("poll_name"));
    };
    let scalar = |key: &str| p.get(key).and_then(params::scalar);
    let outcome = match action {
        Action::Vote => {
            let options = p.get("options").filter(|o| match o {
                Value::Array(a) => !a.is_empty(),
                Value::Object(m) => !m.is_empty(),
                Value::String(s) => !s.trim().is_empty(),
                Value::Null => false,
                _ => true,
            });
            let Some(options) = options else {
                return Ok(super::accounts::param_missing("options"));
            };
            vote::vote(&mut tx, &ctx, &guardian, post_id, &poll_name, options).await?
        }
        Action::RemoveVote => {
            vote::remove_vote(&mut tx, &ctx, &guardian, post_id, &poll_name).await?
        }
        Action::ToggleStatus => {
            let Some(status) = required(&p, "status") else {
                return Ok(super::accounts::param_missing("status"));
            };
            match vote::toggle_status(&mut tx, &ctx, &guardian, post_id, &poll_name, &status, true)
                .await?
            {
                Outcome::Done(poll) => Outcome::Done(json!({ "poll": poll })),
                other => other,
            }
        }
        Action::Voters => {
            vote::voters(
                &mut tx,
                &ctx,
                &guardian,
                post_id,
                &poll_name,
                &vote::VoterPage {
                    page: scalar("page").as_deref(),
                    limit: scalar("limit").as_deref(),
                    option_id: scalar("option_id").as_deref(),
                },
            )
            .await?
        }
        Action::GroupedResults => {
            let Some(field) = required(&p, "user_field_name") else {
                return Ok(super::accounts::param_missing("user_field_name"));
            };
            vote::grouped_poll_results(&mut tx, &ctx, &guardian, post_id, &poll_name, &field)
                .await?
        }
    };
    if matches!(outcome, Outcome::Done(_)) {
        tx.commit().await?;
    }
    Ok(respond(&state, outcome))
}

/// PUT /polls/vote
pub async fn vote(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    run(state, guardian, headers, uri, body, "PUT", Action::Vote).await
}

/// DELETE /polls/vote
pub async fn remove_vote(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    run(
        state,
        guardian,
        headers,
        uri,
        body,
        "DELETE",
        Action::RemoveVote,
    )
    .await
}

/// PUT /polls/toggle_status
pub async fn toggle_status(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    run(
        state,
        guardian,
        headers,
        uri,
        body,
        "PUT",
        Action::ToggleStatus,
    )
    .await
}

/// GET /polls/voters
pub async fn voters(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    run(state, guardian, headers, uri, body, "GET", Action::Voters).await
}

/// GET /polls/grouped_poll_results
pub async fn grouped_poll_results(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    run(
        state,
        guardian,
        headers,
        uri,
        body,
        "GET",
        Action::GroupedResults,
    )
    .await
}
