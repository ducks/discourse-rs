//! The server-rendered pages for anonymous readers.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;

async fn get(pool: &PgPool, path: &str) -> (StatusCode, String, String) {
    let app = discourse_rs::app(state(pool.clone(), config(RailsEnv::Test, &[])).await);
    let response = app
        .oneshot(
            Request::get(path)
                .header(header::HOST, "test.localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        content_type,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

#[tokio::test]
async fn home_and_latest_render_the_topic_list() {
    let db = TestDb::new().await;
    for path in ["/", "/latest"] {
        let (status, content_type, html) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(
            content_type.starts_with("text/html"),
            "{path}: {content_type}"
        );
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<title>Latest topics - Discourse</title>"));
        assert!(
            html.contains(r#"href="/t/welcome-to-discourse/5""#),
            "{path}"
        );
        assert!(html.contains("Welcome to Discourse! 👋"), "emoji title");
        assert!(html.contains(r#"<span class="category-name">General</span>"#));
        assert!(html.contains(r#"class="discourse-tag">howto</a>"#));
        assert!(!html.contains("Parity fixture: unlisted topic"));
        assert!(!html.contains("Parity fixture: deleted topic"));
    }
}

#[tokio::test]
async fn latest_pages_link_to_the_next_html_page() {
    let db = TestDb::new().await;
    let (_, _, html) = get(&db.pool, "/latest?per_page=2").await;
    assert!(
        html.contains(r#"href="/latest?page=1&#38;per_page=2" rel="next""#),
        "{html}"
    );
    assert!(!html.contains("no_definitions"));
    let (_, _, last) = get(&db.pool, "/latest?page=2&per_page=2").await;
    assert!(!last.contains(r#"rel="next""#));
}

#[tokio::test]
async fn topic_page_renders_posts_and_small_actions() {
    let db = TestDb::new().await;
    let (status, content_type, html) =
        get(&db.pool, "/t/parity-fixture-pinned-and-closed/37").await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type.starts_with("text/html"));
    assert!(html.contains("<title>Parity fixture: pinned and closed - Discourse</title>"));
    assert!(
        html.contains(r#"<link rel="canonical" href="http://test.localhost/t/parity-fixture-pinned-and-closed/37">"#)
    );
    assert!(html.contains(r#"<span class="category-name">Site Feedback</span>"#));
    assert!(html.contains(r#"id="post_1""#));
    assert!(html.contains("<p>A pinned, closed topic in Site Feedback"));
    assert!(html.contains(r#"<span class="action">Closed</span>"#));
    assert!(html.contains(r#"href="/u/admin">admin</a>"#));
}

#[tokio::test]
async fn topic_page_shows_subcategory_breadcrumbs_and_likes() {
    let db = TestDb::new().await;
    let (_, _, html) = get(&db.pool, "/t/parity-fixture-tagged-in-a-subcategory/38").await;
    assert!(html.contains(r#"href="/c/general/4""#), "parent crumb");
    assert!(
        html.contains(r#"href="/c/general/sub-general/34""#),
        "child crumb"
    );
    assert!(html.contains(r#"href="/tag/howto" class="discourse-tag""#));

    let (_, _, html) = get(&db.pool, "/t/parity-fixture-liked-and-archived/41").await;
    assert!(
        html.contains(r#"<div class="post-likes">&#x2764; 1</div>"#),
        "{html}"
    );
}

#[tokio::test]
async fn html_requests_without_a_slug_redirect() {
    let db = TestDb::new().await;
    let (status, _, _) = get(&db.pool, "/t/35").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    let (status, _, body) = get(&db.pool, "/t/999999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("not_found"));
}

#[tokio::test]
async fn stylesheet_is_served() {
    let db = TestDb::new().await;
    let (status, content_type, css) = get(&db.pool, "/assets/site.css").await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type.starts_with("text/css"));
    assert!(css.contains("topic-list"));
}

/// Each page says where its live updates start, as a pg-bus position, and
/// polling from there delivers what changed after the page was read.
#[tokio::test(flavor = "multi_thread")]
async fn pages_carry_where_their_live_updates_start() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let position_of = |path: &'static str| {
        let st = st.clone();
        async move {
            let response = discourse_rs::app(st)
                .oneshot(
                    Request::get(path)
                        .header(header::HOST, "test.localhost")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let body = String::from_utf8_lossy(&bytes).into_owned();
            let marker = "<meta name=\"bus-position\" content=\"";
            let start = body
                .find(marker)
                .unwrap_or_else(|| panic!("{path} has no bus position"))
                + marker.len();
            let end = start + body[start..].find('"').unwrap();
            body[start..end]
                .parse::<pg_bus::Position>()
                .unwrap_or_else(|e| panic!("{path}: {e}"))
        }
    };
    for path in [
        "/",
        "/categories",
        "/t/parity-fixture-replies-and-posters/35",
        "/u/user1",
        "/search?q=fixture",
    ] {
        position_of(path).await;
    }

    let from = position_of("/latest").await;
    let mut conn = db.pool.acquire().await.unwrap();
    let settings = discourse_rs::site_settings::SiteSettings::load(
        &mut conn,
        &st.site_setting_defs,
        &st.config.globals,
    )
    .await
    .unwrap();
    drop(conn);
    let mut tx = db.pool.begin().await.unwrap();
    discourse_rs::topic_tracking_state::publish_latest(&st.bus, &settings, &mut tx, 35)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let messages = common::bus_messages(&st, from, &["/latest"]).await;
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0].data["topic_id"], 35);
}
