//! TopicsController#invite and #remove_allowed_user: POST
//! /t/:topic_id/invite puts a user on a message, PUT
//! /t/:topic_id/remove-allowed-user takes one off.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::pm_invite::{self, Invited};
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_guardian::TopicCtx;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn failed() -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"failed": "FAILED"})),
    )
        .into_response()
}

fn errors(message: String) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"errors": [message]})),
    )
        .into_response()
}

/// `Topic.find_by(id: params[:topic_id])`: a live topic.
async fn find_topic(
    conn: &mut sqlx::PgConnection,
    topic_id: &str,
) -> Result<Option<(i32, String)>, sqlx::Error> {
    let id = i32::try_from(crate::ruby::to_i(topic_id)).unwrap_or(0);
    sqlx::query_as("SELECT id, archetype FROM topics WHERE id = $1 AND deleted_at IS NULL")
        .bind(id)
        .fetch_optional(conn)
        .await
}

/// BasicUserSerializer with root "user".
async fn basic_user(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<Value, AppError> {
    let (username, name, avatar): (String, Option<String>, Option<i32>) =
        sqlx::query_as("SELECT username, name, uploaded_avatar_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let logo_small = crate::admin_users::logo_small_url(&mut *conn, settings).await?;
    let mut out = Map::new();
    out.insert("id".into(), json!(user_id));
    out.insert("username".into(), json!(username));
    if settings.get("enable_names")?.truthy() {
        out.insert("name".into(), json!(name));
    }
    out.insert(
        "avatar_template".into(),
        json!(crate::avatar::avatar_template(
            &urls,
            user_id,
            &username,
            avatar,
            logo_small.as_deref()
        )?),
    );
    Ok(json!({ "user": Value::Object(out) }))
}

/// POST /t/:topic_id/invite
pub async fn invite(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(topic_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    // requires_login
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let Some((topic_id, archetype)) = find_topic(&mut tx, &topic_id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if archetype != "private_message" {
        return Ok(errors(
            state
                .i18n
                .t("topic_invite.not_pm")
                .unwrap_or_default()
                .to_string(),
        ));
    }
    let topic = TopicCtx::load(&mut tx, &settings, &guardian, topic_id)
        .await?
        .ok_or(Unsupported("loading a message to invite to"))?;
    // pm_has_slots?
    let limit = settings.get("max_allowed_message_recipients")?.to_i();
    if !guardian.is_staff() && topic.recipients >= limit {
        let limit = limit.to_string();
        return Ok(errors(
            state
                .i18n
                .t_with(
                    "pm_reached_recipients_limit",
                    &[("recipients_limit", &limit)],
                )
                .unwrap_or_default(),
        ));
    }
    // ensure_can_invite_to!
    let secure = guardian.secure_category_ids(&mut tx, &settings).await?;
    let can_see = guardian.can_see_topic(&settings, &topic, true, &secure)?;
    if !guardian.can_invite_to(&settings, &topic, can_see)? {
        return Ok(super::search::invalid_access(&state));
    }
    // params[:user] ? fetch_username : fetch_email (each params.require)
    let key = if p.contains_key("user") {
        "user"
    } else {
        "email"
    };
    let Some(target) = p
        .get(key)
        .and_then(params::scalar)
        .filter(|v| !v.is_empty())
    else {
        return Ok(super::accounts::param_missing(key));
    };
    if crate::accounts::valid_email(&target) {
        if !guardian.can_invite_via_email(&settings, &topic, can_see)? {
            return Ok(failed());
        }
        let existing: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM user_emails WHERE lower(email) = lower($1))",
        )
        .bind(&target)
        .fetch_one(&mut *tx)
        .await?;
        if existing {
            return Ok(Json(json!({"success": "OK"})).into_response());
        }
    }
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let outcome =
        pm_invite::invite(&state.pool, &mut tx, &ctx, &guardian, topic_id, &target).await?;
    Ok(match outcome {
        Invited::User(user_id) => {
            let body = basic_user(&mut tx, &state, &settings, user_id).await?;
            tx.commit().await?;
            Json(body).into_response()
        }
        Invited::Failed => failed(),
        Invited::ByEmail => {
            if !guardian.can_invite_via_email(&settings, &topic, can_see)? {
                return Ok(failed());
            }
            let group_ids =
                crate::invites::lookup_groups(&mut tx, p.get("group_ids"), p.get("group_names"))
                    .await?
                    .map(|groups| groups.iter().map(|g| g.id).collect())
                    .unwrap_or_default();
            let inviter = crate::invites::Inviter::load(
                &mut tx,
                guardian
                    .user_id()
                    .ok_or(Unsupported("inviting anonymously"))?,
            )
            .await?
            .ok_or(Unsupported("an inviter who is gone"))?;
            let opts = crate::invites::Generate {
                email: Some(target.clone()),
                topic_id: Some(topic_id),
                group_ids,
                custom_message: p.get("custom_message").and_then(params::scalar),
                invite_to_topic: json!(true),
                ..crate::invites::Generate::default()
            };
            let limits = crate::invites::Limits {
                pool: &state.pool,
                enabled: state.config.rate_limits,
            };
            match crate::invites::generate(
                &mut tx,
                &settings,
                &state.i18n,
                &limits,
                &inviter,
                &opts,
            )
            .await?
            {
                crate::invites::Generated::Invite(_) => {
                    tx.commit().await?;
                    Json(json!({"success": "OK"})).into_response()
                }
                crate::invites::Generated::RateLimited(refused) => {
                    refused.into_response(&state.i18n)
                }
                crate::invites::Generated::UserExists(_)
                | crate::invites::Generated::Invalid(_) => {
                    return Err(Unsupported(
                        "an invite to a message that fails to save (a 500 in Rails)",
                    )
                    .into());
                }
            }
        }
        Invited::Refused(message) => errors(message),
        Invited::RateLimited(refused) => refused.into_response(&state.i18n),
    })
}

/// PUT /t/:topic_id/remove-allowed-user
pub async fn remove_allowed_user(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(topic_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    let Some(actor) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let Some(username) = p
        .get("username")
        .and_then(params::scalar)
        .filter(|v| !v.is_empty())
    else {
        return Ok(super::accounts::param_missing("username"));
    };
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let Some((topic_id, _)) = find_topic(&mut tx, &topic_id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    // User.find_by(username:): the exact username.
    let user: Option<i32> = sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
        .bind(&username)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(user_id) = user else {
        return Ok(super::topics::not_found_response(&state));
    };
    let topic = TopicCtx::load(&mut tx, &settings, &guardian, topic_id)
        .await?
        .ok_or(Unsupported("loading a message to remove a user from"))?;
    let allowed_users: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM topic_allowed_users WHERE topic_id = $1")
            .bind(topic_id)
            .fetch_one(&mut *tx)
            .await?;
    if !guardian.can_remove_allowed_users(&topic, Some(user_id), allowed_users) {
        return Ok(super::search::invalid_access(&state));
    }
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    if !pm_invite::remove_allowed_user(&mut tx, &ctx, actor, topic_id, user_id).await? {
        return Ok(failed());
    }
    tx.commit().await?;
    Ok(Json(json!({"success": "OK"})).into_response())
}

/// POST /t/:topic_id/invite-group
pub async fn invite_group(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(topic_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    // :topic_id is constrained to digits.
    if topic_id.is_empty() || !topic_id.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(super::topics::not_found_response(&state));
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // Group.find_by(name: params[:group])
    let name = p.get("group").and_then(params::scalar).unwrap_or_default();
    let group: Option<crate::posting::group_messages::AllowedGroup> =
        sqlx::query_as("SELECT id, name, user_count, has_messages FROM groups WHERE name = $1")
            .bind(&name)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(group) = group else {
        return Ok(super::topics::not_found_response(&state));
    };
    let Some((topic_id, archetype)) = find_topic(&mut tx, &topic_id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let topic = TopicCtx::load(&mut tx, &settings, &guardian, topic_id)
        .await?
        .ok_or(Unsupported("loading a message to invite a group to"))?;
    // pm_has_slots? (reached_recipients_limit? is false outside messages)
    let limit = settings.get("max_allowed_message_recipients")?.to_i();
    if !guardian.is_staff() && archetype == "private_message" && topic.recipients >= limit {
        let limit = limit.to_string();
        return Ok(errors(
            state
                .i18n
                .t_with(
                    "pm_reached_recipients_limit",
                    &[("recipients_limit", &limit)],
                )
                .unwrap_or_default(),
        ));
    }
    if archetype != "private_message" {
        return Ok(failed());
    }
    // ensure_can_invite_group_to_private_message!
    let secure = guardian.secure_category_ids(&mut tx, &settings).await?;
    let can_see = guardian.can_see_topic(&settings, &topic, true, &secure)?;
    let can_send = crate::posting::group_messages::can_send_to(
        &mut tx,
        &guardian,
        guardian.can_send_private_messages(&settings)?,
        false,
        &group,
    )
    .await?;
    if !(can_see && can_send) {
        return Ok(super::search::invalid_access(&state));
    }
    let should_notify = match p.get("should_notify").and_then(params::scalar) {
        Some(v) if !v.trim().is_empty() => v == "true",
        _ => true,
    };
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    pm_invite::invite_group(&mut tx, &ctx, user_id, group.id, topic_id, should_notify).await?;
    // BasicGroupSerializer, root "group"
    let basic: crate::groups::BasicGroup = sqlx::query_as(&format!(
        "SELECT {} FROM groups g LEFT JOIN uploads u ON u.id = g.flair_upload_id WHERE g.id = $1",
        crate::groups::BASIC_GROUP_COLUMNS
    ))
    .bind(group.id)
    .fetch_one(&mut *tx)
    .await?;
    let owner: Option<bool> =
        sqlx::query_scalar("SELECT owner FROM group_users WHERE group_id = $1 AND user_id = $2")
            .bind(group.id)
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    let json = basic.json(&state.i18n, &guardian, &settings, owner.map(|o| (true, o)))?;
    tx.commit().await?;
    Ok(Json(json!({ "group": json })).into_response())
}

/// PUT /t/:topic_id/remove-allowed-group
pub async fn remove_allowed_group(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(topic_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    let Some(actor) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let Some(name) = p
        .get("name")
        .and_then(params::scalar)
        .filter(|v| !v.is_empty())
    else {
        return Ok(super::accounts::param_missing("name"));
    };
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let Some((topic_id, _)) = find_topic(&mut tx, &topic_id).await? else {
        // Rails goes on with a nil topic and fails with a NoMethodError.
        return Err(Unsupported("removing a group from a missing topic (a 500 in Rails)").into());
    };
    let topic = TopicCtx::load(&mut tx, &settings, &guardian, topic_id)
        .await?
        .ok_or(Unsupported("loading a message to remove a group from"))?;
    if !guardian.can_remove_allowed_users(&topic, None, 0) {
        return Ok(super::search::invalid_access(&state));
    }
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    if !pm_invite::remove_allowed_group(&mut tx, &ctx, actor, topic_id, &name).await? {
        return Ok(failed());
    }
    tx.commit().await?;
    Ok(Json(json!({"success": "OK"})).into_response())
}
