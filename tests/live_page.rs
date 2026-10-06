//! The topic page's live updates for htmx: GET /t/:id/live streams each
//! post change as HTML for that viewer, and the page wires it up.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, next_sse_event, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use discourse_rs::site_settings::SiteSettings;
use http_body_util::BodyExt;
use tower::ServiceExt;

/// The public fixture topic: (topic id, a reply's id and post number).
const TOPIC: i32 = 35;

async fn a_reply(st: &AppState) -> (i32, i32) {
    sqlx::query_as(
        "SELECT id, post_number FROM posts WHERE topic_id = $1 AND post_number > 1 \
           AND post_type = 1 AND deleted_at IS NULL ORDER BY post_number LIMIT 1",
    )
    .bind(TOPIC)
    .fetch_one(&st.pool)
    .await
    .unwrap()
}

async fn get(st: &AppState, path: &str) -> axum::response::Response {
    discourse_rs::app(st.clone())
        .oneshot(
            Request::get(path)
                .header(header::HOST, "test.localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn publish(st: &AppState, post_id: i32, kind: &str) {
    let mut conn = st.pool.acquire().await.unwrap();
    let settings = SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap();
    drop(conn);
    let host = discourse_rs::pretty_text::Host::from_state(st);
    let ctx = discourse_rs::posting::Ctx {
        host: &host,
        settings: &settings,
        config: &st.config,
        i18n: &st.i18n,
        bus: &st.bus,
    };
    let mut tx = st.pool.begin().await.unwrap();
    discourse_rs::bus::publish_post_change(&ctx, &mut tx, post_id, kind, Default::default(), true)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_topic_page_connects_to_its_live_stream() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let response = get(&st, "/t/parity-fixture-replies-and-posters/35").await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&bytes);
    assert!(html.contains("/assets/htmx.min.js"));
    assert!(html.contains(r#"<div class="post-stream" id="posts">"#));
    assert!(
        html.contains(r#"sse-connect="/live?position="#) && html.contains("&amp;topic=35"),
        "the stream starts at the page's position"
    );
    assert!(
        !html.contains("hx-post="),
        "no reply form for an anonymous reader"
    );

    let js = get(&st, "/assets/htmx-ext-sse.js").await;
    assert_eq!(js.status(), StatusCode::OK);
    assert_eq!(
        js.headers()[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn changes_arrive_as_html_for_the_viewer() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (post_id, post_number) = a_reply(&st).await;
    let from = st.bus.now().await.unwrap();
    let response = get(&st, &format!("/live?topic={TOPIC}&tail=1&position={from}")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let mut body = response.into_body();
    let mut buffer = String::new();

    // An edit replaces the post where it is.
    publish(&st, post_id, "revised").await;
    let html = next_sse_event(&mut body, &mut buffer, "post").await;
    assert!(
        html.contains(&format!(
            r#"data-post-number="{post_number}" hx-swap-oob="outerHTML:#posts > [data-post-number='{post_number}']""#
        )),
        "{html}"
    );
    assert!(
        html.contains(&format!(r#"id="post_{post_number}""#)),
        "{html}"
    );
    assert!(
        html.contains(r#"<div class="cooked">"#),
        "the cooked post: {html}"
    );

    // A new post is appended to the posts.
    publish(&st, post_id, "created").await;
    let html = next_sse_event(&mut body, &mut buffer, "post").await;
    assert!(
        html.starts_with(
            r##"<div hx-swap-oob="beforebegin:#posts > .post-stream__bottom-boundary">"##
        ),
        "{html}"
    );

    // A deleted post goes away for a reader who cannot see deleted posts.
    sqlx::query("UPDATE posts SET deleted_at = now() WHERE id = $1")
        .bind(post_id)
        .execute(&db.pool)
        .await
        .unwrap();
    publish(&st, post_id, "deleted").await;
    let html = next_sse_event(&mut body, &mut buffer, "post").await;
    assert_eq!(
        html,
        format!(r#"<div hx-swap-oob="delete:#posts > [data-post-number='{post_number}']"></div>"#)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_topic_the_viewer_cannot_see_has_no_stream() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let category: i32 = sqlx::query_scalar("SELECT category_id FROM topics WHERE id = $1")
        .bind(TOPIC)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE categories SET read_restricted = TRUE WHERE id = $1")
        .bind(category)
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        get(&st, &format!("/live?topic={TOPIC}")).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&st, "/live?topic=nope").await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_latest_list_learns_of_new_and_updated_topics() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let response = get(&st, "/latest").await;
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&bytes);
    assert!(html.contains(r#"<div id="list-updates""#));
    assert!(html.contains("&amp;filter=latest&amp;since="), "{html}");
    assert!(
        !html.contains("unread-count"),
        "no counts for an anonymous reader"
    );

    let since = discourse_rs::clock::now().timestamp_millis();
    let from = st.bus.now().await.unwrap();
    let response = get(
        &st,
        &format!("/live?filter=latest&since={since}&position={from}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let mut buffer = String::new();
    // Nothing new yet: an empty banner.
    assert_eq!(
        next_sse_event(&mut body, &mut buffer, "list").await,
        r#"<div id="list-updates" class="list-updates" hx-swap-oob="true"></div>"#
    );

    // A topic bumped after the page, and the message that says so.
    sqlx::query("UPDATE topics SET bumped_at = now() + interval '1 second' WHERE id = $1")
        .bind(TOPIC)
        .execute(&db.pool)
        .await
        .unwrap();
    let mut conn = db.pool.acquire().await.unwrap();
    let settings = SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap();
    drop(conn);
    let mut tx = db.pool.begin().await.unwrap();
    discourse_rs::topic_tracking_state::publish_latest(&st.bus, &settings, &mut tx, TOPIC)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let html = next_sse_event(&mut body, &mut buffer, "list").await;
    assert!(html.contains("1 new or updated topic. Show"), "{html}");
}

/// An earlier page of a topic hears its posts change, but new posts are
/// left to the last page.
#[tokio::test(flavor = "multi_thread")]
async fn earlier_pages_change_posts_without_appending() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (post_id, post_number) = a_reply(&st).await;
    let from = st.bus.now().await.unwrap();
    let response = get(&st, &format!("/live?topic={TOPIC}&position={from}")).await;
    let mut body = response.into_body();
    let mut buffer = String::new();

    publish(&st, post_id, "created").await;
    publish(&st, post_id, "revised").await;
    // The first post event is the edit: the new post was not sent.
    let html = next_sse_event(&mut body, &mut buffer, "post").await;
    assert!(
        html.contains(&format!(
            r#"hx-swap-oob="outerHTML:#posts > [data-post-number='{post_number}']""#
        )),
        "{html}"
    );
}
