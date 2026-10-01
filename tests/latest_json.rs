//! GET /latest.json for anonymous users, on the fixture topics of the
//! snapshot (scripts/fixtures/topics.rb). The full document is covered by
//! the parity golden files.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{TestDb, config, set_setting, state};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

const BOOL: i32 = 5;

async fn get(pool: &PgPool, path: &str) -> (StatusCode, Value) {
    let app = discourse_rs::app(state(pool.clone(), config(RailsEnv::Test, &[])));
    let response = app
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

fn ids(json: &Value) -> Vec<i64> {
    json["topic_list"]["topics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn lists_visible_topics_with_globally_pinned_first() {
    let db = TestDb::new().await;
    let (status, json) = get(&db.pool, "/latest.json").await;
    assert_eq!(status, StatusCode::OK);

    // 5 = Welcome (pinned globally), 35/41/38/37 by bumped_at desc (35 was
    // bumped by the mention fixture).
    assert_eq!(ids(&json), vec![5, 35, 41, 38, 37]);
    // Unlisted (39), deleted (40), staff (2, 4, 6) and category definition
    // topics (1, 3) never appear.
    for hidden in [39, 40, 2, 4, 6, 1, 3] {
        assert!(!ids(&json).contains(&hidden), "topic {hidden} leaked");
    }
    assert_eq!(json["topic_list"]["can_create_topic"], false);
    assert_eq!(json["topic_list"]["filter"], "latest");
    assert_eq!(json["topic_list"]["per_page"], 30);
    assert!(json["topic_list"].get("more_topics_url").is_none());
}

#[tokio::test]
async fn category_pinned_topics_keep_their_bump_position() {
    let db = TestDb::new().await;
    let (_, json) = get(&db.pool, "/latest.json").await;
    let topics = json["topic_list"]["topics"].as_array().unwrap();
    let pinned_in_category = topics.iter().find(|t| t["id"] == 37).unwrap();
    assert_eq!(pinned_in_category["pinned"], true);
    assert_eq!(pinned_in_category["pinned_globally"], false);
    assert_eq!(pinned_in_category["closed"], true);
    // Excerpts are only serialized for pinned topics.
    assert!(pinned_in_category["excerpt"].is_string());
    let plain = topics.iter().find(|t| t["id"] == 35).unwrap();
    assert!(plain.get("excerpt").is_none());
}

#[tokio::test]
async fn paging_offsets_after_the_pinned_topics_and_links_more() {
    let db = TestDb::new().await;
    let (_, page0) = get(&db.pool, "/latest.json?per_page=2").await;
    assert_eq!(ids(&page0), vec![5, 35]);
    assert_eq!(
        page0["topic_list"]["more_topics_url"],
        "/latest?no_definitions=true&page=1&per_page=2"
    );

    let (_, page1) = get(&db.pool, "/latest.json?page=1&per_page=2").await;
    // offset = page * per_page - pinned = 1
    assert_eq!(ids(&page1), vec![41, 38]);
    assert_eq!(
        page1["topic_list"]["more_topics_url"],
        "/latest?no_definitions=true&page=2&per_page=2"
    );

    let (_, page2) = get(&db.pool, "/latest.json?page=2&per_page=2").await;
    assert_eq!(ids(&page2), vec![37]);
    assert!(
        page2["topic_list"].get("more_topics_url").is_none(),
        "partial page has no more url"
    );
}

#[tokio::test]
async fn invalid_params_are_bad_requests() {
    let db = TestDb::new().await;
    for path in [
        "/latest.json?page=abc",
        "/latest.json?page=99999",
        "/latest.json?per_page=0",
        "/latest.json?per_page=101",
        "/latest.json?ascending=maybe",
    ] {
        let (status, _) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
    }
}

#[tokio::test]
async fn posters_and_users_follow_topic_posters_summary() {
    let db = TestDb::new().await;
    let (_, json) = get(&db.pool, "/latest.json").await;
    let topics = json["topic_list"]["topics"].as_array().unwrap();
    let replies = topics.iter().find(|t| t["id"] == 35).unwrap();
    let posters = replies["posters"].as_array().unwrap();
    // user0 is both OP and, since the mention fixture, the last poster, so
    // it stays first with both descriptions; then featured user1/user2.
    let user_ids: Vec<i64> = posters
        .iter()
        .map(|p| p["user_id"].as_i64().unwrap())
        .collect();
    assert_eq!(user_ids, vec![2, 3, 4]);
    assert_eq!(
        posters[0]["description"],
        "Original Poster, Most Recent Poster"
    );
    assert_eq!(posters[1]["description"], "Frequent Poster");
    assert_eq!(posters[0]["extras"], "latest");
    assert_eq!(posters[1]["extras"], Value::Null);
    assert_eq!(replies["last_poster_username"], "user0");

    let single = topics.iter().find(|t| t["id"] == 38).unwrap();
    assert_eq!(single["posters"][0]["extras"], "latest single");
    assert_eq!(
        single["posters"][0]["description"],
        "Original Poster, Most Recent Poster"
    );

    // Users are side-loaded once each, in first-seen order.
    let users = json["users"].as_array().unwrap();
    let names: Vec<&str> = users
        .iter()
        .map(|u| u["username"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["system", "user0", "user1", "user2", "admin"]);
    let user0 = &users[1];
    assert!(
        user0["avatar_template"]
            .as_str()
            .unwrap()
            .starts_with("/letter_avatar_proxy/v4/letter/u/")
    );
    assert!(
        user0["avatar_template"]
            .as_str()
            .unwrap()
            .ends_with("/{size}.png")
    );
    assert_eq!(user0["trust_level"], 1);
    assert!(user0.get("admin").is_none());
    // The system user's avatar is the small site logo.
    assert_eq!(
        users[0]["avatar_template"],
        "/images/discourse-logo-sketch-small.png"
    );
    assert_eq!(users[0]["admin"], true);
    assert_eq!(json["primary_groups"], serde_json::json!([]));
}

#[tokio::test]
async fn unicode_title_tags_and_likes() {
    let db = TestDb::new().await;
    let (_, json) = get(&db.pool, "/latest.json").await;
    let topics = json["topic_list"]["topics"].as_array().unwrap();
    let welcome = topics.iter().find(|t| t["id"] == 5).unwrap();
    assert_eq!(welcome["unicode_title"], "Welcome to Discourse! 👋");
    let tagged = topics.iter().find(|t| t["id"] == 38).unwrap();
    assert_eq!(tagged["tags"][0]["name"], "howto");
    assert_eq!(tagged["category_id"], 34);
    assert!(tagged.get("unicode_title").is_none());
    let liked = topics.iter().find(|t| t["id"] == 41).unwrap();
    assert_eq!(liked["like_count"], 1);
    assert_eq!(liked["op_like_count"], 1);
    assert_eq!(liked["archived"], true);
}

#[tokio::test]
async fn definitions_appear_when_the_setting_allows() {
    let db = TestDb::new().await;
    set_setting(
        &db.pool,
        "show_category_definitions_in_topic_lists",
        BOOL,
        "t",
    )
    .await;
    let (_, json) = get(&db.pool, "/latest.json?per_page=30").await;
    assert!(ids(&json).contains(&1), "{:?}", ids(&json));
    assert!(ids(&json).contains(&3));
}
