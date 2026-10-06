//! Port of session_controller.rb (#create, #destroy, #csrf, #current) and
//! static_controller.rb#enter for local logins. Rate limits (redis) and
//! second factors are not ported; a user with 2FA enabled gets Rails'
//! failure payload so the client shows its 2FA prompt, which then fails.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use super::search::Peer;
use crate::guardian::Guardian;
use crate::session::cookie::Scalar;
use crate::session::current::{self, Incoming, SESSION_USER_COLUMNS, SessionUser};
use crate::session::forum::load_forum_session;
use crate::session::{csrf, token};
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// `Cache-Control` on every session response.
const NO_STORE: &str = "no-store, must-revalidate, private, max-age=0";

/// `verify_authenticity_token` for a non-GET request: the session's token
/// against `X-CSRF-Token` or the `authenticity_token` field.
pub(super) fn csrf_ok(
    state: &AppState,
    headers: &HeaderMap,
    form: &[(String, String)],
    path: &str,
    method: &str,
) -> bool {
    // API requests are not checked (handle_unverified_request); a key that
    // reached a handler was valid, the session middleware saw to it.
    if headers.contains_key(crate::session::api_key::HEADER_API_KEY) {
        return true;
    }
    let session = load_forum_session(state, headers);
    let Some(Scalar::Str(stored)) = session.map.get("_csrf_token") else {
        return false;
    };
    let presented = headers
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| {
            form.iter()
                .find(|(k, _)| k == "authenticity_token")
                .map(|(_, v)| v.clone())
        });
    presented.is_some_and(|p| csrf::valid(stored, &p, path, method))
}

/// `handle_unverified_request`
pub(super) fn bad_csrf() -> Response {
    (
        StatusCode::FORBIDDEN,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "[\"BAD CSRF\"]",
    )
        .into_response()
}

fn is_xhr_or_json(headers: &HeaderMap, path: &str) -> bool {
    path.ends_with(".json")
        || headers
            .get("x-requested-with")
            .is_some_and(|v| v.as_bytes() == b"XMLHttpRequest")
        || headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|a| a.contains("application/json"))
}

/// `check_xhr`'s RenderEmpty: the empty HTML template.
fn render_empty() -> Response {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], "").into_response()
}

fn json_response(body: serde_json::Value) -> Response {
    ([(header::CACHE_CONTROL, NO_STORE)], Json(body)).into_response()
}

/// The body's top-level params as strings, a form or a JSON object alike.
fn parse_form(headers: &HeaderMap, body: &Bytes) -> Vec<(String, String)> {
    crate::params::parse(None, headers, body)
        .iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn param<'a>(form: &'a [(String, String)], name: &str) -> Option<&'a str> {
    form.iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// GET /session/csrf(.json)
pub async fn csrf(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let mut session = load_forum_session(&state, &headers);
    let stored = session.csrf_token();
    let masked = csrf::mask(&stored).ok_or(Unsupported("session csrf token not decodable"))?;
    let mut response = json_response(json!({"csrf": masked}));
    if let Some(cookie) = session.set_cookie(&state, &settings)? {
        response
            .headers_mut()
            .append(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
    }
    Ok(response)
}

/// A masked CSRF token for an anonymous visitor's form, and the
/// `_forum_session` cookie to set when minting it created the session. Only
/// for pages that are never cached, such as the login page.
pub(super) fn anonymous_csrf(
    state: &AppState,
    headers: &HeaderMap,
    settings: &SiteSettings,
) -> Result<(String, Option<String>), AppError> {
    let mut session = load_forum_session(state, headers);
    let stored = session.csrf_token();
    let masked = csrf::mask(&stored).ok_or(Unsupported("session csrf token not decodable"))?;
    Ok((masked, session.set_cookie(state, settings)?))
}

/// The page shell's viewer block for a logged-in guardian: the username
/// and a masked CSRF token from the `_forum_session` (created when
/// absent, which sets the cookie).
pub(super) fn viewer_state(
    state: &AppState,
    headers: &HeaderMap,
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<crate::html::ViewerState, AppError> {
    let Some(user) = guardian.user() else {
        return Ok(crate::html::ViewerState::default());
    };
    let mut session = load_forum_session(state, headers);
    let stored = session.csrf_token();
    let masked = csrf::mask(&stored).ok_or(Unsupported("session csrf token not decodable"))?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    // The header's avatar, at the size the Ember client asks for.
    let avatar_url = crate::avatar::avatar_template(
        &urls,
        user.id,
        &user.username,
        user.uploaded_avatar_id,
        None,
    )?
    .replace("{size}", "48");
    Ok(crate::html::ViewerState {
        viewer: Some(crate::html::Viewer {
            username: user.username.clone(),
            csrf_token: masked,
            avatar_url,
            trust_level: user.trust_level,
        }),
        set_cookie: session.set_cookie(state, settings)?,
    })
}
/// `invalid_credentials`
fn invalid_credentials(state: &AppState) -> Response {
    json_response(json!({
        "error": state.i18n.t("login.incorrect_username_email_or_password")
            .unwrap_or("Incorrect username, email or password")
    }))
}

/// POST /session(.json)
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Peer(peer): Peer,
    body: Bytes,
) -> Result<Response, AppError> {
    let form = parse_form(&headers, &body);
    if !csrf_ok(&state, &headers, &form, "/session", "POST") {
        return Ok(bad_csrf());
    }
    if !is_xhr_or_json(&headers, "/session") {
        return Ok(render_empty());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // check_local_login_allowed
    if settings.get("enable_discourse_connect")?.truthy()
        || !settings.get("enable_local_logins")?.truthy()
    {
        return Ok((
            StatusCode::FORBIDDEN,
            Json(json!({
                "errors": [state.i18n.t("invalid_access").unwrap_or("You are not permitted to view the requested resource.")],
                "error_type": "invalid_access"
            })),
        )
            .into_response());
    }
    for required in ["login", "password"] {
        if param(&form, required).is_none_or(|v| v.trim().is_empty()) {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(json!({"errors": [format!("param is missing or the value is empty or invalid: {required}")]})),
            )
                .into_response());
        }
    }
    let password = param(&form, "password").unwrap_or_default();
    if password.chars().count() > 200 {
        return Ok(invalid_credentials(&state));
    }

    // normalized_login_param + User.find_by_username_or_email
    let login = param(&form, "login").unwrap_or_default();
    let login = login.strip_prefix('@').unwrap_or(login);
    let login: String = login.trim().to_lowercase().chars().take(101).collect();
    let user: Option<SessionUser> = if login.contains('@') {
        sqlx::query_as(&format!(
            "SELECT {SESSION_USER_COLUMNS} FROM users \
             INNER JOIN user_emails ON user_emails.user_id = users.id \
             WHERE lower(user_emails.email) = $1 LIMIT 1"
        ))
        .bind(&login)
        .fetch_optional(&mut *conn)
        .await?
    } else {
        sqlx::query_as(&format!(
            "SELECT {SESSION_USER_COLUMNS} FROM users WHERE username_lower = $1"
        ))
        .bind(&login)
        .fetch_optional(&mut *conn)
        .await?
    };
    let Some(user) = user else {
        return Ok(invalid_credentials(&state));
    };
    let Some(check) = token::confirm_password(&mut conn, user.id, password).await? else {
        return Ok(invalid_credentials(&state));
    };
    if !check.matches {
        return Ok(invalid_credentials(&state));
    }
    if settings.get("must_approve_users")?.truthy() && !user.admin {
        let approved: bool = sqlx::query_scalar("SELECT approved FROM users WHERE id = $1")
            .bind(user.id)
            .fetch_one(&mut *conn)
            .await?;
        if !approved {
            return Ok(json_response(json!({
                "error": state.i18n.t("login.not_approved")
                    .unwrap_or("Your account hasn't been approved yet. You will be notified by email when you are ready to log in.")
            })));
        }
    }
    // Invite.invalidate_for_email
    let email: Option<String> =
        sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"")
            .bind(user.id)
            .fetch_optional(&mut *conn)
            .await?;
    if let Some(email) = &email {
        sqlx::query("UPDATE invites SET invalidated_at = now() WHERE lower(email) = lower($1) AND invalidated_at IS NULL")
            .bind(email)
            .execute(&mut *conn)
            .await?;
    }
    if check.expired {
        return Ok(json_response(
            json!({"error": "expired", "reason": "expired"}),
        ));
    }
    // login_error_check
    if user.suspended() {
        return Err(Unsupported("suspended users at login (staff reason sanitizing)").into());
    }
    let blocked: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM screened_ip_addresses WHERE action_type = 1)",
    )
    .fetch_one(&mut *conn)
    .await?;
    if blocked {
        return Err(Unsupported("screened IP addresses").into());
    }
    // Second factors: refused with Rails' failure payload.
    let (totp, backup, keys): (bool, bool, bool) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM user_second_factors WHERE user_id = $1 AND enabled AND method = 1), \
                EXISTS (SELECT 1 FROM user_second_factors WHERE user_id = $1 AND enabled AND method = 2), \
                EXISTS (SELECT 1 FROM user_security_keys WHERE user_id = $1 AND enabled AND factor_type = 0)",
    )
    .bind(user.id)
    .fetch_one(&mut *conn)
    .await?;
    if totp || backup || keys {
        return Ok(json_response(json!({
            "failed": "FAILED",
            "error": state.i18n.t("login.invalid_second_factor_method").unwrap_or("The selected two-factor method is invalid."),
            "reason": "invalid_second_factor_method",
            "backup_enabled": backup,
            "security_key_enabled": keys,
            "totp_enabled": totp,
            "multiple_second_factor_methods": keys && (totp || backup),
        })));
    }
    // user.active && user.email_confirmed?
    let email_confirmed = match &email {
        Some(email) => {
            let confirmed: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM email_tokens WHERE user_id = $1 AND email = $2 AND confirmed)",
            )
            .bind(user.id)
            .bind(email)
            .fetch_one(&mut *conn)
            .await?;
            let any_tokens: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM email_tokens WHERE user_id = $1)")
                    .bind(user.id)
                    .fetch_one(&mut *conn)
                    .await?;
            let sso_email: Option<Option<String>> = sqlx::query_scalar(
                "SELECT external_email FROM single_sign_on_records WHERE user_id = $1",
            )
            .bind(user.id)
            .fetch_optional(&mut *conn)
            .await?;
            confirmed
                || !any_tokens
                || sso_email
                    .flatten()
                    .is_some_and(|e| e.to_lowercase() == email.to_lowercase())
        }
        None => false,
    };
    if !user.active || !email_confirmed {
        return Ok(json_response(json!({
            "error": state.i18n.t("login.not_activated")
                .unwrap_or("You can't log in yet. We sent an activation email to you. Please follow the instructions in the email to activate your account."),
            "reason": "not_activated",
            "sent_to_email": email,
            "current_email": email,
        })));
    }

    // login(user): timezone, log_on_user, UserSerializer.
    if let Some(tz) = param(&form, "timezone").filter(|t| !t.is_empty())
        && (tz.contains('/') || tz == "UTC")
    {
        sqlx::query(
            "UPDATE user_options SET timezone = $2 WHERE user_id = $1 AND timezone IS NULL",
        )
        .bind(user.id)
        .bind(tz)
        .execute(&mut *conn)
        .await?;
    }
    let ip = current::remote_ip(&headers, peer);
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let (_, unhashed) = token::generate(
        &mut conn,
        user.id,
        &state.keys.secret_key_base,
        user_agent.as_deref(),
        &ip,
        Some("/session"),
    )
    .await?;
    if user.staged {
        sqlx::query("UPDATE users SET staged = false WHERE id = $1")
            .bind(user.id)
            .execute(&mut *conn)
            .await?;
    }
    token::enforce_session_count_limit(
        &mut conn,
        user.id,
        settings.get("maximum_session_age")?.to_i(),
    )
    .await?;
    let cookie = current::auth_cookie(&state.keys, &settings, &user, &unhashed)?;

    // The body: UserSerializer for the user themselves; until the
    // logged-in serializers are ported this is the public profile
    // document, which the client ignores (it reloads the page).
    let guardian = crate::guardian::Guardian::anonymous();
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let profile = crate::users::User::find_active(&mut conn, &user.username)
        .await?
        .ok_or(Unsupported(
            "logging in a user the profile loader cannot see",
        ))?;
    let doc = crate::users::Users {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        base_path: state.config.globals.relative_url_root(),
        auth_token: None,
    }
    .show(&profile)
    .await?;
    let mut response = json_response(doc);
    response.headers_mut().insert(
        "x-discourse-username",
        HeaderValue::from_str(&user.username)?,
    );
    response
        .headers_mut()
        .append(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
    Ok(response)
}

#[derive(Deserialize, Default)]
pub struct EnterParams {
    redirect: Option<String>,
}

/// POST /login (static#enter): the hidden form the client submits after a
/// successful login; redirects to a path on this site.
pub async fn enter(
    State(state): State<AppState>,
    Query(query): Query<EnterParams>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    let form = parse_form(&headers, &body);
    let redirect = param(&form, "redirect")
        .map(str::to_string)
        .or(query.redirect)
        .unwrap_or_default();
    let base_path = state.config.globals.relative_url_root();
    // extract_redirect_param: a path on this host, not the login page.
    let safe = redirect.starts_with('/')
        && !redirect.starts_with("//")
        && !redirect.starts_with(&format!("{base_path}/login"))
        && !redirect.contains('.')
        && !redirect.contains(char::is_whitespace);
    let destination = if safe && !redirect.is_empty() {
        redirect
    } else {
        format!("{base_path}/")
    };
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, format!("http://{host}{destination}"))],
    )
        .into_response())
}

/// DELETE /session/{username}(.json)
pub async fn destroy(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
    axum::Extension(incoming): axum::Extension<Incoming>,
    body: Bytes,
) -> Result<Response, AppError> {
    let form = parse_form(&headers, &body);
    let path = format!("/session/{username}");
    if !csrf_ok(&state, &headers, &form, &path, "DELETE") {
        return Ok(bad_csrf());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let base_path = state.config.globals.relative_url_root();
    let return_url = param(&form, "return_url")
        .filter(|u| u.starts_with('/') && !u.starts_with("//"))
        .map(str::to_string);
    let redirect_url = return_url
        .or_else(|| settings.get("logout_redirect").ok()?.presence())
        .unwrap_or_else(|| format!("{base_path}/"));
    if let Some(session) = &incoming.session {
        if settings.get("log_out_strict")?.truthy() {
            sqlx::query("DELETE FROM user_auth_tokens WHERE user_id = $1")
                .bind(session.user.id)
                .execute(&mut *conn)
                .await?;
        } else {
            sqlx::query("DELETE FROM user_auth_tokens WHERE id = $1")
                .bind(session.token.id)
                .execute(&mut *conn)
                .await?;
        }
    }
    let xhr = headers
        .get("x-requested-with")
        .is_some_and(|v| v.as_bytes() == b"XMLHttpRequest");
    let mut response = if xhr {
        json_response(json!({"redirect_url": redirect_url}))
    } else {
        let host = headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("localhost");
        (
            StatusCode::FOUND,
            [(header::LOCATION, format!("http://{host}{redirect_url}"))],
        )
            .into_response()
    };
    if let Some(session) = &incoming.session {
        response.headers_mut().insert(
            "x-discourse-username",
            HeaderValue::from_str(&session.user.username)?,
        );
    }
    if incoming.had_token_cookie {
        let cookie = current::delete_auth_cookie(&settings)?;
        response
            .headers_mut()
            .append(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
    }
    Ok(response)
}

/// GET /session/current(.json): anonymous is an empty 404.
pub async fn current(
    State(state): State<AppState>,
    axum::Extension(incoming): axum::Extension<Incoming>,
) -> Result<Response, AppError> {
    let Some(session) = &incoming.session else {
        return Ok((
            StatusCode::NOT_FOUND,
            [
                (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache, no-store"),
            ],
            "",
        )
            .into_response());
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let doc = crate::current_user::serialize(&mut conn, &state, &settings, session).await?;
    let mut response = json_response(json!({"current_user": doc}));
    response.headers_mut().insert(
        "x-discourse-username",
        HeaderValue::from_str(&session.user.username)?,
    );
    Ok(response)
}
