mod srv;

use axum::Router;
use axum::routing::get;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/srv/status", get(srv::status))
}
