//! Port of SessionController#passkey_challenge and #passkey_login: GET
//! /session/passkey/challenge stages a webauthn challenge, POST
//! /session/passkey/auth logs on the passkey's owner.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::search::Peer;
use super::session::{bad_csrf, csrf_ok, json_response};
use crate::params;
use crate::session::current::{AuthGuardian, SessionUser};
use crate::session::forum::load_forum_session;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// GET /session/passkey/challenge(.json):
/// `DiscourseWebauthn.stage_challenge(current_user, server_session)`.
pub async fn challenge(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if let Some(forbidden) = super::session::check_local_login_allowed(&state, &settings)? {
        return Ok(forbidden);
    }
    let mut session = load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let challenge =
        crate::webauthn::stage_challenge(&mut conn, &session_id, guardian.user_id()).await?;
    let cookie = session.set_cookie(&state, &settings)?;
    super::accounts::with_session_cookie(
        Json(json!({ "challenge": challenge })).into_response(),
        cookie,
    )
}

/// POST /session/passkey/auth(.json)
pub async fn login(
    State(state): State<AppState>,
    Peer(peer): Peer,
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
    if let Some(forbidden) = super::session::check_local_login_allowed(&state, &settings)? {
        return Ok(forbidden);
    }
    if !settings.get("enable_passkeys")?.truthy() {
        return Ok(super::topics::not_found_response(&state));
    }
    // params.require(:publicKeyCredential)
    let credential = p.get("publicKeyCredential");
    let present = match credential {
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Object(m)) => !m.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Null) | None => false,
        Some(_) => true,
    };
    if !present {
        return Ok(super::accounts::param_missing("publicKeyCredential"));
    }
    let rp = crate::webauthn::RelyingParty::new(&state.config, &settings)?;
    let mut session = load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let key = crate::webauthn::authenticate(
        &mut conn,
        &rp,
        &session_id,
        None,
        credential,
        &[crate::webauthn::FIRST_FACTOR],
    )
    .await?;
    let key = match key {
        Ok(key) => key,
        Err(e) => {
            // render_json_error(err.message, status: 401)
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
    };
    // User.where(id: security_key.user_id, active: true).first
    let user = SessionUser::load(&mut conn, key.user_id)
        .await?
        .filter(|u| u.active)
        .ok_or(Unsupported("a passkey login for an inactive user"))?;
    // login_not_approved_for?(user)
    if settings.get("must_approve_users")?.truthy() && !user.admin {
        let approved: bool = sqlx::query_scalar("SELECT approved FROM users WHERE id = $1")
            .bind(user.id)
            .fetch_one(&mut *conn)
            .await?;
        if !approved {
            return Ok(json_response(json!({
                "error": state.i18n.t("login.not_approved").unwrap_or_default()
            })));
        }
    }
    // login_error_check(user)
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
    let email: Option<String> =
        sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"")
            .bind(user.id)
            .fetch_optional(&mut *conn)
            .await?;
    if !super::session::email_confirmed(&mut conn, user.id, email.as_deref()).await? {
        return Ok(super::session::not_activated(&state, email.as_deref()));
    }
    let timezone = params::string(&p, "timezone");
    let response = super::session::complete_login(
        &state,
        &settings,
        conn,
        &headers,
        peer,
        &uri,
        user,
        timezone.as_deref(),
        true,
    )
    .await?;
    let cookie = session.set_cookie(&state, &settings)?;
    super::accounts::with_session_cookie(response, cookie)
}
