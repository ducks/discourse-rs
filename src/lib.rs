pub mod color_scheme;
pub mod config;
pub mod parity;
pub mod routes;
pub mod ruby;
pub mod schema;
pub mod site_icons;
pub mod site_settings;
pub mod url;

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sqlx::PgPool;
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::site_settings::Definitions;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Config,
    pub site_setting_defs: Arc<Definitions>,
}

pub fn app(state: AppState) -> Router {
    routes::router()
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

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
