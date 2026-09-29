pub mod config;
pub mod parity;
pub mod routes;
pub mod schema;

use axum::Router;
use sqlx::PgPool;
use tower_http::trace::TraceLayer;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Config,
}

pub fn app(state: AppState) -> Router {
    routes::router()
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}
