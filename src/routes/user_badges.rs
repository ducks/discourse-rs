//! UserBadgesController's writes: POST /user_badges (grant), DELETE
//! /user_badges/:id (revoke) and PUT /user_badges/:id/toggle_favorite;
//! and UsersController#badge_title, PUT /u/:username/preferences/badge_title,
//! and #read_faq, POST /u/read-faq (which queues the user's badges).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::badge_granter::{self, Badge, GrantOpts, History};
use crate::badges_view::{BadgeRow, BadgesView, Side, detailed_document};
use crate::guardian::Guardian;
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// `failed_json` with a status.
fn failed(status: StatusCode) -> Response {
    (status, Json(json!({"failed": "FAILED"}))).into_response()
}

/// A present (non-blank) param.
fn present(p: &Map<String, Value>, key: &str) -> Option<String> {
    p.get(key)
        .and_then(params::scalar)
        .filter(|v| !v.trim().is_empty())
}

/// The shared front: CSRF, `ensure_badges_enabled`, `ensure_logged_in`.
async fn front(
    state: &AppState,
    guardian: &Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
    method: &str,
) -> Result<Result<(Map<String, Value>, SiteSettings), Response>, AppError> {
    let p = params::parse(uri.query(), headers, body);
    if !csrf_ok(state, headers, &form_pairs(&p), uri.path(), method) {
        return Ok(Err(bad_csrf()));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_badges")?.truthy() {
        return Ok(Err(super::topics::not_found_response(state)));
    }
    if guardian.is_anonymous() {
        return Ok(Err(super::login_required::not_logged_in(state)));
    }
    Ok(Ok((p, settings)))
}

/// `can_assign_badge_to_user?`: `can_grant_badges?` (badges are on).
fn can_assign(guardian: &Guardian) -> bool {
    guardian.is_staff()
}

/// `fetch_badge_from_params`: by badge_name, else the required badge_id;
/// enabled badges only. Err is the response.
async fn fetch_badge(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    p: &Map<String, Value>,
) -> Result<Result<Badge, Response>, AppError> {
    let badge = match p.get("badge_name").and_then(params::scalar) {
        Some(name) => Badge::find_enabled_by_name(&mut *conn, &name).await?,
        None => {
            let Some(id) = present(p, "badge_id") else {
                return Ok(Err(super::accounts::param_missing("badge_id")));
            };
            let id = i32::try_from(crate::ruby::to_i(&id)).unwrap_or(0);
            Badge::find(&mut *conn, id).await?.filter(|b| b.enabled)
        }
    };
    Ok(badge.ok_or_else(|| super::topics::not_found_response(state)))
}

/// The grant's post from a `reason` link: a topic URL's post (the first
/// when it names none). Err is the 400 for a link that isn't the site's
/// posts or topics.
async fn reason_post(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    reason: &str,
) -> Result<Result<Option<i32>, Response>, AppError> {
    let invalid = || {
        let message = state
            .i18n
            .t("invalid_grant_badge_reason_link")
            .unwrap_or("invalid_grant_badge_reason_link");
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"failed": "FAILED", "message": message})),
        )
            .into_response()
    };
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let hostname = urls.current_hostname()?;
    let base_path = state.config.globals.relative_url_root();
    let (host, path) = match reason.split_once("://") {
        Some((_, rest)) => {
            let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            let authority = &rest[..end];
            let host = authority.rsplit('@').next().unwrap_or(authority);
            let host = host.split(':').next().unwrap_or(host);
            (Some(host.to_lowercase()), &rest[end..])
        }
        None if reason.starts_with("//") => {
            return Err(Unsupported("protocol-relative badge reason links").into());
        }
        None => (None, reason),
    };
    let path = path.split(['?', '#']).next().unwrap_or("");
    if host
        .as_deref()
        .is_some_and(|h| h != hostname || !path.starts_with(base_path))
    {
        return Ok(Err(invalid()));
    }
    let path = &path[base_path.len().min(path.len())..];
    let segments: Vec<&str> = path
        .trim_start_matches('/')
        .trim_end_matches('/')
        .split('/')
        .collect();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    // topics#show: the post the link names.
    let show = match segments.as_slice() {
        ["t", id] if digits(id) => Some((*id, None)),
        ["t", _, id] if digits(id) => Some((*id, None)),
        ["t", id, n] if digits(id) && digits(n) => Some((*id, Some(*n))),
        ["t", _, id, n] if digits(id) && digits(n) => Some((*id, Some(*n))),
        _ => None,
    };
    if let Some((topic_id, post_number)) = show {
        let topic_id = i32::try_from(crate::ruby::to_i(topic_id)).unwrap_or(0);
        let post_number = post_number.map_or(1, crate::ruby::to_i);
        let post_id: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM posts WHERE topic_id = $1 AND post_number = $2 AND deleted_at IS NULL \
             ORDER BY id LIMIT 1",
        )
        .bind(topic_id)
        .bind(i32::try_from(post_number).unwrap_or(0))
        .fetch_optional(&mut *conn)
        .await?;
        return Ok(Ok(post_id.filter(|_| topic_id > 0)));
    }
    match crate::onebox::recognize(path) {
        // Another route of the topics or posts controllers is a valid
        // reason without a post; telling those apart isn't ported.
        Some(crate::onebox::Route::Topic { .. }) => {
            Err(Unsupported("badge reason links to other topic routes").into())
        }
        Some(_) => Ok(Err(invalid())),
        None if segments.first().is_some_and(|s| *s == "p" || *s == "posts") => {
            Err(Unsupported("badge reason links to post routes").into())
        }
        None => Err(Unsupported("badge reason links no route here recognizes").into()),
    }
}

/// POST /user_badges(.json)
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let (p, settings) = match front(&state, &guardian, &headers, &uri, &body, "POST").await? {
        Ok(v) => v,
        Err(response) => return Ok(response),
    };
    let Some(username) = present(&p, "username") else {
        return Ok(super::accounts::param_missing("username"));
    };
    let mut tx = state.pool.begin().await?;
    let Some(user) = crate::user_updater::find_target(&mut tx, &guardian, &username).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !can_assign(&guardian) {
        return Ok(failed(StatusCode::FORBIDDEN));
    }
    let badge = match fetch_badge(&state, &mut tx, &p).await? {
        Ok(b) => b,
        Err(response) => return Ok(response),
    };
    let mut post_id = None;
    if p.contains_key("post_id") {
        if present(&p, "reason").is_some() {
            return Ok(super::search::invalid_parameters(&state, "post_id"));
        }
        // Integer(params[:post_id].to_s, 10, exception: false)
        let raw = p
            .get("post_id")
            .and_then(params::scalar)
            .unwrap_or_default();
        let id = raw.trim().replace('_', "").parse::<i64>().ok();
        let Some(id) = id.filter(|id| *id >= 1) else {
            return Ok(super::search::invalid_parameters(&state, "post_id"));
        };
        let id = i32::try_from(id).unwrap_or(0);
        let Some(post) = crate::topic_guardian::PostCtx::load(&mut tx, id).await? else {
            return Ok(super::topics::not_found_response(&state));
        };
        if post.trashed() {
            return Ok(super::topics::not_found_response(&state));
        }
        // ensure_can_see!(post)
        let topic_id: i32 = sqlx::query_scalar("SELECT topic_id FROM posts WHERE id = $1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        let topic_can_see =
            match crate::topic_guardian::TopicCtx::load(&mut tx, &settings, &guardian, topic_id)
                .await?
            {
                Some(topic) => {
                    if topic.private_message() && !guardian.is_admin() {
                        return Err(Unsupported("granting badges for posts in messages").into());
                    }
                    let secure = guardian.secure_category_ids(&mut tx, &settings).await?;
                    guardian.can_see_topic(&settings, &topic, true, &secure)?
                }
                None => false,
            };
        if !guardian.can_see_post(&settings, &post, topic_can_see)? {
            return Ok(super::search::invalid_access(&state));
        }
        post_id = Some(id);
    } else if let Some(reason) = present(&p, "reason") {
        match reason_post(&state, &mut tx, &settings, &reason).await? {
            Ok(id) => post_id = id,
            Err(response) => return Ok(response),
        }
    }
    // The user_badges_badge_grant_opts modifier: no bundled plugin has one.
    let suppress = p
        .get("suppress_notification")
        .and_then(params::scalar)
        .is_some_and(|v| v == "true");
    // can_assign_badge_to_user?'s master_api_call (no current user, an
    // API request) never holds: an API key without a user is invalid
    // credentials before the action runs, so a granter is always here.
    let Some(granted_by) = guardian.user_id() else {
        return Ok(super::search::invalid_access(&state));
    };
    // The badge as fetched, before the grant's count increment.
    let badge_row = BadgeRow::load(&mut tx, badge.id).await?;
    let user_badge_id = badge_granter::grant(
        &mut tx,
        &state.bus,
        &settings,
        &state.i18n,
        &badge,
        user.id,
        &GrantOpts {
            granted_by,
            post_id,
            suppress_notification: suppress,
            created_at: None,
        },
    )
    .await?
    .ok_or(Unsupported("a badge grant that grants nothing"))?;

    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let mut view = BadgesView {
        conn: &mut tx,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        base_path: state.config.globals.relative_url_root(),
    };
    let row = view.user_badge_row(user_badge_id).await?;
    let mut side = Side::default();
    let json = view
        .detailed_user_badge(&mut side, &row, Some(badge_row), None)
        .await?;
    tx.commit().await?;
    Ok(Json(detailed_document(side, "user_badge", json)).into_response())
}

/// DELETE /user_badges/:id(.json)
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let (_, settings) = match front(&state, &guardian, &headers, &uri, &body, "DELETE").await? {
        Ok(v) => v,
        Err(response) => return Ok(response),
    };
    let id = id.strip_suffix(".json").unwrap_or(&id);
    let id = i32::try_from(crate::ruby::to_i(id)).unwrap_or(0);
    let mut tx = state.pool.begin().await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_badges WHERE id = $1)")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if !exists {
        return Ok(super::topics::not_found_response(&state));
    }
    if !can_assign(&guardian) {
        return Ok(failed(StatusCode::FORBIDDEN));
    }
    badge_granter::revoke(
        &mut tx,
        &state.bus,
        &settings,
        &state.i18n,
        id,
        guardian.user_id(),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"success": "OK"})).into_response())
}

/// PUT /user_badges/:user_badge_id/toggle_favorite(.json)
pub async fn toggle_favorite(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let (_, settings) = match front(&state, &guardian, &headers, &uri, &body, "PUT").await? {
        Ok(v) => v,
        Err(response) => return Ok(response),
    };
    let id = i32::try_from(crate::ruby::to_i(&id)).unwrap_or(0);
    let mut tx = state.pool.begin().await?;
    let row: Option<(i32, i32)> =
        sqlx::query_as("SELECT user_id, badge_id FROM user_badges WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((user_id, badge_id)) = row else {
        return Ok(super::topics::not_found_response(&state));
    };
    // can_favorite_badge?
    if !guardian.is_me(user_id) || (1..=4).contains(&badge_id) {
        return Ok(failed(StatusCode::FORBIDDEN));
    }
    // user.user_badges: the enabled badges'.
    let (is_favorite, favorites): (bool, i64) = sqlx::query_as(
        "SELECT COALESCE(BOOL_OR(badge_id = $2), FALSE), COUNT(DISTINCT badge_id) \
         FROM user_badges WHERE user_id = $1 AND is_favorite \
         AND badge_id IN (SELECT id FROM badges WHERE enabled)",
    )
    .bind(user_id)
    .bind(badge_id)
    .fetch_one(&mut *tx)
    .await?;
    if !is_favorite && favorites >= settings.get("max_favorite_badges")?.to_i() {
        return Ok(failed(StatusCode::BAD_REQUEST));
    }
    sqlx::query("UPDATE user_badges SET is_favorite = $3 WHERE user_id = $1 AND badge_id = $2")
        .bind(user_id)
        .bind(badge_id)
        .bind(!is_favorite)
        .execute(&mut *tx)
        .await?;
    badge_granter::update_featured_ranks(&mut tx, Some(&[user_id])).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// POST /u/:username: only /u/read-faq is a POST route.
pub async fn post_user(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(username): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if username != "read-faq" && username != "read-faq.json" {
        return Ok(super::topics::not_found_response(&state));
    }
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    // UsersController#read_faq: the user's stat saved (its after_save
    // queues the user's badges).
    if let Some(user_id) = guardian.user_id() {
        let mut tx = state.pool.begin().await?;
        let settings =
            SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
        sqlx::query(
            "UPDATE user_stats SET read_faq = clock_timestamp() - interval '1 second' \
             WHERE user_id = $1",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        badge_granter::queue_user_change(&mut tx, &settings, user_id).await?;
        tx.commit().await?;
    }
    Ok(Json(json!({"success": "OK"})).into_response())
}

/// PUT /u/:username/*rest: only preferences/badge_title has a PUT.
pub async fn put_user_tail(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path((username, rest)): Path<(String, String)>,
    headers: HeaderMap,
    super::search::Peer(peer): super::search::Peer,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(endpoint) = super::users_email::Endpoint::from_tail("PUT", &rest) {
        return super::users_email::handle(
            state.0, guardian.0, username, endpoint, headers, peer, uri, body,
        )
        .await;
    }
    match rest.as_str() {
        "preferences/badge_title" | "preferences/badge_title.json" => {
            badge_title(state, guardian, username, headers, uri, body).await
        }
        "preferences/username" | "preferences/username.json" => {
            super::user_update::username(state.0, guardian.0, username, headers, uri, body).await
        }
        "notification_level" | "notification_level.json" => {
            super::user_update::notification_level(
                state.0, guardian.0, username, headers, uri, body,
            )
            .await
        }
        rest => match super::user_preferences::refused_tail("PUT", rest) {
            Some(what) => Err(crate::Unsupported(what).into()),
            None => Ok(super::topics::not_found_response(&state.0)),
        },
    }
}

/// PUT /u/:username/preferences/badge_title(.json)
async fn badge_title(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    username: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let Some(user_badge_id) = present(&p, "user_badge_id") else {
        return Ok(super::accounts::param_missing("user_badge_id"));
    };
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let Some(user) = crate::user_updater::find_target(&mut tx, &guardian, &username).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !crate::user_updater::can_edit_user(&guardian, &user) {
        return Ok(super::search::invalid_access(&state));
    }
    let user_badge_id = i32::try_from(crate::ruby::to_i(&user_badge_id)).unwrap_or(0);
    let grant: Option<(i32, i32)> =
        sqlx::query_as("SELECT user_id, badge_id FROM user_badges WHERE id = $1")
            .bind(user_badge_id)
            .fetch_optional(&mut *tx)
            .await?;
    let badge = match grant {
        Some((owner, badge_id)) if owner == user.id => Badge::find(&mut tx, badge_id)
            .await?
            .filter(|b| b.allow_title),
        _ => None,
    };
    let previous = user.title.clone();
    let staff_other = guardian.is_staff() && !guardian.is_me(user.id);
    let acting = if staff_other {
        guardian.user_id()
    } else {
        None
    };
    match badge {
        Some(badge) => {
            let title = badge.display_name(&state.i18n);
            badge_granter::set_title(&mut tx, &settings, &state.i18n, user.id, Some(&title))
                .await?;
            let details = format!("title matching badge id {}", badge.id);
            badge_granter::user_history(
                &mut tx,
                &History {
                    action: badge_granter::CHANGE_TITLE,
                    acting_user_id: acting,
                    target_user_id: user.id,
                    details: Some(&details),
                    previous_value: previous.as_deref(),
                    new_value: Some(&title),
                },
            )
            .await?;
        }
        None => {
            badge_granter::set_title(&mut tx, &settings, &state.i18n, user.id, Some("")).await?;
            badge_granter::user_history(
                &mut tx,
                &History {
                    action: badge_granter::REVOKE_TITLE,
                    acting_user_id: acting,
                    target_user_id: user.id,
                    details: staff_other.then_some(badge_granter::TITLE_REVOKE_REASON),
                    previous_value: previous.as_deref(),
                    new_value: None,
                },
            )
            .await?;
        }
    }
    tx.commit().await?;
    // render body: nil
    Ok(StatusCode::OK.into_response())
}
