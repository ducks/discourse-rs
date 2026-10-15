//! The preferences security endpoints: UsersController#trusted_session,
//! #confirm_session (a password; a passkey is 155's), #list_second_factors
//! (the second factor page's lists; adding, editing and removing them is
//! 155's), #remove_password and #revoke_auth_token, and
//! CalendarSubscriptionsController (the calendar tab's private feed URLs).

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::search::Peer;
use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::rate_limiter::{LimitUser, RateLimiter};
use crate::session::current::AuthGuardian;
use crate::session::forum::load_forum_session;
use crate::site_settings::SiteSettings;
use crate::topic_list::time_json;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn success() -> Response {
    Json(json!({ "success": "OK" })).into_response()
}

fn failed() -> Response {
    Json(json!({ "failed": "FAILED" })).into_response()
}

/// `enable_discourse_connect || !enable_local_logins`: the session
/// confirmation and second factor endpoints are not found.
fn local_logins_off(s: &SiteSettings) -> Result<bool, AppError> {
    Ok(s.get("enable_discourse_connect")?.truthy() || !s.get("enable_local_logins")?.truthy())
}

fn confirmed_key(user_id: i32) -> String {
    format!("confirmed-session-{user_id}")
}

/// `session_is_trusted?`: the server session confirmed for the user, or
/// the user created in the last five minutes (user_just_created).
pub async fn session_is_trusted(
    conn: &mut PgConnection,
    session_id: &str,
    user_id: i32,
) -> Result<bool, sqlx::Error> {
    let confirmed = crate::session::server::get(conn, session_id, &confirmed_key(user_id)).await?;
    if confirmed.as_ref().and_then(Value::as_str) == Some("true") {
        return Ok(true);
    }
    sqlx::query_scalar("SELECT created_at > now() - interval '5 minutes' FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(conn)
        .await
        .map(|v| v.unwrap_or(false))
}

/// GET /u/trusted-session(.json)
pub async fn trusted_session(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if !super::session::is_xhr_or_json(&headers, uri.path()) {
        return Ok(super::session::render_empty());
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let mut session = load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let trusted = session_is_trusted(&mut conn, &session_id, user_id).await?;
    let cookie = session.set_cookie(&state, &settings)?;
    super::accounts::with_session_cookie(if trusted { success() } else { failed() }, cookie)
}

/// POST /u/confirm-session(.json)
pub async fn confirm_session(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Peer(peer): Peer,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let Some(user) = guardian.user() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if local_logins_off(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    let ip = crate::remote_ip::remote_ip(&headers, peer)?;
    if let Err(refused) = crate::rate_limiter::perform_each(
        &state.pool,
        state.config.rate_limits,
        [
            RateLimiter::new(
                None,
                &format!("login-hr-{ip}"),
                settings.get("max_logins_per_ip_per_hour")?.to_i(),
                3600,
            ),
            RateLimiter::new(
                None,
                &format!("login-min-{ip}"),
                settings.get("max_logins_per_ip_per_minute")?.to_i(),
                60,
            ),
        ],
    )
    .await?
    {
        return Ok(refused.into_response(&state.i18n));
    }
    let password = params::string(&p, "password").filter(|v| !v.trim().is_empty());
    let credential = p.get("publicKeyCredential").is_some_and(|v| match v {
        Value::String(s) => !s.trim().is_empty(),
        Value::Null => false,
        _ => true,
    });
    if password.is_none() && !credential {
        return Ok(super::search::invalid_parameters(
            &state,
            "Missing password or passkey",
        ));
    }
    let incorrect = || {
        let error = state
            .i18n
            .t("login.incorrect_password_or_passkey")
            .unwrap_or_default();
        Json(json!({ "failed": "FAILED", "error": error })).into_response()
    };
    if let Some(password) = password {
        let check = crate::session::token::confirm_password(&mut conn, user.id, &password)
            .await
            .map_err(AppError::from)?;
        if !check.is_some_and(|c| c.matches) {
            return Ok(incorrect());
        }
    }
    if credential {
        return Err(Unsupported("confirming the session with a passkey (155)").into());
    }
    let mut session = load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    crate::session::server::set(
        &mut conn,
        &session_id,
        &confirmed_key(user.id),
        &json!("true"),
        crate::session::server::EXPIRY_SECONDS,
    )
    .await?;
    let cookie = session.set_cookie(&state, &settings)?;
    super::accounts::with_session_cookie(success(), cookie)
}

/// POST /u/second_factors(.json): the enabled authenticators and second
/// factor security keys, once the session is trusted.
pub async fn list_second_factors(
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
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if local_logins_off(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    let mut session = load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let body = if session_is_trusted(&mut conn, &session_id, user_id).await? {
        let lists = second_factors(&mut conn, user_id).await?;
        json!({
            "success": "OK",
            "totps": lists.totps,
            "security_keys": lists.security_keys,
        })
    } else {
        json!({ "success": "OK", "unconfirmed_session": true })
    };
    let cookie = session.set_cookie(&state, &settings)?;
    super::accounts::with_session_cookie(Json(body).into_response(), cookie)
}

/// The second factor page's lists, as list_second_factors' as_json has
/// them.
pub struct SecondFactors {
    pub totps: Vec<Value>,
    pub security_keys: Vec<Value>,
}

pub async fn second_factors(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<SecondFactors, sqlx::Error> {
    type Totp = (i64, Option<String>, Option<chrono::NaiveDateTime>, i32);
    let totps: Vec<Totp> = sqlx::query_as(
        "SELECT id, name, last_used, method FROM user_second_factors \
         WHERE user_id = $1 AND method = 1 AND enabled ORDER BY created_at",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    type Key = (
        i64,
        i64,
        String,
        String,
        i32,
        bool,
        String,
        Option<chrono::NaiveDateTime>,
    );
    let keys: Vec<Key> = sqlx::query_as(
        "SELECT id, user_id, credential_id, public_key, factor_type, enabled, name, last_used \
         FROM user_security_keys WHERE user_id = $1 AND factor_type = 0 ORDER BY created_at",
    )
    .bind(i64::from(user_id))
    .fetch_all(&mut *conn)
    .await?;
    Ok(SecondFactors {
        totps: totps
            .into_iter()
            .map(|(id, name, last_used, method)| {
                json!({
                    "id": id,
                    "name": name,
                    "last_used": last_used.map(time_json),
                    "method": method,
                })
            })
            .collect(),
        security_keys: keys
            .into_iter()
            .map(
                |(
                    id,
                    user_id,
                    credential_id,
                    public_key,
                    factor_type,
                    enabled,
                    name,
                    last_used,
                )| {
                    json!({
                        "id": id,
                        "user_id": user_id,
                        "credential_id": credential_id,
                        "public_key": public_key,
                        "factor_type": factor_type,
                        "enabled": enabled,
                        "name": name,
                        "last_used": last_used.map(time_json),
                    })
                },
            )
            .collect(),
    })
}

/// The second factor and passkey writes, all 155's: refused after the
/// filters Rails runs first (login, then the confirmed session where
/// check_confirmed_session applies).
pub async fn second_factor_write(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: axum::http::Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let path = uri.path().to_string();
    if !csrf_ok(&state, &headers, &form_pairs(&p), &path, method.as_str()) {
        return Ok(bad_csrf());
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let unconfirmed_ok = path.contains("create_passkey") || path.contains("rename_passkey");
    if !unconfirmed_ok {
        let mut conn = state.pool.acquire().await?;
        let settings =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
        if local_logins_off(&settings)? {
            return Ok(super::topics::not_found_response(&state));
        }
        let mut session = load_forum_session(&state, &headers);
        let session_id = session.server_session_id();
        if !session_is_trusted(&mut conn, &session_id, user_id).await? {
            return Ok(super::search::invalid_access(&state));
        }
    }
    Err(Unsupported("managing second factors and passkeys (155)").into())
}

/// PUT /u/:username/remove-password(.json)
pub async fn remove_password(
    state: AppState,
    guardian: crate::guardian::Guardian,
    username: String,
    headers: HeaderMap,
    peer: Option<std::net::SocketAddr>,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    let Some(me) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let ip = crate::remote_ip::remote_ip(&headers, peer)?;
    let ip_limits = [
        RateLimiter::new(None, &format!("remove-password-hr-{ip}"), 6, 3600),
        RateLimiter::new(None, &format!("remove-password-min-{ip}"), 3, 3600),
    ];
    if let Err(refused) =
        crate::rate_limiter::perform_each(&state.pool, state.config.rate_limits, ip_limits).await?
    {
        return Ok(refused.into_response(&state.i18n));
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let Some(user) = crate::user_updater::find_target(&mut tx, &guardian, &username).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !crate::user_updater::can_edit_user(&guardian, &user) {
        return Ok(super::search::invalid_access(&state));
    }
    let limit = RateLimiter::new(
        None,
        &format!("remove-password-hr-{}", user.username),
        6,
        3600,
    );
    if let Err(refused) = limit
        .performed(&state.pool, state.config.rate_limits)
        .await?
    {
        return Ok(refused.into_response(&state.i18n));
    }
    let has_password: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_passwords WHERE user_id = $1)")
            .bind(user.id)
            .fetch_one(&mut *tx)
            .await?;
    if !has_password {
        return Ok(super::topics::not_found_response(&state));
    }
    let mut session = load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    if !session_is_trusted(&mut tx, &session_id, me).await? {
        return Ok(super::search::invalid_access(&state));
    }
    // User#can_remove_password?: an associated account or a passkey.
    let other_login: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_associated_accounts WHERE user_id = $1) \
         OR EXISTS (SELECT 1 FROM user_security_keys WHERE user_id = $1 AND factor_type = 1)",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    if !other_login {
        return Ok(super::search::invalid_access(&state));
    }
    sqlx::query("DELETE FROM user_passwords WHERE user_id = $1")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let cookie = session.set_cookie(&state, &settings)?;
    super::accounts::with_session_cookie(success(), cookie)
}

/// POST /u/:username/preferences/revoke-auth-token(.json): one device
/// (not the current one) or, without a token_id, all of them.
pub async fn revoke_auth_token(
    state: AppState,
    incoming: &crate::session::current::Incoming,
    username: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let guardian = &incoming.guardian;
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let Some(user) = crate::user_updater::find_target(&mut tx, guardian, &username).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !crate::user_updater::can_edit_user(guardian, &user) {
        return Ok(super::search::invalid_access(&state));
    }
    if settings.get("verbose_auth_token_logging")?.truthy() {
        return Err(Unsupported("verbose auth token logging").into());
    }
    match params::string(&p, "token_id").filter(|t| !t.is_empty()) {
        Some(token_id) => {
            let current = incoming
                .session
                .as_ref()
                .map(|s| s.token.auth_token.as_str());
            let token: Option<String> =
                match token_id.parse::<i64>() {
                    Ok(id) => sqlx::query_scalar(
                        "SELECT auth_token FROM user_auth_tokens WHERE id = $1 AND user_id = $2",
                    )
                    .bind(id)
                    .bind(user.id)
                    .fetch_optional(&mut *tx)
                    .await?,
                    Err(_) => None,
                };
            let Some(token) = token.filter(|t| Some(t.as_str()) != current) else {
                return Ok(super::search::invalid_parameters(&state, "token_id"));
            };
            sqlx::query("DELETE FROM user_auth_tokens WHERE auth_token = $1 AND user_id = $2")
                .bind(&token)
                .bind(user.id)
                .execute(&mut *tx)
                .await?;
            state
                .bus
                .publish(
                    &mut tx,
                    "/file-change",
                    &json!(["refresh"]),
                    Some(&[crate::bus::user_tag(user.id)]),
                )
                .await?;
        }
        None => {
            sqlx::query("DELETE FROM user_auth_tokens WHERE user_id = $1")
                .bind(user.id)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    Ok(success())
}

/// CalendarSubscriptionsController::CLIENT_ID
const CALENDAR_CLIENT_ID: &str = "calendar-subscriptions";

/// find_calendar_api_key: the user's active key of the calendar client.
async fn calendar_key(conn: &mut PgConnection, user_id: i32) -> Result<Option<i32>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT k.id FROM user_api_keys k JOIN user_api_key_clients c ON c.id = k.user_api_key_client_id \
         WHERE k.user_id = $1 AND c.client_id = $2 AND k.revoked_at IS NULL \
           AND (k.expires_at IS NULL OR k.expires_at > now()) \
         ORDER BY k.id LIMIT 1",
    )
    .bind(user_id)
    .bind(CALENDAR_CLIENT_ID)
    .fetch_optional(conn)
    .await
}

/// The calendar tab's `has_subscription`.
pub async fn has_calendar_subscription(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<bool, sqlx::Error> {
    Ok(calendar_key(conn, user_id).await?.is_some())
}

async fn revoke_calendar_key(conn: &mut PgConnection, user_id: i32) -> Result<(), sqlx::Error> {
    if let Some(id) = calendar_key(conn, user_id).await? {
        sqlx::query(
            "UPDATE user_api_keys SET revoked_at = now(), updated_at = now() WHERE id = $1",
        )
        .bind(id)
        .execute(conn)
        .await?;
    }
    Ok(())
}

/// GET, POST and DELETE /calendar-subscriptions(.json). The only feed is
/// the bookmarks one: no plugin here registers another.
pub async fn calendar_subscriptions(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: axum::http::Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if method != axum::http::Method::GET
        && !csrf_ok(
            &state,
            &headers,
            &form_pairs(&p),
            uri.path(),
            method.as_str(),
        )
    {
        return Ok(bad_csrf());
    }
    if method == axum::http::Method::GET && !super::session::is_xhr_or_json(&headers, uri.path()) {
        return Ok(super::session::render_empty());
    }
    let Some(user) = guardian.user() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut tx = state.pool.begin().await?;
    match method {
        axum::http::Method::GET => {
            let key = calendar_key(&mut tx, user.id).await?;
            let generated: Vec<&str> = Vec::new();
            Ok(Json(json!({
                "has_subscription": key.is_some(),
                "feeds": ["bookmarks"],
                "generated_feeds": generated,
            }))
            .into_response())
        }
        axum::http::Method::POST => {
            let limit = RateLimiter::new(
                Some(LimitUser {
                    id: user.id,
                    staff: guardian.is_staff(),
                }),
                "calendar-subscriptions-create",
                5,
                60,
            );
            if let Err(refused) = limit
                .performed(&state.pool, state.config.rate_limits)
                .await?
            {
                return Ok(refused.into_response(&state.i18n));
            }
            let settings =
                SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals)
                    .await?;
            revoke_calendar_key(&mut tx, user.id).await?;
            let client: Option<i64> =
                sqlx::query_scalar("SELECT id FROM user_api_key_clients WHERE client_id = $1")
                    .bind(CALENDAR_CLIENT_ID)
                    .fetch_optional(&mut *tx)
                    .await?;
            let client = match client {
                Some(id) => id,
                None => {
                    let name = state
                        .i18n
                        .t("calendar_subscriptions.application_name")
                        .unwrap_or("Calendar Subscriptions");
                    sqlx::query_scalar(
                        "INSERT INTO user_api_key_clients (client_id, application_name, created_at, updated_at) \
                         VALUES ($1, $2, now(), now()) RETURNING id",
                    )
                    .bind(CALENDAR_CLIENT_ID)
                    .bind(name)
                    .fetch_one(&mut *tx)
                    .await?
                }
            };
            let key = crate::accounts::random_hex();
            let key_id: i32 = sqlx::query_scalar(
                "INSERT INTO user_api_keys (user_id, created_at, updated_at, key_hash, user_api_key_client_id) \
                 VALUES ($1, now(), now(), $2, $3) RETURNING id",
            )
            .bind(user.id)
            .bind(crate::session::user_api_key::hash_key(&key))
            .bind(client)
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO user_api_key_scopes (user_api_key_id, name, created_at, updated_at) \
                 VALUES ($1, 'bookmarks_calendar', now(), now())",
            )
            .bind(key_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            let urls = crate::url::Urls {
                config: &state.config,
                settings: &settings,
            };
            let base = urls.base_url()?;
            Ok(Json(json!({
                "key": key,
                "urls": {
                    "bookmarks": format!(
                        "{base}/u/{}/bookmarks.ics?user_api_key={key}",
                        user.username.to_lowercase()
                    ),
                },
            }))
            .into_response())
        }
        axum::http::Method::DELETE => {
            revoke_calendar_key(&mut tx, user.id).await?;
            tx.commit().await?;
            Ok(StatusCode::NO_CONTENT.into_response())
        }
        _ => Ok(super::topics::not_found_response(&state)),
    }
}
