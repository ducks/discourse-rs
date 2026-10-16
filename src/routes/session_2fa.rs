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
    if !wants_json(&headers, &uri) {
        return Err(Unsupported("the 2FA page's HTML (its preloaded 2fa_challenge_data)").into());
    }
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
            if enabled.security_keys || enabled.passkeys {
                return Err(
                    Unsupported("security keys and passkeys on the 2FA page (webauthn)").into(),
                );
            }
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
            json.insert("security_keys_enabled".into(), json!(false));
            json.insert("passkeys_enabled".into(), json!(false));
            if let Some(description) = challenge.get("description") {
                json.insert("description".into(), description.clone());
            }
            (Value::Object(json), StatusCode::OK)
        }
        Err(bad) => (json!({ "error": bad.message(&state.i18n) }), bad.status),
    };
    let cookie = session.set_cookie(&state, &settings)?;
    super::accounts::with_session_cookie((status, Json(body)).into_response(), cookie)
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
        let method_param = text("second_factor_method");
        let token = text("second_factor_token");
        let outcome = crate::second_factor::authenticate(
            &mut conn,
            &state.i18n,
            user_id,
            &enabled,
            method_param.as_deref(),
            token.as_deref(),
            crate::clock::now().timestamp(),
            crate::second_factor::backup_code_iterations(state.config.rails_env),
        )
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
