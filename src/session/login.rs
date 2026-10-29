//! `CurrentUser#log_on_user(user, opts, replay_anonymous_action:)` for
//! logins that are not a password's: DiscourseConnect's and external
//! logins'. A new auth token (marked when OAuth authenticated it) and
//! its cookie, the first admin's moderation, the session count kept, and
//! `user.logged_in`'s web hooks.

use axum::http::{HeaderMap, header};
use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// The `_t` cookie for the logged on user.
#[allow(clippy::too_many_arguments)]
pub async fn log_on_user(
    state: &AppState,
    settings: &SiteSettings,
    conn: &mut PgConnection,
    headers: &HeaderMap,
    peer: Option<std::net::SocketAddr>,
    path: &str,
    user_id: i32,
    authenticated_with_oauth: bool,
) -> Result<String, AppError> {
    // AnonymousAction.consume: not ported.
    if crate::session::current::cookie(headers, "anonymous_action").is_some() {
        return Err(Unsupported("replaying an anonymous action after a login").into());
    }
    let ip = crate::remote_ip::remote_ip(headers, peer)?;
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let (_, unhashed) = crate::session::token::generate_with(
        &mut *conn,
        user_id,
        &state.keys.secret_key_base,
        user_agent.as_deref(),
        &ip,
        Some(path),
        authenticated_with_oauth,
    )
    .await?;
    let staged: bool = sqlx::query_scalar("SELECT staged FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    if staged {
        return Err(Unsupported("unstaging a user at login").into());
    }
    crate::roles::bootstrap_first_admin(&mut *conn, &state.i18n, &state.config.globals, user_id)
        .await?;
    crate::session::token::enforce_session_count_limit(
        &mut *conn,
        user_id,
        settings.get("maximum_session_age")?.to_i(),
    )
    .await?;
    // user.logged_in: user_logged_in's web hooks.
    crate::web_hooks::enqueue_user_hooks(
        &mut *conn,
        &crate::web_hooks::Payloads {
            settings,
            i18n: &state.i18n,
            config: &state.config,
        },
        "user_logged_in",
        user_id,
    )
    .await?;
    // ... and user_first_logged_in, to discourse-narrative-bot.
    crate::plugins::narrative_bot::user_logged_in(&mut *conn, settings, user_id).await?;
    let user = crate::session::current::SessionUser::load(&mut *conn, user_id)
        .await?
        .ok_or(Unsupported("logging on a user that vanished"))?;
    crate::session::current::auth_cookie(&state.keys, settings, &user, &unhashed)
}
