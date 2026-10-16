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
    let mut session = load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    if credential {
        let rp = crate::webauthn::RelyingParty::new(&state.config, &settings)?;
        let passkey = crate::webauthn::authenticate(
            &mut conn,
            &rp,
            &session_id,
            Some(user.id),
            p.get("publicKeyCredential"),
            &[crate::webauthn::FIRST_FACTOR],
        )
        .await?;
        if let Err(e) = passkey {
            // rescue SecurityKeyError: render_json_error(err.message, 401)
            let cookie = session.set_cookie(&state, &settings)?;
            return super::accounts::with_session_cookie(
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "errors": [e.message(&state.i18n)] })),
                )
                    .into_response(),
                cookie,
            );
        }
    }
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
         FROM user_security_keys WHERE user_id = $1 AND enabled AND factor_type = 0 ORDER BY created_at",
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

/// The second factor and passkey writes, after the filters Rails runs
/// first (login, then the confirmed session where check_confirmed_session
/// applies): the security key and passkey ones here, the authenticator
/// and backup code ones refused.
pub async fn second_factor_write(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Peer(peer): Peer,
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
    // The action: the segment after /u or /users, less .json; the id after
    // it for rename_passkey and delete_passkey.
    let mut segments = path.trim_start_matches('/').split('/').skip(1);
    let action = segments
        .next()
        .unwrap_or_default()
        .trim_end_matches(".json")
        .to_string();
    let id = segments
        .next()
        .map(|s| s.trim_end_matches(".json").to_string());
    let unconfirmed_ok = action == "create_passkey" || action == "rename_passkey";
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let mut session = load_forum_session(&state, &headers);
    if !unconfirmed_ok {
        if local_logins_off(&settings)? {
            return Ok(super::topics::not_found_response(&state));
        }
        let session_id = session.server_session_id();
        if !session_is_trusted(&mut conn, &session_id, user_id).await? {
            let cookie = session.set_cookie(&state, &settings)?;
            return super::accounts::with_session_cookie(
                super::search::invalid_access(&state),
                cookie,
            );
        }
    }
    let response = match action.as_str() {
        "create_second_factor_security_key" => {
            create_webauthn(&state, &settings, &mut conn, &mut session, user_id, false).await?
        }
        "create_passkey" => {
            if !settings.get("enable_passkeys")?.truthy() {
                return Ok(super::topics::not_found_response(&state));
            }
            create_webauthn(&state, &settings, &mut conn, &mut session, user_id, true).await?
        }
        "register_second_factor_security_key" | "register_passkey" => {
            let passkey = action == "register_passkey";
            if passkey && !settings.get("enable_passkeys")?.truthy() {
                return Ok(super::topics::not_found_response(&state));
            }
            for required in ["name", "attestation", "clientData"] {
                if params::string(&p, required).is_none_or(|v| v.trim().is_empty()) {
                    return Ok(super::accounts::param_missing(required));
                }
            }
            let rp = crate::webauthn::RelyingParty::new(&state.config, &settings)?;
            let session_id = session.server_session_id();
            let factor_type = if passkey {
                crate::webauthn::FIRST_FACTOR
            } else {
                crate::webauthn::SECOND_FACTOR
            };
            let registered =
                crate::webauthn::register(&mut conn, &rp, &session_id, user_id, &p, factor_type)
                    .await?;
            match (registered, passkey) {
                (Ok(_), false) => success(),
                (Ok(key), true) => Json(json!({
                    "success": "OK",
                    "id": key.id,
                    "name": key.name,
                }))
                .into_response(),
                // failed_json.merge(error:)
                (Err(e), false) => {
                    Json(json!({ "failed": "FAILED", "error": e.message(&state.i18n) }))
                        .into_response()
                }
                // render_json_error(err.message, status: 401)
                (Err(e), true) => (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "errors": [e.message(&state.i18n)] })),
                )
                    .into_response(),
            }
        }
        "rename_passkey" => {
            if !settings.get("enable_passkeys")?.truthy() {
                return Ok(super::topics::not_found_response(&state));
            }
            // params.require(:id), params.require(:name)
            let Some(name) = params::string(&p, "name").filter(|n| !n.trim().is_empty()) else {
                return Ok(super::accounts::param_missing("name"));
            };
            let id = id.as_deref().map(crate::ruby::to_i).unwrap_or(0);
            if name.chars().count() > 300 {
                return Err(
                    Unsupported("a passkey name over 300 characters (RecordInvalid)").into(),
                );
            }
            let renamed = sqlx::query(
                "UPDATE user_security_keys SET name = $3, updated_at = CASE WHEN name IS DISTINCT FROM $3 THEN clock_timestamp() ELSE updated_at END \
                 WHERE id = $1 AND user_id = $2 AND enabled",
            )
            .bind(id)
            .bind(i64::from(user_id))
            .bind(&name)
            .execute(&mut *conn)
            .await?;
            if renamed.rows_affected() == 0 {
                return Ok(super::search::invalid_parameters(&state, "id"));
            }
            success()
        }
        "delete_passkey" => {
            if !settings.get("enable_passkeys")?.truthy() {
                return Ok(super::topics::not_found_response(&state));
            }
            let id = id.as_deref().map(crate::ruby::to_i).unwrap_or(0);
            delete_passkey(&state, &mut conn, user_id, id).await?
        }
        "security_key" => {
            // update_security_key
            let id = params::string(&p, "id")
                .map(|v| crate::ruby::to_i(&v))
                .unwrap_or(0);
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM user_security_keys \
                 WHERE id = $1 AND user_id = $2 AND enabled)",
            )
            .bind(id)
            .bind(i64::from(user_id))
            .fetch_one(&mut *conn)
            .await?;
            if !exists {
                return Ok(super::search::invalid_parameters(
                    &state,
                    "Discourse::InvalidParameters",
                ));
            }
            if let Some(name) = params::string(&p, "name").filter(|n| !n.trim().is_empty()) {
                if name.chars().count() > 300 {
                    return Err(Unsupported(
                        "a security key name over 300 characters (RecordInvalid)",
                    )
                    .into());
                }
                sqlx::query(
                    "UPDATE user_security_keys SET name = $2, updated_at = CASE WHEN name IS DISTINCT FROM $2 THEN clock_timestamp() ELSE updated_at END \
                     WHERE id = $1",
                )
                .bind(id)
                .bind(&name)
                .execute(&mut *conn)
                .await?;
            }
            if params::string(&p, "disable").as_deref() == Some("true") {
                sqlx::query(
                    "UPDATE user_security_keys SET enabled = false, updated_at = clock_timestamp() \
                     WHERE id = $1",
                )
                .bind(id)
                .execute(&mut *conn)
                .await?;
            }
            success()
        }
        "create_second_factor_totp" => {
            // ROTP::Base32.random, staged for enable_second_factor_totp.
            let secret = crate::second_factor::random_secret();
            let session_id = session.server_session_id();
            crate::session::server::set(
                &mut conn,
                &session_id,
                &format!("staged-totp-{user_id}"),
                &json!(secret),
                crate::session::server::EXPIRY_SECONDS,
            )
            .await?;
            let email: Option<String> = sqlx::query_scalar(
                "SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"",
            )
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
            let uri = crate::second_factor::provisioning_uri(
                &secret,
                &settings.get("title")?.to_s(),
                email.as_deref().unwrap_or_default(),
            );
            let key = secret
                .as_bytes()
                .chunks(4)
                .filter(|c| c.len() == 4)
                .map(|c| std::str::from_utf8(c).unwrap_or_default())
                .collect::<Vec<_>>()
                .join(" ");
            Json(json!({
                "success": "OK",
                "key": key,
                "qr": crate::second_factor::qr_data_url(&uri)?,
            }))
            .into_response()
        }
        "enable_second_factor_totp" => {
            enable_totp(&state, &mut conn, &mut session, &p, peer, &headers, user_id).await?
        }
        "disable_second_factor" => {
            // user_second_factors.destroy_all, second_factor_security_keys
            // .destroy_all, then the email.
            sqlx::query("DELETE FROM user_second_factors WHERE user_id = $1")
                .bind(user_id)
                .execute(&mut *conn)
                .await?;
            sqlx::query(
                "DELETE FROM user_security_keys WHERE user_id = $1 AND enabled AND factor_type = 0",
            )
            .bind(i64::from(user_id))
            .execute(&mut *conn)
            .await?;
            crate::jobs::enqueue(
                &mut conn,
                "critical_user_email",
                json!({"type": "account_second_factor_disabled", "user_id": user_id}),
            )
            .await?;
            success()
        }
        "second_factor" => update_second_factor(&state, &mut conn, &p, user_id).await?,
        "second_factors_backup" => {
            let codes = crate::second_factor::generate_backup_codes(
                &mut conn,
                user_id,
                crate::second_factor::backup_code_iterations(state.config.rails_env),
            )
            .await?;
            Json(json!({ "success": "OK", "backup_codes": codes })).into_response()
        }
        _ => return Ok(super::topics::not_found_response(&state)),
    };
    let cookie = session.set_cookie(&state, &settings)?;
    super::accounts::with_session_cookie(response, cookie)
}

/// #create_second_factor_security_key and #create_passkey: a challenge
/// staged, with what navigator.credentials.create needs.
async fn create_webauthn(
    state: &AppState,
    settings: &SiteSettings,
    conn: &mut PgConnection,
    session: &mut crate::session::forum::ForumSession,
    user_id: i32,
    passkey: bool,
) -> Result<Response, AppError> {
    let factor_type = if passkey {
        crate::webauthn::FIRST_FACTOR
    } else {
        // all_security_keys.count >= MAX_KEYS_PER_USER
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM user_security_keys WHERE user_id = $1")
                .bind(i64::from(user_id))
                .fetch_one(&mut *conn)
                .await?;
        if count >= 50 {
            let error = state
                .i18n
                .t("login.too_many_security_keys")
                .unwrap_or_default();
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "errors": [error] })),
            )
                .into_response());
        }
        crate::webauthn::SECOND_FACTOR
    };
    let session_id = session.server_session_id();
    let challenge =
        crate::webauthn::stage_challenge(&mut *conn, &session_id, Some(user_id)).await?;
    let rp = crate::webauthn::RelyingParty::new(&state.config, settings)?;
    let secure_id = crate::webauthn::secure_identifier(&mut *conn, user_id).await?;
    let existing: Vec<String> = sqlx::query_scalar(
        "SELECT credential_id FROM user_security_keys WHERE user_id = $1 AND enabled \
         AND factor_type = $2 ORDER BY id",
    )
    .bind(i64::from(user_id))
    .bind(factor_type)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Map::new();
    out.insert("success".into(), json!("OK"));
    out.insert("challenge".into(), json!(challenge));
    out.insert("rp_id".into(), json!(rp.id));
    out.insert("rp_name".into(), json!(rp.name));
    out.insert(
        "supported_algorithms".into(),
        json!(crate::webauthn::SUPPORTED_ALGORITHMS),
    );
    out.insert("user_secure_id".into(), json!(secure_id));
    out.insert(
        if passkey {
            "existing_passkey_credential_ids"
        } else {
            "existing_active_credential_ids"
        }
        .into(),
        json!(existing),
    );
    Ok(Json(Value::Object(out)).into_response())
}

/// #enable_second_factor_totp: the staged secret saved as an authenticator
/// once a code from it checks out.
#[allow(clippy::too_many_arguments)]
async fn enable_totp(
    state: &AppState,
    conn: &mut PgConnection,
    session: &mut crate::session::forum::ForumSession,
    p: &Map<String, Value>,
    peer: Option<std::net::SocketAddr>,
    headers: &HeaderMap,
    user_id: i32,
) -> Result<Response, AppError> {
    let failed = |key: &str| {
        Json(json!({
            "failed": "FAILED",
            "error": state.i18n.t(key).unwrap_or_default(),
        }))
        .into_response()
    };
    let Some(token) = params::string(p, "second_factor_token").filter(|t| !t.trim().is_empty())
    else {
        return Ok(failed("login.missing_second_factor_code"));
    };
    let Some(name) = params::string(p, "name").filter(|n| !n.trim().is_empty()) else {
        return Ok(failed("login.missing_second_factor_name"));
    };
    let session_id = session.server_session_id();
    let staged =
        crate::session::server::get(&mut *conn, &session_id, &format!("staged-totp-{user_id}"))
            .await?;
    let Some(Value::String(secret)) = staged else {
        return Err(Unsupported("enabling an authenticator that was never staged").into());
    };
    // rate_limit_second_factor!(current_user)
    let ip = crate::remote_ip::remote_ip(headers, peer)?;
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    if let Err(refused) = crate::rate_limiter::perform_each(
        &state.pool,
        state.config.rate_limits,
        [
            RateLimiter::new(None, &format!("second-factor-min-{ip}"), 6, 60),
            RateLimiter::new(None, &format!("second-factor-min-{username}"), 6, 60),
        ],
    )
    .await?
    {
        return Ok(refused.into_response(&state.i18n));
    }
    let now = crate::clock::now().timestamp();
    if !crate::second_factor::verify_code(&secret, &token, now)? {
        return Ok(failed("login.invalid_second_factor_code"));
    }
    // create_totp: UserSecondFactor's validations.
    if name.chars().count() > 300 {
        return Err(
            Unsupported("an authenticator name over 300 characters (RecordInvalid)").into(),
        );
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_second_factors WHERE user_id = $1 AND method = 1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if count >= 50 {
        return Err(Unsupported("a 51st authenticator (RecordInvalid)").into());
    }
    sqlx::query(
        "INSERT INTO user_second_factors (user_id, method, data, enabled, name, created_at, updated_at) \
         VALUES ($1, 1, $2, TRUE, $3, clock_timestamp(), clock_timestamp())",
    )
    .bind(user_id)
    .bind(&secret)
    .bind(&name)
    .execute(&mut *conn)
    .await?;
    Ok(success())
}

/// #update_second_factor: an authenticator (by id) or the backup codes
/// renamed or disabled; disabling backup codes deletes them all.
async fn update_second_factor(
    state: &AppState,
    conn: &mut PgConnection,
    p: &Map<String, Value>,
    user_id: i32,
) -> Result<Response, AppError> {
    let Some(target) = params::string(p, "second_factor_target").filter(|t| !t.trim().is_empty())
    else {
        return Ok(super::accounts::param_missing("second_factor_target"));
    };
    let invalid = || super::search::invalid_parameters(state, "Discourse::InvalidParameters");
    let ids: Vec<i64> =
        match crate::ruby::to_i(&target) {
            crate::second_factor::TOTP => {
                let Some(id) = params::string(p, "id").filter(|t| !t.trim().is_empty()) else {
                    return Ok(super::accounts::param_missing("id"));
                };
                let found: Option<i64> = sqlx::query_scalar(
                "SELECT id FROM user_second_factors WHERE user_id = $1 AND method = 1 AND enabled \
                 AND id = $2",
            )
            .bind(user_id)
            .bind(crate::ruby::to_i(&id))
            .fetch_optional(&mut *conn)
            .await?;
                match found {
                    Some(id) => vec![id],
                    None => return Ok(invalid()),
                }
            }
            crate::second_factor::BACKUP_CODES => sqlx::query_scalar(
                "SELECT id FROM user_second_factors WHERE user_id = $1 AND method = 2 AND enabled \
                 ORDER BY id",
            )
            .bind(user_id)
            .fetch_all(&mut *conn)
            .await?,
            _ => return Ok(invalid()),
        };
    let backup = crate::ruby::to_i(&target) == crate::second_factor::BACKUP_CODES;
    if let Some(name) = params::string(p, "name").filter(|n| !n.trim().is_empty()) {
        if name.chars().count() > 300 {
            return Err(
                Unsupported("an authenticator name over 300 characters (RecordInvalid)").into(),
            );
        }
        sqlx::query(
            "UPDATE user_second_factors SET name = $2, updated_at = clock_timestamp() \
             WHERE id = ANY($1) AND name IS DISTINCT FROM $2",
        )
        .bind(&ids)
        .bind(&name)
        .execute(&mut *conn)
        .await?;
    }
    if params::string(p, "disable").as_deref() == Some("true") {
        if backup {
            sqlx::query("DELETE FROM user_second_factors WHERE user_id = $1 AND method = 2")
                .bind(user_id)
                .execute(&mut *conn)
                .await?;
        } else {
            sqlx::query(
                "UPDATE user_second_factors SET enabled = false, updated_at = clock_timestamp() \
                 WHERE id = ANY($1)",
            )
            .bind(&ids)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(success())
}

/// #delete_passkey: the user's own key by id (any factor type), unless it
/// is the last passkey of a user with no password and no associated
/// account.
async fn delete_passkey(
    state: &AppState,
    conn: &mut PgConnection,
    user_id: i32,
    id: i64,
) -> Result<Response, AppError> {
    let key: Option<i32> = sqlx::query_scalar(
        "SELECT factor_type FROM user_security_keys WHERE id = $1 AND user_id = $2 AND enabled",
    )
    .bind(id)
    .bind(i64::from(user_id))
    .fetch_optional(&mut *conn)
    .await?;
    if key == Some(crate::webauthn::FIRST_FACTOR) {
        let (passkeys, password): (i64, bool) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM user_security_keys WHERE user_id = $1 AND enabled \
                     AND factor_type = 1), \
                    EXISTS (SELECT 1 FROM user_passwords WHERE user_id = $1)",
        )
        .bind(i64::from(user_id))
        .fetch_one(&mut *conn)
        .await?;
        let accounts = crate::auth::associated_accounts(&mut *conn, &state.i18n, user_id)
            .await?
            .as_array()
            .is_some_and(|a| !a.is_empty());
        if passkeys == 1 && !password && !accounts {
            return Ok(Json(json!({
                "success": false,
                "message": state.i18n.t("user.cannot_remove_all_auth").unwrap_or_default(),
            }))
            .into_response());
        }
    }
    if key.is_some() {
        sqlx::query("DELETE FROM user_security_keys WHERE id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(success())
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
