//! Port of SessionController#second_factor_auth_show and
//! #second_factor_auth_perform: GET and POST /session/2fa, where the
//! acting user completes the challenge an AuthManager action staged.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::search::Peer;
use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::rate_limiter::RateLimiter;
use crate::second_factor_auth::{self as auth, BadChallenge};
use crate::session::current::AuthGuardian;
use crate::session::forum::load_forum_session;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// `params.require(key)` failing: 400 ParameterMissing.
fn param_missing(key: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "errors": [format!("param is missing or the value is empty or invalid: {key}")]
        })),
    )
        .into_response()
}

/// The request asks for JSON (respond_to's format).
fn wants_json(headers: &HeaderMap, uri: &Uri) -> bool {
    uri.path().ends_with(".json")
        || headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|a| a.contains("application/json"))
}

/// GET /session/2fa(.json)
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &[]);
    let Some(nonce) = p
        .get("nonce")
        .and_then(params::scalar)
        .filter(|n| !n.is_empty())
    else {
        return Ok(param_missing("nonce"));
    };
    let json_wanted = wants_json(&headers, &uri);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let mut session = load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let challenge = auth::find(&mut conn, &session_id, &nonce, guardian.user_id()).await?;
    let (body, status) = match challenge {
        Ok(challenge) => {
            let user_id = challenge
                .get("target_user_id")
                .and_then(Value::as_i64)
                .and_then(|n| i32::try_from(n).ok())
                .ok_or(Unsupported("a second factor challenge without its user"))?;
            let enabled =
                crate::second_factor::Enabled::load(&mut conn, &settings, user_id).await?;
            let mut json = Map::new();
            json.insert("totp_enabled".into(), json!(enabled.totp));
            json.insert("backup_enabled".into(), json!(enabled.backup));
            json.insert(
                "allowed_methods".into(),
                challenge
                    .get("allowed_methods")
                    .cloned()
                    .unwrap_or(json!([])),
            );
            if enabled.security_keys || enabled.passkeys {
                crate::webauthn::stage_challenge(&mut conn, &session_id, Some(user_id)).await?;
                json.extend(
                    crate::webauthn::allowed_credentials(
                        &mut conn,
                        &session_id,
                        user_id,
                        &enabled,
                        enabled.passkeys,
                    )
                    .await?,
                );
            }
            json.insert("security_keys_enabled".into(), json!(enabled.security_keys));
            json.insert("passkeys_enabled".into(), json!(enabled.passkeys));
            if let Some(description) = challenge.get("description") {
                json.insert("description".into(), description.clone());
            }
            (Value::Object(json), StatusCode::OK)
        }
        Err(bad) => (json!({ "error": bad.message(&state.i18n) }), bad.status),
    };
    if json_wanted {
        let cookie = session.set_cookie(&state, &settings)?;
        return super::accounts::with_session_cookie((status, Json(body)).into_response(), cookie);
    }
    // format.html: the page with 2fa_challenge_data, a 200 either way.
    drop(conn);
    page(
        &state,
        &settings,
        &headers,
        &uri,
        &guardian,
        &mut session,
        &nonce,
        body,
    )
    .await
}

#[derive(askama::Template)]
#[template(path = "second_factor_auth.html")]
struct SecondFactorAuthPage {
    site_title: String,
    lang: String,
    base_path: String,
    crawler: crate::html::Crawler,
    viewer: Option<crate::html::Viewer>,
    bus_position: String,
    chrome: crate::html::Chrome,
    /// What the page's script draws from, as JSON.
    data: String,
}

/// The 2FA page (templates/second-factor-auth.gjs with its controller's
/// choices), without the sidebar (hideApplicationSidebar).
#[allow(clippy::too_many_arguments)]
async fn page(
    state: &AppState,
    settings: &SiteSettings,
    headers: &HeaderMap,
    uri: &Uri,
    guardian: &crate::guardian::Guardian,
    session: &mut crate::session::forum::ForumSession,
    nonce: &str,
    model: Value,
) -> Result<Response, AppError> {
    use crate::second_factor::{BACKUP_CODES, PASSKEY, SECURITY_KEY, TOTP};
    use askama::Template;
    let t = |key: &str| {
        state
            .i18n
            .t(&format!("js.{key}"))
            .unwrap_or_default()
            .to_string()
    };
    // The one session cookie: its csrf token for the script, and the
    // viewer read from it as it now is.
    let csrf = crate::session::csrf::mask(&session.csrf_token())
        .ok_or(Unsupported("session csrf token not decodable"))?;
    let cookie = session.set_cookie(state, settings)?;
    let mut headers = headers.clone();
    if let Some(cookie) = &cookie {
        let pair = cookie.split(';').next().unwrap_or_default().to_string();
        let name = crate::session::current::SESSION_COOKIE;
        let others: Vec<String> = headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|h| h.to_str().ok())
            .flat_map(|h| h.split(';'))
            .map(str::trim)
            .filter(|c| !c.is_empty() && !c.starts_with(&format!("{name}=")))
            .map(str::to_string)
            .collect();
        let joined = others
            .into_iter()
            .chain(std::iter::once(pair))
            .collect::<Vec<_>>()
            .join("; ");
        headers.remove(header::COOKIE);
        headers.insert(header::COOKIE, joined.parse()?);
    }
    let vs = super::session::viewer_state(state, &headers, settings, guardian)?;
    let base_path = state.config.globals.relative_url_root().to_string();
    let mut site = crate::html::Site::from_settings(settings, &base_path)?;
    site.viewer = vs.viewer.clone();
    let mut conn = state.pool.acquire().await?;
    site.load_chrome(
        &mut conn,
        state,
        settings,
        guardian,
        crate::sidebar::Active::None,
    )
    .await?;
    site.bus_position = crate::bus::page_position(&state.bus).await?;
    drop(conn);
    site.chrome.sidebar = String::new();
    let enabled = |k: &str| model.get(k).and_then(Value::as_bool).unwrap_or(false);
    let allowed: Vec<i64> = model
        .get("allowed_methods")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default();
    // passkeysAvailable, securityKeysAvailable, totpAvailable,
    // backupCodesAvailable, in shownSecondFactorMethod's order.
    let available: Vec<i64> = [
        (PASSKEY, "passkeys_enabled"),
        (SECURITY_KEY, "security_keys_enabled"),
        (TOTP, "totp_enabled"),
        (BACKUP_CODES, "backup_enabled"),
    ]
    .into_iter()
    .filter(|(m, k)| enabled(k) && allowed.contains(m))
    .map(|(m, _)| m)
    .collect();
    let error = model
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_string);
    let data = json!({
        "model": model,
        "nonce": nonce,
        "error": error,
        "load_error": error.is_some(),
        "available": available,
        "shown": available.first().copied().unwrap_or(TOTP),
        "csrf": csrf,
        "base_path": base_path,
        "icons": {
            "key": crate::svg_sprite::raw_svg("key")?,
            "user": crate::svg_sprite::raw_svg("user")?,
        },
        "strings": {
            "order": [PASSKEY, SECURITY_KEY, TOTP, BACKUP_CODES],
            "titles": {
                "1": t("login.second_factor_title"),
                "2": t("login.second_factor_backup_title"),
                "3": t("login.second_factor_title"),
                "4": t("login.second_factor_title"),
            },
            "descriptions": {
                "1": t("login.second_factor_description"),
                "2": t("login.second_factor_backup_description"),
                "3": t("login.security_key_description"),
                "4": t("login.passkey_2fa_description"),
            },
            "toggles": {
                "1": t("login.second_factor_toggle.totp"),
                "2": t("login.second_factor_toggle.backup_code"),
                "3": t("login.second_factor_toggle.security_key"),
                "4": t("login.second_factor_toggle.passkey"),
            },
            "classes": { "1": "totp", "2": "backup-code", "3": "security-key", "4": "passkey" },
            "use_passkey": t("login.use_passkey"),
            "security_key_authenticate": t("login.security_key_authenticate"),
            "submit": t("submit"),
            "otp_label": t("d_otp.screen_reader.other").replace("%{count}", "6"),
            "redirect_after_success": t("second_factor_auth.redirect_after_success"),
            "not_allowed": t("login.security_key_not_allowed_error"),
            "generic_error": t("generic_error"),
        },
    });
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let page = SecondFactorAuthPage {
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        crawler: crate::html::Crawler::for_request(&urls, uri, None)?,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        data: data.to_string().replace("</", "<\\/"),
    };
    let response = (
        [(header::CACHE_CONTROL, "no-cache, no-store")],
        axum::response::Html(page.render().map_err(crate::html::HtmlError::from)?),
    )
        .into_response();
    let response = super::accounts::with_session_cookie(response, cookie)?;
    Ok(crate::html::with_viewer_headers(response, &vs))
}

/// POST /session/2fa(.json)
pub async fn perform(
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
    let text = |k: &str| p.get(k).and_then(params::scalar);
    let Some(nonce) = text("nonce").filter(|n| !n.is_empty()) else {
        return Ok(param_missing("nonce"));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let mut session = load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let respond = |session: &crate::session::forum::ForumSession, response: Response| {
        let cookie = session.set_cookie(&state, &settings)?;
        super::accounts::with_session_cookie(response, cookie)
    };
    let mut challenge = match auth::find(&mut conn, &session_id, &nonce, guardian.user_id()).await?
    {
        Ok(challenge) => challenge,
        Err(BadChallenge { key, status }) => {
            let bad = BadChallenge { key, status };
            let body = json!({
                "failed": "FAILED",
                "ok": false,
                "error": bad.message(&state.i18n),
                "reason": "challenge_not_found_or_expired",
            });
            return respond(&session, (status, Json(body)).into_response());
        }
    };
    let user_id = challenge
        .get("target_user_id")
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .ok_or(Unsupported("a second factor challenge without its user"))?;
    // User.find
    let username: Option<String> = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&mut *conn)
        .await?;
    let Some(username) = username else {
        return Ok(super::topics::not_found_response(&state));
    };
    let method = text("second_factor_method")
        .map(|m| crate::ruby::to_i(&m))
        .unwrap_or(0);
    let allowed = challenge
        .get("allowed_methods")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|m| m.as_i64() == Some(method)));
    if !auth::valid_method_for_user(&mut conn, &settings, user_id, method).await? || !allowed {
        return respond(&session, super::search::invalid_access(&state));
    }
    if challenge.get("successful") != Some(&Value::Bool(true)) {
        // rate_limit_second_factor!(user)
        if text("second_factor_token").is_some_and(|t| !t.trim().is_empty()) {
            let ip = crate::remote_ip::remote_ip(&headers, peer)?;
            let limiters = vec![
                RateLimiter::new(None, &format!("second-factor-min-{ip}"), 6, 60),
                RateLimiter::new(None, &format!("second-factor-min-{username}"), 6, 60),
            ];
            if let Err(refused) =
                crate::rate_limiter::perform_each(&state.pool, state.config.rate_limits, limiters)
                    .await?
            {
                return respond(&session, refused.into_response(&state.i18n));
            }
        }
        let enabled = crate::second_factor::Enabled::load(&mut conn, &settings, user_id).await?;
        let rp = crate::webauthn::RelyingParty::new(&state.config, &settings)?;
        let attempt = crate::second_factor::Attempt::new(&p, &state.config, &rp, &mut session);
        let outcome =
            crate::second_factor::authenticate(&mut conn, &state.i18n, user_id, &enabled, attempt)
                .await?;
        match outcome {
            Err(failure) => {
                // to_h.slice(:ok, :error, :reason).merge(failed_json)
                let body = json!({
                    "ok": failure["ok"],
                    "error": failure["error"],
                    "reason": failure["reason"],
                    "failed": "FAILED",
                });
                return respond(
                    &session,
                    (StatusCode::BAD_REQUEST, Json(body)).into_response(),
                );
            }
            Ok(_) => {
                challenge.insert("successful".into(), json!(true));
                let generated_at = challenge
                    .get("generated_at")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                challenge.insert("generated_at".into(), json!(generated_at + 60));
                auth::store(&mut conn, &session_id, &challenge).await?;
            }
        }
    }
    let body = json!({
        "ok": true,
        "callback_method": challenge.get("callback_method"),
        "callback_path": challenge.get("callback_path"),
        "redirect_url": challenge.get("redirect_url"),
    });
    respond(&session, Json(body).into_response())
}
