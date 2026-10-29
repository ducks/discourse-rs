//! discourse-captcha's engine, mounted at /captcha: HcaptchaController
//! and RecaptchaController#create (POST /captcha/hcaptcha/create and
//! /captcha/recaptcha/create), which keep the widget's token in the
//! server session for users#create to verify.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::plugins::captcha::{self, Created, Provider};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// POST /captcha/hcaptcha/create(.json)
pub async fn hcaptcha(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    create(&state, Provider::Hcaptcha, &headers, &uri, &body).await
}

/// POST /captcha/recaptcha/create(.json)
pub async fn recaptcha(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    create(&state, Provider::Recaptcha, &headers, &uri, &body).await
}

/// CaptchaController#create: the CSRF check, requires_plugin and
/// `ensure_config` (both 404s, then the token's InvalidParameters), the
/// token kept for TOKEN_TTL. No login required, even on a login required
/// site (`skip_before_action :redirect_to_login_if_required`).
async fn create(
    state: &AppState,
    provider: Provider,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), headers, body);
    if !csrf_ok(state, headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let mut session = crate::session::forum::load_forum_session(state, headers);
    let session_id = session.server_session_id();
    let token = params::string(&p, "token");
    match captcha::create(
        &mut conn,
        &settings,
        &session_id,
        provider,
        token.as_deref(),
    )
    .await?
    {
        Created::NotFound => Ok(super::topics::not_found_response(state)),
        Created::MissingToken => Ok(super::search::invalid_parameters(state, "token")),
        Created::Kept => super::accounts::with_session_cookie(
            Json(json!({"success": "OK"})).into_response(),
            session.set_cookie(state, &settings)?,
        ),
    }
}
