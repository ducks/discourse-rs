//! /top and /hot lists for anonymous users. The documents are covered by
//! the parity golden files; these pin period selection, redirects and
//! the definition-topic and pinning differences from /latest.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

async fn get(pool: &PgPool, path: &str) -> (StatusCode, Option<String>, String) {
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

fn json(body: &str) -> Value {
    serde_json::from_str(body).unwrap()
}

fn ids(v: &Value) -> Vec<i64> {
    v["topic_list"]["topics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn top_picks_the_period_and_lists_scored_topics() {
    let db = TestDb::new().await;
    let (status, _, body) = get(&db.pool, "/top.json").await;
    assert_eq!(status, StatusCode::OK);
    let v = json(&body);
    assert_eq!(v["topic_list"]["filter"], "top");
    assert_eq!(v["topic_list"]["for_period"], "yearly");
    assert_eq!(v["topic_list"]["per_page"], 50);
    // Only topic 35 has a positive score in the snapshot.
    assert_eq!(ids(&v), vec![35]);

    let (_, _, body) = get(&db.pool, "/top.json?period=weekly&per_page=2").await;
    let v = json(&body);
    assert_eq!(v["topic_list"]["for_period"], "weekly");
    assert_eq!(v["topic_list"]["per_page"], 2);

    // A category uses its default_top_period ("all" by default).
    let (_, _, body) = get(&db.pool, "/c/general/4/l/top.json").await;
    assert_eq!(json(&body)["topic_list"]["for_period"], "all");

    // The site default applies when the category's period is unusable.
    sqlx::query("UPDATE categories SET default_top_period = 'monthly' WHERE id = 4")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, _, body) = get(&db.pool, "/c/general/4/l/top.json").await;
    assert_eq!(json(&body)["topic_list"]["for_period"], "monthly");
    set_setting(&db.pool, "top_page_default_timeframe", 7, "monthly").await;
    let (_, _, body) = get(&db.pool, "/top.json").await;
    assert_eq!(json(&body)["topic_list"]["for_period"], "monthly");
}

#[tokio::test]
async fn top_period_routes_redirect_and_validate() {
    let db = TestDb::new().await;
    let (status, location, _) = get(&db.pool, "/top/weekly.json").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/top.json?period=weekly")
    );
    let (status, location, _) = get(&db.pool, "/top/all").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/top?period=all")
    );
    let (status, _, _) = get(&db.pool, "/top.json?period=decadely").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn hot_keeps_definition_topics_and_floats_pins() {
    let db = TestDb::new().await;
    let (status, _, body) = get(&db.pool, "/hot.json").await;
    assert_eq!(status, StatusCode::OK);
    let v = json(&body);
    assert_eq!(v["topic_list"]["filter"], "hot");
    assert!(v["topic_list"].get("for_period").is_none());
    assert_eq!(v["topic_list"]["per_page"], 30);
    let list = ids(&v);
    // Globally pinned first, then by score; category definition topics are
    // not filtered out of hot.
    assert_eq!(&list[..3], &[5, 35, 41]);
    for definition in [1, 3, 34] {
        assert!(list.contains(&definition), "topic {definition}");
    }

    // In a category, its own pinned topics float, including its About topic.
    let (_, _, body) = get(&db.pool, "/c/general/4/l/hot.json").await;
    assert_eq!(ids(&json(&body)), vec![5, 3, 35, 41, 38]);
}

#[tokio::test]
async fn category_default_view_selects_the_list() {
    let db = TestDb::new().await;
    sqlx::query("UPDATE categories SET default_view = 'hot' WHERE id = 4")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, _, body) = get(&db.pool, "/c/general/4.json").await;
    let v = json(&body);
    assert_eq!(v["topic_list"]["filter"], "hot");
    assert_eq!(ids(&v)[..2], [5, 3]);
    // An explicit filter still wins.
    let (_, _, body) = get(&db.pool, "/c/general/4/l/latest.json").await;
    assert_eq!(json(&body)["topic_list"]["filter"], "latest");
}

#[tokio::test]
async fn top_and_hot_render_html() {
    let db = TestDb::new().await;
    for path in ["/top", "/hot", "/c/general/4/l/top"] {
        let (status, _, html) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(html.contains("<!DOCTYPE html>"), "{path}");
        assert!(html.contains(r#"href="/top">Top</a>"#));
    }
}
