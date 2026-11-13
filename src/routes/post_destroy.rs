//! Port of PostsController#destroy, #destroy_many and #recover, and
//! TopicsController#destroy: DELETE /posts/:id, DELETE /posts/destroy_many,
//! PUT /posts/:post_id/recover, DELETE /t/:id.

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
pub(super) enum Front {
    Params(Map<String, Value>),
    Stop(Response),
}

/// The shared front: params, CSRF and requires_login.
pub(super) fn front(
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
        Outcome::RateLimited(refused) => refused.into_response(&state.i18n),
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
            reviewable_id: None,
            deferred: None,
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
    let outcome =
        post_destroyer::recover_post(&state.pool, &mut tx, &ctx, &guardian, &access).await?;
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
            reviewable_id: None,
            deferred: None,
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
        Outcome::RateLimited(refused) => refused.into_response(&state.i18n),
    })
}

/// A list param's members as `to_i` has them; ids no post could have are
/// dropped.
pub(super) fn ids_of(list: &[Value], what: &'static str) -> Result<Vec<i32>, Unsupported> {
    let mut ids = Vec::with_capacity(list.len());
    for item in list {
        let item = params::scalar(item).ok_or(Unsupported(what))?;
        if let Ok(id) = i32::try_from(crate::ruby::to_i(&item)) {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// DELETE /posts/destroy_many(.json)
pub async fn destroy_many(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match front(&state, &guardian, &headers, &uri, &body, "DELETE") {
        Front::Params(p) => p,
        Front::Stop(response) => return Ok(response),
    };
    // params.require(:post_ids), then post_ids_including_replies:
    // params[:post_ids].map(&:to_i)
    let mut post_ids = match p.get("post_ids") {
        None | Some(Value::Null) => return Ok(super::accounts::param_missing("post_ids")),
        Some(Value::String(s)) if s.is_empty() => {
            return Ok(super::accounts::param_missing("post_ids"));
        }
        Some(Value::Array(list)) if list.is_empty() => {
            return Ok(super::accounts::param_missing("post_ids"));
        }
        Some(Value::Array(list)) => ids_of(list, "post_ids holding lists or hashes")?,
        Some(_) => {
            return Err(
                Unsupported("post_ids that are not a list (NoMethodError in Rails)").into(),
            );
        }
    };
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
    // PostReply.where(post_id: params[:reply_post_ids]).pluck(:reply_post_id)
    let parents = match p.get("reply_post_ids") {
        None | Some(Value::Null | Value::Bool(false)) => None,
        Some(Value::Array(list)) => Some(ids_of(list, "reply_post_ids holding lists or hashes")?),
        Some(Value::Object(_)) => return Err(Unsupported("reply_post_ids as a hash").into()),
        Some(one) => Some(ids_of(
            std::slice::from_ref(one),
            "reply_post_ids as a hash",
        )?),
    };
    if let Some(parents) = parents {
        let replies: Vec<i32> =
            sqlx::query_scalar("SELECT reply_post_id FROM post_replies WHERE post_id = ANY($1)")
                .bind(&parents)
                .fetch_all(&mut *conn)
                .await?;
        post_ids.extend(replies);
    }
    // Post.where(id:).order(:id)
    let posts: Vec<(i32, i32)> = sqlx::query_as(
        "SELECT id, topic_id FROM posts WHERE id = ANY($1) AND deleted_at IS NULL ORDER BY id",
    )
    .bind(&post_ids)
    .fetch_all(&mut *conn)
    .await?;
    if posts.is_empty() {
        return Ok(super::search::invalid_parameters(&state, "post_ids"));
    }
    // guardian.ensure_can_delete_post_or_topic!(p), each before any goes.
    let mut sees = Vec::with_capacity(posts.len());
    for &(id, topic_id) in &posts {
        match post_destroyer::can_delete(&mut conn, &ctx, &guardian, id, topic_id, true).await? {
            Some(can_see_topic) => sees.push(can_see_topic),
            None => return Ok(super::search::invalid_access(&state)),
        }
    }
    let mut prepared = Vec::with_capacity(posts.len());
    for (&(id, _), &can_see_topic) in posts.iter().zip(&sees) {
        prepared
            .push(post_destroyer::prepare(&mut conn, &ctx, &guardian, id, can_see_topic).await?);
    }
    drop(conn);
    // Post.transaction. `defer_flags` (agree_with_first_reply_flag) only
    // reaches a post with a pending flag, which the destroyer refuses.
    let mut tx = state.pool.begin().await?;
    let held = post_destroyer::Deferred::default();
    let req = Request {
        context: None,
        reviewable_id: None,
        deferred: Some(&held),
    };
    for prepared in &prepared {
        post_destroyer::destroy_prepared(&mut tx, &ctx, &guardian, prepared, &req).await?;
    }
    for (name, args) in held
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    {
        crate::jobs::enqueue(&mut tx, name, args).await?;
    }
    tx.commit().await?;
    // DiscourseEvent :posts_destroyed: nothing listens.
    Ok(StatusCode::OK.into_response())
}

/// Another verb on a collection route's path (DELETE /posts/merge_posts):
/// Rails routes it to posts#destroy with the name as `:id`.
pub async fn destroy_named(
    state: State<AppState>,
    guardian: AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let id = super::posts::last_segment(&uri);
    destroy_post(state, guardian, Path(id), headers, uri, body).await
}
