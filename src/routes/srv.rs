//! Port of app/controllers/forums_controller.rb.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::AppState;

#[derive(Deserialize)]
pub struct StatusParams {
    cluster: Option<String>,
}

/// GET /srv/status. `params[:cluster]` is truthy in Ruby even when empty, so
/// any present `cluster` param triggers the check.
pub async fn status(State(state): State<AppState>, Query(params): Query<StatusParams>) -> Response {
    if let Some(cluster) = params.cluster {
        match state.config.globals.cluster_name() {
            None => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "cluster name not configured",
                )
                    .into_response();
            }
            Some(name) if name != cluster => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "cluster name does not match",
                )
                    .into_response();
            }
            Some(_) => {}
        }
    }

    "ok".into_response()
}
