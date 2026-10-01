//! Category topic lists (/c/...) and the categories index for anonymous
//! users. The documents are covered by the parity golden files; these pin
//! routing, redirects and the HTML.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

async fn get(pool: &PgPool, path: &str) -> (StatusCode, Option<String>, String) {
    let app = discourse_rs::app(state(pool.clone(), config(RailsEnv::Test, &[])));
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
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        location,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn ids(body: &str) -> Vec<i64> {
    let json: Value = serde_json::from_str(body).unwrap();
    json["topic_list"]["topics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn category_lists_include_the_definition_topic_and_subcategories() {
    let db = TestDb::new().await;
    let (status, _, body) = get(&db.pool, "/c/general/4.json").await;
    assert_eq!(status, StatusCode::OK);
    // Welcome (pinned globally, but only category pins float here), then by
    // bump: About General (3) shows, the subcategory's About (34) doesn't.
    assert_eq!(ids(&body), vec![5, 3, 35, 41, 38]);
    let json: Value = serde_json::from_str(&body).unwrap();
    // guide and howto tie on count; name breaks the tie.
    assert_eq!(json["topic_list"]["top_tags"][0]["name"], "guide");
    assert_eq!(json["topic_list"]["top_tags"][1]["name"], "howto");

    let (_, _, body) = get(&db.pool, "/c/general/sub-general/34.json").await;
    assert_eq!(ids(&body), vec![34, 38]);

    let (_, _, body) = get(&db.pool, "/c/general/4.json?per_page=2").await;
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["topic_list"]["more_topics_url"],
        "/c/general/4?page=1&per_page=2"
    );
    let (_, _, body) = get(&db.pool, "/c/general/4/l/latest.json?per_page=2").await;
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["topic_list"]["more_topics_url"],
        "/c/general/4/l/latest?page=1&per_page=2"
    );
}

#[tokio::test]
async fn category_pins_float_only_in_their_own_category() {
    let db = TestDb::new().await;
    // 37 is pinned in Site Feedback (not globally).
    let (_, _, body) = get(&db.pool, "/c/site-feedback/2.json").await;
    assert_eq!(ids(&body)[0], 37);
}

#[tokio::test]
async fn category_routes_redirect_and_404() {
    let db = TestDb::new().await;
    let (status, location, _) = get(&db.pool, "/c/wrong/4.json?page=1").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/c/general/4.json?page=1")
    );

    let (status, location, _) = get(&db.pool, "/c/4").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/c/general/4")
    );

    // Slug-only paths resolve through the slug chain.
    let (status, location, _) = get(&db.pool, "/c/general/sub-general").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/c/general/sub-general/34")
    );

    for path in ["/c/staff/3.json", "/c/nope/999.json", "/c/nope.json"] {
        let (status, _, body) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert!(!body.contains("extras"), "{path}");
    }
}

#[tokio::test]
async fn category_page_renders_heading_and_subcategories() {
    let db = TestDb::new().await;
    let (status, _, html) = get(&db.pool, "/c/general/4").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<title>General - Discourse</title>"));
    assert!(html.contains(r#"<a href="/c/general/4">General</a>"#));
    assert!(
        html.contains(
            r#"<div class="subcategory"><a href="/c/general/sub-general/34">Sub General</a>"#
        ),
        "{html}"
    );
    assert!(html.contains("About the General category"));

    let (_, _, html) = get(&db.pool, "/c/general/sub-general/34").await;
    assert!(html.contains(r#"<a href="/c/general/4">General</a> &rsaquo; <a href="/c/general/sub-general/34">Sub General</a>"#));
    let (_, _, html) = get(&db.pool, "/c/general/4?page=1").await;
    assert!(
        !html.contains(r#"class="subcategory""#),
        "subcategories only on the first page"
    );
}

#[tokio::test]
async fn categories_index_lists_top_level_categories_with_featured_topics() {
    let db = TestDb::new().await;
    let (status, _, body) = get(&db.pool, "/categories.json").await;
    assert_eq!(status, StatusCode::OK);
    let json: Value = serde_json::from_str(&body).unwrap();
    let list = json["category_list"]["categories"].as_array().unwrap();
    let ids: Vec<i64> = list.iter().map(|c| c["id"].as_i64().unwrap()).collect();
    // Ordered by latest featured activity; Staff hidden; subcategory folded in.
    assert_eq!(ids, vec![4, 2, 1]);
    assert_eq!(list[0]["subcategory_ids"], serde_json::json!([34]));
    assert_eq!(list[0]["topics_all_time"], 4);
    assert_eq!(list[0]["topics"].as_array().unwrap().len(), 3);
    assert_eq!(list[0]["topics"][0]["last_poster"]["username"], "system");
    assert_eq!(json["category_list"]["can_create_topic"], false);

    let (status, _, html) = get(&db.pool, "/categories").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<title>Categories - Discourse</title>"));
    assert!(html.contains(r#"<h3><a href="/c/general/4">General</a></h3>"#));
    assert!(html.contains(r#"<a href="/c/general/sub-general/34">Sub General</a>"#));
    assert!(html.contains(
        r#"class="featured-topic"><a href="/t/welcome-to-discourse/5">Welcome to Discourse! 👋</a>"#
    ));
}
