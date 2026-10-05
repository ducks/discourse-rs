//! GET /t/:slug/:id.json for anonymous users, on the snapshot's fixture
//! topics. The full documents are covered by the parity golden files.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

async fn get(pool: &PgPool, path: &str) -> (StatusCode, Option<String>, Value) {
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
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, location, json)
}

#[tokio::test]
async fn renders_the_post_stream_in_order() {
    let db = TestDb::new().await;
    let (status, _, json) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["id"], 35);
    assert_eq!(json["posts_count"], 5);
    assert_eq!(json["highest_post_number"], 5);
    assert_eq!(json["current_post_number"], 1);
    assert_eq!(json["chunk_size"], 20);
    let posts = json["post_stream"]["posts"].as_array().unwrap();
    let numbers: Vec<i64> = posts
        .iter()
        .map(|p| p["post_number"].as_i64().unwrap())
        .collect();
    assert_eq!(numbers, vec![1, 2, 3, 4, 5]);
    assert_eq!(json["post_stream"]["stream"].as_array().unwrap().len(), 5);
    // [post_number, days ago]: the fixture ages, so only the shape is fixed.
    let lookup = json["timeline_lookup"].as_array().unwrap();
    // Two days of posts since the mention fixture: one entry per day.
    assert_eq!(lookup.len(), 2);
    assert_eq!(lookup[0][0], 1);
    assert!(lookup[0][1].as_i64().unwrap() >= 0);
    assert_eq!(
        posts[0]["post_url"],
        "/t/parity-fixture-replies-and-posters/35/1"
    );
    assert_eq!(posts[0]["yours"], false);
    assert_eq!(posts[0]["read"], true);
    assert_eq!(posts[0]["can_edit"], false);
    assert!(posts[0]["cooked"].as_str().unwrap().starts_with("<p>"));
    // The page is complete, so suggested topics are present.
    assert!(json["suggested_topics"].is_array());
}

#[tokio::test]
async fn details_list_participants_creator_and_last_poster() {
    let db = TestDb::new().await;
    let (_, _, json) = get(&db.pool, "/t/35.json").await;
    let details = &json["details"];
    assert_eq!(details["can_edit"], false);
    assert_eq!(details["notification_level"], 1);
    let participants = details["participants"].as_array().unwrap();
    assert_eq!(participants.len(), 4);
    // user0 has the opener and the mention-fixture reply.
    assert!(
        participants
            .iter()
            .all(|p| p["post_count"] == 1 || p["id"] == 2)
    );
    assert_eq!(details["created_by"]["username"], "user0");
    assert_eq!(details["last_poster"]["username"], "user0");
    assert_eq!(json["participant_count"], 4);
    assert_eq!(json["actions_summary"].as_array().unwrap().len(), 4);
    assert_eq!(json["bookmarks"], serde_json::json!([]));
}

#[tokio::test]
async fn closed_topics_carry_the_small_action_post() {
    let db = TestDb::new().await;
    let (_, _, json) = get(&db.pool, "/t/parity-fixture-pinned-and-closed/37.json").await;
    assert_eq!(json["closed"], true);
    assert_eq!(json["pinned"], true);
    assert_eq!(json["pinned_globally"], false);
    let posts = json["post_stream"]["posts"].as_array().unwrap();
    let action = posts.iter().find(|p| p["post_type"] == 3).unwrap();
    assert_eq!(action["action_code"], "closed.enabled");
    assert!(action.get("action_code_who").is_none());
}

#[tokio::test]
async fn post_number_selects_the_current_post() {
    let db = TestDb::new().await;
    let (status, _, json) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35/3.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["current_post_number"], 3);
    // Past the end clamps to the highest post.
    let (_, _, json) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35/99.json").await;
    assert_eq!(json["current_post_number"], 5);
}

#[tokio::test]
async fn redirects_to_the_canonical_url() {
    let db = TestDb::new().await;
    let (status, location, _) = get(&db.pool, "/t/wrong-slug/35.json").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/t/parity-fixture-replies-and-posters/35.json")
    );

    let (status, location, _) = get(&db.pool, "/t/parity-fixture-replies-and-posters.json").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/t/parity-fixture-replies-and-posters/35.json")
    );

    let (status, location, _) = get(&db.pool, "/t/wrong/35/2.json").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/t/parity-fixture-replies-and-posters/35/2.json")
    );

    // A JSON request without a slug renders directly.
    let (status, _, json) = get(&db.pool, "/t/35.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["id"], 35);

    // Pages past the end go to the last page.
    let (status, location, _) = get(
        &db.pool,
        "/t/parity-fixture-replies-and-posters/35.json?page=5",
    )
    .await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/t/parity-fixture-replies-and-posters/35.json")
    );
}

#[tokio::test]
async fn hidden_topics_are_not_found() {
    let db = TestDb::new().await;
    for path in ["/t/999999.json", "/t/4.json", "/t/40.json", "/t/nope.json"] {
        let (status, _, json) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(json["error_type"], "not_found", "{path}");
    }
    // Unlisted topics are readable, just not listed.
    let (status, _, json) = get(&db.pool, "/t/39.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["visible"], false);
    // A negative page is a 404 too.
    let (status, _, _) = get(&db.pool, "/t/35.json?page=-1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
