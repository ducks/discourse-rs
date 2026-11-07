//! Port of the PostsController actions the post admin menu calls:
//! PUT /posts/:id/post_type, /notice, /unhide, /locked, /wiki and
//! /rebake.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::posting::Ctx;
use crate::posting::post_types::{MODERATOR_ACTION, REGULAR, WHISPER};
use crate::posting::revise::{self, Changes};
use crate::posting::revisions::{PostAccess, find_post_with_deleted};
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// The id without its `.json`.
fn id_of(raw: &str) -> i32 {
    crate::ruby::to_i(raw.strip_suffix(".json").unwrap_or(raw)) as i32
}

/// The params, or the response that ends the request: the CSRF check and
/// ensure_logged_in.
fn front(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
) -> Result<Map<String, Value>, Box<Response>> {
    let p = params::parse(uri.query(), headers, body);
    if !csrf_ok(state, headers, &form_pairs(&p), uri.path(), "PUT") {
        return Err(Box::new(bad_csrf()));
    }
    if guardian.is_anonymous() {
        return Err(Box::new(super::login_required::not_logged_in(state)));
    }
    Ok(p)
}

/// `params.require(name)`: a value that isn't blank.
fn required(p: &Map<String, Value>, name: &str) -> Result<String, Box<Response>> {
    match p.get(name).and_then(params::scalar) {
        Some(v) if !crate::ruby::is_blank(&v) => Ok(v),
        _ => Err(Box::new(
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "errors": [format!("param is missing or the value is empty or invalid: {name}")]
                })),
            )
                .into_response(),
        )),
    }
}

fn not_found(state: &AppState) -> Response {
    super::topics::not_found_response(state)
}

fn forbidden(state: &AppState) -> Response {
    super::search::invalid_access(state)
}

/// What each action runs with: the site's settings and the request's
/// guardian.
struct Site {
    settings: SiteSettings,
    host: Host,
}

impl Site {
    async fn load(state: &AppState) -> Result<Site, AppError> {
        let mut conn = state.pool.acquire().await?;
        let settings =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
        Ok(Site {
            settings,
            host: Host::from_state(state),
        })
    }

    fn ctx<'a>(&'a self, state: &'a AppState) -> Ctx<'a> {
        Ctx {
            host: &self.host,
            settings: &self.settings,
            config: &state.config,
            i18n: &state.i18n,
            bus: &state.bus,
        }
    }
}

/// `find_post_from_params`: the post when the viewer may see it (a
/// deleted one when they moderate its topic).
async fn find(
    state: &AppState,
    ctx: &Ctx<'_>,
    guardian: &crate::guardian::Guardian,
    id: &str,
) -> Result<Option<PostAccess>, AppError> {
    let mut conn = state.pool.acquire().await?;
    find_post_with_deleted(&mut conn, ctx, guardian, id_of(id)).await
}

/// PUT /posts/:id/post_type(.json): staff turning a post into a moderator
/// action or a whisper and back, through the revisor.
pub async fn post_type(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match front(&state, &guardian, &headers, &uri, &body) {
        Ok(p) => p,
        Err(response) => return Ok(*response),
    };
    // ensure_can_change_post_type!
    if !guardian.is_staff() {
        return Ok(forbidden(&state));
    }
    let site = Site::load(&state).await?;
    let ctx = site.ctx(&state);
    let Some(access) = find(&state, &ctx, &guardian, &id).await? else {
        return Ok(not_found(&state));
    };
    let raw = match required(&p, "post_type") {
        Ok(v) => v,
        Err(response) => return Ok(*response),
    };
    let post_type = crate::ruby::to_i(&raw) as i32;
    // PostRevisor.valid_post_type?
    if ![REGULAR, MODERATOR_ACTION, WHISPER].contains(&post_type)
        || (access.post.post_number == 1 && post_type == WHISPER)
    {
        return Ok(super::search::invalid_parameters(&state, "post_type"));
    }
    if post_type == WHISPER && !guardian.is_whisperer(&site.settings)? {
        return Ok(super::search::invalid_access_with(
            &state,
            "invalid_whisper_access",
        ));
    }
    let changes = Changes {
        post_type: Some(post_type),
        ..Default::default()
    };
    // render body: nil, whatever the revisor made of it.
    match revise::revise(&state.pool, &ctx, &guardian, access.post.id, changes).await? {
        revise::Outcome::RateLimited(refused) => Ok(refused.into_response(&state.i18n)),
        _ => Ok(StatusCode::OK.into_response()),
    }
}

/// PUT /posts/:id/wiki(.json): the post made a wiki or not, through the
/// revisor, by who can_wiki?.
pub async fn wiki(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match front(&state, &guardian, &headers, &uri, &body) {
        Ok(p) => p,
        Err(response) => return Ok(*response),
    };
    let site = Site::load(&state).await?;
    let ctx = site.ctx(&state);
    let Some(access) = find(&state, &ctx, &guardian, &id).await? else {
        return Ok(not_found(&state));
    };
    let raw = match required(&p, "wiki") {
        Ok(v) => v,
        Err(response) => return Ok(*response),
    };
    if !guardian.can_wiki(&site.settings, &access.post)? {
        return Ok(forbidden(&state));
    }
    // The column's boolean cast of the param.
    let wiki = crate::ruby::cast_boolean(Some(&raw)) == Some(true);
    let changes = Changes {
        wiki: Some(wiki),
        ..Default::default()
    };
    match revise::revise(&state.pool, &ctx, &guardian, access.post.id, changes).await? {
        // render_json_error(post)
        revise::Outcome::Invalid(errors) => Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "errors": errors })),
        )
            .into_response()),
        revise::Outcome::RateLimited(refused) => Ok(refused.into_response(&state.i18n)),
        _ => Ok(StatusCode::OK.into_response()),
    }
}

/// PUT /posts/:id/unhide(.json): staff showing a hidden post again.
pub async fn unhide(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Err(response) = front(&state, &guardian, &headers, &uri, &body) {
        return Ok(*response);
    }
    let site = Site::load(&state).await?;
    let ctx = site.ctx(&state);
    let Some(access) = find(&state, &ctx, &guardian, &id).await? else {
        return Ok(not_found(&state));
    };
    // ensure_can_unhide!
    if !(access.post.hidden && guardian.is_staff()) {
        return Ok(forbidden(&state));
    }
    let mut tx = state.pool.begin().await?;
    crate::posts::admin::unhide(&mut tx, &ctx, access.post.id).await?;
    tx.commit().await?;
    Ok(StatusCode::OK.into_response())
}

/// PUT /posts/:id/locked(.json): PostLocker, locking for `locked=true`
/// and unlocking for anything else; answered with the post's state.
pub async fn locked(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match front(&state, &guardian, &headers, &uri, &body) {
        Ok(p) => p,
        Err(response) => return Ok(*response),
    };
    let site = Site::load(&state).await?;
    let ctx = site.ctx(&state);
    let Some(access) = find(&state, &ctx, &guardian, &id).await? else {
        return Ok(not_found(&state));
    };
    // ensure_can_lock_post!: can_see_post? and staff.
    if !guardian.is_staff() {
        return Ok(forbidden(&state));
    }
    let lock = p.get("locked").and_then(params::scalar).as_deref() == Some("true");
    let user_id = guardian.user_id().unwrap_or_default();
    let mut tx = state.pool.begin().await?;
    crate::posts::admin::set_locked(&mut tx, &ctx, access.post.id, user_id, lock).await?;
    tx.commit().await?;
    Ok((StatusCode::OK, Json(json!({ "locked": lock }))).into_response())
}

/// PUT /posts/:id/notice(.json): the staff notice set or removed, by who
/// may edit staff notes in the topic (others get a 404).
pub async fn notice(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match front(&state, &guardian, &headers, &uri, &body) {
        Ok(p) => p,
        Err(response) => return Ok(*response),
    };
    let site = Site::load(&state).await?;
    let ctx = site.ctx(&state);
    let Some(access) = find(&state, &ctx, &guardian, &id).await? else {
        return Ok(not_found(&state));
    };
    // can_edit_staff_notes?(topic)
    if !guardian
        .can_perform_action_available_to_group_moderators(&site.settings, access.can_see_topic)?
    {
        return Ok(not_found(&state));
    }
    let text = p.get("notice").and_then(params::scalar);
    let user_id = guardian.user_id().unwrap_or_default();
    let mut tx = state.pool.begin().await?;
    let cooked =
        crate::posts::admin::set_notice(&mut tx, &ctx, access.post.id, user_id, text.as_deref())
            .await?;
    tx.commit().await?;
    Ok((
        StatusCode::OK,
        Json(json!({ "success": "OK", "cooked_notice": cooked })),
    )
        .into_response())
}

/// PUT /posts/:id/rebake(.json): staff and trust level 4 cooking a post
/// again, its oneboxes and failed images tried anew.
pub async fn rebake(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Err(response) = front(&state, &guardian, &headers, &uri, &body) {
        return Ok(*response);
    }
    // ensure_can_rebake!
    if !(guardian.is_staff() || guardian.has_trust_level(4)) {
        return Ok(forbidden(&state));
    }
    let site = Site::load(&state).await?;
    let ctx = site.ctx(&state);
    let Some(access) = find(&state, &ctx, &guardian, &id).await? else {
        return Ok(not_found(&state));
    };
    crate::posting::rebake::rebake_with(
        &state.pool,
        &ctx,
        access.post.id,
        &crate::posting::rebake::Options {
            invalidate_oneboxes: true,
            invalidate_broken_images: true,
            ..Default::default()
        },
    )
    .await?;
    Ok(StatusCode::OK.into_response())
}
