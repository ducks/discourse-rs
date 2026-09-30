mod list;
mod site;
mod srv;
mod topics;

use axum::Router;
use axum::routing::get;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/srv/status", get(srv::status))
        .route("/latest", get(list::latest))
        .route("/latest.json", get(list::latest))
        .route("/t/{id}", get(topics::show_by_id))
        .route("/t/{slug}/{id}", get(topics::show_with_slug))
        .route("/t/{slug}/{id}/{post_number}", get(topics::show_post))
        .route("/site", get(site::site))
        .route("/site.json", get(site::site))
        .route("/site/basic-info", get(site::basic_info))
        .route("/site/basic-info.json", get(site::basic_info))
}
