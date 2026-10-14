//! Members joining, leaving and asking to join groups
//! (GroupsController#join, #leave, #request_membership,
//! #handle_membership_request) and their notification level in one
//! (#set_notifications).
//!
//! Refused: membership request messages in locales other than English
//! (the title is looked up in the requester's locale), and accepting a
//! request whose message is gone (a 500 in Rails).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::group_manager::{self, ADD_USER_TO_GROUP, REMOVE_USER_FROM_GROUP};
use crate::guardian::Guardian;
use crate::params;
use crate::posting::create::{self, NewPost, Outcome};
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `GroupsController::MAX_NOTIFIED_OWNERS`
const MAX_NOTIFIED_OWNERS: i64 = 20;

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn strip_json(s: &str) -> &str {
    s.strip_suffix(".json").unwrap_or(s)
}

fn param_missing(key: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "errors": [format!("param is missing or the value is empty or invalid: {key}")]
        })),
    )
        .into_response()
}

struct Req {
    p: Map<String, Value>,
    settings: SiteSettings,
}

async fn begin(
    state: &AppState,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
    method: &str,
) -> Result<Result<Req, Response>, AppError> {
    let p = params::parse(uri.query(), headers, body);
    if !csrf_ok(state, headers, &form_pairs(&p), uri.path(), method) {
        return Ok(Err(bad_csrf()));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    Ok(Ok(Req { p, settings }))
}

/// `RateLimiter.new(current_user, "public_group_membership", 3, 1.minute)`
/// for non-staff.
async fn rate_limit(state: &AppState, guardian: &Guardian) -> Result<Option<Response>, AppError> {
    if guardian.is_staff() {
        return Ok(None);
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(None);
    };
    let limiter = crate::rate_limiter::RateLimiter::new(
        Some(crate::rate_limiter::LimitUser {
            id: user_id,
            staff: false,
        }),
        "public_group_membership",
        3,
        60,
    );
    match limiter
        .performed(&state.pool, state.config.rate_limits)
        .await?
    {
        Ok(()) => Ok(None),
        Err(refused) => Ok(Some(refused.into_response(&state.i18n))),
    }
}

fn manager<'a>(
    state: &'a AppState,
    promotion: &'a crate::promotion::Ctx<'a>,
) -> group_manager::Cx<'a> {
    group_manager::Cx { state, promotion }
}

/// PUT /groups/:id/join(.json) (GroupsController#join)
pub async fn join(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match begin(&state, &headers, &uri, &body, "PUT").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    if let Some(refused) = rate_limit(&state, &guardian).await? {
        return Ok(refused);
    }
    let mut tx = state.pool.begin().await?;
    let id = strip_json(&id).parse::<i32>().ok();
    let Some((group, _)) = super::groups::find_visible(&mut tx, &guardian, None, id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !group.public_admission {
        return Ok(super::search::invalid_access(&state));
    }
    let member: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM group_users WHERE group_id = $1 AND user_id = $2)",
    )
    .bind(group.id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !member {
        let host = Host::from_state(&state);
        let promotion = crate::promotion::Ctx {
            settings: &req.settings,
            i18n: &state.i18n,
            host: &host,
        };
        let cx = manager(&state, &promotion);
        let g = group_manager::find(&mut tx, group.id)
            .await?
            .ok_or(Unsupported("a group gone"))?;
        let added = group_manager::add_many(&mut tx, &cx, &g, &[user_id], false).await?;
        group_manager::log_many(&mut tx, group.id, user_id, &added, ADD_USER_TO_GROUP).await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// DELETE /groups/:id/leave(.json) (GroupsController#leave)
pub async fn leave(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match begin(&state, &headers, &uri, &body, "DELETE").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    if let Some(refused) = rate_limit(&state, &guardian).await? {
        return Ok(refused);
    }
    let mut tx = state.pool.begin().await?;
    let Ok(id) = strip_json(&id).parse::<i32>() else {
        return Ok(super::topics::not_found_response(&state));
    };
    let Some(g) = group_manager::find(&mut tx, id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let public_exit: bool = sqlx::query_scalar("SELECT public_exit FROM groups WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    if !public_exit {
        return Ok(super::search::invalid_access(&state));
    }
    let host = Host::from_state(&state);
    let promotion = crate::promotion::Ctx {
        settings: &req.settings,
        i18n: &state.i18n,
        host: &host,
    };
    let cx = manager(&state, &promotion);
    let removed = group_manager::remove_many(&mut tx, &cx, &g, &[user_id]).await?;
    group_manager::log_many(&mut tx, id, user_id, &removed, REMOVE_USER_FROM_GROUP).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// POST /groups/:name/request_membership(.json)
/// (GroupsController#request_membership)
pub async fn request_membership(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(name): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match begin(&state, &headers, &uri, &body, "POST").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    let Some(user) = guardian.user() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let user_id = user.id;
    let Some(reason) = params::string(&req.p, "reason").filter(|r| !r.is_empty()) else {
        return Ok(param_missing("reason"));
    };
    let mut conn = state.pool.acquire().await?;
    let Some((group, membership)) =
        super::groups::find_visible(&mut conn, &guardian, Some(&name), None).await?
    else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !group.allow_membership_requests || membership.is_some() {
        return Ok(super::search::invalid_access(&state));
    }
    let inserted = sqlx::query(
        "INSERT INTO group_requests (group_id, user_id, reason, created_at, updated_at) \
         VALUES ($1, $2, $3, now(), now()) ON CONFLICT (group_id, user_id) DO NOTHING",
    )
    .bind(group.id)
    .bind(user_id)
    .bind(&reason)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Ok((
            StatusCode::CONFLICT,
            Json(json!({
                "failed": "FAILED",
                "error": state.i18n.t("groups.errors.already_requested_membership").unwrap_or_default(),
            })),
        )
            .into_response());
    }
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    let owners: Vec<String> = sqlx::query_scalar(
        "SELECT u.username FROM users u JOIN group_users gu ON gu.user_id = u.id \
         WHERE gu.group_id = $1 AND gu.owner ORDER BY u.last_seen_at DESC LIMIT $2",
    )
    .bind(group.id)
    .bind(MAX_NOTIFIED_OWNERS)
    .fetch_all(&mut *conn)
    .await?;
    drop(conn);
    let mut recipients = vec![username.to_lowercase()];
    recipients.extend(owners.iter().map(|o| o.to_lowercase()));
    let title = state
        .i18n
        .t_with(
            "groups.request_membership_pm.title",
            &[("group_name", &group.name)],
        )
        .unwrap_or_default();
    let host = Host::from_state(&state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &req.settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let outcome = create::create(
        &state.pool,
        &ctx,
        &guardian,
        NewPost {
            raw: reason,
            title: Some(title),
            pm_recipients: Some(recipients),
            skip_validations: true,
            topic_custom_fields: vec![("requested_group_id".into(), group.id.to_string())],
            ..Default::default()
        },
    )
    .await?;
    let Outcome::Created { post_id } = outcome else {
        return Err(Unsupported("a membership request PostCreator refused").into());
    };
    let (topic_id, slug): (i32, String) = sqlx::query_as(
        "SELECT t.id, t.slug FROM topics t JOIN posts p ON p.topic_id = t.id WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_one(&state.pool)
    .await?;
    let base = state.config.globals.relative_url_root();
    Ok(Json(json!({
        "success": "OK",
        "relative_url": format!("{base}/t/{slug}/{topic_id}"),
    }))
    .into_response())
}

/// PUT /groups/:id/handle_membership_request(.json)
/// (GroupsController#handle_membership_request)
pub async fn handle_membership_request(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match begin(&state, &headers, &uri, &body, "PUT").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    let Some(acting) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut tx = state.pool.begin().await?;
    let id = strip_json(&id).parse::<i32>().ok();
    let Some(group) = crate::groups::find_basic(&mut tx, None, id.or(Some(0))).await? else {
        return Ok(super::search::invalid_parameters(&state, "id"));
    };
    let membership = crate::groups::membership(&mut tx, &guardian, group.id).await?;
    if !crate::groups::can_edit_group(&guardian, &req.settings, &group, membership)? {
        return Ok(super::search::invalid_access(&state));
    }
    let user: Option<(i32, String, Option<String>)> = match params::integer(&req.p, "user_id") {
        Some(uid) => {
            sqlx::query_as("SELECT id, username, locale FROM users WHERE id = $1")
                .bind(uid as i32)
                .fetch_optional(&mut *tx)
                .await?
        }
        None => None,
    };
    let Some((user_id, _username, locale)) = user else {
        return Ok(super::search::invalid_parameters(&state, "user_id"));
    };
    let user_locale = locale
        .filter(|l| {
            !l.is_empty()
                && req
                    .settings
                    .get("allow_user_locale")
                    .is_ok_and(|v| v.truthy())
        })
        .unwrap_or_else(|| {
            req.settings
                .get("default_locale")
                .map(|v| v.to_s())
                .unwrap_or_default()
        });
    if user_locale != "en" {
        return Err(Unsupported("membership request messages in other locales").into());
    }
    let title = state
        .i18n
        .t_with(
            "groups.request_membership_pm.title",
            &[("group_name", &group.name)],
        )
        .unwrap_or_default();
    let request_topic: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM topics WHERE title = $1 AND archetype = 'private_message' AND user_id = $2 \
         ORDER BY id LIMIT 1",
    )
    .bind(&title)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let accept = req.p.get("accept").is_some_and(|v| match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        _ => true,
    });
    if accept {
        if request_topic.is_none() {
            return Err(Unsupported("accepting a request without its message (a 500)").into());
        }
        let host = Host::from_state(&state);
        let promotion = crate::promotion::Ctx {
            settings: &req.settings,
            i18n: &state.i18n,
            host: &host,
        };
        let cx = manager(&state, &promotion);
        let g = group_manager::find(&mut tx, group.id)
            .await?
            .ok_or(Unsupported("a group gone"))?;
        group_manager::add_many(&mut tx, &cx, &g, &[user_id], false).await?;
        // log_add_user_to_group, whether or not they were added.
        group_manager::log_many(&mut tx, group.id, acting, &[user_id], ADD_USER_TO_GROUP).await?;
    }
    sqlx::query("DELETE FROM group_requests WHERE group_id = $1 AND user_id = $2")
        .bind(group.id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if accept && let Some(topic_id) = request_topic {
        let raw = state
            .i18n
            .t_with(
                "groups.request_accepted_pm.body",
                &[("group_name", &group.name)],
            )
            .unwrap_or_default();
        let host = Host::from_state(&state);
        let ctx = crate::posting::Ctx {
            host: &host,
            settings: &req.settings,
            config: &state.config,
            i18n: &state.i18n,
            bus: &state.bus,
        };
        let outcome = create::create(
            &state.pool,
            &ctx,
            &guardian,
            NewPost {
                raw,
                topic_id: Some(topic_id),
                reply_to_post_number: Some(1),
                pm_recipients: None,
                skip_validations: true,
                ..Default::default()
            },
        )
        .await?;
        if !matches!(outcome, Outcome::Created { .. }) {
            return Err(Unsupported("an acceptance reply PostCreator refused").into());
        }
    }
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// POST /groups/:name/notifications(.json) (GroupsController#set_notifications)
pub async fn set_notifications(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(name): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match begin(&state, &headers, &uri, &body, "POST").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    let Some(current) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut tx = state.pool.begin().await?;
    let Some((group, _)) =
        super::groups::find_visible(&mut tx, &guardian, Some(&name), None).await?
    else {
        return Ok(super::topics::not_found_response(&state));
    };
    let Some(level) = params::string(&req.p, "notification_level").filter(|v| !v.is_empty()) else {
        return Ok(param_missing("notification_level"));
    };
    let mut user_id = current;
    if guardian.is_staff()
        && let Some(given) = params::string(&req.p, "user_id").filter(|v| !v.is_empty())
    {
        user_id = crate::ruby::to_i(&given) as i32;
    }
    let level = crate::ruby::to_i(&level) as i32;
    let updated = sqlx::query(
        "UPDATE group_users SET notification_level = $3, \
                updated_at = CASE WHEN notification_level = $3 THEN updated_at ELSE now() END \
         WHERE group_id = $1 AND user_id = $2",
    )
    .bind(group.id)
    .bind(user_id)
    .bind(level)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if updated == 0 {
        return Ok(super::search::invalid_parameters(&state, "user_id"));
    }
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}
