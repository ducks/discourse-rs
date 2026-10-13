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
