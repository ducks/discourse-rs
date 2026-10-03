pub mod accounts;
pub mod avatar;
pub mod bookmark_manager;
pub mod bookmarks;
pub mod categories;
pub mod category;
pub mod category_list;
pub mod clock;
pub mod color_scheme;
pub mod config;
pub mod current_user;
pub mod discourse_diff;
pub mod drafts;
pub mod email;
pub mod emoji;
pub mod excerpt;
pub mod flags;
pub mod groups;
pub mod guardian;
pub mod html;
pub mod i18n;
pub mod jobs;
pub mod letter_avatar;
pub mod likes;
pub mod modifications;
pub mod notifications;
pub mod owned_schema;
pub mod params;
pub mod parity;
pub mod pm_lists;
pub mod post_actions;
pub mod post_destroyer;
pub mod posting;
pub mod pretty_text;
pub mod read_tracking;
pub mod review;
pub mod reviewables;
pub mod routes;
pub mod ruby;
pub mod schema;
pub mod search;
pub mod session;
pub mod signup;
pub mod site;
pub mod site_icons;
pub mod site_setting_update;
pub mod site_settings;
pub mod system_message;
pub mod tags;
pub mod topic_guardian;
pub mod topic_list;
pub mod topic_query;
pub mod topic_status;
pub mod topic_view;
pub mod uploads;
pub mod url;
pub mod user_penalties;
pub mod user_private;
pub mod users;

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sqlx::PgPool;
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::i18n::I18n;
use crate::site_settings::Definitions;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Config,
    pub site_setting_defs: Arc<Definitions>,
    pub i18n: Arc<I18n>,
    /// `SearchLog.log`'s per-IP dedupe window.
    pub search_log_cache: Arc<search::SearchLogCache>,
    /// secret_key_base and the cookie codec derived from it.
    pub keys: Arc<session::current::Keys>,
    /// Where outgoing mail goes (SMTP, sendmail, or memory in tests).
    pub mailer: email::Mailer,
}

pub fn app(state: AppState) -> Router {
    // The method override must run before routing (a POST becomes a
    // DELETE), and the session is resolved before anything else, so both
    // wrap the routed app from the outside.
    let routed = routes::router(&state).with_state(state.clone());
    Router::new()
        .fallback_service(routed)
        .layer(axum::middleware::from_fn(routes::method_override))
        .layer(axum::middleware::from_fn_with_state(
            state,
            session::current::layer,
        ))
        .layer(TraceLayer::new_for_http())
}

/// A Discourse behavior this port doesn't cover yet, hit at runtime. Failing
/// loudly beats serving a plausible but wrong response in a parity port.
#[derive(Debug)]
pub struct Unsupported(pub &'static str);

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "not ported yet: {}", self.0)
    }
}

impl std::error::Error for Unsupported {}

/// Any failure a handler can't turn into a proper response: logged in full,
/// returned as a bare 500.
#[derive(Debug)]
pub struct AppError(Box<dyn std::error::Error + Send + Sync>);

impl<E> From<E> for AppError
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn from(e: E) -> Self {
        AppError(Box::new(e))
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl AppError {
    /// Whether the failure is something not ported (`Unsupported`), as
    /// opposed to an error.
    pub fn is_unsupported(&self) -> bool {
        self.0.downcast_ref::<Unsupported>().is_some()
            || self.0.to_string().starts_with("not ported yet:")
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!(error = %self.0, "request failed");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    }
}
