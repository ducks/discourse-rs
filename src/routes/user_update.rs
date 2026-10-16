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
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // An admin's API request sets external ids (the managed authenticators'
    // and DiscourseConnect's); everyone else's is ignored.
    let is_api = headers.contains_key(crate::session::api_key::HEADER_API_KEY)
        || headers.contains_key(crate::session::user_api_key::HEADER_USER_API_KEY);
    // `attributes[:discourse_connect]` (given: the id, blank to remove)
    // and `attributes[:user_associated_accounts]`.
    let mut discourse_connect: Option<Option<String>> = None;
    let mut associated: Vec<(String, Option<String>)> = Vec::new();
    if let Some(Value::Object(ids)) = p.get("external_ids")
        && guardian.is_admin()
        && is_api
    {
        for (provider, uid) in ids {
            let uid = crate::params::scalar(uid).filter(|u| !crate::ruby::is_blank(u));
            if provider == "discourse_connect" {
                if !settings.get("enable_discourse_connect")?.truthy() {
                    return Ok(super::search::invalid_parameters(&state, "external_ids"));
                }
                discourse_connect = Some(uid);
                continue;
            }
            if !crate::auth::enabled_managed(&settings, provider)? {
                return Ok(super::search::invalid_parameters(&state, "external_ids"));
            }
            associated.push((provider.clone(), uid));
        }
    }
    let user_fields = p.get("user_fields").cloned();
    // user_params.except(:username, :email, :password)
    let mut attrs = user_updater::user_params(p, &settings)?;
    for key in ["username", "email", "password"] {
        attrs.remove(key);
    }
    if let Some(fields) = user_fields {
        attrs.insert("user_fields".into(), fields);
    }
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    // `params[:user_fields].present?`: the values, into custom_fields.
    let custom_fields = match attrs.remove("user_fields") {
        None | Some(Value::Null) => None,
        Some(Value::Object(given)) if given.is_empty() => None,
        Some(Value::String(s)) if crate::ruby::is_blank(&s) => None,
        Some(Value::Object(given)) => {
            match crate::user_fields::update_values(&mut tx, &given, guardian.is_staff(), target.id)
                .await?
            {
                Ok(values) => Some(values),
                Err(()) => {
                    let message = state.i18n.t("login.missing_user_field").unwrap_or("");
                    return Ok(errors(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        message.to_string(),
                    ));
                }
            }
        }
        Some(_) => {
            return Err(crate::Unsupported("user_fields that isn't a hash (NoMethodError)").into());
        }
    };
    if !user_updater::update(
        &mut tx,
        &ctx,
        &guardian,
        &target,
        &attrs,
        custom_fields.as_deref(),
    )
    .await?
    {
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

    // update_discourse_connect: the record's external id (and a payload
    // naming it), or the record removed.
    if let Some(external_id) = discourse_connect {
        match external_id {
            Some(id) => {
                let payload = format!("external_id={id}");
                let updated = sqlx::query(
                    "UPDATE single_sign_on_records SET external_id = $2, last_payload = $3, updated_at = now() \
                     WHERE user_id = $1 AND (external_id, last_payload) IS DISTINCT FROM ($2, $3)",
                )
                .bind(target.id)
                .bind(&id)
                .bind(&payload)
                .execute(&mut *tx)
                .await?;
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM single_sign_on_records WHERE user_id = $1)",
                )
                .bind(target.id)
                .fetch_one(&mut *tx)
                .await?;
                if updated.rows_affected() == 0 && !exists {
                    sqlx::query(
                        "INSERT INTO single_sign_on_records (user_id, external_id, last_payload, created_at, updated_at) \
                         VALUES ($1, $2, $3, now(), now())",
                    )
                    .bind(target.id)
                    .bind(&id)
                    .bind(&payload)
                    .execute(&mut *tx)
                    .await?;
                }
            }
            // destroy!, of a new record too (nothing to remove).
            None => {
                sqlx::query("DELETE FROM single_sign_on_records WHERE user_id = $1")
                    .bind(target.id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }
    // updated_associated_accounts: each provider's uid, or its account
    // removed.
    for (provider, uid) in associated {
        match uid {
            Some(uid) => {
                sqlx::query(
                    "INSERT INTO user_associated_accounts (provider_name, provider_uid, user_id, info, credentials, extra, created_at, updated_at) \
                     VALUES ($1, $2, $3, '{}', '{}', '{}', now(), now()) \
                     ON CONFLICT (provider_name, user_id) DO UPDATE SET provider_uid = EXCLUDED.provider_uid, updated_at = now() \
                     WHERE user_associated_accounts.provider_uid IS DISTINCT FROM EXCLUDED.provider_uid",
                )
                .bind(&provider)
                .bind(&uid)
                .bind(target.id)
                .execute(&mut *tx)
                .await?;
            }
            None => {
                sqlx::query(
                    "DELETE FROM user_associated_accounts WHERE provider_name = $1 AND user_id = $2",
                )
                .bind(&provider)
                .bind(target.id)
                .execute(&mut *tx)
                .await?;
            }
        }
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
/// #notification_level: normal, mute, and ignore until a time
/// (IgnoreDurationModal's).
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
            // User.find(params[:acting_user_id]): staff act for someone.
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await?;
            if !exists {
                return Ok(super::topics::not_found_response(&state));
            }
        }
        acting = id;
    }
    let target_staff = target_admin || target_moderator;
    let level = params::string(&p, "notification_level").unwrap_or_default();
    match level.as_str() {
        "ignore" => {
            // guardian.ensure_can_ignore_user!
            if !(guardian.can_ignore_users(&settings)? && target_id != me.id && !target_staff) {
                return Ok(errors(StatusCode::UNPROCESSABLE_ENTITY, t("ignore_error")));
            }
            // Time.parse(params[:expiring_at]): what IgnoreDurationModal
            // sends is ISO 8601.
            let raw = params::string(&p, "expiring_at").unwrap_or_default();
            // FutureDateInputSelector's FORMAT ("YYYY-MM-DD HH:mmZ") or ISO 8601.
            let expiring_at = chrono::DateTime::parse_from_str(raw.trim(), "%Y-%m-%d %H:%M%:z")
                .or_else(|_| chrono::DateTime::parse_from_rfc3339(raw.trim()))
                .map(|d| d.naive_utc())
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(raw.trim(), "%Y-%m-%d %H:%M:%S"))
                .map_err(|_| {
                    crate::Unsupported("an expiring_at Time.parse reads but ISO 8601 does not")
                })?;
            sqlx::query("DELETE FROM muted_users WHERE user_id = $1 AND muted_user_id = $2")
                .bind(acting)
                .bind(target_id)
                .execute(&mut *tx)
                .await?;
            let existing: Option<i32> = sqlx::query_scalar(
                "SELECT id::int FROM ignored_users WHERE user_id = $1 AND ignored_user_id = $2 ORDER BY id LIMIT 1",
            )
            .bind(acting)
            .bind(target_id)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(id) = existing {
                // update: only a change is saved.
                sqlx::query(
                    "UPDATE ignored_users SET expiring_at = $2, updated_at = now() \
                     WHERE id = $1 AND expiring_at IS DISTINCT FROM $2",
                )
                .bind(id)
                .bind(expiring_at)
                .execute(&mut *tx)
                .await?;
            } else {
                sqlx::query(
                    "INSERT INTO ignored_users (user_id, ignored_user_id, expiring_at, created_at, updated_at) \
                     VALUES ($1, $2, $3, now(), now())",
                )
                .bind(acting)
                .bind(target_id)
                .bind(expiring_at)
                .execute(&mut *tx)
                .await?;
            }
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
