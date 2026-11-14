//! Flags on hidden posts in the review queue
//! (ReviewableFlaggedPost#build_combined_actions and perform_disagree):
//! a post hidden by its flags or queued by staff offers
//! agree_and_keep_hidden and disagree_and_restore, which shows it again
//! (Post#unhide!) and tells its author; ignoring is the system user's
//! (Jobs::AutoQueueHandler). A first post's hide unlists its topic
//! (Post#hide!) and its restore lists it again. These rest on reading the
//! Rails source: no recorded case covers them.

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
const OFF_TOPIC: i32 = 3;
const INAPPROPRIATE: i32 = 4;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn send(&mut self, method: Method, path: &str, body: &str) -> (StatusCode, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest");
        if let Some(token) = &self.csrf {
            request = request.header("x-csrf-token", token.as_str());
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

    async fn perform(&mut self, id: i64, action: &str, version: i64) -> (StatusCode, String) {
        self.send(
            Method::PUT,
            &format!("/review/{id}/perform/{action}?version={version}"),
            "",
        )
        .await
    }

    /// The pending queue: the reviewable of the post, and the actions the
    /// queue offers on it in order.
    async fn queued(&mut self, post_id: i32) -> (i64, i64, Vec<String>) {
        let (status, body) = self.send(Method::GET, "/review.json", "").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let doc: Value = serde_json::from_str(&body).unwrap();
        let reviewable = doc["reviewables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["post_id"] == post_id)
            .unwrap_or_else(|| panic!("no reviewable for post {post_id}: {body}"));
        let id = reviewable["id"].as_i64().unwrap();
        let prefix = format!("{id}-");
        let mut offered = Vec::new();
        for bundle_id in reviewable["bundled_action_ids"].as_array().unwrap() {
            let bundle = doc["bundled_actions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|b| b["id"] == *bundle_id)
                .unwrap();
            for action_id in bundle["action_ids"].as_array().unwrap() {
                let action = doc["actions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|a| a["id"] == *action_id)
                    .unwrap();
                assert!(action_id.as_str().unwrap().starts_with(&prefix));
                offered.push(action["server_action"].as_str().unwrap().to_string());
            }
        }
        (id, reviewable["version"].as_i64().unwrap(), offered)
    }
}

async fn setup() -> (TestDb, AppState) {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    (db, st)
}

async fn exec(db: &TestDb, sql: &str) {
    sqlx::query(sql).execute(&db.pool).await.unwrap();
}

async fn count(db: &TestDb, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&db.pool).await.unwrap()
}

/// hidden, hidden_reason_id, hidden_at present.
async fn hidden(db: &TestDb, post_id: i32) -> (bool, Option<i32>, bool) {
    sqlx::query_as(
        "SELECT hidden, hidden_reason_id, hidden_at IS NOT NULL FROM posts WHERE id = $1",
    )
    .bind(post_id)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

/// The send_system_message jobs enqueued, oldest first.
async fn system_messages(db: &TestDb) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT args FROM discourse_rs.jobs WHERE name = 'send_system_message' ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

/// Post 52 (user0's, the last reply of topic 35) flagged off topic by
/// user2 with a score that hides it at the next flag, as the recorded
/// flag_in_review_hides_post sets it up; then user1's flag hides it.
async fn hidden_by_flags(db: &TestDb, st: &AppState) {
    for sql in [
        "INSERT INTO post_actions (id, post_id, user_id, post_action_type_id, created_at, updated_at, staff_took_action, targets_topic) VALUES (9301, 52, 4, 3, '2026-10-02 10:00:00', '2026-10-02 10:00:00', FALSE, FALSE)",
        "UPDATE posts SET off_topic_count = 1 WHERE id = 52",
        "INSERT INTO reviewables (id, type, type_source, status, created_by_id, reviewable_by_moderator, category_id, topic_id, score, potential_spam, target_id, target_type, target_created_by_id, payload, version, latest_score, force_review, potentially_illegal, created_at, updated_at) VALUES (9301, 'ReviewableFlaggedPost', 'core', 0, 4, TRUE, 4, 35, 10, FALSE, 52, 'Post', 2, '{\"targets_topic\":false}', 0, '2026-10-02 10:00:00', FALSE, FALSE, '2026-10-02 10:00:00', '2026-10-02 10:00:00')",
        "INSERT INTO reviewable_scores (id, reviewable_id, user_id, reviewable_score_type, status, score, take_action_bonus, user_accuracy_bonus, created_at, updated_at) VALUES (9301, 9301, 4, 3, 0, 10, 0, 0, '2026-10-02 10:00:00', '2026-10-02 10:00:00')",
        "INSERT INTO reviewable_histories (id, reviewable_id, reviewable_history_type, status, created_by_id, created_at, updated_at) VALUES (9301, 9301, 0, 0, 4, '2026-10-02 10:00:00', '2026-10-02 10:00:00')",
        "UPDATE topics SET reviewable_score = 10 WHERE id = 35",
    ] {
        exec(db, sql).await;
    }
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, body) = user1
        .flag("id=52&post_action_type_id=4&flag_topic=false")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(hidden(db, 52).await, (true, Some(1), true));
}

/// A post its flags hid: the queue offers keeping it hidden or restoring
/// it, and disagree_and_restore shows it again with everything Post#hide!
/// took given back.
#[tokio::test]
async fn disagreeing_restores_a_post_hidden_by_flags() {
    let (db, st) = setup().await;
    let before: (i32, chrono::NaiveDateTime) = sqlx::query_as(
        "SELECT s.post_count, t.bumped_at FROM user_stats s, topics t WHERE s.user_id = 2 AND t.id = 35",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    hidden_by_flags(&db, &st).await;
    // Post#hide!: the author's count down, the topic bumped back to the
    // reply before, the author told.
    let after_hide: (i32, chrono::NaiveDateTime) = sqlx::query_as(
        "SELECT s.post_count, t.bumped_at FROM user_stats s, topics t WHERE s.user_id = 2 AND t.id = 35",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(after_hide.0, before.0 - 1);
    let fourth: chrono::NaiveDateTime =
        sqlx::query_scalar("SELECT created_at FROM posts WHERE id = 38")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(after_hide.1, fourth);
    let jobs = system_messages(&db).await;
    assert_eq!(jobs.len(), 1, "{jobs:?}");
    assert_eq!(jobs[0]["message_type"], "post_hidden");
    assert_eq!(
        jobs[0]["message_options"]["url"],
        "/t/parity-fixture-replies-and-posters/35/5"
    );

    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (id, version, offered) = admin.queued(52).await;
    assert_eq!(id, 9301);
    // No hiding, keeping or editing in the agree bundle; no plain
    // disagree and no ignoring in the other.
    assert_eq!(
        offered,
        [
            "agree_and_keep_hidden",
            "delete_and_agree",
            "agree_and_silence",
            "agree_and_suspend",
            "disagree_and_restore",
            "delete_and_ignore",
        ]
    );

    // Not staff: no queue.
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, body) = user1.perform(id, "disagree_and_restore", version).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    // Actions a hidden post doesn't build: InvalidAction.
    for action in ["ignore_and_do_nothing", "ignore", "agree_and_hide"] {
        let (status, body) = admin.perform(id, action, version).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{action}: {body}");
    }
    // A stale version.
    let (status, body) = admin.perform(id, "disagree_and_restore", version + 1).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(hidden(&db, 52).await, (true, Some(1), true));
    assert_eq!(
        count(&db, "SELECT status::int8 FROM reviewables WHERE id = 9301").await,
        0
    );

    let (status, body) = admin.perform(id, "disagree_and_restore", version).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result: Value = serde_json::from_str(&body).unwrap();
    let result = &result["reviewable_perform_result"];
    assert_eq!(result["success"], true);
    assert_eq!(result["remove_reviewable_ids"], serde_json::json!([9301]));
    assert_eq!(result["version"], version + 1);

    // Post#unhide!: shown, the reason and date of the hide kept.
    assert_eq!(hidden(&db, 52).await, (false, Some(1), true));
    let after: (i32, chrono::NaiveDateTime) = sqlx::query_as(
        "SELECT s.post_count, t.bumped_at FROM user_stats s, topics t WHERE s.user_id = 2 AND t.id = 35",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(after, before);
    let visible: bool = sqlx::query_scalar("SELECT visible FROM topics WHERE id = 35")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert!(visible);
    // The flags disagreed with, their counts on the post zeroed.
    let actions: Vec<(i32, i32, Option<i32>, bool, bool)> = sqlx::query_as(
        "SELECT user_id, post_action_type_id, disagreed_by_id, disagreed_at IS NOT NULL, \
                agreed_at IS NULL AND deferred_at IS NULL AND deleted_at IS NULL \
         FROM post_actions WHERE post_id = 52 ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        actions,
        [
            (3, INAPPROPRIATE, Some(1), true, true),
            (4, OFF_TOPIC, Some(1), true, true),
        ]
    );
    let counts: (i32, i32) =
        sqlx::query_as("SELECT off_topic_count, inappropriate_count FROM posts WHERE id = 52")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(counts, (0, 0));
    // Rejected, by the admin, the scores disagreed.
    assert_eq!(
        count(&db, "SELECT status::int8 FROM reviewables WHERE id = 9301").await,
        2
    );
    let scores: Vec<(i32, i32, Option<i32>, bool)> = sqlx::query_as(
        "SELECT user_id, status, reviewed_by_id, reviewed_at IS NOT NULL FROM reviewable_scores \
         WHERE reviewable_id = 9301 ORDER BY user_id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(scores, [(3, 2, Some(1), true), (4, 2, Some(1), true)]);
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM reviewable_histories WHERE reviewable_id = 9301 \
               AND reviewable_history_type = 1 AND status = 2 AND created_by_id = 1"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &db,
            "SELECT SUM(flags_disagreed) FROM user_stats WHERE user_id IN (3, 4)"
        )
        .await,
        2
    );
    // notify_poster: the author told with the post's raw.
    let jobs = system_messages(&db).await;
    assert_eq!(jobs.len(), 2, "{jobs:?}");
    assert_eq!(jobs[1]["user_id"], 2);
    assert_eq!(jobs[1]["message_type"], "flags_disagreed");
    let raw: String = sqlx::query_scalar("SELECT raw FROM posts WHERE id = 52")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        jobs[1]["message_options"],
        serde_json::json!({
            "flagged_post_raw_content": raw,
            "url": "/t/parity-fixture-replies-and-posters/35/5",
        })
    );
    // Nobody was silenced for it.
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM user_histories WHERE action = 31").await,
        0
    );

    // Out of the queue, and a member reads the post again.
    let (status, body) = admin.send(Method::GET, "/review.json", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let doc: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(doc["reviewables"], serde_json::json!([]));
    let (status, body) = user1.send(Method::GET, "/posts/52.json", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let post: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(post["hidden"], false);
    assert_eq!(post["raw"], raw);
    // Reviewed: nothing more to perform.
    let (status, body) = admin.perform(id, "disagree_and_restore", version + 1).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

/// A reply staff queued for review: `disagree` is valid by its alias and
/// restores it; agree_and_keep_hidden agrees and leaves it hidden.
#[tokio::test]
async fn a_queued_reply_is_restored_or_kept_hidden() {
    let (db, st) = setup().await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    // Post 37 is user2's reply (post 3 of 5), 36 user1's (post 2).
    for post in [37, 36] {
        let (status, body) = admin
            .flag(&format!(
                "id={post}&post_action_type_id=4&queue_for_review=true&flag_topic=false"
            ))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(hidden(&db, post).await, (true, Some(1), true));
    }
    let counts: (i32, i32) = sqlx::query_as(
        "SELECT (SELECT post_count FROM user_stats WHERE user_id = 4), \
                (SELECT post_count FROM user_stats WHERE user_id = 3)",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();

    let (id, version, offered) = admin.queued(37).await;
    assert_eq!(
        offered,
        [
            "agree_and_keep_hidden",
            "delete_and_agree",
            "agree_and_silence",
            "agree_and_suspend",
            "disagree_and_restore",
            "delete_and_ignore",
        ]
    );
    let (status, body) = admin.perform(id, "disagree", version).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(hidden(&db, 37).await, (false, Some(1), true));
    let reviewable: (i32, i32, i32) = sqlx::query_as(
        "SELECT r.status, s.status, p.inappropriate_count FROM reviewables r \
         JOIN reviewable_scores s ON s.reviewable_id = r.id JOIN posts p ON p.id = r.target_id \
         WHERE r.id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(reviewable, (2, 2, 0));
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM post_actions WHERE post_id = 37 AND disagreed_by_id = 1 \
               AND disagreed_at IS NOT NULL"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &db,
            "SELECT post_count::int8 FROM user_stats WHERE user_id = 4"
        )
        .await,
        i64::from(counts.0) + 1
    );
    // Not the last reply: the topic's bump is as it was.
    let bumped: bool = sqlx::query_scalar(
        "SELECT t.bumped_at = p.created_at FROM topics t, posts p WHERE t.id = 35 AND p.id = 52",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(bumped);
    let jobs = system_messages(&db).await;
    let types: Vec<&str> = jobs
        .iter()
        .map(|j| j["message_type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        ["queued_by_staff", "queued_by_staff", "flags_disagreed"]
    );
    assert_eq!(jobs[2]["user_id"], 4);

    // Agreeing keeps the other hidden.
    let (id, version, _) = admin.queued(36).await;
    let (status, body) = admin.perform(id, "agree_and_keep_hidden", version).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(hidden(&db, 36).await, (true, Some(1), true));
    let reviewable: (i32, i32) = sqlx::query_as(
        "SELECT r.status, s.status FROM reviewables r \
         JOIN reviewable_scores s ON s.reviewable_id = r.id WHERE r.id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(reviewable, (1, 1));
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM post_actions WHERE post_id = 36 AND agreed_by_id = 1 \
               AND agreed_at IS NOT NULL"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &db,
            "SELECT post_count::int8 FROM user_stats WHERE user_id = 3"
        )
        .await,
        i64::from(counts.1)
    );
    assert_eq!(system_messages(&db).await.len(), 3);
}

/// Queueing a topic's first post unlists the topic (by the flagger, with
/// its small action), and restoring the post lists it again (by the
/// system user).
#[tokio::test]
async fn a_queued_first_post_unlists_its_topic_until_restored() {
    let (db, st) = setup().await;
    let before: (i32, i32, i32) = sqlx::query_as(
        "SELECT (SELECT topic_count FROM user_stats WHERE user_id = 2), \
                (SELECT topic_count FROM categories WHERE id = 4), \
                (SELECT highest_post_number FROM topics WHERE id = 35)",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    // Post 35 is user0's first post of topic 35.
    let (status, body) = admin
        .flag("id=35&post_action_type_id=4&queue_for_review=true&flag_topic=false")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(hidden(&db, 35).await, (true, Some(1), true));
    // Topic.visibility_reasons[:op_flag_threshold_reached]
    let topic: (bool, Option<i32>) =
        sqlx::query_as("SELECT visible, visibility_reason_id FROM topics WHERE id = 35")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(topic, (false, Some(0)));
    // One small action, the flagger's: Post#hide! finds the topic
    // unlisted already.
    let small: Vec<(i32, Option<String>, i32)> = sqlx::query_as(
        "SELECT user_id, action_code, post_number FROM posts \
         WHERE topic_id = 35 AND post_type = 3 ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        small,
        [(1, Some("visible.disabled".to_string()), before.2 + 1)]
    );
    // The author's topic and the category's, counted out once.
    let unlisted: (i32, i32) = sqlx::query_as(
        "SELECT (SELECT topic_count FROM user_stats WHERE user_id = 2), \
                (SELECT topic_count FROM categories WHERE id = 4)",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(unlisted, (before.0 - 1, before.1 - 1));
    let jobs = system_messages(&db).await;
    assert_eq!(jobs.len(), 1, "{jobs:?}");
    assert_eq!(jobs[0]["user_id"], 2);
    assert_eq!(jobs[0]["message_type"], "queued_by_staff");
    assert_eq!(
        jobs[0]["message_options"]["url"],
        "/t/parity-fixture-replies-and-posters/35/1"
    );

    // The flag modal draws the post again (postStream.refreshPost).
    let (status, body) = admin.send(Method::GET, "/live/post/35", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("data-post-id=\"35\""), "{body}");

    let (id, version, offered) = admin.queued(35).await;
    assert!(
        offered.contains(&"disagree_and_restore".to_string()),
        "{offered:?}"
    );
    assert!(!offered.contains(&"ignore_and_do_nothing".to_string()));
    let (status, body) = admin.perform(id, "disagree_and_restore", version).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(hidden(&db, 35).await, (false, Some(1), true));
    // Topic.visibility_reasons[:op_unhidden]
    let topic: (bool, Option<i32>) =
        sqlx::query_as("SELECT visible, visibility_reason_id FROM topics WHERE id = 35")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(topic, (true, Some(1)));
    let small: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT user_id, action_code FROM posts WHERE topic_id = 35 AND post_type = 3 ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        small,
        [
            (1, Some("visible.disabled".to_string())),
            (-1, Some("visible.enabled".to_string())),
        ]
    );
    let relisted: (i32, i32) = sqlx::query_as(
        "SELECT (SELECT topic_count FROM user_stats WHERE user_id = 2), \
                (SELECT topic_count FROM categories WHERE id = 4)",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(relisted, (before.0, before.1));
    assert_eq!(
        count(
            &db,
            "SELECT status::int8 FROM reviewables WHERE target_id = 35 AND target_type = 'Post'"
        )
        .await,
        2
    );
}

/// A first post its flags hide (agree_and_hide from the queue) unlists
/// the topic as the system user.
#[tokio::test]
async fn hiding_a_first_post_from_the_queue_unlists_its_topic() {
    let (db, st) = setup().await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, body) = user1
        .flag("id=35&post_action_type_id=4&flag_topic=false")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(hidden(&db, 35).await, (false, None, false));
    let before: i32 = sqlx::query_scalar("SELECT topic_count FROM user_stats WHERE user_id = 2")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (id, version, offered) = admin.queued(35).await;
    // A visible post: hide, keep or edit; disagree or ignore.
    assert_eq!(
        offered,
        [
            "agree_and_hide",
            "agree_and_keep",
            "agree_and_edit",
            "delete_and_agree",
            "agree_and_silence",
            "agree_and_suspend",
            "disagree",
            "ignore_and_do_nothing",
            "delete_and_ignore",
        ]
    );
    // Restoring is a hidden post's.
    let (status, body) = admin.perform(id, "disagree_and_restore", version).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = admin.perform(id, "agree_and_keep_hidden", version).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = admin.perform(id, "agree_and_hide", version).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(hidden(&db, 35).await, (true, Some(1), true));
    let topic: (bool, Option<i32>) =
        sqlx::query_as("SELECT visible, visibility_reason_id FROM topics WHERE id = 35")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(topic, (false, Some(0)));
    let small: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT user_id, action_code FROM posts WHERE topic_id = 35 AND post_type = 3 ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(small, [(-1, Some("visible.disabled".to_string()))]);
    // The topic's count once, by TopicStatusUpdater alone.
    assert_eq!(
        count(
            &db,
            "SELECT topic_count::int8 FROM user_stats WHERE user_id = 2"
        )
        .await,
        i64::from(before) - 1
    );
    let jobs = system_messages(&db).await;
    assert_eq!(jobs.len(), 1, "{jobs:?}");
    assert_eq!(jobs[0]["message_type"], "post_hidden");
}

/// Jobs::AutoQueueHandler ignores an old flag as the system user, who may
/// ignore a hidden post's: it stays hidden.
#[tokio::test]
async fn an_expired_flag_on_a_hidden_post_is_ignored() {
    let (db, st) = setup().await;
    hidden_by_flags(&db, &st).await;
    exec(
        &db,
        "UPDATE reviewables SET created_at = now() - interval '61 days' WHERE id = 9301",
    )
    .await;
    discourse_rs::jobs::run_scheduled(&st, discourse_rs::jobs::moderation::AUTO_QUEUE_HANDLER)
        .await
        .unwrap();
    assert_eq!(hidden(&db, 52).await, (true, Some(1), true));
    assert_eq!(
        count(&db, "SELECT status::int8 FROM reviewables WHERE id = 9301").await,
        3
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM post_actions WHERE post_id = 52 AND deferred_by_id = -1 \
               AND deferred_at IS NOT NULL"
        )
        .await,
        2
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM reviewable_scores WHERE reviewable_id = 9301 AND status = 3"
        )
        .await,
        2
    );
    // No flags_disagreed message: only the hide's.
    assert_eq!(system_messages(&db).await.len(), 1);
}

/// An author silenced for the hidden post (UserSilencer.was_silenced_for?)
/// is unsilenced when its flags are disagreed with: the queue says so
/// (penalty_effect_for), the staff action carries the reviewable.
#[tokio::test(flavor = "multi_thread")]
async fn disagreeing_unsilences_an_author_silenced_for_the_post() {
    let (db, st) = setup().await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, body) = admin
        .flag("id=37&post_action_type_id=4&queue_for_review=true&flag_topic=false")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // UserHistory.actions[:silence_user] on the post.
    exec(
        &db,
        "UPDATE users SET silenced_till = now() + interval '1 day' WHERE id = 4",
    )
    .await;
    exec(
        &db,
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, post_id, created_at, updated_at) \
         VALUES (30, 1, 4, 37, now(), now())",
    )
    .await;
    let (status, body) = admin.send(Method::GET, "/review.json", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let doc: Value = serde_json::from_str(&body).unwrap();
    let restore = doc["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["server_action"] == "disagree_and_restore")
        .unwrap();
    assert_eq!(restore["penalty_effect"], "lifts_penalty");

    let (id, version, _) = admin.queued(37).await;
    let (status, body) = admin.perform(id, "disagree_and_restore", version).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(hidden(&db, 37).await, (false, Some(1), true));
    let silenced: bool =
        sqlx::query_scalar("SELECT silenced_till IS NOT NULL FROM users WHERE id = 4")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(!silenced);
    // UserHistory.actions[:unsilence_user], with the reviewable.
    let logged: Vec<(i32, Option<i32>, Option<i64>)> = sqlx::query_as(
        "SELECT acting_user_id, target_user_id, reviewable_id::int8 FROM user_histories WHERE action = 31",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(logged, [(1, Some(4), Some(id))]);
    // The unsilenced system message, to the author.
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM topics t JOIN topic_allowed_users a ON a.topic_id = t.id \
             WHERE t.archetype = 'private_message' AND t.subtype = 'system_message' AND a.user_id = 4 \
               AND t.title = 'Account no longer on hold'"
        )
        .await,
        1
    );
}
