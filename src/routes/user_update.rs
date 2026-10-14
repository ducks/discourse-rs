//! Port of UsersController#update: PUT /u/:username (also with .json).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::session::current::Incoming;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::user_updater;
use crate::users::{User, Users};
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// PUT /u/{username}
/// DELETE /u/:username(.json): UsersController#destroy, the user deleting
/// their own account (posts and all), or staff any they may delete.
pub async fn destroy(
    State(state): State<AppState>,
    axum::Extension(incoming): axum::Extension<Incoming>,
    Path(username): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let guardian = incoming.guardian.clone();
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "DELETE") {
        return Ok(bad_csrf());
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let mut tx = state.pool.begin().await?;
    let Some(target) = user_updater::find_target(&mut tx, &guardian, &username).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let Some(deletable) = crate::guardian::DeletableUser::load(&mut tx, target.id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !guardian
        .can_delete_user(&mut tx, &settings, &deletable)
        .await?
    {
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
    let opts = crate::user_destroyer::Options {
        delete_posts: true,
        context: p.get("context").and_then(params::scalar),
        ..Default::default()
    };
    match crate::user_destroyer::destroy(&mut tx, &ctx, &guardian, target.id, &opts).await? {
        crate::user_destroyer::Outcome::Destroyed => {}
        // delete_posts is given, and the guardian passed above.
        crate::user_destroyer::Outcome::PostsExist | crate::user_destroyer::Outcome::Forbidden => {
            return Ok(super::search::invalid_access(&state));
        }
    }
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}

pub async fn update(
    State(state): State<AppState>,
    axum::Extension(incoming): axum::Extension<Incoming>,
    Path(username): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let guardian = incoming.guardian.clone();
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    // requires_login
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let mut tx = state.pool.begin().await?;
    let Some(target) = user_updater::find_target(&mut tx, &guardian, &username).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    // guardian.ensure_can_edit!(user)
    if !user_updater::can_edit_user(&guardian, &target) {
        return Ok(super::search::invalid_access(&state));
    }
    // user_params.except(:username, :email, :password)
    let mut attrs = p;
    for key in ["username", "email", "password"] {
        attrs.remove(key);
    }
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
    if !user_updater::update(&mut tx, &ctx, &guardian, &target, &attrs).await? {
        // json_result's render_json_error(user): no errors on the user,
        // its profile or options, so the generic one. What the updater
        // wrote before failing stays.
        tx.commit().await?;
        let message = state
            .i18n
            .t("js.generic_error")
            .unwrap_or("Sorry, an error has occurred.");
        return Ok((
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            axum::Json(serde_json::json!({ "errors": [message] })),
        )
            .into_response());
    }

    // json_result(user, serializer: UserSerializer)
    let Some(user) = User::find_by_id(&mut tx, target.id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let auth_token: Option<String> = incoming
        .session
        .as_ref()
        .map(|s| s.token.auth_token.clone());
    let mut users = Users {
        conn: &mut tx,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        base_path: state.config.globals.relative_url_root(),
        auth_token: auth_token.as_deref(),
    };
    let doc = users.show(&user).await?;
    tx.commit().await?;
    Ok((
        StatusCode::OK,
        Json(json!({ "success": "OK", "user": doc["user"] })),
    )
        .into_response())
}

fn errors(status: StatusCode, message: String) -> Response {
    (status, Json(json!({ "errors": [message] }))).into_response()
}

/// PUT /u/:username/notification_level(.json): UsersController
/// #notification_level, the user notifications dropdown's normal and mute;
/// the ignore level (IgnoreDurationModal's) is refused.
pub async fn notification_level(
    state: AppState,
    guardian: crate::guardian::Guardian,
    username: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    let Some(me) = guardian.user().cloned() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let t = |key: &str| {
        state
            .i18n
            .t(&format!("notification_level.{key}"))
            .unwrap_or_default()
            .to_string()
    };
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // fetch_user_from_params: active users only, but for staff.
    let lower = username.to_lowercase();
    let lower = lower.strip_suffix(".json").unwrap_or(&lower);
    let target: Option<(i32, bool, bool)> = sqlx::query_as(
        "SELECT id, admin, moderator FROM users WHERE username_lower = $1 AND (active OR $2 OR id = $3)",
    )
    .bind(lower)
    .bind(guardian.is_staff())
    .bind(me.id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((target_id, target_admin, target_moderator)) = target else {
        return Ok(super::topics::not_found_response(&state));
    };
    // acting_user_id: staff may act for someone else.
    let mut acting = me.id;
    if let Some(id) = params::string(&p, "acting_user_id").filter(|v| !v.is_empty()) {
        let id = crate::ruby::to_i(&id) as i32;
        if id != me.id {
            if !guardian.is_staff() {
                return Ok(errors(StatusCode::UNPROCESSABLE_ENTITY, t("error")));
            }
            return Err(crate::Unsupported("notification_level for another acting user").into());
        }
        acting = id;
    }
    let target_staff = target_admin || target_moderator;
    let level = params::string(&p, "notification_level").unwrap_or_default();
    match level.as_str() {
        "ignore" => {
            return Err(crate::Unsupported("the ignore notification level").into());
        }
        "mute" => {
            // guardian.ensure_can_mute_user!
            if !(guardian.can_mute_users() && target_id != me.id && !target_staff) {
                return Ok(errors(StatusCode::UNPROCESSABLE_ENTITY, t("mute_error")));
            }
            sqlx::query("DELETE FROM ignored_users WHERE user_id = $1 AND ignored_user_id = $2")
                .bind(acting)
                .bind(target_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO muted_users (user_id, muted_user_id, created_at, updated_at) \
                 SELECT $1, $2, now(), now() WHERE NOT EXISTS \
                 (SELECT 1 FROM muted_users WHERE user_id = $1 AND muted_user_id = $2)",
            )
            .bind(acting)
            .bind(target_id)
            .execute(&mut *tx)
            .await?;
        }
        "normal" => {
            sqlx::query("DELETE FROM muted_users WHERE user_id = $1 AND muted_user_id = $2")
                .bind(acting)
                .bind(target_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM ignored_users WHERE user_id = $1 AND ignored_user_id = $2")
                .bind(acting)
                .bind(target_id)
                .execute(&mut *tx)
                .await?;
        }
        other => {
            let message = state
                .i18n
                .t_with("notification_level.invalid_value", &[("value", other)])
                .unwrap_or_default();
            return Ok(errors(StatusCode::UNPROCESSABLE_ENTITY, message));
        }
    }
    let _ = &settings;
    tx.commit().await?;
    Ok((StatusCode::OK, Json(json!({ "success": "OK" }))).into_response())
}

/// PUT /u/:username/preferences/username(.json): UsersController#username.
pub async fn username(
    state: AppState,
    guardian: crate::guardian::Guardian,
    username: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    use crate::username_changer::{self, Changed, Update};
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    // params.require(:new_username)
    let new_username = match p.get("new_username") {
        None => None,
        Some(v) => match params::scalar(v) {
            Some(s) => Some(s),
            None => return Err(crate::Unsupported("a new_username that is not a string").into()),
        },
    };
    let Some(new_username) = new_username.filter(|v| !crate::ruby::is_blank(v)) else {
        return Ok(super::accounts::param_missing("new_username"));
    };
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // UsernameValidator::MAX_CHARS * 3
    if new_username.chars().count() > 180 {
        let max = settings.get("max_username_length")?.to_i();
        let form = if max == 1 { "one" } else { "other" };
        let message = state
            .i18n
            .t_with(
                &format!("user.username.long.{form}"),
                &[("count", &max.to_string())],
            )
            .unwrap_or_default();
        return Ok(errors(StatusCode::UNPROCESSABLE_ENTITY, message));
    }
    if crate::signup::clashing_with_existing_route(&new_username)?
        || (crate::signup::reserved_by_settings(&settings, &new_username)? && !guardian.is_admin())
    {
        let message = state.i18n.t("login.reserved_username").unwrap_or("");
        return Ok(errors(
            StatusCode::UNPROCESSABLE_ENTITY,
            message.to_string(),
        ));
    }
    let Some(target) = user_updater::find_target(&mut tx, &guardian, &username).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !username_changer::can_edit_username(&mut tx, &settings, &guardian, target.id).await? {
        // rescue Discourse::InvalidAccess
        if guardian.is_staff() {
            let message = state
                .i18n
                .t("errors.messages.auth_overrides_username")
                .unwrap_or("");
            return Ok(errors(
                StatusCode::UNPROCESSABLE_ENTITY,
                message.to_string(),
            ));
        }
        return Ok((StatusCode::FORBIDDEN, Json(json!({ "failed": "FAILED" }))).into_response());
    }
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let changed = username_changer::change(
        &mut tx,
        &ctx,
        target.id,
        &new_username,
        guardian.user_id(),
        Update::Enqueue,
    )
    .await?;
    let response = match changed {
        Changed::Changed => {
            Json(json!({ "id": target.id, "username": new_username })).into_response()
        }
        Changed::Same => errors(StatusCode::UNPROCESSABLE_ENTITY, String::new()),
        Changed::Invalid(messages) => errors(StatusCode::UNPROCESSABLE_ENTITY, messages.join(",")),
    };
    tx.commit().await?;
    Ok(response)
}
