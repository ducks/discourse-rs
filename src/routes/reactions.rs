//! discourse-reactions' CustomReactionsController: PUT
//! /discourse-reactions/posts/:post_id/custom-reactions/:reaction/toggle,
//! answering with the post as PostSerializer has it.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::plugins::reactions::toggle::{self, Outcome};
use crate::plugins::reactions::users::{Reader, ReceivedParams};
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

/// PUT /discourse-reactions/posts/:post_id/custom-reactions/:reaction/toggle
pub async fn toggle(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((post_id, reaction)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // requires_plugin
    if !crate::plugins::reactions::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let post_id = crate::ruby::to_i(&post_id) as i32;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    match toggle::toggle(&mut tx, &ctx, &guardian, post_id, &reaction).await? {
        Outcome::Done(like_count) => {
            tx.commit().await?;
            let mut post =
                super::posts::serialize_post_alone(&state, &settings, &guardian, post_id).await?;
            stale_like_count(&mut post, like_count);
            Ok((StatusCode::OK, Json(post)).into_response())
        }
        Outcome::NotFound => Ok(super::topics::not_found_response(&state)),
        Outcome::InvalidAccess => Ok(super::search::invalid_access(&state)),
        // render_json_error(post): a post without errors gets
        // JsonError.generic_error, the client locale's js.generic_error.
        Outcome::InvalidReaction => Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"errors": ["Sorry, an error has occurred."]})),
        )
            .into_response()),
    }
}

/// The service serializes the post it loaded before toggling, whose
/// like_count the like's counters (an update_all) never touched: the like
/// summary counts what was there before, and says nothing at none.
fn stale_like_count(post: &mut Value, like_count: i32) {
    let Some(Value::Array(summary)) = post.get_mut("actions_summary") else {
        return;
    };
    let like = crate::post_actions::LIKE;
    let Some(entry) = summary
        .iter_mut()
        .find(|e| e.get("id").and_then(Value::as_i64) == Some(like))
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let mut rebuilt = Map::new();
    for (key, value) in std::mem::take(entry) {
        if key == "count" {
            continue;
        }
        let is_id = key == "id";
        rebuilt.insert(key, value);
        if is_id && like_count > 0 {
            rebuilt.insert("count".into(), json!(like_count));
        }
    }
    *entry = rebuilt;
}

fn query_param(p: &Map<String, Value>, key: &str) -> Option<String> {
    p.get(key).and_then(params::scalar)
}

/// The settings and the plugin check the reads share.
async fn front(state: &AppState) -> Result<Result<SiteSettings, Response>, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // requires_plugin
    if !crate::plugins::reactions::enabled(&settings)? {
        return Ok(Err(super::topics::not_found_response(state)));
    }
    Ok(Ok(settings))
}

/// `fetch_post_from_params`: a live post (404) the viewer can see (403).
async fn fetch_post(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    id: &str,
) -> Result<Result<crate::posting::revisions::PostAccess, Response>, AppError> {
    let post_id = crate::ruby::to_i(id.strip_suffix(".json").unwrap_or(id)) as i32;
    let live: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM posts WHERE id = $1 AND deleted_at IS NULL)",
    )
    .bind(post_id)
    .fetch_one(&mut *conn)
    .await?;
    if !live {
        return Ok(Err(super::topics::not_found_response(state)));
    }
    let host = Host::from_state(state);
    let ctx = Ctx {
        host: &host,
        settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    match crate::posting::revisions::find_post(&mut *conn, &ctx, guardian, post_id).await? {
        Some(access) => Ok(Ok(access)),
        None => Ok(Err(super::search::invalid_access(state))),
    }
}

/// GET /discourse-reactions/posts/:id/reactions-users
pub async fn post_reactions_users(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let settings = match front(&state).await? {
        Ok(s) => s,
        Err(r) => return Ok(r),
    };
    let p = params::parse(uri.query(), &headers, &body);
    let mut conn = state.pool.acquire().await?;
    let post = match fetch_post(&state, &mut conn, &settings, &guardian, &id).await? {
        Ok(post) => post,
        Err(r) => return Ok(r),
    };
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let mut reader = Reader {
        conn: &mut conn,
        settings: &settings,
        urls: &urls,
        guardian: &guardian,
    };
    let value = query_param(&p, "reaction_value");
    let body = reader.post_reactions_users(&post, value.as_deref()).await?;
    Ok(Json(body).into_response())
}

/// GET /discourse-reactions/posts/:id/reactions-users-list
pub async fn reactions_users_list(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let settings = match front(&state).await? {
        Ok(s) => s,
        Err(r) => return Ok(r),
    };
    let p = params::parse(uri.query(), &headers, &body);
    let mut conn = state.pool.acquire().await?;
    let post = match fetch_post(&state, &mut conn, &settings, &guardian, &id).await? {
        Ok(post) => post,
        Err(r) => return Ok(r),
    };
    let page = query_param(&p, "page")
        .map(|v| crate::ruby::to_i(&v))
        .unwrap_or(0)
        .max(0);
    let limit = query_param(&p, "limit")
        .filter(|v| !v.trim().is_empty())
        .map(|v| crate::ruby::to_i(&v).clamp(1, 50))
        .unwrap_or(30);
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let mut reader = Reader {
        conn: &mut conn,
        settings: &settings,
        urls: &urls,
        guardian: &guardian,
    };
    let filter = query_param(&p, "reaction_value");
    let body = reader
        .reactions_users_list(post.post.id, filter.as_deref(), limit, page * limit)
        .await?;
    Ok(Json(body).into_response())
}

#[derive(Clone, Copy)]
enum Feed {
    Given,
    Received,
}

/// GET /discourse-reactions/posts/reactions
pub async fn reactions_given(
    state: State<AppState>,
    guardian: AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    feed(state, guardian, headers, uri, body, Feed::Given).await
}

/// GET /discourse-reactions/posts/reactions-received
pub async fn reactions_received(
    state: State<AppState>,
    guardian: AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    feed(state, guardian, headers, uri, body, Feed::Received).await
}

async fn feed(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    kind: Feed,
) -> Result<Response, AppError> {
    let settings = match front(&state).await? {
        Ok(s) => s,
        Err(r) => return Ok(r),
    };
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let p = params::parse(uri.query(), &headers, &body);
    let Some(username) = query_param(&p, "username").filter(|u| !u.is_empty()) else {
        return Ok(super::accounts::param_missing("username"));
    };
    let mut conn = state.pool.acquire().await?;
    // fetch_user_from_params(include_inactive: staff, or show_inactive_accounts)
    let username_lower = username.to_lowercase();
    let username_lower = username_lower
        .strip_suffix(".json")
        .unwrap_or(&username_lower)
        .to_string();
    let include_inactive = guardian.is_staff() || settings.get("show_inactive_accounts")?.truthy();
    let user_id: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM users WHERE username_lower = $1 AND (active OR $2 OR id = $3) LIMIT 1",
    )
    .bind(&username_lower)
    .bind(include_inactive)
    .bind(guardian.user_id().unwrap_or(0))
    .fetch_optional(&mut *conn)
    .await?;
    let Some(user_id) = user_id else {
        return Ok(super::topics::not_found_response(&state));
    };
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let body = match kind {
        Feed::Given => {
            let Some(user) = crate::users::User::find_by_id(&mut conn, user_id).await? else {
                return Ok(super::topics::not_found_response(&state));
            };
            // can_see_profile?, can_see_user_actions?
            let sees_private = guardian.is_me(user_id) || guardian.is_admin();
            if !user.visible_to(&settings, &guardian)?
                || (!sees_private && settings.get("hide_user_activity_tab")?.truthy())
            {
                return Ok(super::topics::not_found_response(&state));
            }
            let before = query_param(&p, "before_reaction_user_id").map(|v| crate::ruby::to_i(&v));
            let mut reader = Reader {
                conn: &mut conn,
                settings: &settings,
                urls: &urls,
                guardian: &guardian,
            };
            reader.reactions_given(user_id, before).await?
        }
        Feed::Received => {
            // can_see_notifications?
            if !(guardian.is_me(user_id) || guardian.is_admin()) {
                return Ok(super::search::invalid_access(&state));
            }
            let before_reaction = query_param(&p, "before_reaction_user_id")
                .filter(|v| !v.trim().is_empty())
                .or_else(|| query_param(&p, "before_post_id"));
            let acting = query_param(&p, "acting_username");
            let params = ReceivedParams {
                before_reaction_user_id: before_reaction.map(|v| crate::ruby::to_i(&v)),
                before_like_id: query_param(&p, "before_like_id").map(|v| crate::ruby::to_i(&v)),
                acting_username: acting.as_deref(),
                include_likes: p.contains_key("include_likes"),
            };
            let mut reader = Reader {
                conn: &mut conn,
                settings: &settings,
                urls: &urls,
                guardian: &guardian,
            };
            reader.reactions_received(user_id, &params).await?
        }
    };
    Ok(Json(body).into_response())
}
