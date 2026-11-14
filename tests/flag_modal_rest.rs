//! What the flag modal's other buttons send POST /post_actions
//! (PostActionCreator): a flag in a message, an official warning
//! (is_warning: TopicCreator#create_warning's UserWarning and the
//! moderator_warning subtype), and staff queueing a post for review
//! (queue_for_review: the post hidden, its score's reason queued_by_staff).
//! These rest on reading the Rails source: no recorded case covers them.

use crate::common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

/// PostActionType ids.
const INAPPROPRIATE: i32 = 4;
const NOTIFY_USER: i32 = 6;
const NOTIFY_MODERATORS: i32 = 7;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn send(&mut self, method: Method, path: &str, body: &str) -> (StatusCode, String) {
        self.request(method, path, body, true).await
    }

    /// A page, as a browser asks for it.
    async fn page(&mut self, path: &str) -> (StatusCode, String) {
        self.request(Method::GET, path, "", false).await
    }

    async fn request(
        &mut self,
        method: Method,
        path: &str,
        body: &str,
        xhr: bool,
    ) -> (StatusCode, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        if xhr {
            request = request.header("x-requested-with", "XMLHttpRequest");
            if let Some(token) = &self.csrf {
                request = request.header("x-csrf-token", token.as_str());
            }
        }
        if !self.cookies.is_empty() {
            let cookies: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookies.join("; "));
        }
        if !body.is_empty() {
            request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let pair = v.to_str().unwrap().split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client.send(Method::GET, "/session/csrf.json", "").await;
        let csrf: Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        let (status, body) = client
            .send(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        client
    }

    async fn flag(&mut self, body: &str) -> (StatusCode, String) {
        self.send(Method::POST, "/post_actions.json", body).await
    }
}

async fn setup() -> (TestDb, AppState) {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    (db, st)
}

async fn count(db: &TestDb, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&db.pool).await.unwrap()
}

/// A post in a message is flagged as any other: post 50 is the admin's
/// message to user1 and user2 (topic 43).
#[tokio::test]
async fn a_flag_on_a_message_post() {
    let (db, st) = setup().await;
    // Not for a member the message isn't for.
    let mut user0 = Client::logged_in(st.clone(), "user0").await;
    let (status, body) = user0
        .flag("id=50&post_action_type_id=4&flag_topic=false")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM post_actions WHERE post_id = 50").await,
        0
    );

    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, body) = user1
        .flag("id=50&post_action_type_id=4&flag_topic=false")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let post: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(post["id"], 50, "{body}");
    let action: (i32, i32, bool, bool) = sqlx::query_as(
        "SELECT user_id, post_action_type_id, staff_took_action, targets_topic FROM post_actions \
         WHERE post_id = 50",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(action, (3, INAPPROPRIATE, false, false));
    // The message's reviewable: no category, pending, the flag its score.
    let reviewable: (i64, String, i32, Option<i32>, Option<i32>, Option<i32>) = sqlx::query_as(
        "SELECT id, type, status, topic_id, category_id, target_created_by_id FROM reviewables \
         WHERE target_type = 'Post' AND target_id = 50",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        (
            reviewable.1.as_str(),
            reviewable.2,
            reviewable.3,
            reviewable.4,
            reviewable.5
        ),
        ("ReviewableFlaggedPost", 0, Some(43), None, Some(1))
    );
    let score: (i32, i32, i32, Option<String>) = sqlx::query_as(
        "SELECT user_id, reviewable_score_type, status, reason FROM reviewable_scores \
         WHERE reviewable_id = $1",
    )
    .bind(reviewable.0)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(score, (3, INAPPROPRIATE, 0, None));
    let hidden: bool = sqlx::query_scalar("SELECT hidden FROM posts WHERE id = 50")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert!(!hidden);

    // The message itself (flag_topic), by its other member: its first
    // post's flag, not targeting the topic while that is its only post.
    let mut user2 = Client::logged_in(st.clone(), "user2").await;
    let (status, body) = user2
        .flag("id=43&post_action_type_id=4&flag_topic=true")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let targets: Vec<(i32, bool)> = sqlx::query_as(
        "SELECT user_id, targets_topic FROM post_actions WHERE post_id = 50 ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(targets, vec![(3, false), (4, false)]);

    // The admin, who reviews everything, sees it in the queue.
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, body) = admin.send(Method::GET, "/review.json", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let json: Value = serde_json::from_str(&body).unwrap();
    assert!(
        json["reviewables"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == reviewable.0),
        "{body}"
    );
}

/// Staff message a post's author as an official warning: the message's
/// subtype is moderator_warning and the user is warned (UserWarning).
#[tokio::test]
async fn an_official_warning() {
    let (db, st) = setup().await;
    let message = "Please+keep+replies+on+the+subject+of+the+topic.";

    // "no warnings except for staff": refused before anything is written.
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, body) = user1
        .flag(&format!(
            "id=52&post_action_type_id={NOTIFY_USER}&message={message}&is_warning=true&flag_topic=false"
        ))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["errors"][0],
        "You are not permitted to view the requested resource."
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM user_warnings").await, 0);
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM post_actions WHERE post_id = 52").await,
        0
    );

    // A warning goes to one user: a message to the moderators isn't one
    // (create_warning's too_many_users), and nothing of it stays.
    let topics = count(&db, "SELECT COUNT(*) FROM topics").await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, body) = admin
        .flag(&format!(
            "id=52&post_action_type_id={NOTIFY_MODERATORS}&message={message}&is_warning=true&flag_topic=false"
        ))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["errors"][0],
        "You can only send warnings to one user at a time."
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM topics").await, topics);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM user_warnings").await, 0);
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM post_actions WHERE post_id = 52").await,
        0
    );

    // The warning: post 52 is user0's.
    let (status, body) = admin
        .flag(&format!(
            "id=52&post_action_type_id={NOTIFY_USER}&message={message}&is_warning=true&flag_topic=false"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (related,): (Option<i32>,) = sqlx::query_as(
        "SELECT related_post_id FROM post_actions WHERE post_id = 52 AND user_id = 1 \
           AND post_action_type_id = 6",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let related = related.expect("the warning's post");
    let topic: (i32, String, Option<String>, i32, String) = sqlx::query_as(
        "SELECT t.id, t.archetype, t.subtype, t.user_id, p.raw FROM posts p \
         JOIN topics t ON t.id = p.topic_id WHERE p.id = $1",
    )
    .bind(related)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        (topic.1.as_str(), topic.2.as_deref(), topic.3),
        ("private_message", Some("moderator_warning"), 1)
    );
    assert!(
        topic
            .4
            .contains("Please keep replies on the subject of the topic."),
        "{}",
        topic.4
    );
    let warnings: Vec<(i32, i32, i32)> =
        sqlx::query_as("SELECT topic_id, user_id, created_by_id FROM user_warnings")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(warnings, vec![(topic.0, 2, 1)]);
    let allowed: Vec<i32> = sqlx::query_scalar(
        "SELECT user_id FROM topic_allowed_users WHERE topic_id = $1 ORDER BY user_id",
    )
    .bind(topic.0)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(allowed, vec![1, 2]);
    // A message to the user is no flag: nothing to review, nothing hidden.
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM reviewables WHERE target_type = 'Post' AND target_id = 52"
        )
        .await,
        0
    );

    // The same message without is_warning stays a notify_user message.
    let (status, body) = admin
        .flag(&format!(
            "id=37&post_action_type_id={NOTIFY_USER}&message={message}&flag_topic=false"
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let subtype: Option<String> = sqlx::query_scalar(
        "SELECT t.subtype FROM post_actions a JOIN posts p ON p.id = a.related_post_id \
         JOIN topics t ON t.id = p.topic_id WHERE a.post_id = 37 AND a.user_id = 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(subtype.as_deref(), Some("notify_user"));
    assert_eq!(count(&db, "SELECT COUNT(*) FROM user_warnings").await, 1);
}

/// Staff queue a reply for review: flagged, hidden at once whatever the
/// flag's score, its author told with queued_by_staff, and the score's
/// reason queued_by_staff.
#[tokio::test]
async fn queueing_a_post_for_review() {
    let (db, st) = setup().await;
    // Staff only.
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, body) = user1
        .flag("id=37&post_action_type_id=4&queue_for_review=true&flag_topic=false")
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM post_actions WHERE post_id = 37").await,
        0
    );

    // Post 37 is user2's reply (post 3 of 5) in topic 35.
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, body) = admin
        .flag("id=37&post_action_type_id=4&queue_for_review=true&flag_topic=false")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let post: (bool, Option<i32>, bool) = sqlx::query_as(
        "SELECT hidden, hidden_reason_id, hidden_at IS NOT NULL FROM posts WHERE id = 37",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    // Post.hidden_reasons[:flag_threshold_reached]
    assert_eq!(post, (true, Some(1), true));
    let action: (i32, bool) = sqlx::query_as(
        "SELECT post_action_type_id, staff_took_action FROM post_actions \
         WHERE post_id = 37 AND user_id = 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(action, (INAPPROPRIATE, false));
    // Pending: queued, not agreed with as take_action is.
    let reviewable: (i64, i32) = sqlx::query_as(
        "SELECT id, status FROM reviewables WHERE target_type = 'Post' AND target_id = 37",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(reviewable.1, 0);
    let score: (i32, i32, i32, Option<String>) = sqlx::query_as(
        "SELECT user_id, reviewable_score_type, status, reason FROM reviewable_scores \
         WHERE reviewable_id = $1",
    )
    .bind(reviewable.0)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        score,
        (1, INAPPROPRIATE, 0, Some("queued_by_staff".to_string()))
    );
    // The topic stays listed; the author hears why.
    let visible: bool = sqlx::query_scalar("SELECT visible FROM topics WHERE id = 35")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert!(visible);
    let jobs: Vec<Value> =
        sqlx::query_scalar("SELECT args FROM discourse_rs.jobs WHERE name = 'send_system_message'")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(jobs.len(), 1, "{jobs:?}");
    assert_eq!(jobs[0]["user_id"], 4);
    assert_eq!(jobs[0]["message_type"], "queued_by_staff");

    // A hidden post takes no more flags.
    let (status, body) = admin
        .flag("id=37&post_action_type_id=3&queue_for_review=true&flag_topic=false")
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // A topic's first post unlists the topic (review_hidden_posts.rs).
    let (status, body) = admin
        .flag("id=35&post_action_type_id=4&queue_for_review=true&flag_topic=false")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let hidden: (bool, bool) = sqlx::query_as(
        "SELECT p.hidden, t.visible FROM posts p JOIN topics t ON t.id = p.topic_id WHERE p.id = 35",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(hidden, (true, false));
}

/// Staff on a chat channel get the penalty modal the flag modal's take
/// action opens (adminTools); members' pages are as they were.
#[tokio::test]
async fn the_chat_page_brings_staff_the_penalty_modal() {
    let (_db, st) = setup().await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, html) = admin.page("/chat/c/general/2").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(
        html.contains("<template class=\"chat-flag-modal\" data-src=\"/assets/flag-modal.js?v"),
        "{html}"
    );
    assert!(
        html.contains("\" data-penalize-src=\"/assets/penalize-user.js?v"),
        "{html}"
    );
    assert!(
        html.contains("<script type=\"application/json\" id=\"penalize-user-config\">"),
        "{html}"
    );
    let mut user1 = Client::logged_in(st, "user1").await;
    let (status, html) = user1.page("/chat/c/general/2").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(
        html.contains("<template class=\"chat-flag-modal\" data-src=\"/assets/flag-modal.js?v"),
        "{html}"
    );
    assert!(!html.contains("penalize-user"), "{html}");
}
