//! discourse-lazy-videos on the server-rendered pages: the client's script
//! while lazy_videos_enabled, the per-provider settings and the play_video
//! label its decorator reads, and lazy-videos.scss. The oneboxes are in
//! tests/onebox_engines.rs.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

async fn get(state: &AppState, path: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .uri(path)
        .header(header::HOST, "test.localhost")
        .body(Body::empty())
        .unwrap();
    let response = discourse_rs::app(state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The page's #cooked-config.
fn cooked_config(page: &str) -> Value {
    let open = "<script type=\"application/json\" id=\"cooked-config\">";
    let from = page.find(open).expect("the cooked config") + open.len();
    let to = from + page[from..].find("</script>").unwrap();
    serde_json::from_str(&page[from..to]).unwrap()
}

#[tokio::test]
async fn the_client_loads_while_the_plugin_is_on() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (status, page) = get(&st, "/latest").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        page.contains("/assets/lazy-videos.js"),
        "on by default: {page}"
    );
    // config/settings.yml's defaults, and the thumbnail's label.
    let config_json = cooked_config(&page);
    assert_eq!(config_json["settings"]["lazy_youtube_enabled"], true);
    assert_eq!(config_json["settings"]["lazy_vimeo_enabled"], true);
    assert_eq!(config_json["settings"]["lazy_tiktok_enabled"], false);
    assert_eq!(
        config_json["strings"]["lazy_videos.play_video"],
        "Play %{title}"
    );

    let (status, js) = get(&st, "/assets/lazy-videos.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(js.contains("lazy-video-wrapper"));
    let (_, css) = get(&st, "/assets/discourse.css").await;
    assert!(css.contains(".lazy-video-container .icon.youtube-icon"));

    set_setting(&db.pool, "lazy_videos_enabled", 5, "f").await;
    set_setting(&db.pool, "lazy_vimeo_enabled", 5, "f").await;
    set_setting(&db.pool, "lazy_tiktok_enabled", 5, "t").await;
    // A new process: the anonymous page cache holds the last page.
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (_, page) = get(&st, "/latest").await;
    assert!(!page.contains("/assets/lazy-videos.js"), "off: {page}");
    let config_json = cooked_config(&page);
    assert_eq!(config_json["settings"]["lazy_vimeo_enabled"], false);
    assert_eq!(config_json["settings"]["lazy_tiktok_enabled"], true);
}
