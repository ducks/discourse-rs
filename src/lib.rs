pub mod avatar;
pub mod categories;
pub mod color_scheme;
pub mod config;
pub mod emoji;
pub mod groups;
pub mod guardian;
pub mod html;
pub mod i18n;
pub mod letter_avatar;
pub mod parity;
pub mod routes;
pub mod ruby;
pub mod schema;
pub mod site;
pub mod site_icons;
pub mod site_settings;
pub mod topic_list;
pub mod topic_query;
pub mod topic_view;
pub mod url;

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
}

pub fn app(state: AppState) -> Router {
    routes::router(&state.config.clone())
        .with_state(state)
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

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!(error = %self.0, "request failed");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    }
}
