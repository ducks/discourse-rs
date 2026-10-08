pub mod accounts;
pub mod admin_site_settings;
pub mod admin_user_show;
pub mod admin_users;
pub mod avatar;
pub mod bookmark_manager;
pub mod bookmarks;
pub mod bus;
pub mod categories;
pub mod category;
pub mod category_badge;
pub mod category_list;
pub mod clock;
pub mod color_scheme;
pub mod composer_view;
pub mod config;
pub mod current_user;
pub mod discourse_diff;
pub mod drafts;
pub mod email;
pub use discourse_markdown::emoji;
pub mod excerpt;
pub mod file_store;
pub mod flags;
pub mod groups;
pub mod guardian;
pub mod html;
pub mod html_prettify;
pub mod i18n;
pub mod images;
pub mod jobs;
pub mod letter_avatar;
pub mod likes;
pub mod modifications;
pub mod not_found_page;
pub mod notifications;
pub mod optimized_images;
pub mod owned_schema;
pub mod params;
pub mod parity;
pub mod pm_lists;
pub mod post_actions;
pub mod post_destroyer;
pub mod post_view;
pub mod posting;
pub mod pretty_text;
pub mod promotion;
pub mod read_tracking;
pub mod review;
pub mod review_list;
pub mod reviewable_user;
pub mod reviewables;
pub mod roles;
pub mod routes;
pub mod ruby;
pub mod schema;
pub mod search;
pub mod second_factor;
pub mod session;
pub mod setting_enums;
pub mod sidebar;
pub mod signup;
pub mod site;
pub mod site_icons;
pub mod site_setting_update;
pub mod site_settings;
pub mod staff_action_logs;
pub mod stylesheet;
pub mod svg_sprite;
pub mod system_message;
pub mod tags;
pub mod topic_guardian;
pub mod topic_list;
pub mod topic_list_view;
pub mod topic_query;
pub mod topic_status;
pub mod topic_tracking_report;
pub mod topic_tracking_state;
pub mod topic_view;
pub mod upload_references;
pub mod uploads;
pub mod url;
pub mod user_penalties;
pub mod user_private;
pub mod user_updater;
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
    /// Live updates to browsers (MessageBus's role).
    pub bus: pg_bus::Bus,
}

pub fn app(state: AppState) -> Router {
    // The method override and the request format must run before routing
    // (a POST becomes a DELETE, an Accept-led JSON request its `.json`
    // route), and the session is resolved before anything else, so they
    // wrap the routed app from the outside.
    let routed = routes::router(&state).with_state(state.clone());
    Router::new()
        .fallback_service(routed)
        .layer(axum::middleware::from_fn(routes::method_override))
        .layer(axum::middleware::from_fn(routes::request_format))
        .layer(axum::middleware::from_fn_with_state(
            state,
            session::current::layer,
        ))
        .layer(TraceLayer::new_for_http())
}

pub use discourse_markdown::Unsupported;

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
