//! GET /composer/mentions (ComposerController#mentions), after
//! spec/requests/composer_controller_spec.rb: the users and visible groups
//! a post's names are, the reasons a mention would notify nobody (shown
//! to staff, the topic's access to anyone), group sizes and @here's
//! reach. The seed's groups are automatic, visible to members, with
//! mentionable_level 0 (nobody); topic 35 is a category topic.

use crate::common;

use axum::body::Body;
use axum::http::{Request, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn get(state: &AppState, cookie: Option<&str>, path: &str) -> (u16, Value) {
    let mut request = Request::builder()
        .uri(path)
        .header(header::HOST, "test.localhost")
        .header("x-requested-with", "XMLHttpRequest")
        .header(header::ACCEPT, "application/json");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    let response = discourse_rs::app(state.clone())
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// A session cookie for the user, as a login leaves it.
async fn login(state: &AppState, username: &str) -> String {
    let csrf = discourse_rs::app(state.clone())
        .oneshot(
            Request::builder()
                .uri("/session/csrf.json")
                .header(header::HOST, "test.localhost")
                .header("x-requested-with", "XMLHttpRequest")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let mut cookies: Vec<String> = csrf
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string())
        .collect();
    let bytes = csrf.into_body().collect().await.unwrap().to_bytes();
    let token = serde_json::from_slice::<Value>(&bytes).unwrap()["csrf"]
        .as_str()
        .unwrap()
        .to_string();
    let response = discourse_rs::app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/session")
                .header(header::HOST, "test.localhost")
                .header("x-requested-with", "XMLHttpRequest")
                .header("x-csrf-token", token)
                .header(header::COOKIE, cookies.join("; "))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("login={username}&password=password")))
                .unwrap(),
        )
        .await
        .unwrap();
    for v in response.headers().get_all(header::SET_COOKIE) {
        let pair = v.to_str().unwrap().split(';').next().unwrap().to_string();
        let name = pair.split('=').next().unwrap().to_string();
        cookies.retain(|c| !c.starts_with(&format!("{name}=")));
        cookies.push(pair);
    }
    cookies.join("; ")
}

async fn setup(db: &TestDb) -> AppState {
    state(db.pool.clone(), config(RailsEnv::Test, &[])).await
}

#[tokio::test]
async fn requires_login_and_names() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let (status, _) = get(&st, None, "/composer/mentions?names[]=user0").await;
    assert_eq!(status, 403);

    let cookie = login(&st, "user1").await;
    let (status, body) = get(&st, Some(&cookie), "/composer/mentions").await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = get(&st, Some(&cookie), "/composer/mentions?names=user0").await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error_type"], "invalid_parameters");
    let many: Vec<String> = (0..21).map(|i| format!("names[]=u{i}")).collect();
    let (status, _) = get(
        &st,
        Some(&cookie),
        &format!("/composer/mentions?{}", many.join("&")),
    )
    .await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn users_and_groups_with_their_reasons() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let cookie = login(&st, "user1").await;
    let (status, body) = get(
        &st,
        Some(&cookie),
        "/composer/mentions?names[]=user0&names[]=Admin&names[]=nobody&names[]=staff",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let mut users: Vec<String> = body["users"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u.as_str().unwrap().to_string())
        .collect();
    users.sort();
    assert_eq!(users, ["admin", "user0"]);
    assert_eq!(body["user_reasons"], json!({}));
    // Visible with its members, but nobody may mention it.
    assert_eq!(body["groups"], json!({"staff": {"user_count": 1}}));
    assert_eq!(body["group_reasons"], json!({"staff": "not_mentionable"}));
    assert_eq!(body["here_count"], Value::Null);
    assert_eq!(body["max_users_notified_per_group_mention"], 100);
}

#[tokio::test]
async fn a_topic_the_member_cannot_see_is_refused() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let cookie = login(&st, "user1").await;
    let (status, body) = get(
        &st,
        Some(&cookie),
        "/composer/mentions?names[]=user0&topic_id=999999",
    )
    .await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error_type"], "invalid_access");
}

#[tokio::test]
async fn a_muted_topic_is_a_reason_for_staff_only() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    sqlx::query(
        "INSERT INTO topic_users (user_id, topic_id, notification_level) \
         SELECT id, 35, 0 FROM users WHERE username = 'user0' \
         ON CONFLICT (user_id, topic_id) DO UPDATE SET notification_level = 0",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let admin = login(&st, "admin").await;
    let (status, body) = get(
        &st,
        Some(&admin),
        "/composer/mentions?names[]=user0&topic_id=35",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["user_reasons"], json!({"user0": "muted_topic"}));
    let member = login(&st, "user1").await;
    let (_, body) = get(
        &st,
        Some(&member),
        "/composer/mentions?names[]=user0&topic_id=35",
    )
    .await;
    assert_eq!(body["user_reasons"], json!({}));
}

#[tokio::test]
async fn here_counts_the_topics_posters_but_the_member() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let expected: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT p.user_id) FROM posts p JOIN users u ON u.id = p.user_id \
         WHERE p.topic_id = 35 AND p.deleted_at IS NULL AND u.id > 0 AND u.username <> 'admin' \
           AND (p.post_type IN (1, 2) OR (p.post_type = 4 AND (SELECT f.admin OR f.moderator FROM posts fp \
                JOIN users f ON f.id = fp.user_id WHERE fp.topic_id = 35 AND fp.post_number = 1)))",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    // here_mention_allowed_groups is trust_level_2 by default; admin is in it.
    let admin = login(&st, "admin").await;
    let (status, body) = get(
        &st,
        Some(&admin),
        "/composer/mentions?names[]=here&topic_id=35",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["here_count"], json!(expected.min(10)));
}
