mod list;
mod login_required;
mod search;
mod site;
mod srv;
mod tags;
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

pub fn router(state: &AppState) -> Router<AppState> {
    let config = &state.config;
    let public = &config.public_dir;
    // Site images first (a backup's or an install's public/), then the
    // stock ones from a Discourse checkout.
    let images = match &config.discourse_src {
        Some(src) => {
            ServeDir::new(public.join("images")).fallback(ServeDir::new(src.join("public/images")))
        }
        None => ServeDir::new(public.join("images")).fallback(ServeDir::new(public.join("images"))),
    };
    let mut router = Router::new()
        .route("/srv/status", get(srv::status))
        .route("/", get(list::latest))
        .route("/latest", get(list::latest))
        .route("/latest.json", get(list::latest_json))
        .route("/top", get(list::top))
        .route("/top.json", get(list::top_json))
        .route("/top/{period}", get(list::top_period_redirect))
        .route("/hot", get(list::hot))
        .route("/hot.json", get(list::hot_json))
        .route("/t/{id}", get(topics::show_by_id))
        .route("/t/{slug}/{id}", get(topics::show_with_slug))
        .route("/t/{slug}/{id}/{post_number}", get(topics::show_post))
        .route("/c/{*path}", get(list::category))
        .route("/search", get(search::show))
        .route("/search.json", get(search::show_json))
        .route("/search/query", get(search::query))
        .route("/search/query.json", get(search::query))
        .route("/tag/{*path}", get(tags::show))
        .route("/tags", get(tags::index))
        .route("/tags.json", get(tags::index_json))
        .route("/tags/c/{*path}", get(tags::show_in_category))
        .route("/categories", get(list::categories))
        .route("/categories.json", get(list::categories_json))
        .route("/site", get(site::site))
        .route("/site.json", get(site::site))
        .route("/site/basic-info", get(site::basic_info))
        .route("/site/basic-info.json", get(site::basic_info))
        .route("/assets/site.css", get(site_css))
        .nest_service("/uploads", ServeDir::new(public.join("uploads")));
    if let Some(emoji) = config.emoji_dir() {
        router = router.nest_service("/images/emoji", ServeDir::new(emoji));
    }
    router
        .nest_service("/images", images)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            login_required::gate,
        ))
}
