//! Port of PostsController#destroy and #recover, and TopicsController#destroy:
//! DELETE /posts/:id, PUT /posts/:post_id/recover, DELETE /t/:id.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::post_destroyer::{self, Outcome, Request};
use crate::posting::Ctx;
use crate::posting::revisions::{find_post, find_post_with_deleted};
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// What the shared front lets through: the params, or the response that
/// ends the request.
enum Front {
    Params(Map<String, Value>),
    Stop(Response),
}

/// The shared front: params, CSRF and requires_login.
fn front(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
    method: &str,
) -> Front {
    let p = params::parse(uri.query(), headers, body);
    if !csrf_ok(state, headers, &form_pairs(&p), uri.path(), method) {
        return Front::Stop(bad_csrf());
    }
    if guardian.is_anonymous() {
        return Front::Stop(super::login_required::not_logged_in(state));
    }
    Front::Params(p)
}

/// The id without its `.json`.
fn id_of(raw: &str) -> i32 {
    crate::ruby::to_i(raw.strip_suffix(".json").unwrap_or(raw)) as i32
}

fn respond(state: &AppState, outcome: Outcome) -> Response {
    match outcome {
        // render body: nil
        Outcome::Done => StatusCode::OK.into_response(),
        Outcome::NotFound => super::topics::not_found_response(state),
        Outcome::Forbidden => super::search::invalid_access(state),
    }
}

/// `force_destroy` is permanent deletion, not ported.
fn refuse_force_destroy(p: &Map<String, Value>) -> Result<(), AppError> {
    let force = p
        .get("force_destroy")
        .and_then(params::scalar)
        .is_some_and(|v| {
            !matches!(
                v.as_str(),
                "" | "0" | "f" | "F" | "false" | "FALSE" | "off" | "OFF"
            )
        });
    if force {
        return Err(Unsupported("permanently deleting posts and topics (force_destroy)").into());
    }
    Ok(())
}

/// DELETE /posts/:id(.json)
pub async fn destroy_post(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match front(&state, &guardian, &headers, &uri, &body, "DELETE") {
        Front::Params(p) => p,
        Front::Stop(response) => return Ok(response),
    };
    refuse_force_destroy(&p)?;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let Some(access) = find_post(&mut conn, &ctx, &guardian, id_of(&id)).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    drop(conn);
    let context = p.get("context").and_then(params::scalar);
    let outcome = post_destroyer::destroy_post(
        &state.pool,
        &ctx,
        &guardian,
        &access,
        &Request {
            context: context.as_deref(),
        },
    )
    .await?;
    Ok(respond(&state, outcome))
}

/// PUT /posts/:post_id/recover(.json)
pub async fn recover_post(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Front::Stop(response) = front(&state, &guardian, &headers, &uri, &body, "PUT") {
        return Ok(response);
    }
    let post_id = id_of(&id);
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let Some(access) = find_post_with_deleted(&mut tx, &ctx, &guardian, post_id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let outcome = post_destroyer::recover_post(&mut tx, &ctx, &guardian, &access).await?;
    if !matches!(outcome, Outcome::Done) {
        return Ok(respond(&state, outcome));
    }
    tx.commit().await?;
    // render_post_json(post) after post.reload: with raw (include_raw?: a
    // hidden post's only for staff and its author), without a draft
    // sequence.
    let mut post =
        super::posts::serialize_post(&state, &settings, &guardian, post_id, false, false).await?;
    let (raw, hidden, author): (String, bool, Option<i32>) =
        sqlx::query_as("SELECT raw, hidden, user_id FROM posts WHERE id = $1")
            .bind(post_id)
            .fetch_one(&state.pool)
            .await?;
    let yours = guardian.user_id().is_some() && guardian.user_id() == author;
    if let Value::Object(map) = &mut post
        && (!hidden || guardian.is_staff() || yours)
    {
        map.insert("raw".into(), json!(raw));
    }
    Ok((StatusCode::OK, Json(post)).into_response())
}

/// DELETE /t/:id(.json)
pub async fn destroy_topic(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match front(&state, &guardian, &headers, &uri, &body, "DELETE") {
        Front::Params(p) => p,
        Front::Stop(response) => return Ok(response),
    };
    refuse_force_destroy(&p)?;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    // context: params[:context].presence || "Deleted via API"
    let context = p
        .get("context")
        .and_then(params::scalar)
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| ctx.t("staff_action_logs.api_post_delete"));
    drop(conn);
    let outcome = post_destroyer::destroy_topic(
        &state.pool,
        &ctx,
        &guardian,
        id_of(&id),
        &Request {
            context: Some(&context),
        },
    )
    .await?;
    Ok(match outcome {
        Outcome::Done => StatusCode::OK.into_response(),
        // rescue Discourse::InvalidAccess -> render_json_error delete_topic_failed
        Outcome::Forbidden | Outcome::NotFound => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "errors": [ctx.t("delete_topic_failed")] })),
        )
            .into_response(),
    })
}
