//! Port of the account endpoints of users_controller.rb and
//! session_controller.rb: email login so far.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Redirect, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::accounts::{self, token_scopes};
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `expires_now`
const NO_STORE: &str = "no-cache, no-store";

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// `ActionController::ParameterMissing`, as ApplicationController renders it.
pub(super) fn param_missing(name: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"errors": [format!("param is missing or the value is empty or invalid: {name}")]})),
    )
        .into_response()
}

/// POST /u/email-login(.json): a login link for the user, said to have
/// worked either way when hide_email_address_taken is on.
pub async fn email_login(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_local_logins_via_email")?.truthy() {
        return Ok(super::topics::not_found_response(&state, false));
    }
    if guardian.is_authenticated() {
        return Ok(
            Redirect::to(&format!("{}/", state.config.globals.relative_url_root())).into_response(),
        );
    }
    let Some(login) = params::string(&p, "login").filter(|l| !l.trim().is_empty()) else {
        return Ok(param_missing("login"));
    };
    let user = accounts::find_by_username_or_email(&mut conn, &login).await?;
    let user_presence = user.as_ref().is_some_and(|u| !u.staged);
    if let Some(user) = user.filter(|u| !u.staged) {
        let email = user.email.clone().unwrap_or_default();
        let mut tx = state.pool.begin().await?;
        let token =
            accounts::create_email_token(&mut tx, user.id, &email, token_scopes::EMAIL_LOGIN)
                .await?;
        crate::jobs::enqueue(
            &mut tx,
            "critical_user_email",
            json!({"type": "email_login", "user_id": user.id, "email_token": token}),
        )
        .await?;
        tx.commit().await?;
    }
    let hide_taken = settings.get("hide_email_address_taken")?.truthy();
    let mut out = Map::new();
    out.insert("success".into(), json!("OK"));
    out.insert("hide_taken".into(), json!(hide_taken));
    if !hide_taken {
        out.insert("user_found".into(), json!(user_presence));
    }
    Ok((
        StatusCode::OK,
        [(header::CACHE_CONTROL, NO_STORE)],
        Json(Value::Object(out)),
    )
        .into_response())
}

/// The `password_reset_code` server-session value.
const RESET_CODE_KEY: &str = "password_reset_code";

/// `normalized_login_param`
fn normalized_login(p: &Map<String, Value>) -> Result<Option<String>, AppError> {
    let Some(login) = params::string(p, "login").filter(|l| !l.trim().is_empty()) else {
        return Ok(None);
    };
    let login = login.strip_prefix('@').unwrap_or(&login).trim().to_string();
    let lower = if login.is_ascii() {
        login.to_ascii_lowercase()
    } else {
        return Err(crate::Unsupported("non-ASCII logins (unicode normalization)").into());
    };
    Ok(Some(lower.chars().take(101).collect()))
}

/// The response with the session cookie when the session changed.
fn with_session_cookie(
    mut response: Response,
    cookie: Option<String>,
) -> Result<Response, AppError> {
    if let Some(cookie) = cookie {
        response.headers_mut().append(
            header::SET_COOKIE,
            cookie
                .parse()
                .map_err(|_| crate::Unsupported("an unencodable cookie"))?,
        );
    }
    Ok(response)
}

/// POST /session/forgot_password(.json): a reset code (or link) for the
/// address, said to have worked either way.
pub async fn forgot_password(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    super::search::Peer(peer): super::search::Peer,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let Some(login) = normalized_login(&p)? else {
        return Ok(param_missing("login"));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let ip = crate::session::current::remote_ip(&headers, peer);
    let mut tx = state.pool.begin().await?;
    if accounts::ip_blocked(&mut tx, &ip).await? {
        tx.commit().await?;
        return Ok(json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            vec![
                state
                    .i18n
                    .t("login.reset_not_allowed_from_ip_address")
                    .unwrap_or("")
                    .to_string(),
            ],
        ));
    }
    let hide_taken = settings.get("hide_email_address_taken")?.truthy();
    let user = if hide_taken && !guardian.is_staff() {
        if !accounts::valid_email(&login) {
            tx.commit().await?;
            return Ok(super::search::invalid_parameters(&state, "login"));
        }
        accounts::find_by_username_or_email(&mut tx, &login).await?
    } else {
        accounts::find_by_username_or_email(&mut tx, &login).await?
    }
    .filter(|u| !u.staged);
    // password_reset_via_code?: the upcoming change, for the requester.
    let via_code = guardian
        .upcoming_change_enabled(&mut tx, &settings, "enable_local_logins_via_code")
        .await?
        && guardian
            .user_id()
            .is_none_or(|id| user.as_ref().is_some_and(|u| u.id == id));
    let mut session = crate::session::forum::load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    match &user {
        Some(user) => {
            let email = user.email.clone().unwrap_or_default();
            if via_code {
                sqlx::query(
                    "UPDATE email_tokens SET expired = TRUE WHERE user_id = $1 AND (scope IS NULL OR scope = $2)",
                )
                .bind(user.id)
                .bind(token_scopes::PASSWORD_RESET)
                .execute(&mut *tx)
                .await?;
                let (code_id, code) = accounts::generate_login_code(
                    &mut tx,
                    &email,
                    accounts::code_purposes::PASSWORD_RESET,
                    &state.keys.secret_key_base,
                )
                .await?;
                crate::session::server::set(
                    &mut tx,
                    &session_id,
                    RESET_CODE_KEY,
                    &json!({"login_code_id": code_id, "user_id": user.id}),
                    crate::session::server::EXPIRY_SECONDS,
                )
                .await?;
                crate::jobs::enqueue(
                    &mut tx,
                    "send_email_login_code",
                    json!({"to_address": email, "code": code, "password_reset": true}),
                )
                .await?;
            } else {
                let token = accounts::create_email_token(
                    &mut tx,
                    user.id,
                    &email,
                    token_scopes::PASSWORD_RESET,
                )
                .await?;
                crate::jobs::enqueue(
                    &mut tx,
                    "critical_user_email",
                    json!({"type": "forgot_password", "user_id": user.id, "email_token": token}),
                )
                .await?;
            }
        }
        None => {
            crate::session::server::delete(&mut tx, &session_id, RESET_CODE_KEY).await?;
        }
    }
    tx.commit().await?;
    let mut out = Map::new();
    out.insert("success".into(), json!("OK"));
    if via_code {
        out.insert("email_code".into(), json!(true));
    }
    if !hide_taken {
        out.insert("user_found".into(), json!(user.is_some()));
    }
    let cookie = session.set_cookie(&state, &settings)?;
    with_session_cookie(
        (StatusCode::OK, Json(Value::Object(out))).into_response(),
        cookie,
    )
}

/// POST /session/password-reset-code/verify(.json): the emailed code
/// traded for a password reset token (User::CreatePasswordResetToken).
pub async fn redeem_password_reset_code(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !guardian
        .upcoming_change_enabled(&mut conn, &settings, "enable_local_logins_via_code")
        .await?
    {
        return Ok(super::topics::not_found_response(&state, false));
    }
    let invalid = || {
        (
            StatusCode::OK,
            [(header::CACHE_CONTROL, NO_STORE)],
            Json(json!({"error": state.i18n.t("email_login_code.invalid_code").unwrap_or("")})),
        )
            .into_response()
    };
    let code = params::string(&p, "code")
        .unwrap_or_default()
        .trim()
        .to_string();
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return Ok(invalid());
    }
    let mut session = crate::session::forum::load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let mut tx = state.pool.begin().await?;
    let stored = crate::session::server::get(&mut tx, &session_id, RESET_CODE_KEY).await?;
    let login_code_id = stored.as_ref().and_then(|v| v["login_code_id"].as_i64());
    let user_id = stored.as_ref().and_then(|v| v["user_id"].as_i64());
    // fetch_login_code: an active password reset code.
    let login_code: Option<(i64, String, String)> = sqlx::query_as(
        "SELECT id, email, code_hash FROM email_login_codes WHERE id = $1 AND purpose = 1 \
           AND consumed_at IS NULL AND expires_at > clock_timestamp() AND attempts < 5",
    )
    .bind(login_code_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((code_id, code_email, code_hash)) = login_code else {
        tx.commit().await?;
        return Ok(invalid());
    };
    // verify: one more attempt, reset on a match.
    let attempts: Option<i32> = sqlx::query_scalar(
        "UPDATE email_login_codes SET attempts = attempts + 1 WHERE id = $1 AND attempts < 5 RETURNING attempts",
    )
    .bind(code_id)
    .fetch_optional(&mut *tx)
    .await?;
    let matches = attempts.is_some()
        && accounts::hash_login_code(&code, &state.keys.secret_key_base) == code_hash;
    if !matches {
        tx.commit().await?;
        return Ok(invalid());
    }
    sqlx::query("UPDATE email_login_codes SET attempts = 0 WHERE id = $1")
        .bind(code_id)
        .execute(&mut *tx)
        .await?;
    // fetch_user: the real user the code was sent to.
    let user: Option<(i32, Option<String>)> = sqlx::query_as(
        "SELECT u.id, (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\" LIMIT 1) \
         FROM users u WHERE u.id = $1 AND u.id > 0 AND NOT u.staged",
    )
    .bind(user_id.map(|v| v as i32))
    .fetch_optional(&mut *tx)
    .await?;
    let Some((uid, Some(email))) = user.filter(|(_, e)| {
        e.as_deref()
            .is_some_and(|e| e.eq_ignore_ascii_case(&code_email))
    }) else {
        tx.commit().await?;
        return Ok(invalid());
    };
    let consumed = sqlx::query(
        "UPDATE email_login_codes SET consumed_at = clock_timestamp() WHERE id = $1 AND consumed_at IS NULL",
    )
    .bind(code_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if consumed != 1 {
        return Ok(invalid());
    }
    let token =
        accounts::create_email_token(&mut tx, uid, &email, token_scopes::PASSWORD_RESET).await?;
    crate::session::server::delete(&mut tx, &session_id, RESET_CODE_KEY).await?;
    tx.commit().await?;
    let cookie = session.set_cookie(&state, &settings)?;
    let response = (
        StatusCode::OK,
        [(header::CACHE_CONTROL, NO_STORE)],
        Json(json!({
            "success": "OK",
            "redirect_url": format!("{}/u/password-reset/{token}", state.config.globals.relative_url_root()),
        })),
    )
        .into_response();
    with_session_cookie(response, cookie)
}

/// `render_json_error(message)`
fn json_error(status: StatusCode, errors: Vec<String>) -> Response {
    (status, Json(json!({"errors": errors}))).into_response()
}

/// PUT /u/password-reset/:token(.json): the token confirmed, the new
/// password set, every session ended, and the user logged in.
pub async fn password_reset_update(
    State(state): State<AppState>,
    axum::extract::Path(token): axum::extract::Path<String>,
    headers: HeaderMap,
    super::search::Peer(peer): super::search::Peer,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let token = token.strip_suffix(".json").unwrap_or(&token).to_string();
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    drop(conn);
    let mut session = crate::session::forum::load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let mut tx = state.pool.begin().await?;

    // password_reset_find_user(token, committing_change: true)
    let user_id = match confirm_email_token(
        &mut tx,
        &settings,
        &token,
        token_scopes::PASSWORD_RESET,
    )
    .await?
    {
        Some(user_id) => {
            crate::session::server::set(
                &mut tx,
                &session_id,
                &format!("password-{token}"),
                &json!(user_id),
                crate::session::server::EXPIRY_SECONDS,
            )
            .await?;
            Some(user_id)
        }
        None => {
            match crate::session::server::get(&mut tx, &session_id, &format!("password-{token}"))
                .await?
            {
                Some(id) => {
                    let confirmed: Option<i32> = sqlx::query_scalar(
                    "SELECT user_id FROM email_tokens WHERE user_id = $1 AND token_hash = $2 AND scope = $3 AND confirmed",
                )
                .bind(id.as_i64().unwrap_or(0) as i32)
                .bind(accounts::hash_token(&token))
                .bind(token_scopes::PASSWORD_RESET)
                .fetch_optional(&mut *tx)
                .await?;
                    confirmed
                }
                None => None,
            }
        }
    };
    let Some(user_id) = user_id else {
        tx.commit().await?;
        return Err(crate::Unsupported("password reset with an unknown or used token").into());
    };
    let second_factors: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_second_factors WHERE user_id = $1 AND enabled) \
         OR EXISTS (SELECT 1 FROM user_security_keys WHERE user_id = $1 AND enabled)",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if second_factors {
        return Err(crate::Unsupported("password reset with second factors").into());
    }
    crate::session::server::set(
        &mut tx,
        &session_id,
        &format!("second-factor-{token}"),
        &json!(true),
        crate::session::server::EXPIRY_SECONDS,
    )
    .await?;
    let password = params::string(&p, "password").unwrap_or_default();
    #[derive(sqlx::FromRow)]
    struct Owner {
        admin: bool,
        moderator: bool,
        approved: bool,
        username: String,
        name: Option<String>,
        email: Option<String>,
        password_hash: Option<String>,
        password_salt: Option<String>,
        password_algorithm: Option<String>,
    }
    let owner: Owner = sqlx::query_as(
        "SELECT u.admin, u.moderator, u.approved, u.username, u.name, \
                (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\" LIMIT 1) AS email, \
                p.password_hash, p.password_salt, p.password_algorithm \
         FROM users u LEFT JOIN user_passwords p ON p.user_id = u.id WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if password.trim().is_empty() || password.chars().count() > 200 {
        return Err(crate::Unsupported("password reset error responses").into());
    }
    let current = match (
        &owner.password_hash,
        &owner.password_salt,
        &owner.password_algorithm,
    ) {
        (Some(h), Some(s), Some(a)) => Some((h.as_str(), s.as_str(), a.as_str())),
        _ => None,
    };
    let error = accounts::password_error(
        &settings,
        &password,
        &accounts::PasswordOwner {
            admin: owner.admin,
            username: &owner.username,
            name: owner.name.as_deref(),
            email: owner.email.as_deref(),
            current,
        },
    )?;
    if error.is_some() {
        return Err(crate::Unsupported("password reset error responses").into());
    }
    accounts::set_password(&mut tx, user_id, &password).await?;
    // expire_tokens_if_password_changed
    sqlx::query("DELETE FROM user_auth_tokens WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE email_tokens SET expired = TRUE WHERE user_id = $1 AND NOT expired")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    // Invite.invalidate_for_email
    let invites: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM invites WHERE lower(email) = lower($1) AND invalidated_at IS NULL \
           AND deleted_at IS NULL)",
    )
    .bind(&owner.email)
    .fetch_one(&mut *tx)
    .await?;
    if invites {
        return Err(crate::Unsupported("invalidating invites on password reset").into());
    }
    crate::session::server::delete(&mut tx, &session_id, &format!("password-{token}")).await?;
    crate::session::server::delete(&mut tx, &session_id, &format!("second-factor-{token}")).await?;
    if settings
        .get("delete_associated_accounts_on_password_reset")?
        .truthy()
    {
        sqlx::query("DELETE FROM user_associated_accounts WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    }
    // UserHistory.actions[:change_password]
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, admin_only, created_at, updated_at) \
         VALUES (67, $1, $1, FALSE, clock_timestamp(), clock_timestamp())",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    // reset_csrf_token, then logon_after_password_reset.
    session.map.remove("_csrf_token");
    session.changed = true;
    let can_access = owner.admin
        || owner.moderator
        || owner.approved
        || !settings.get("must_approve_users")?.truthy();
    let mut cookies = Vec::new();
    let message = if can_access {
        let ip = crate::session::current::remote_ip(&headers, peer);
        let user_agent = headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let (_, unhashed) = crate::session::token::generate(
            &mut tx,
            user_id,
            &state.keys.secret_key_base,
            user_agent.as_deref(),
            &ip,
            None,
        )
        .await?;
        crate::session::token::enforce_session_count_limit(
            &mut tx,
            user_id,
            settings.get("maximum_session_age")?.to_i(),
        )
        .await?;
        let session_user = crate::session::current::SessionUser::load(&mut tx, user_id)
            .await?
            .ok_or(crate::Unsupported("a user that vanished"))?;
        cookies.push(crate::session::current::auth_cookie(
            &state.keys,
            &settings,
            &session_user,
            &unhashed,
        )?);
        "password_reset.success"
    } else {
        "password_reset.success_unapproved"
    };
    tx.commit().await?;
    let mut response = (
        StatusCode::OK,
        [(header::CACHE_CONTROL, NO_STORE)],
        Json(json!({
            "success": true,
            "message": state.i18n.t(message).unwrap_or(""),
            "requires_approval": !can_access,
            "redirect_to": null,
        })),
    )
        .into_response();
    for cookie in cookies {
        response.headers_mut().append(
            header::SET_COOKIE,
            cookie
                .parse()
                .map_err(|_| crate::Unsupported("an unencodable cookie"))?,
        );
    }
    let cookie = session.set_cookie(&state, &settings)?;
    with_session_cookie(response, cookie)
}

/// `EmailToken.confirm(token, scope:)`: an unconfirmed, unexpired token of
/// the scope (or none) confirmed, its user activated; the user's id.
pub(super) async fn confirm_email_token(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    token: &str,
    scope: i32,
) -> Result<Option<i32>, AppError> {
    if token.is_empty() {
        return Ok(None);
    }
    let valid_hours = settings.get("email_token_valid_hours")?.to_i();
    let found: Option<(i32, i32, String)> = sqlx::query_as(
        "SELECT id, user_id, email FROM email_tokens \
         WHERE token_hash = $1 AND NOT confirmed AND NOT expired \
           AND created_at >= now() - make_interval(hours => $2) AND (scope = $3 OR scope IS NULL) \
         ORDER BY (scope = $3) DESC NULLS LAST LIMIT 1",
    )
    .bind(accounts::hash_token(token))
    .bind(valid_hours as i32)
    .bind(scope)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((id, user_id, email)) = found else {
        return Ok(None);
    };
    sqlx::query(
        "UPDATE email_tokens SET confirmed = TRUE, updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *conn)
    .await?;
    let (active, primary): (bool, Option<String>) = sqlx::query_as(
        "SELECT active, (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\" LIMIT 1) \
         FROM users u WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if primary.as_deref() != Some(email.as_str()) {
        return Err(crate::Unsupported("confirming a token for another address").into());
    }
    if settings.get("must_approve_users")?.truthy() {
        return Err(
            crate::Unsupported("reviewables for confirmed users (must_approve_users)").into(),
        );
    }
    let automatic: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM groups WHERE NOT automatic \
           AND LENGTH(COALESCE(automatic_membership_email_domains, '')) > 0)",
    )
    .fetch_one(&mut *conn)
    .await?;
    if automatic {
        return Err(crate::Unsupported("automatic group membership by email domain").into());
    }
    sqlx::query(
        "DELETE FROM user_custom_fields WHERE user_id = $1 AND name = 'activation_reminder'",
    )
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    if !active {
        crate::signup::activate_user(&mut *conn, settings, user_id).await?;
    }
    Ok(Some(user_id))
}

/// `HONEYPOT_KEY` and `CHALLENGE_KEY` in the server session.
const HONEYPOT_KEY: &str = "HONEYPOT_KEY";
const CHALLENGE_KEY: &str = "CHALLENGE_KEY";

/// `server_session[key] ||= SecureRandom.hex`
async fn server_session_value(
    conn: &mut sqlx::PgConnection,
    session_id: &str,
    key: &str,
) -> Result<String, AppError> {
    if let Some(v) = crate::session::server::get(&mut *conn, session_id, key)
        .await?
        .and_then(|v| v.as_str().map(str::to_string))
    {
        return Ok(v);
    }
    let value = accounts::random_hex();
    crate::session::server::set(
        &mut *conn,
        session_id,
        key,
        &json!(value),
        crate::session::server::EXPIRY_SECONDS,
    )
    .await?;
    Ok(value)
}

/// `honeypot_or_challenge_fails?(params)`
async fn honeypot_or_challenge_fails(
    conn: &mut sqlx::PgConnection,
    session_id: &str,
    p: &Map<String, Value>,
) -> Result<bool, AppError> {
    let honeypot = server_session_value(&mut *conn, session_id, HONEYPOT_KEY).await?;
    if params::string(p, "password_confirmation").as_deref() != Some(honeypot.as_str()) {
        return Ok(true);
    }
    let challenge = server_session_value(&mut *conn, session_id, CHALLENGE_KEY).await?;
    let reversed: String = challenge.chars().rev().collect();
    Ok(params::string(p, "challenge").as_deref() != Some(reversed.as_str()))
}

/// GET /session/hp(.json): the honeypot value and challenge signup and
/// activation must echo back.
pub async fn honeypot(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let mut session = crate::session::forum::load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let value = server_session_value(&mut conn, &session_id, HONEYPOT_KEY).await?;
    let challenge = server_session_value(&mut conn, &session_id, CHALLENGE_KEY).await?;
    let cookie = session.set_cookie(&state, &settings)?;
    with_session_cookie(
        Json(json!({
            "value": value,
            "challenge": challenge,
            "expires_in": crate::session::server::EXPIRY_SECONDS,
        }))
        .into_response(),
        cookie,
    )
}

/// POST /u(.json): users#create for a local signup (no CSRF check).
pub async fn create_user(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    super::search::Peer(peer): super::search::Peer,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    drop(conn);
    let mut session = crate::session::forum::load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let mut tx = state.pool.begin().await?;
    // respond_to_suspicious_request runs before the action's own checks.
    if honeypot_or_challenge_fails(&mut tx, &session_id, &p).await?
        || settings.get("invite_only")?.truthy()
    {
        tx.commit().await?;
        let email = params::string(&p, "email").unwrap_or_default();
        let message = state
            .i18n
            .t_with("login.activate_email", &[("email", &email)])
            .unwrap_or_default();
        let cookie = session.set_cookie(&state, &settings)?;
        return with_session_cookie(
            Json(json!({"success": true, "active": false, "message": message})).into_response(),
            cookie,
        );
    }
    if guardian.is_authenticated() {
        return Ok(super::search::invalid_access(&state));
    }
    let Some(email) = params::string(&p, "email").filter(|e| !e.is_empty()) else {
        return Ok(param_missing("email"));
    };
    let Some(username) = params::string(&p, "username").filter(|u| !u.is_empty()) else {
        return Ok(param_missing("username"));
    };
    let name = params::string(&p, "name");
    let password = params::string(&p, "password");
    let locale = params::string(&p, "locale");
    let timezone = params::string(&p, "timezone");
    let ip = crate::session::current::remote_ip(&headers, peer);
    let created = crate::signup::create(
        &mut tx,
        &settings,
        &state.i18n,
        &crate::signup::Signup {
            name: name.as_deref(),
            email: &email,
            password: password.as_deref(),
            username: &username,
            locale: locale.as_deref(),
            timezone: timezone.as_deref(),
            ip: &ip,
        },
    )
    .await?;
    let body = match created {
        crate::signup::Created::Json(body) => {
            tx.commit().await?;
            body
        }
        crate::signup::Created::Signed { user_id, body } => {
            crate::session::server::delete(&mut tx, &session_id, HONEYPOT_KEY).await?;
            crate::session::server::delete(&mut tx, &session_id, CHALLENGE_KEY).await?;
            crate::session::server::set(
                &mut tx,
                &session_id,
                "user_created_message",
                &body["message"],
                crate::session::server::EXPIRY_SECONDS,
            )
            .await?;
            tx.commit().await?;
            session.map.insert(
                "activate_user".into(),
                crate::session::cookie::Scalar::Int(user_id.into()),
            );
            session.changed = true;
            body
        }
    };
    let cookie = session.set_cookie(&state, &settings)?;
    with_session_cookie(Json(body).into_response(), cookie)
}

/// PUT /u/activate-account/:token(.json): the signup token confirmed, the
/// user activated and logged in.
pub async fn perform_account_activation(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    axum::extract::Path(token): axum::extract::Path<String>,
    headers: HeaderMap,
    super::search::Peer(peer): super::search::Peer,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let token = token.strip_suffix(".json").unwrap_or(&token).to_string();
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    drop(conn);
    let mut session = crate::session::forum::load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let mut tx = state.pool.begin().await?;
    if honeypot_or_challenge_fails(&mut tx, &session_id, &p).await? {
        tx.commit().await?;
        return Ok(super::search::invalid_access(&state));
    }
    if guardian.is_authenticated() {
        tx.commit().await?;
        return Ok(super::topics::not_found_response(&state, false));
    }
    let Some(user_id) =
        confirm_email_token(&mut tx, &settings, &token, token_scopes::SIGNUP).await?
    else {
        tx.commit().await?;
        return Ok(json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            vec![
                state
                    .i18n
                    .t("activation.already_done")
                    .unwrap_or("")
                    .to_string(),
            ],
        ));
    };
    let session_user = crate::session::current::SessionUser::load(&mut tx, user_id)
        .await?
        .ok_or(crate::Unsupported("a user that vanished"))?;
    if session_user.admin {
        return Err(crate::Unsupported("the wizard redirect for admins").into());
    }
    let ip = crate::session::current::remote_ip(&headers, peer);
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let (_, unhashed) = crate::session::token::generate(
        &mut tx,
        user_id,
        &state.keys.secret_key_base,
        user_agent.as_deref(),
        &ip,
        None,
    )
    .await?;
    crate::session::token::enforce_session_count_limit(
        &mut tx,
        user_id,
        settings.get("maximum_session_age")?.to_i(),
    )
    .await?;
    let auth =
        crate::session::current::auth_cookie(&state.keys, &settings, &session_user, &unhashed)?;
    tx.commit().await?;
    let response = with_session_cookie(
        Json(json!({"success": "OK", "redirect_to": null, "needs_approval": false}))
            .into_response(),
        Some(auth),
    )?;
    let cookie = session.set_cookie(&state, &settings)?;
    with_session_cookie(response, cookie)
}
