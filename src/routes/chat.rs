//! The chat plugin's API (Chat::Api::*): what Chat::BaseController
//! guards every action with, and the actions ported so far.

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};

use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// Chat::BaseController's before actions: requires_plugin (404),
/// ensure_logged_in, ensure_can_chat. None when the request may go on.
async fn guard(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
) -> Result<Option<Response>, AppError> {
    if !crate::plugins::chat::enabled(settings)? {
        return Ok(Some(super::topics::not_found_response(state)));
    }
    if guardian.is_anonymous() {
        return Ok(Some(super::login_required::not_logged_in(state)));
    }
    if !crate::plugins::chat::can_chat(conn, settings, guardian).await? {
        return Ok(Some(super::search::invalid_access(state)));
    }
    Ok(None)
}

/// GET /chat/api/me/channels: Chat::Api::CurrentUserChannelsController#index.
pub async fn me_channels(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if let Some(refused) = guard(&state, &mut conn, &settings, &guardian).await? {
        return Ok(refused);
    }
    let mut cx = crate::plugins::chat::channels::Context {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        base_path: state.config.globals.relative_url_root(),
    };
    Ok(Json(cx.me_channels().await?).into_response())
}
