//! Port of PostActionsController for likes and flags: POST
//! /post_actions(.json), and DELETE /post_actions/:id(.json) for likes,
//! answering with the post as `render_post_json(post, add_raw: false)` does.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::flags::{self, FlagRequest};
use crate::likes::{self, Outcome};
use crate::params;
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// `fetch_post_action_type_id_from_params`
fn action_type(p: &Map<String, Value>) -> Option<i64> {
    params::scalar(p.get("post_action_type_id").unwrap_or(&Value::Null))
        .map(|t| crate::ruby::to_i(&t))
}

/// A param as Rails' `params[:x] == "true"` reads it.
fn is_true(p: &Map<String, Value>, key: &str) -> bool {
    p.get(key)
        .and_then(params::scalar)
        .is_some_and(|v| v == "true")
}

async fn respond(
    state: &AppState,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    outcome: Outcome,
    post_id: i32,
) -> Result<Response, AppError> {
    match outcome {
        Outcome::Done => {
            let post =
                super::posts::serialize_post(state, settings, guardian, post_id, false, false)
                    .await?;
            Ok((StatusCode::OK, Json(post)).into_response())
        }
        Outcome::NotFound => Ok(super::topics::not_found_response(state)),
        Outcome::ResultNotFound => Ok((
            StatusCode::NOT_FOUND,
            Json(json!({"errors": [state.i18n.t("not_found").unwrap_or("not_found")]})),
        )
            .into_response()),
        Outcome::NoContent => Ok(StatusCode::NO_CONTENT.into_response()),
        Outcome::Forbidden(key) => Ok((
            StatusCode::FORBIDDEN,
            Json(json!({"errors": [state.i18n.t(key).unwrap_or(key)]})),
        )
            .into_response()),
        Outcome::RateLimited(refused) => Ok(refused.into_response(&state.i18n)),
        Outcome::Invalid(errors) => Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "errors": errors })),
        )
            .into_response()),
    }
}

/// The shared front of create and destroy: CSRF, `requires_login`, the
/// required params, the settings and the cooking context.
async fn front(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
    method: &str,
) -> Result<Result<(Map<String, Value>, SiteSettings, i64), Response>, AppError> {
    let p = params::parse(uri.query(), headers, body);
    if !csrf_ok(state, headers, &form_pairs(&p), uri.path(), method) {
        return Ok(Err(bad_csrf()));
    }
    if guardian.is_anonymous() {
        return Ok(Err(super::login_required::not_logged_in(state)));
    }
    let Some(type_id) = action_type(&p) else {
        return Ok(Err(super::accounts::param_missing("post_action_type_id")));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    Ok(Ok((p, settings, type_id)))
}

/// POST /post_actions(.json)
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let (p, settings, type_id) =
        match front(&state, &guardian, &headers, &uri, &body, "POST").await? {
            Ok(v) => v,
            Err(response) => return Ok(response),
        };
    let Some(id) = p
        .get("id")
        .and_then(params::scalar)
        .filter(|i| !i.is_empty())
    else {
        return Ok(super::accounts::param_missing("id"));
    };
    // fetch_post_from_params: a topic's flag is its first post's, for a
    // topic the viewer can see.
    let post_id = if is_true(&p, "flag_topic") {
        let topic_id = crate::ruby::to_i(&id) as i32;
        let mut conn = state.pool.acquire().await?;
        let Some(topic) =
            crate::topic_guardian::TopicCtx::load(&mut conn, &settings, &guardian, topic_id)
                .await?
        else {
            return Ok(super::topics::not_found_response(&state));
        };
        let secure = guardian.secure_category_ids(&mut conn, &settings).await?;
        if !guardian.can_see_topic(&settings, &topic, true, &secure)? {
            return Ok(super::topics::not_found_response(&state));
        }
        let first: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM posts WHERE topic_id = $1 AND deleted_at IS NULL ORDER BY id LIMIT 1",
        )
        .bind(topic_id)
        .fetch_optional(&mut *conn)
        .await?;
        let Some(first) = first else {
            return Ok(super::topics::not_found_response(&state));
        };
        first
    } else {
        crate::ruby::to_i(&id) as i32
    };
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let outcome = if type_id == crate::post_actions::LIKE {
        let outcome = likes::like(&state.pool, &ctx, &guardian, post_id).await?;
        if let Outcome::Done = outcome
            && let Some(user) = guardian.user()
        {
            // The like's limiter: what is left of the day's likes.
            let limiter = crate::post_actions::daily_limiter(
                &settings,
                likes::limit_user(&guardian, user),
                user.trust_level,
                "like",
            )?;
            let remaining = limiter.remaining(&state.pool).await?;
            let mut response = respond(&state, &settings, &guardian, outcome, post_id).await?;
            let headers = response.headers_mut();
            headers.insert("Discourse-Actions-Remaining", remaining.into());
            headers.insert("Discourse-Actions-Max", limiter.max().into());
            return Ok(response);
        }
        outcome
    } else {
        let request = FlagRequest {
            type_id,
            take_action: is_true(&p, "take_action"),
            queue_for_review: is_true(&p, "queue_for_review"),
            message: p.get("message").and_then(params::scalar),
            is_warning: crate::ruby::cast_boolean(
                p.get("is_warning").and_then(params::scalar).as_deref(),
            )
            .unwrap_or(false),
            flag_topic: is_true(&p, "flag_topic"),
            reason: None,
            context: None,
        };
        flags::flag(&state.pool, &ctx, &guardian, post_id, &request).await?
    };
    respond(&state, &settings, &guardian, outcome, post_id).await
}

/// DELETE /post_actions/:id(.json)
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let (p, settings, type_id) =
        match front(&state, &guardian, &headers, &uri, &body, "DELETE").await? {
            Ok(v) => v,
            Err(response) => return Ok(response),
        };
    if type_id != crate::post_actions::LIKE {
        return Err(Unsupported("undoing flags").into());
    }
    if is_true(&p, "flag_topic") {
        return Err(Unsupported("undoing a topic's flag").into());
    }
    let post_id = crate::ruby::to_i(id.strip_suffix(".json").unwrap_or(&id)) as i32;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let outcome = likes::unlike(&state.pool, &ctx, &guardian, post_id).await?;
    respond(&state, &settings, &guardian, outcome, post_id).await
}
