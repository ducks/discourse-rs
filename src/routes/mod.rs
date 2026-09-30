mod list;
mod site;
mod srv;
mod topics;

use axum::Router;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use tower_http::services::ServeDir;

use crate::AppState;

const SITE_CSS: &str = include_str!("../../static/site.css");

async fn site_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        SITE_CSS,
    )
}

pub fn router(public_dir: &std::path::Path) -> Router<AppState> {
    Router::new()
        .route("/srv/status", get(srv::status))
        .route("/", get(list::latest))
        .route("/latest", get(list::latest))
        .route("/latest.json", get(list::latest_json))
        .route("/t/{id}", get(topics::show_by_id))
        .route("/t/{slug}/{id}", get(topics::show_with_slug))
        .route("/t/{slug}/{id}/{post_number}", get(topics::show_post))
        .route("/site", get(site::site))
        .route("/site.json", get(site::site))
        .route("/site/basic-info", get(site::basic_info))
        .route("/site/basic-info.json", get(site::basic_info))
        .route("/assets/site.css", get(site_css))
        .nest_service("/images", ServeDir::new(public_dir.join("images")))
        .nest_service("/uploads", ServeDir::new(public_dir.join("uploads")))
}
