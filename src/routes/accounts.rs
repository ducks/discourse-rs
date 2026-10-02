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
