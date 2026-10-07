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
            html.contains(r#"href="/t/welcome-to-discourse/5/1""#),
            "{path}"
        );
        assert!(
            html.contains(
                r#"Welcome to Discourse! <img width="20" height="20" src='/images/emoji/twitter/wave.png?v=15' title='wave' alt='wave' class='emoji'>"#
            ),
            "emoji title"
        );
        assert!(html.contains(r#"<span class="badge-category__name">General</span>"#));
        assert!(html.contains(
            r#"<a href='/tag/howto/1'  data-tag-name=howto class='discourse-tag simple'>howto</a>"#
        ));
        assert!(html.contains(r#"<table class="topic-list">"#));
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
    assert!(html.contains(r#"<span class="badge-category__name">Site Feedback</span>"#));
    assert!(html.contains(r#"<span class="topic-status --closed""#));
    assert!(html.contains(r#"id="post_1""#));
    assert!(html.contains(r#"<div class="cooked"><p>A pinned, closed topic in Site Feedback"#));
    // The close is a small action: its icon, the actor, what they did.
    assert!(html.contains(r#"class="small-action onscreen-post""#));
    assert!(html.contains(r#"<p aria-hidden="true">Closed <span class="relative-date""#));
    assert!(html.contains(r#"data-user-card="admin" href="/u/admin""#));
}

#[tokio::test]
async fn topic_page_shows_subcategory_breadcrumbs_and_likes() {
    let db = TestDb::new().await;
    let (_, _, html) = get(&db.pool, "/t/parity-fixture-tagged-in-a-subcategory/38").await;
    // The subcategory's badge, marked with its parent.
    assert!(html.contains(
        r#"href="/c/general/sub-general/34"><span data-category-id="34" data-parent-category-id="4" data-drop-close="true" class="badge-category --has-parent --style-square">"#
    ));
    assert!(html.contains(
        r#"<li><a href='/tag/howto/1'  data-tag-name=howto class='discourse-tag simple'>howto</a>"#
    ));

    let (_, _, html) = get(&db.pool, "/t/parity-fixture-liked-and-archived/41").await;
    // The like count, and the like button an anonymous reader may press
    // (it asks them to log in), disabled in an archived topic.
    assert!(
        html.contains(
            r#"aria-label="1 person liked this post." class="btn btn-flat no-text post-action-menu__like-count like-count button-count highlight-action regular-likes btn-flat""#
        ),
        "{html}"
    );
    assert!(html.contains(r#"title="like this post" type="button" disabled>"#));
}

#[tokio::test]
async fn html_requests_without_a_slug_redirect() {
    let db = TestDb::new().await;
    let (status, _, _) = get(&db.pool, "/t/35").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    // A browser gets the not-found page (tests/not_found_page.rs).
    let (status, _, body) = get(&db.pool, "/t/999999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains(r#"<div class="page-not-found">"#));
}

#[tokio::test]
async fn stylesheet_is_served() {
    let db = TestDb::new().await;
    let (status, content_type, css) = get(&db.pool, "/assets/site.css").await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type.starts_with("text/css"));
    assert!(css.contains("topic-pagination"));
    let (status, content_type, css) = get(&db.pool, "/assets/discourse.css").await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type.starts_with("text/css"));
    assert!(css.contains(".topic-list .posters"));
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

/// The site's color schemes as stylesheets, each custom property as Rails
/// compiled it on the reference: the default theme has no scheme of its
/// own (the base light palette) and dark mode uses the Dark scheme.
#[tokio::test]
async fn color_definitions_are_the_sites_schemes() {
    use discourse_rs::stylesheet::color_definitions::properties;
    let db = TestDb::new().await;
    for (path, rails) in [
        (
            "/assets/color_definitions_light.css",
            include_str!("../parity/stylesheets/color_definitions_light-default.css"),
        ),
        (
            "/assets/color_definitions_dark.css",
            include_str!("../parity/stylesheets/color_definitions_dark-13.css"),
        ),
    ] {
        let (status, content_type, css) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::OK, "{path}: {css}");
        assert_eq!(content_type, "text/css; charset=utf-8");
        let rails = properties(rails);
        for (name, value) in properties(&css) {
            if name == "--topic-timeline-handle-color" {
                continue;
            }
            let theirs = rails
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.as_str());
            assert_eq!(theirs, Some(value.as_str()), "{path} {name}");
        }
    }
}

/// Around the posts: the topic map in the first post and at the bottom,
/// the timeline, an anonymous reader's reply and the suggested topics.
#[tokio::test]
async fn topic_page_has_its_map_timeline_footer_and_suggestions() {
    let db = TestDb::new().await;
    let (_, _, html) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35").await;
    let first = html.find(r#"id="post_1""#).unwrap();
    let second = html.find(r#"id="post_2""#).unwrap();
    let op_map = html
        .find(r#"<div class="post__topic-map topic-map --op">"#)
        .expect("the first post's topic map");
    assert!(first < op_map && op_map < second);
    assert!(html.contains(r#"<div class="topic-map --bottom">"#));
    assert!(html.contains(r#"<div class="topic-map__users-list --users-summary">"#));
    assert!(html.contains(r#"<span class="post-count">2</span>"#));
    // The timeline at the first of five posts.
    assert!(html.contains(r#"<div class="with-timeline topic-navigation">"#));
    assert!(html.contains(r#"<div class="timeline-replies">1 / 5</div>"#));
    assert!(html.contains(r#"<a class="start-date" href="/t/parity-fixture-replies-and-posters/35/1" title="Jump to the first post">"#));
    assert!(html.contains(
        r#"<a class="now-date" href="/t/parity-fixture-replies-and-posters/35/5"><span><span class="relative-date" title="Jump to the last post""#
    ));
    // The footer's reply asks an anonymous reader to log in.
    assert!(html.contains(
        r#"<div id="topic-footer-buttons" role="region"><div class="topic-footer-main-buttons"><button class="btn btn-icon-text btn-primary" data-login-url="/login" type="button">"#
    ));
    // The suggested topics, and where to read more.
    assert!(html.contains(r#"<h3 class="more-topics__list-title" id="suggested-topics-title">New &amp; Unread Topics</h3>"#));
    assert!(html.contains(r#"<thead class="topic-list-header --has-tabs">"#));
    assert!(
        !html.contains(r#"<td class="posters topic-list-data">"#),
        "no posters column"
    );
    assert!(html.contains(r#"<h3 class="more-topics__browse-more">Want to read more? Browse other topics in <a class="badge-category__wrapper ""#));
    assert!(html.contains(r#"<script src="/assets/topic.js"></script>"#));
}
