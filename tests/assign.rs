//! discourse-assign: AssignController's actions, the Assigner's checks,
//! records and notifications, and the keys it adds to serializers, cases
//! ported from the plugin's specs (spec/requests/assign_controller_spec.rb,
//! spec/lib/assigner_spec.rb, spec/serializers/*).

mod common;

use axum::body::Body;
use axum::http::{Method, Request, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const STRING: i32 = 1;
const INTEGER: i32 = 3;
const BOOL: i32 = 5;

/// Seeded users: admin (1, staff), user0 (2), user1 (3), user2 (4).
const USER0: i32 = 2;
const USER1: i32 = 3;
/// A topic by user0 in General, five posts.
const TOPIC: i32 = 35;
const STAFF: i32 = 3;

/// Logged in as a seeded user (or not), with its cookies and CSRF token.
struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn send(&mut self, method: Method, path: &str, body: Option<&Value>) -> (u16, Value) {
        let (status, text) = self.send_text(method, path, body).await;
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn send_text(
        &mut self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> (u16, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest");
        if !self.cookies.is_empty() {
            let cookie: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookie.join("; "));
        }
        if let Some(token) = &self.csrf {
            request = request.header("x-csrf-token", token.as_str());
        }
        let body = match body {
            Some(json) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap_or("");
            let (pair, _) = v.split_once(';').unwrap_or((v, ""));
            if let Some((name, value)) = pair.split_once('=') {
                self.cookies.retain(|(n, _)| n != name);
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn anonymous(state: AppState) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client.send(Method::GET, "/session/csrf.json", None).await;
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
    }

    async fn login(state: AppState, user: &str) -> Client {
        let mut client = Client::anonymous(state).await;
        let (status, body) = client
            .send(
                Method::POST,
                "/session.json",
                Some(&json!({"login": user, "password": "password"})),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        let (_, csrf) = client.send(Method::GET, "/session/csrf.json", None).await;
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
    }

    async fn put(&mut self, path: &str, body: Value) -> (u16, Value) {
        self.send(Method::PUT, path, Some(&body)).await
    }

    async fn get(&mut self, path: &str) -> (u16, Value) {
        self.send(Method::GET, path, None).await
    }
}

async fn site(db: &TestDb) -> AppState {
    set_setting(&db.pool, "assign_enabled", BOOL, "t").await;
    state(db.pool.clone(), config(RailsEnv::Test, &[])).await
}

async fn add_to_group(pool: &PgPool, group_id: i32, user_id: i32) {
    sqlx::query(
        "INSERT INTO group_users (group_id, user_id, created_at, updated_at) \
         VALUES ($1, $2, now(), now()) ON CONFLICT DO NOTHING",
    )
    .bind(group_id)
    .bind(user_id)
    .execute(pool)
    .await
    .unwrap();
}

async fn assignments(pool: &PgPool) -> Vec<(i32, String, i32, String, bool)> {
    sqlx::query_as(
        "SELECT target_id, target_type, assigned_to_id, assigned_to_type, active IS TRUE \
         FROM assignments ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn last_post(pool: &PgPool, topic_id: i32) -> (i32, i32, Option<String>, Option<String>) {
    sqlx::query_as(
        "SELECT p.id, p.post_type, p.action_code, \
                (SELECT value FROM post_custom_fields WHERE post_id = p.id AND name = 'action_code_who') \
         FROM posts p WHERE p.topic_id = $1 ORDER BY p.post_number DESC LIMIT 1",
    )
    .bind(topic_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn assign_body(target_type: &str, target_id: i32, username: &str) -> Value {
    json!({"target_type": target_type, "target_id": target_id, "username": username})
}

#[tokio::test]
async fn the_controller_wants_the_plugin_a_user_and_permission() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::login(st.clone(), "admin").await;
    let (status, _) = admin.get("/assign/suggestions.json").await;
    assert_eq!(status, 404, "requires_plugin");

    let st = site(&db).await;
    let mut anon = Client::anonymous(st.clone()).await;
    let (status, body) = anon.get("/assign/suggestions.json").await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error_type"], "not_logged_in");

    // A user outside assign_allowed_on_groups.
    let mut user = Client::login(st.clone(), "user0").await;
    let (status, body) = user
        .put("/assign/assign.json", assign_body("Topic", TOPIC, "admin"))
        .await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error_type"], "invalid_access");
}

#[tokio::test]
async fn assigning_a_topic_records_it_with_a_whisper_and_notifies() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    add_to_group(&db.pool, STAFF, USER1).await;
    let mut admin = Client::login(st.clone(), "admin").await;
    let (status, body) = admin
        .put("/assign/assign.json", assign_body("Topic", TOPIC, "user1"))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"success": "OK"}));
    assert_eq!(
        assignments(&db.pool).await,
        vec![(TOPIC, "Topic".into(), USER1, "User".into(), true)]
    );
    // assigns_public is off: a whisper.
    let (_, post_type, action_code, who) = last_post(&db.pool, TOPIC).await;
    assert_eq!(post_type, 4);
    assert_eq!(action_code.as_deref(), Some("assigned"));
    assert_eq!(who.as_deref(), Some("user1"));
    // The assignee watches the topic (notification_level_when_assigned 3),
    // for plugin_changed.
    let tracking: (i32, Option<i32>) = sqlx::query_as(
        "SELECT notification_level, notifications_reason_id FROM topic_users \
         WHERE user_id = $1 AND topic_id = $2",
    )
    .bind(USER1)
    .bind(TOPIC)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(tracking, (3, Some(9)));

    discourse_rs::jobs::drain(&st).await.unwrap();
    let (notification_type, high_priority, read, data, post_number): (
        i32,
        bool,
        bool,
        String,
        Option<i32>,
    ) = sqlx::query_as(
        "SELECT notification_type, high_priority, read, data, post_number FROM notifications \
         WHERE user_id = $1 AND notification_type = 34",
    )
    .bind(USER1)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(notification_type, 34);
    assert!(high_priority);
    assert!(!read);
    assert_eq!(post_number, Some(1));
    let data: Value = serde_json::from_str(&data).unwrap();
    let assignment_id: i64 = sqlx::query_scalar("SELECT id FROM assignments")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        data,
        json!({
            "message": "discourse_assign.assign_notification",
            "display_username": "admin",
            "topic_title": "Parity fixture: replies and posters",
            "assignment_id": assignment_id,
        })
    );

    // Reassigning to the same user with nothing new is refused.
    let (status, body) = admin
        .put("/assign/assign.json", assign_body("Topic", TOPIC, "user1"))
        .await;
    assert_eq!(status, 400);
    assert_eq!(body["error"], "Topic is already assigned to @user1");

    // Unassigning: the row goes, an unassigned whisper, and the job takes
    // the notification away.
    let (status, body) = admin
        .put(
            "/assign/unassign.json",
            json!({"target_type": "Topic", "target_id": TOPIC}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(assignments(&db.pool).await.is_empty());
    let (_, post_type, action_code, who) = last_post(&db.pool, TOPIC).await;
    assert_eq!(
        (post_type, action_code.as_deref(), who.as_deref()),
        (4, Some("unassigned"), Some("user1"))
    );
    discourse_rs::jobs::drain(&st).await.unwrap();
    let left: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id = $1 AND notification_type = 34",
    )
    .bind(USER1)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn assigning_a_post_marks_the_small_action_with_its_path() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    set_setting(&db.pool, "assigns_public", BOOL, "t").await;
    let post_id: i32 =
        sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = $1 AND post_number = 2")
            .bind(TOPIC)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let mut admin = Client::login(st.clone(), "admin").await;
    let (status, body) = admin
        .put("/assign/assign.json", assign_body("Post", post_id, "admin"))
        .await;
    assert_eq!(status, 200, "{body}");
    let (small_action, post_type, action_code, who) = last_post(&db.pool, TOPIC).await;
    assert_eq!(
        (post_type, action_code.as_deref(), who.as_deref()),
        (3, Some("assigned_to_post"), Some("admin"))
    );
    let fields: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, value FROM post_custom_fields WHERE post_id = $1 \
         AND name IN ('action_code_path', 'action_code_post_id') ORDER BY name",
    )
    .bind(small_action)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        fields,
        vec![
            ("action_code_path".into(), format!("/p/{post_id}")),
            ("action_code_post_id".into(), post_id.to_string()),
        ]
    );
    // Self-assigned: the notification is read and no alert goes out.
    discourse_rs::jobs::drain(&st).await.unwrap();
    let read: bool = sqlx::query_scalar(
        "SELECT read FROM notifications WHERE user_id = 1 AND notification_type = 34",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(read);
}

#[tokio::test]
async fn the_assigner_refuses_with_its_reasons() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    add_to_group(&db.pool, STAFF, USER1).await;
    let mut admin = Client::login(st.clone(), "admin").await;

    // Outside the allowed groups.
    let (status, body) = admin
        .put("/assign/assign.json", assign_body("Topic", TOPIC, "user2"))
        .await;
    assert_eq!(status, 400);
    assert_eq!(
        body["error"],
        "@user2 can't be assigned since they don't belong to assigned allowed groups."
    );

    // A note longer than max_post_length.
    set_setting(&db.pool, "max_post_length", INTEGER, "10").await;
    let (status, body) = admin
        .put(
            "/assign/assign.json",
            json!({"target_type": "Topic", "target_id": TOPIC, "username": "user1",
                   "note": "a note longer than ten"}),
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(
        body["error"],
        "Assignment note must be 10 characters or fewer."
    );

    // max_assigned_topics: one, already taken.
    set_setting(&db.pool, "max_assigned_topics", INTEGER, "1").await;
    let (status, _) = admin
        .put("/assign/assign.json", assign_body("Topic", 38, "user1"))
        .await;
    assert_eq!(status, 200);
    let (status, body) = admin
        .put("/assign/assign.json", assign_body("Topic", TOPIC, "user1"))
        .await;
    assert_eq!(status, 400);
    assert_eq!(
        body["error"],
        "@user1 has already reached the maximum number of assigned topics (1)."
    );
    // Assigning yourself has no limit.
    let (status, _) = admin
        .put("/assign/assign.json", assign_body("Topic", TOPIC, "admin"))
        .await;
    assert_eq!(status, 200);

    // A message the assignee isn't in (admin's to user1 and user2).
    add_to_group(&db.pool, STAFF, USER0).await;
    let (status, body) = admin
        .put("/assign/assign.json", assign_body("Topic", 43, "user0"))
        .await;
    assert_eq!(status, 400);
    assert_eq!(
        body["error"],
        "@user0 can't be assigned because they don't have access to this PM. \
         You can grant @user0 access by inviting them to this PM."
    );

    // Neither a user nor a group.
    let (status, body) = admin
        .put(
            "/assign/assign.json",
            json!({"target_type": "Topic", "target_id": TOPIC}),
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(body["error_type"], "invalid_parameters");
    // A user who doesn't exist, a target that doesn't.
    let (status, _) = admin
        .put(
            "/assign/assign.json",
            assign_body("Topic", TOPIC, "nobody_here"),
        )
        .await;
    assert_eq!(status, 404);
    let (status, _) = admin
        .put(
            "/assign/assign.json",
            assign_body("Topic", 999_999, "admin"),
        )
        .await;
    assert_eq!(status, 404);
    let (status, _) = admin
        .put("/assign/assign.json", assign_body("Category", 1, "admin"))
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn groups_are_assigned_by_their_assignable_level() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    let mut admin = Client::login(st.clone(), "admin").await;
    let body = json!({"target_type": "Topic", "target_id": TOPIC, "group_name": "staff"});
    let (status, res) = admin.put("/assign/assign.json", body.clone()).await;
    assert_eq!(status, 400);
    assert_eq!(
        res["error"],
        "@staff can't be assigned since they don't belong to assigned allowed groups."
    );
    sqlx::query("UPDATE groups SET assignable_level = 99 WHERE id = $1")
        .bind(STAFF)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, res) = admin.put("/assign/assign.json", body).await;
    assert_eq!(status, 200, "{res}");
    let (_, _, action_code, who) = last_post(&db.pool, TOPIC).await;
    assert_eq!(
        (action_code.as_deref(), who.as_deref()),
        (Some("assigned_group"), Some("staff"))
    );
    // Every member hears of it; the assigner's own is read.
    discourse_rs::jobs::drain(&st).await.unwrap();
    let rows: Vec<(i32, bool, String)> = sqlx::query_as(
        "SELECT user_id, read, data FROM notifications WHERE notification_type = 34 ORDER BY user_id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert!(rows.iter().any(|(u, read, _)| *u == 1 && *read));
    let data: Value = serde_json::from_str(&rows[0].2).unwrap();
    assert_eq!(
        data["message"],
        "discourse_assign.assign_group_notification"
    );
    assert_eq!(data["display_username"], "staff");

    // The group's members, by assignments (none of their own here).
    let (status, res) = admin.get("/assign/members/staff.json").await;
    assert_eq!(status, 200, "{res}");
    assert_eq!(res["group_assignment_count"], 1);
    assert_eq!(res["assignment_count"], 1);
    assert_eq!(res["members"], json!([]));
}

#[tokio::test]
async fn suggestions_offer_the_viewer_then_recent_assignees_and_groups() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    add_to_group(&db.pool, STAFF, USER1).await;
    sqlx::query("UPDATE groups SET assignable_level = 99 WHERE id = $1")
        .bind(STAFF)
        .execute(&db.pool)
        .await
        .unwrap();
    let mut admin = Client::login(st.clone(), "admin").await;
    let (status, _) = admin
        .put("/assign/assign.json", assign_body("Topic", TOPIC, "user1"))
        .await;
    assert_eq!(status, 200);
    let (status, body) = admin.get("/assign/suggestions.json").await;
    assert_eq!(status, 200, "{body}");
    let names: Vec<&str> = body["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["username"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["admin", "user1"]);
    // The groups whose members may be assigned, and those that may be.
    assert_eq!(body["assign_allowed_on_groups"], json!(["staff"]));
    assert!(
        body["assign_allowed_for_groups"]
            .as_array()
            .unwrap()
            .contains(&json!("staff"))
    );

    // On holiday: left out.
    sqlx::query(
        "INSERT INTO user_custom_fields (user_id, name, value, created_at, updated_at) \
         VALUES ($1, 'on_holiday', 't', now(), now())",
    )
    .bind(USER1)
    .execute(&db.pool)
    .await
    .unwrap();
    let (_, body) = admin.get("/assign/suggestions.json").await;
    assert_eq!(body["suggestions"].as_array().unwrap().len(), 1);

    // For a target: its type and id, a target that is gone not found.
    let (status, _) = admin
        .get(&format!(
            "/assign/suggestions.json?target_type=Topic&target_id={TOPIC}"
        ))
        .await;
    assert_eq!(status, 200);
    let (status, _) = admin
        .get("/assign/suggestions.json?target_type=Topic&target_id=999999")
        .await;
    assert_eq!(status, 404);
    // bf55a44c has no search action.
    let (status, _) = admin.get("/assign/search.json?term=user").await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn category_groups_may_assign_in_their_category_only() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    // Trust level 1 (group 11) may assign in General (4).
    let category: Option<i32> = sqlx::query_scalar("SELECT category_id FROM topics WHERE id = $1")
        .bind(TOPIC)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO category_custom_fields (category_id, name, value, created_at, updated_at) \
         VALUES ($1, 'additional_assign_allowed_on_groups', '11', now(), now())",
    )
    .bind(category)
    .execute(&db.pool)
    .await
    .unwrap();
    let mut user0 = Client::login(st.clone(), "user0").await;
    let (status, body) = user0
        .put("/assign/assign.json", assign_body("Topic", TOPIC, "user0"))
        .await;
    assert_eq!(status, 200, "{body}");
    // Elsewhere (a topic in another category): refused.
    let other: i32 = sqlx::query_scalar(
        "SELECT id FROM topics WHERE archetype = 'regular' AND deleted_at IS NULL \
         AND category_id IS DISTINCT FROM $1 AND visible ORDER BY id LIMIT 1",
    )
    .bind(category)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let (status, body) = user0
        .put("/assign/assign.json", assign_body("Topic", other, "user0"))
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["error"],
        "You can only assign topics in categories allowed for your groups."
    );
    let _ = (USER0, STRING);
}

async fn assign_topic_to(admin: &mut Client, topic_id: i32, username: &str) {
    let (status, body) = admin
        .put(
            "/assign/assign.json",
            assign_body("Topic", topic_id, username),
        )
        .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn serializers_carry_the_assignee_for_those_who_may_see_it() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    add_to_group(&db.pool, STAFF, USER1).await;
    let mut admin = Client::login(st.clone(), "admin").await;
    assign_topic_to(&mut admin, TOPIC, "user1").await;
    let post_id: i32 =
        sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = $1 AND post_number = 2")
            .bind(TOPIC)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let (status, body) = admin
        .put(
            "/assign/assign.json",
            json!({"target_type": "Post", "target_id": post_id, "username": "admin", "note": "look"}),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    // The topic view: the helpers' shapes.
    let (status, view) = admin.get(&format!("/t/{TOPIC}.json")).await;
    assert_eq!(status, 200, "{view}");
    assert_eq!(view["can_assign"], true);
    assert_eq!(view["assigned_to_user"]["username"], "user1");
    assert!(view["assigned_to_user"].get("id").is_none());
    assert_eq!(view["assignment_note"], Value::Null);
    let indirect = &view["indirectly_assigned_to"][post_id.to_string()];
    assert_eq!(indirect["assigned_to"]["username"], "admin");
    assert_eq!(indirect["post_number"], 2);
    assert_eq!(indirect["assignment_note"], "look");
    // The post: BasicUserSerializer, the note.
    let post = view["post_stream"]["posts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == post_id)
        .unwrap()
        .clone();
    assert_eq!(post["can_assign"], true);
    assert_eq!(post["assigned_to_user"]["id"], 1);
    assert_eq!(post["assignment_note"], "look");

    // Topic lists: can_assign, then the assignees.
    let (_, latest) = admin.get("/latest.json").await;
    let topic = latest["topic_list"]["topics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == TOPIC)
        .unwrap()
        .clone();
    assert_eq!(topic["can_assign"], true);
    assert_eq!(topic["assigned_to_user"]["id"], USER1);
    assert_eq!(
        topic["indirectly_assigned_to"][post_id.to_string()]["assignment_note"],
        Value::Null
    );

    // Someone who may not assign sees none of it, unless assigns_public.
    let mut user2 = Client::login(st.clone(), "user2").await;
    let (_, view) = user2.get(&format!("/t/{TOPIC}.json")).await;
    assert_eq!(view["can_assign"], false);
    assert!(view.get("assigned_to_user").is_none());
    let mut anon = Client::anonymous(st.clone()).await;
    let (_, view) = anon.get(&format!("/t/{TOPIC}.json")).await;
    assert!(view.get("can_assign").is_none());
    assert!(view.get("assigned_to_user").is_none());
    set_setting(&db.pool, "assigns_public", BOOL, "t").await;
    let (_, view) = anon.get(&format!("/t/{TOPIC}.json")).await;
    assert_eq!(view["assigned_to_user"]["username"], "user1");

    // The current user's keys.
    let (_, current) = admin.get("/session/current.json").await;
    let current = &current["current_user"];
    assert_eq!(current["can_assign"], true);
    assert_eq!(current["can_assign_globally"], true);
    assert_eq!(current["never_auto_track_topics"], false);
    assert_eq!(
        current["user_option"]["notification_level_when_assigned"],
        "watch_topic"
    );
}

#[tokio::test]
async fn lists_filter_and_gather_assignments() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    add_to_group(&db.pool, STAFF, USER1).await;
    let mut admin = Client::login(st.clone(), "admin").await;
    assign_topic_to(&mut admin, TOPIC, "user1").await;

    let ids = |list: &Value| -> Vec<i64> {
        list["topic_list"]["topics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_i64().unwrap())
            .collect()
    };
    let (status, list) = admin.get("/latest.json?assigned=user1").await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(ids(&list), vec![i64::from(TOPIC)]);
    let (_, list) = admin.get("/latest.json?assigned=*").await;
    assert_eq!(ids(&list), vec![i64::from(TOPIC)]);
    let (_, list) = admin.get("/latest.json?assigned=nobody").await;
    assert!(!ids(&list).contains(&i64::from(TOPIC)));
    // Not for someone who may not see assignments: the list as it is.
    let mut user2 = Client::login(st.clone(), "user2").await;
    let (_, list) = user2.get("/latest.json?assigned=user1").await;
    assert!(ids(&list).len() > 1);

    let (status, list) = admin.get("/topics/messages-assigned/user1.json").await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(ids(&list), vec![i64::from(TOPIC)]);
    let (status, _) = user2.get("/topics/messages-assigned/user1.json").await;
    assert_eq!(status, 403);

    // A group's: refused until its assigned tab shows, then its members'.
    let (status, _) = admin.get("/topics/group-topics-assigned/staff.json").await;
    assert_eq!(status, 403);
    sqlx::query("UPDATE groups SET assignable_level = 99 WHERE id = $1")
        .bind(STAFF)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, list) = admin.get("/topics/group-topics-assigned/staff.json").await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(ids(&list), vec![i64::from(TOPIC)]);
    let (_, list) = admin
        .get("/topics/group-topics-assigned/staff.json?direct=true")
        .await;
    assert!(ids(&list).is_empty());

    // A message assigned to its participant: their assigned messages.
    let (status, body) = admin
        .put("/assign/assign.json", assign_body("Topic", 43, "user1"))
        .await;
    assert_eq!(status, 200, "{body}");
    let mut user1 = Client::login(st.clone(), "user1").await;
    let (status, list) = user1
        .get("/topics/private-messages-assigned/user1.json")
        .await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(ids(&list), vec![43]);

    // Search's advanced filters.
    let (status, found) = admin.get("/search.json?q=Parity%20in:assigned").await;
    assert_eq!(status, 200, "{found}");
    let topics: Vec<i64> = found["topics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect();
    assert!(topics.contains(&i64::from(TOPIC)));
    assert!(topics.iter().all(|t| *t == i64::from(TOPIC) || *t == 43));
}

#[tokio::test]
async fn closing_unassigns_and_opening_reassigns() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    set_setting(&db.pool, "unassign_on_close", BOOL, "t").await;
    set_setting(&db.pool, "reassign_on_open", BOOL, "t").await;
    let mut admin = Client::login(st.clone(), "admin").await;
    assign_topic_to(&mut admin, TOPIC, "admin").await;
    let (status, body) = admin
        .put(
            &format!("/t/{TOPIC}/status"),
            json!({"status": "closed", "enabled": "true"}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        assignments(&db.pool).await,
        vec![(TOPIC, "Topic".into(), 1, "User".into(), false)]
    );
    // Silent: no unassigned small action.
    let (_, _, action_code, _) = last_post(&db.pool, TOPIC).await;
    assert_ne!(action_code.as_deref(), Some("unassigned"));
    let (status, body) = admin
        .put(
            &format!("/t/{TOPIC}/status"),
            json!({"status": "closed", "enabled": "false"}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        assignments(&db.pool).await,
        vec![(TOPIC, "Topic".into(), 1, "User".into(), true)]
    );
}

#[tokio::test]
async fn statuses_are_validated_and_changes_recorded() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    set_setting(&db.pool, "enable_assign_status", BOOL, "t").await;
    let mut admin = Client::login(st.clone(), "admin").await;
    // The default status, the first.
    assign_topic_to(&mut admin, TOPIC, "admin").await;
    let status: Option<String> = sqlx::query_scalar("SELECT status FROM assignments")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(status.as_deref(), Some("New"));
    // A new status for the same assignee: a status_change small action.
    let (code, body) = admin
        .put(
            "/assign/assign.json",
            json!({"target_type": "Topic", "target_id": TOPIC, "username": "admin", "status": "Done"}),
        )
        .await;
    assert_eq!(code, 200, "{body}");
    let (post_id, _, action_code, _) = last_post(&db.pool, TOPIC).await;
    assert_eq!(action_code.as_deref(), Some("status_change"));
    let raw: String = sqlx::query_scalar("SELECT raw FROM posts WHERE id = $1")
        .bind(post_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(raw, "New → Done");
    let (_, view) = admin.get(&format!("/t/{TOPIC}.json")).await;
    assert_eq!(view["assignment_status"], "Done");
    // Not one of assign_statuses.
    let (code, _) = admin
        .put(
            "/assign/assign.json",
            json!({"target_type": "Topic", "target_id": TOPIC, "username": "admin", "status": "Bogus"}),
        )
        .await;
    assert_eq!(code, 422);
}

#[tokio::test]
async fn group_membership_brings_and_takes_notifications() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    sqlx::query(
        "INSERT INTO groups (id, name, assignable_level, created_at, updated_at) \
         VALUES (9001, 'helpers', 99, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    add_to_group(&db.pool, 9001, 1).await;
    let mut admin = Client::login(st.clone(), "admin").await;
    let (status, body) = admin
        .put(
            "/assign/assign.json",
            json!({"target_type": "Topic", "target_id": TOPIC, "group_name": "helpers"}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    discourse_rs::jobs::drain(&st).await.unwrap();
    let count = |pool: PgPool, user: i32| async move {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM notifications WHERE user_id = $1 AND notification_type = 34",
        )
        .bind(user)
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    assert_eq!(count(db.pool.clone(), USER1).await, 0);
    // Added: a notification of the group's assignment.
    let (status, body) = admin
        .put(
            "/admin/groups/9001/members.json",
            json!({"usernames": "user1"}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    discourse_rs::jobs::drain(&st).await.unwrap();
    assert_eq!(count(db.pool.clone(), USER1).await, 1);
    // Removed: it goes.
    let (status, body) = admin
        .send(
            Method::DELETE,
            "/admin/groups/9001/members.json",
            Some(&json!({"user_id": USER1})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(count(db.pool.clone(), USER1).await, 0);
}

#[tokio::test]
async fn reminders_message_users_with_pending_assignments() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    set_setting(&db.pool, "remind_assigns_frequency", INTEGER, "1440").await;
    let mut admin = Client::login(st.clone(), "admin").await;
    assign_topic_to(&mut admin, TOPIC, "admin").await;
    assign_topic_to(&mut admin, 38, "admin").await;
    // Untouched for longer than the frequency.
    sqlx::query("UPDATE assignments SET updated_at = now() - interval '2 days'")
        .execute(&db.pool)
        .await
        .unwrap();
    discourse_rs::jobs::run_scheduled(&st, "enqueue_reminders")
        .await
        .unwrap();
    discourse_rs::jobs::drain(&st).await.unwrap();
    let (title, raw): (String, String) = sqlx::query_as(
        "SELECT t.title, p.raw FROM topics t JOIN posts p ON p.topic_id = t.id \
         JOIN topic_custom_fields f ON f.topic_id = t.id AND f.name = 'assigns_reminder' \
         WHERE t.archetype = 'private_message' ORDER BY t.id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(title, "You have 2 pending assignments");
    assert!(raw.contains("/u/admin/activity/assigned"), "{raw}");
    assert!(raw.contains("### Newest"), "{raw}");
    assert!(raw.contains("This reminder will be sent daily"), "{raw}");
    let reminded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_custom_fields WHERE user_id = 1 AND name = 'last_reminded_at'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(reminded, 1);
}

impl Client {
    /// A page as a browser asks for it.
    async fn page(&mut self, path: &str) -> (u16, String) {
        let mut request = Request::builder()
            .method(Method::GET)
            .uri(path)
            .header(header::HOST, "test.localhost")
            .header(header::ACCEPT, "text/html");
        if !self.cookies.is_empty() {
            let cookie: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookie.join("; "));
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }
}

#[tokio::test]
async fn the_topic_page_draws_the_assignees_and_the_controls() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    // Staff see whispers, the assign small actions among them.
    set_setting(&db.pool, "whispers_allowed_groups", 20, "3").await;
    let mut admin = Client::login(st.clone(), "admin").await;
    // Nothing assigned: the footer's Assign button and the modal.
    let (status, html) = admin
        .page(&format!("/t/parity-fixture-replies-and-posters/{TOPIC}"))
        .await;
    assert_eq!(status, 200);
    assert!(html.contains("id=\"topic-footer-button-assign\""), "{html}");
    assert!(html.contains("id=\"assign-modal-template\""));
    assert!(html.contains("discourse-assign.js"));

    let post_id: i32 =
        sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = $1 AND post_number = 2")
            .bind(TOPIC)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assign_topic_to(&mut admin, TOPIC, "admin").await;
    let (status, body) = admin
        .put("/assign/assign.json", assign_body("Post", post_id, "admin"))
        .await;
    assert_eq!(status, 200, "{body}");
    let (_, html) = admin
        .page(&format!("/t/parity-fixture-replies-and-posters/{TOPIC}"))
        .await;
    // Under the first post: the topic's assignee and the post's.
    assert!(html.contains("<p class=\"assigned-to\">"), "{html}");
    assert!(html.contains("assigned-to--user"));
    assert!(html.contains(&format!("href=\"/p/{post_id}\"")));
    // The reassign menu in place of the Assign button.
    assert!(html.contains("id=\"topic-footer-dropdown-reassign\""));
    assert!(!html.contains("id=\"topic-footer-button-assign\""));
    // The post's own: unassign in its menu, its assignee under it.
    assert!(html.contains("post-action-menu__unassign-post"));
    assert!(html.contains("assigned-to-username"));
    // The whispered small actions, drawn as small actions with their icons.
    assert!(html.contains("small-action onscreen-post private-assign"));
    assert!(html.contains("#user-plus"));

    // Someone who may not assign sees none of it.
    let mut user2 = Client::login(st.clone(), "user2").await;
    let (_, html) = user2
        .page(&format!("/t/parity-fixture-replies-and-posters/{TOPIC}"))
        .await;
    assert!(!html.contains("assigned-to"));
    assert!(!html.contains("topic-footer-button-assign"));

    // The user's Assigned tab.
    let (status, html) = admin.page("/u/admin/activity/assigned").await;
    assert_eq!(status, 200, "{html}");
    assert!(html.contains("assigned-list"));
    assert!(
        html.contains(&format!("/t/parity-fixture-replies-and-posters/{TOPIC}")),
        "{html}"
    );
}

#[tokio::test]
async fn bulk_actions_assign_and_unassign() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    let mut admin = Client::login(st.clone(), "admin").await;
    let (status, body) = admin
        .put(
            "/topics/bulk.json",
            json!({"topic_ids": [TOPIC, 38], "operation": {"type": "assign", "username": "admin"}}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["topic_ids"], json!([TOPIC, 38]));
    assert_eq!(assignments(&db.pool).await.len(), 2);
    // Without an assignee.
    let (status, _) = admin
        .put(
            "/topics/bulk.json",
            json!({"topic_ids": [TOPIC], "operation": {"type": "assign"}}),
        )
        .await;
    assert_eq!(status, 400);
    let (status, body) = admin
        .put(
            "/topics/bulk.json",
            json!({"topic_ids": [TOPIC, 38], "operation": {"type": "unassign"}}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(assignments(&db.pool).await.is_empty());
    // Someone who may not assign.
    let mut user2 = Client::login(st.clone(), "user2").await;
    let (status, _) = user2
        .put(
            "/topics/bulk.json",
            json!({"topic_ids": [TOPIC], "operation": {"type": "assign", "username": "admin"}}),
        )
        .await;
    assert_eq!(status, 403);
}
