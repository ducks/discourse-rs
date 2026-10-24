//! A moderator (staff, not admin) through the pages they use: every page's
//! current user (its reviewable counts), the review queue and the user
//! menu's review tab through `Reviewable.viewable_by`, a topic with
//! queued posts, their own profile and the admin pages Rails lets them
//! see. The queue holds reviewables they may and may not see: one not
//! reviewable by moderators, one in a category only admins see, one in a
//! private message they are not in, and a flagged message they see
//! through its flag.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

/// user3 made a moderator (with the moderators and staff groups Rails adds
/// them to), a category only admins see, and the reviewables:
/// 9401 a flag on post 52 (General), 9402 a flag on post 37 not
/// reviewable by moderators, 9403 a queued topic in the admins' category,
/// 9404 a queued reply in user2's message to user1, 9405 a queued reply in
/// topic 35, 9406 a flag on the admin's message to user1 and user2, 9407 a
/// queued reply in topic 35 not reviewable by moderators.
const SETUP: [&str; 10] = [
    "UPDATE users SET moderator = true WHERE id = 5",
    "INSERT INTO group_users (group_id, user_id, created_at, updated_at, notification_level) VALUES (2, 5, '2026-10-01 10:00:00', '2026-10-01 10:00:00', 2), (3, 5, '2026-10-01 10:00:00', '2026-10-01 10:00:00', 2)",
    "INSERT INTO categories (id, name, name_lower, slug, color, text_color, user_id, read_restricted, position, created_at, updated_at) VALUES (9401, 'Admins Only', 'admins only', 'admins-only', '0088CC', 'FFFFFF', 1, true, 99, '2026-10-01 10:00:00', '2026-10-01 10:00:00')",
    "INSERT INTO category_groups (category_id, group_id, permission_type, created_at, updated_at) VALUES (9401, 1, 1, '2026-10-01 10:00:00', '2026-10-01 10:00:00')",
    "INSERT INTO post_actions (id, post_id, user_id, post_action_type_id, created_at, updated_at, staff_took_action, targets_topic) VALUES (9401, 52, 3, 4, '2026-10-02 10:00:00', '2026-10-02 10:00:00', FALSE, FALSE), (9402, 37, 3, 4, '2026-10-02 10:01:00', '2026-10-02 10:01:00', FALSE, FALSE), (9406, 50, 4, 4, '2026-10-02 10:06:00', '2026-10-02 10:06:00', FALSE, FALSE)",
    "UPDATE posts SET inappropriate_count = 1 WHERE id IN (52, 37, 50)",
    "INSERT INTO reviewables (id, type, type_source, status, created_by_id, reviewable_by_moderator, category_id, topic_id, score, potential_spam, target_id, target_type, target_created_by_id, payload, version, latest_score, force_review, potentially_illegal, created_at, updated_at) VALUES \
     (9401, 'ReviewableFlaggedPost', 'core', 0, 3, TRUE, 4, 35, 5, FALSE, 52, 'Post', 2, '{\"targets_topic\":false}', 0, '2026-10-02 10:00:00', FALSE, FALSE, '2026-10-02 10:00:00', '2026-10-02 10:00:00'), \
     (9402, 'ReviewableFlaggedPost', 'core', 0, 3, FALSE, 4, 35, 5, FALSE, 37, 'Post', 4, '{\"targets_topic\":false}', 0, '2026-10-02 10:01:00', FALSE, FALSE, '2026-10-02 10:01:00', '2026-10-02 10:01:00'), \
     (9403, 'ReviewableQueuedPost', 'core', 0, 3, TRUE, 9401, NULL, 6, FALSE, NULL, NULL, 3, '{\"raw\": \"A new topic for the admins waiting for approval.\", \"title\": \"A queued topic for admins\"}', 0, '2026-10-02 10:02:00', TRUE, FALSE, '2026-10-02 10:02:00', '2026-10-02 10:02:00'), \
     (9404, 'ReviewableQueuedPost', 'core', 0, 4, TRUE, NULL, 44, 6, FALSE, NULL, NULL, 4, '{\"raw\": \"A queued reply in a message.\"}', 0, '2026-10-02 10:03:00', TRUE, FALSE, '2026-10-02 10:03:00', '2026-10-02 10:03:00'), \
     (9405, 'ReviewableQueuedPost', 'core', 0, 2, TRUE, 4, 35, 6, FALSE, NULL, NULL, 2, '{\"raw\": \"A queued reply moderators review.\"}', 0, '2026-10-02 10:04:00', TRUE, FALSE, '2026-10-02 10:04:00', '2026-10-02 10:04:00'), \
     (9406, 'ReviewableFlaggedPost', 'core', 0, 4, TRUE, NULL, 43, 5, FALSE, 50, 'Post', 1, '{\"targets_topic\":false}', 0, '2026-10-02 10:06:00', FALSE, FALSE, '2026-10-02 10:06:00', '2026-10-02 10:06:00'), \
     (9407, 'ReviewableQueuedPost', 'core', 0, 2, FALSE, 4, 35, 6, FALSE, NULL, NULL, 2, '{\"raw\": \"A queued reply only admins review.\"}', 0, '2026-10-02 10:07:00', TRUE, FALSE, '2026-10-02 10:07:00', '2026-10-02 10:07:00')",
    "INSERT INTO reviewable_scores (id, reviewable_id, user_id, reviewable_score_type, status, score, take_action_bonus, user_accuracy_bonus, created_at, updated_at) VALUES (9401, 9401, 3, 4, 0, 5, 0, 0, '2026-10-02 10:00:00', '2026-10-02 10:00:00'), (9402, 9402, 3, 4, 0, 5, 0, 0, '2026-10-02 10:01:00', '2026-10-02 10:01:00'), (9406, 9406, 4, 4, 0, 5, 0, 0, '2026-10-02 10:06:00', '2026-10-02 10:06:00')",
    "INSERT INTO reviewable_histories (id, reviewable_id, reviewable_history_type, status, created_by_id, created_at, updated_at) VALUES (9401, 9401, 0, 0, 3, '2026-10-02 10:00:00', '2026-10-02 10:00:00'), (9402, 9402, 0, 0, 3, '2026-10-02 10:01:00', '2026-10-02 10:01:00'), (9403, 9403, 0, 0, 3, '2026-10-02 10:02:00', '2026-10-02 10:02:00'), (9404, 9404, 0, 0, 4, '2026-10-02 10:03:00', '2026-10-02 10:03:00'), (9405, 9405, 0, 0, 2, '2026-10-02 10:04:00', '2026-10-02 10:04:00'), (9406, 9406, 0, 0, 4, '2026-10-02 10:06:00', '2026-10-02 10:06:00'), (9407, 9407, 0, 0, 2, '2026-10-02 10:07:00', '2026-10-02 10:07:00')",
    "UPDATE topics SET reviewable_score = 5 WHERE id IN (35, 43)",
];

/// The pending reviewables the moderator sees.
const VISIBLE: [i64; 3] = [9401, 9405, 9406];

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn send(
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
        let (_, csrf) = client
            .send(Method::GET, "/session/csrf.json", "", true)
            .await;
        let csrf: Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        let (status, body) = client
            .send(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
                true,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        client
    }

    async fn page(&mut self, path: &str) -> (StatusCode, String) {
        self.send(Method::GET, path, "", false).await
    }

    async fn json(&mut self, path: &str) -> (StatusCode, Value) {
        let (status, body) = self.send(Method::GET, path, "", true).await;
        let json = serde_json::from_str(&body).unwrap_or_else(|_| panic!("{status}: {body}"));
        (status, json)
    }
}

async fn setup() -> (TestDb, AppState) {
    let db = TestDb::new().await;
    for sql in SETUP {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    (db, st)
}

fn ids(list: &Value) -> Vec<i64> {
    let mut ids: Vec<i64> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_i64().unwrap())
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn the_current_user_counts_what_the_moderator_may_review() {
    let (_db, st) = setup().await;
    let mut moderator = Client::logged_in(st.clone(), "user3").await;
    let (status, json) = moderator.json("/session/current.json").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["current_user"]["moderator"], true);
    assert_eq!(json["current_user"]["reviewable_count"], 3);
    assert_eq!(json["current_user"]["unseen_reviewable_count"], 3);
    let mut admin = Client::logged_in(st, "admin").await;
    let (_, json) = admin.json("/session/current.json").await;
    assert_eq!(json["current_user"]["reviewable_count"], 7);
}

#[tokio::test]
async fn the_moderator_loads_latest_their_profile_and_a_topic() {
    let (db, st) = setup().await;
    // Replies need approval, so the topic counts its queued posts for staff.
    common::set_setting(&db.pool, "approve_post_count", 3, "1").await;
    let mut moderator = Client::logged_in(st, "user3").await;
    for path in [
        "/latest",
        "/u/user3",
        "/u/user3/summary",
        "/u/user3/activity",
    ] {
        let (status, html) = moderator.page(path).await;
        assert_eq!(status, StatusCode::OK, "{path}: {html}");
    }
    let (status, html) = moderator
        .page("/t/parity-fixture-replies-and-posters/35")
        .await;
    assert_eq!(status, StatusCode::OK, "{html}");
    let (status, json) = moderator
        .json("/t/parity-fixture-replies-and-posters/35.json")
        .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    // 9405, not 9407 (not reviewable by moderators).
    assert_eq!(json["queued_posts_count"], 1);
    // The flagged message, through its flag.
    let (status, json) = moderator
        .json("/t/parity-fixture-pm-admin-to-user1-and-user2/43.json")
        .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    // The message they aren't in and isn't flagged.
    let (status, _) = moderator
        .send(
            Method::GET,
            "/t/parity-fixture-pm-user2-to-user1-archived/44.json",
            "",
            true,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_review_queue_lists_what_the_moderator_may_review() {
    let (_db, st) = setup().await;
    let mut moderator = Client::logged_in(st, "user3").await;
    let (status, json) = moderator.json("/review.json").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(ids(&json["reviewables"]), VISIBLE.to_vec());
    assert_eq!(json["meta"]["total_rows_reviewables"], 3);
    assert_eq!(json["meta"]["reviewable_count"], 3);
    // FlaggedUserSerializer for a moderator: the IP address
    // (moderators_view_ips) but no email (moderators_view_emails is off),
    // and an admin's posts aren't theirs to delete.
    let admin = json["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == 1)
        .unwrap();
    assert!(admin.get("email").is_none(), "{admin}");
    assert_eq!(admin["ip_address"], "127.0.0.1");
    assert_eq!(admin["can_delete_all_posts"], false);

    let (status, html) = moderator.page("/review").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("data-reviewable-id=\"9401\""), "{html}");
    assert!(html.contains("data-reviewable-id=\"9406\""));
    assert!(html.contains(
        "<div class=\"review-item reviewable-queued-post\" data-reviewable-id=\"9405\">"
    ));
    assert!(
        html.contains("<div class=\"post-body\"><p>A queued reply moderators review.</p></div>")
    );
    assert!(!html.contains("data-reviewable-id=\"9402\""));
}

#[tokio::test]
async fn the_user_menus_review_tab_for_a_moderator() {
    let (_db, st) = setup().await;
    let mut moderator = Client::logged_in(st, "user3").await;
    let (status, json) = moderator.json("/review/user-menu-list.json").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(ids(&json["reviewables"]), VISIBLE.to_vec());
    assert_eq!(json["reviewable_count"], 3);
    let (status, html) = moderator.page("/user-menu?tab=review-queue").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    for id in VISIBLE {
        assert!(html.contains(&format!("href=\"/review/{id}\"")), "{html}");
    }
    assert!(!html.contains("href=\"/review/9402\""));
    // The all notifications tab carries the pending reviewables too.
    let (status, html) = moderator.page("/user-menu").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    let (status, json) = moderator
        .json("/notifications.json?recent=true&limit=30")
        .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(ids(&json["pending_reviewables"]), VISIBLE.to_vec());
}

#[tokio::test]
async fn the_moderator_acts_on_what_they_may_review() {
    let (_db, st) = setup().await;
    let mut moderator = Client::logged_in(st, "user3").await;
    let (status, body) = moderator
        .send(
            Method::PUT,
            "/review/9401/perform/agree_and_keep?version=0",
            "",
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["reviewable_perform_result"]["reviewable_count"], 2);
    // The queued reply they may review: approved, posted as its author.
    let (status, body) = moderator
        .send(
            Method::PUT,
            "/review/9405/perform/approve_post?version=0",
            "",
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["reviewable_perform_result"]["reviewable_count"], 1);
    assert_eq!(
        json["reviewable_perform_result"]["created_post_topic_id"],
        35
    );
    let (status, json) = moderator.json("/review.json").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(ids(&json["reviewables"]), vec![9406]);
    // Not reviewable by moderators, in a hidden category, in a hidden
    // message: not found.
    for id in [9402, 9403, 9404] {
        let (status, body) = moderator
            .send(
                Method::PUT,
                &format!("/review/{id}/perform/agree_and_keep?version=0"),
                "",
                true,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{id}: {body}");
    }
    let (status, body) = moderator
        .send(
            Method::POST,
            "/review/9402/notes",
            "reviewable_note[content]=A+note",
            true,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn the_moderator_sees_the_admin_pages_rails_shows_them() {
    let (_db, st) = setup().await;
    let mut moderator = Client::logged_in(st, "user3").await;
    // admin#index is admin only: the dashboard comes with the 403 the
    // request is rescued with.
    let (status, html) = moderator.page("/admin").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(html.contains("dashboard general dashboard-next"));
    for path in [
        "/admin/users/list/active",
        "/admin/users/2/user0",
        "/admin/logs/staff_action_logs",
        "/admin/reports",
        "/admin/groups",
    ] {
        let (status, html) = moderator.page(path).await;
        assert_eq!(status, StatusCode::OK, "{path}: {html}");
    }
    // AdminConstraint: the admins' pages are not found for a moderator.
    for path in ["/admin/site_settings", "/admin/backups", "/admin/badges"] {
        let (status, _) = moderator.page(path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
}
