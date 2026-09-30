mod list;
mod site;
mod srv;

use axum::Router;
use axum::routing::get;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/srv/status", get(srv::status))
        .route("/latest", get(list::latest))
        .route("/latest.json", get(list::latest))
        .route("/site", get(site::site))
        .route("/site.json", get(site::site))
        .route("/site/basic-info", get(site::basic_info))
        .route("/site/basic-info.json", get(site::basic_info))
}
