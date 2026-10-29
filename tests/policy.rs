//! discourse-policy, ported from the plugin's specs: a `[policy]` kept in
//! step with its post after processing (plugin_spec, pretty_text_spec),
//! who may add or change one (post_validator_spec), accepting and the
//! user lists (policy_controller_spec), the post serializer's keys
//! (post_serializer_spec), the reminders and renewals
//! (check_policy_spec), the current user's and user options' keys, the
//! policy email and the policy drawn in the post.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, header};
use chrono::{TimeZone, Utc};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn send(&mut self, method: Method, path: &str, body: &str) -> (u16, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest")
            .header(header::ACCEPT, "application/json");
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
        if !body.is_empty() {
            request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap();
            let pair = v.split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn json(&mut self, method: Method, path: &str, body: &str) -> (u16, Value) {
        let (status, text) = self.send(method, path, body).await;
        let value = serde_json::from_str(&text).unwrap_or_else(|_| panic!("{status} {text}"));
        (status, value)
    }

    async fn anonymous(state: AppState) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client.json(Method::GET, "/session/csrf.json", "").await;
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client::anonymous(state).await;
        let (status, body) = client
            .send(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        client
    }

    /// The page as a browser asks for it.
    async fn page(&mut self, path: &str) -> String {
        let cookie: Vec<String> = self
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let request = Request::get(path)
            .header(header::HOST, "test.localhost")
            .header(header::COOKIE, cookie.join("; "));
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

async fn setup() -> (TestDb, AppState) {
    let db = TestDb::new().await;
    common::set_setting(&db.pool, "policy_enabled", 5, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    (db, st)
}

/// A group with these members (`Fabricate(:group)` and `group.add`).
async fn group(pool: &PgPool, name: &str, members: &[i32]) -> i32 {
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO groups (name, created_at, updated_at) VALUES ($1, now(), now()) RETURNING id",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap();
    for user_id in members {
        add_member(pool, id, *user_id, false).await;
    }
    id
}

async fn add_member(pool: &PgPool, group_id: i32, user_id: i32, owner: bool) {
    sqlx::query(
        "INSERT INTO group_users (group_id, user_id, owner, created_at, updated_at) \
         VALUES ($1, $2, $3, now(), now()) ON CONFLICT (group_id, user_id) DO UPDATE SET owner = $3",
    )
    .bind(group_id)
    .bind(user_id)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
}

/// A new topic by `client`, its jobs run: the first post's id.
async fn create_topic(st: &AppState, client: &mut Client, title: &str, raw: &str) -> i32 {
    let body = format!(
        "title={}&raw={}&category=4",
        urlencode(title),
        urlencode(raw)
    );
    let (status, created) = client.json(Method::POST, "/posts", &body).await;
    assert_eq!(status, 200, "{created}");
    discourse_rs::jobs::drain(st).await.unwrap();
    created["id"].as_i64().unwrap() as i32
}

async fn revise(st: &AppState, client: &mut Client, post_id: i32, raw: &str) -> (u16, Value) {
    let out = client
        .json(
            Method::PUT,
            &format!("/posts/{post_id}.json"),
            &format!("post%5Braw%5D={}", urlencode(raw)),
        )
        .await;
    discourse_rs::jobs::drain(st).await.unwrap();
    out
}

#[derive(Debug, sqlx::FromRow)]
struct PolicyRow {
    version: Option<String>,
    reminder: Option<String>,
    last_reminded_at: Option<chrono::NaiveDateTime>,
    renew_days: Option<i32>,
    renew_interval: Option<i32>,
    renew_start: Option<chrono::NaiveDateTime>,
    next_renew_at: Option<chrono::NaiveDateTime>,
    add_users_to_group: Option<i32>,
    last_bumped_at: Option<chrono::NaiveDateTime>,
}

async fn policy_of(pool: &PgPool, post_id: i32) -> Option<PolicyRow> {
    sqlx::query_as(
        "SELECT version, reminder, last_reminded_at, renew_days, renew_interval, renew_start, \
                next_renew_at, add_users_to_group, last_bumped_at \
         FROM post_policies WHERE post_id = $1",
    )
    .bind(i64::from(post_id))
    .fetch_optional(pool)
    .await
    .unwrap()
}

async fn policy_groups(pool: &PgPool, post_id: i32) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT g.name FROM post_policy_groups ppg JOIN post_policies pp ON pp.id = ppg.post_policy_id \
         JOIN groups g ON g.id = ppg.group_id WHERE pp.post_id = $1 ORDER BY g.name",
    )
    .bind(i64::from(post_id))
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn has_field(pool: &PgPool, post_id: i32) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM post_custom_fields WHERE post_id = $1 AND name = 'HasPolicy')",
    )
    .bind(post_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// `post.post_policy.accepted_by.map(&:id)`
async fn accepted_by(pool: &PgPool, post_id: i32) -> Vec<i32> {
    let mut conn = pool.acquire().await.unwrap();
    let policy = discourse_rs::plugins::policy::PostPolicy::for_post(&mut conn, post_id)
        .await
        .unwrap()
        .unwrap();
    policy
        .users(
            &mut conn,
            discourse_rs::plugins::policy::Users::Accepted,
            None,
            0,
        )
        .await
        .unwrap()
}

/// `PolicyUser.add!(user, post.post_policy)`
async fn accept(pool: &PgPool, post_id: i32, user_id: i32) {
    let mut conn = pool.acquire().await.unwrap();
    let policy = discourse_rs::plugins::policy::PostPolicy::for_post(&mut conn, post_id)
        .await
        .unwrap()
        .unwrap();
    discourse_rs::plugins::policy::accept(&mut conn, &policy, user_id)
        .await
        .unwrap();
}

/// The first post as posts#by_number serializes it for `client` (the
/// post serializer, as /posts/:id.json would).
async fn post_json(client: &mut Client, post_id: i32) -> Value {
    let topic_id: i32 = sqlx::query_scalar("SELECT topic_id FROM posts WHERE id = $1")
        .bind(post_id)
        .fetch_one(&client.state.pool)
        .await
        .unwrap();
    let (status, post) = client
        .json(
            Method::GET,
            &format!("/posts/by_number/{topic_id}/1.json"),
            "",
        )
        .await;
    assert_eq!(status, 200, "{post}");
    post
}

fn ids(users: &Value) -> Vec<i64> {
    let mut ids: Vec<i64> = users
        .as_array()
        .unwrap_or_else(|| panic!("a list: {users}"))
        .iter()
        .map(|u| u["id"].as_i64().unwrap())
        .collect();
    ids.sort_unstable();
    ids
}

#[tokio::test]
async fn a_policy_is_kept_with_its_post_and_goes_with_it() {
    let (db, st) = setup().await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let raw = "[policy group=staff reminder=weekly]\nI always open **doors**!\n[/policy]";
    let post_id = create_topic(&st, &mut admin, "A policy to accept here", raw).await;
    let policy = policy_of(&db.pool, post_id).await.expect("the policy");
    assert_eq!(policy_groups(&db.pool, post_id).await, ["staff"]);
    assert!(has_field(&db.pool, post_id).await);
    assert_eq!(policy.reminder.as_deref(), Some("weekly"));
    assert!(policy.last_reminded_at.is_some());
    // Version 1 is never stored.
    assert_eq!(policy.version, None);
    assert_eq!(policy.last_bumped_at, None);

    let (status, body) = revise(
        &st,
        &mut admin,
        post_id,
        "i am new raw, long enough to save",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(policy_of(&db.pool, post_id).await.is_none());
    assert!(!has_field(&db.pool, post_id).await);
}

#[tokio::test]
async fn bumping_the_version_resets_who_accepted() {
    let (db, st) = setup().await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let raw = "[policy group=staff reminder=weekly]\nI always open **doors**!\n[/policy]";
    let post_id = create_topic(&st, &mut admin, "A policy to accept here", raw).await;
    accept(&db.pool, post_id, 1).await;
    assert_eq!(accepted_by(&db.pool, post_id).await, [1]);

    let raw = "[policy groups=trust_level_1,trust_level_0 version=2 reminder=weekly]\nI always open **doors**!\n[/policy]";
    let (status, body) = revise(&st, &mut admin, post_id, raw).await;
    assert_eq!(status, 200, "{body}");
    assert!(accepted_by(&db.pool, post_id).await.is_empty());
    assert_eq!(
        policy_groups(&db.pool, post_id).await,
        ["trust_level_0", "trust_level_1"]
    );
    let policy = policy_of(&db.pool, post_id).await.unwrap();
    assert_eq!(policy.version.as_deref(), Some("2"));
    assert!(policy.last_bumped_at.is_some());
}

#[tokio::test]
async fn renew_start_removed_clears_the_next_renewal() {
    let (db, st) = setup().await;
    group(&db.pool, "team", &[3, 4]).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let start = (Utc::now() + chrono::Duration::days(1)).date_naive();
    let raw = format!(
        "[policy group=team renew=\"10\" renew-start=\"{}\"]\nHere's the new policy\n[/policy]",
        start.format("%F")
    );
    let post_id = create_topic(&st, &mut admin, "A policy that renews", &raw).await;
    let policy = policy_of(&db.pool, post_id).await.unwrap();
    assert_eq!(policy.renew_days, Some(10));
    assert_eq!(policy.renew_start.map(|d| d.date()), Some(start));
    assert_eq!(policy.next_renew_at.map(|d| d.date()), Some(start));

    let raw = "[policy group=team renew=\"10\"]\nHere's the new policy\n[/policy]";
    revise(&st, &mut admin, post_id, raw).await;
    let policy = policy_of(&db.pool, post_id).await.unwrap();
    assert_eq!(policy.renew_days, Some(10));
    assert_eq!(policy.renew_start, None);
    assert_eq!(policy.next_renew_at, None);

    let raw = "[policy group=team renew=quarterly]\nHere's the new policy\n[/policy]";
    revise(&st, &mut admin, post_id, raw).await;
    let policy = policy_of(&db.pool, post_id).await.unwrap();
    assert_eq!(policy.renew_days, None);
    assert_eq!(policy.renew_interval, Some(1));
}

#[tokio::test]
async fn a_quoted_policy_is_not_kept() {
    let (db, st) = setup().await;
    group(&db.pool, "team", &[3]).await;
    group(&db.pool, "target", &[]).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let raw = "[quote=\"someone, post:1, topic:1\"]\n[policy group=team add-users-to-group=target]\nHere's the new policy\n[/policy]\n[/quote]";
    let post_id = create_topic(&st, &mut admin, "A quoted policy here", raw).await;
    assert!(policy_of(&db.pool, post_id).await.is_none());
}

#[tokio::test]
async fn the_group_accepting_adds_to_is_kept_only_when_the_author_manages_it() {
    let (db, st) = setup().await;
    group(&db.pool, "team", &[3]).await;
    let target = group(&db.pool, "target", &[]).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let raw = "[policy group=team add-users-to-group=target]\nHere's the new policy\n[/policy]";
    let post_id = create_topic(&st, &mut admin, "A policy adding members", raw).await;
    // An admin manages any group that is not automatic.
    assert_eq!(
        policy_of(&db.pool, post_id)
            .await
            .unwrap()
            .add_users_to_group,
        Some(target)
    );
    let raw = "[policy group=team add-users-to-group=admins]\nHere's the new policy\n[/policy]";
    revise(&st, &mut admin, post_id, raw).await;
    assert_eq!(
        policy_of(&db.pool, post_id)
            .await
            .unwrap()
            .add_users_to_group,
        None
    );
    let raw = "[policy group=team add-users-to-group=nonexistent_group_xyz]\nHere's the new policy\n[/policy]";
    revise(&st, &mut admin, post_id, raw).await;
    assert_eq!(
        policy_of(&db.pool, post_id)
            .await
            .unwrap()
            .add_users_to_group,
        None
    );
}

#[tokio::test]
async fn only_members_of_the_allowed_groups_add_or_change_policies() {
    let (db, st) = setup().await;
    let policy_group = group(&db.pool, "policy_group", &[3]).await;
    common::set_setting(
        &db.pool,
        "create_policy_allowed_groups",
        20,
        &policy_group.to_string(),
    )
    .await;
    let raw = "[policy group=policy_group]\nI agree\n[/policy]";
    // user2 is not in the group: no policy of theirs.
    let mut user2 = Client::logged_in(st.clone(), "user2").await;
    let (status, body) = user2
        .json(
            Method::POST,
            "/posts",
            &format!(
                "title={}&raw={}&category=4",
                urlencode("A policy of my own here"),
                urlencode(raw)
            ),
        )
        .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(
        body["errors"],
        serde_json::json!(["You do not have permission to modify policies in this post."])
    );

    // user1 is: theirs is kept. user3 (trust level 4, who may edit it) is
    // not, nor staff (the setting's mandatory admins and moderators).
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let post_id = create_topic(&st, &mut user1, "A policy of my own here", raw).await;
    assert!(policy_of(&db.pool, post_id).await.is_some());
    let mut user3 = Client::logged_in(st.clone(), "user3").await;
    let (status, body) = revise(
        &st,
        &mut user3,
        post_id,
        "Policy removed, long enough to save",
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert!(policy_of(&db.pool, post_id).await.is_some());

    // Changing what is outside the policy is anyone's.
    let raw2 = format!("New intro\n\n{raw}");
    let (status, body) = revise(&st, &mut user3, post_id, &raw2).await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn members_accept_and_revoke() {
    let (db, st) = setup().await;
    let team = group(&db.pool, "team", &[3, 4]).await;
    let target = group(&db.pool, "target", &[]).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let raw = "[policy group=team add-users-to-group=target]\nI always open **doors**!\n[/policy]";
    let post_id = create_topic(&st, &mut admin, "A policy to accept here", raw).await;
    let _ = team;

    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, body) = user1
        .json(
            Method::PUT,
            "/policy/accept.json",
            &format!("post_id={post_id}"),
        )
        .await;
    assert_eq!(
        (status, &body["success"]),
        (200, &Value::from("OK")),
        "{body}"
    );
    assert_eq!(accepted_by(&db.pool, post_id).await, [3]);
    let members: Vec<i32> =
        sqlx::query_scalar("SELECT user_id FROM group_users WHERE group_id = $1 ORDER BY user_id")
            .bind(target)
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(members, [3]);

    let mut user2 = Client::logged_in(st.clone(), "user2").await;
    user2
        .json(
            Method::PUT,
            "/policy/accept.json",
            &format!("post_id={post_id}"),
        )
        .await;
    assert_eq!(accepted_by(&db.pool, post_id).await, [3, 4]);
    let (status, _) = user2
        .json(
            Method::PUT,
            "/policy/unaccept.json",
            &format!("post_id={post_id}"),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(accepted_by(&db.pool, post_id).await, [3]);

    user1
        .json(
            Method::PUT,
            "/policy/unaccept.json",
            &format!("post_id={post_id}"),
        )
        .await;
    let members: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM group_users WHERE group_id = $1")
        .bind(target)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(members, 0);

    // Not in the policy's group.
    let mut user3 = Client::logged_in(st.clone(), "user3").await;
    let (status, body) = user3
        .json(
            Method::PUT,
            "/policy/accept.json",
            &format!("post_id={post_id}"),
        )
        .await;
    assert_eq!(status, 422);
    assert_eq!(
        body["errors"],
        serde_json::json!(["User does not exist in group"])
    );

    // An add-to group the author can no longer manage.
    sqlx::query("UPDATE post_policies SET add_users_to_group = 1 WHERE post_id = $1")
        .bind(i64::from(post_id))
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body) = user1
        .json(
            Method::PUT,
            "/policy/accept.json",
            &format!("post_id={post_id}"),
        )
        .await;
    assert_eq!(status, 422);
    assert_eq!(
        body["errors"],
        serde_json::json!([
            "This policy cannot be accepted at this time. Please contact a site administrator."
        ])
    );

    // A post without one.
    let other = create_topic(
        &st,
        &mut admin,
        "No policy in this one",
        "Just words here, nothing to accept.",
    )
    .await;
    let (status, body) = user1
        .json(
            Method::PUT,
            "/policy/accept.json",
            &format!("post_id={other}"),
        )
        .await;
    assert_eq!(status, 422);
    assert_eq!(
        body["errors"],
        serde_json::json!(["No policy exists for post"])
    );
}

#[tokio::test]
async fn the_user_lists_page_and_respect_privacy() {
    let (db, st) = setup().await;
    let team = group(&db.pool, "team", &[3, 4]).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let raw = "[policy group=team]\nI always open **doors**!\n[/policy]";
    let post_id = create_topic(&st, &mut admin, "A policy to accept here", raw).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;

    let (status, body) = user1
        .json(
            Method::GET,
            &format!("/policy/not-accepted.json?post_id={post_id}&offset=0"),
            "",
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(ids(&body["users"]), [3, 4]);
    let (_, body) = user1
        .json(
            Method::GET,
            &format!("/policy/not-accepted.json?post_id={post_id}&offset=1"),
            "",
        )
        .await;
    assert_eq!(ids(&body["users"]), [4]);

    accept(&db.pool, post_id, 3).await;
    accept(&db.pool, post_id, 4).await;
    let (_, body) = user1
        .json(
            Method::GET,
            &format!("/policy/accepted.json?post_id={post_id}&offset=0"),
            "",
        )
        .await;
    assert_eq!(ids(&body["users"]), [3, 4]);
    let (_, body) = user1
        .json(
            Method::GET,
            &format!("/policy/accepted.json?post_id={post_id}&offset=2"),
            "",
        )
        .await;
    assert_eq!(ids(&body["users"]), Vec::<i64>::new());

    // Members visible to owners only: a member who doesn't own it can't
    // see the lists.
    sqlx::query("UPDATE groups SET members_visibility_level = 4 WHERE id = $1")
        .bind(team)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body) = user1
        .json(
            Method::GET,
            &format!("/policy/accepted.json?post_id={post_id}"),
            "",
        )
        .await;
    assert_eq!(status, 422);
    assert_eq!(body["errors"], serde_json::json!(["Not authorized"]));
    add_member(&db.pool, team, 3, true).await;
    let (status, _) = user1
        .json(
            Method::GET,
            &format!("/policy/accepted.json?post_id={post_id}"),
            "",
        )
        .await;
    assert_eq!(status, 200);

    // A private policy: admins only.
    sqlx::query("UPDATE post_policies SET private = TRUE WHERE post_id = $1")
        .bind(i64::from(post_id))
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, _) = user1
        .json(
            Method::GET,
            &format!("/policy/accepted.json?post_id={post_id}"),
            "",
        )
        .await;
    assert_eq!(status, 403);
    let (status, _) = admin
        .json(
            Method::GET,
            &format!("/policy/not-accepted.json?post_id={post_id}"),
            "",
        )
        .await;
    assert_eq!(status, 200);

    // Anonymous: log in first.
    let mut anonymous = Client::anonymous(st.clone()).await;
    let (status, _) = anonymous
        .send(
            Method::GET,
            &format!("/policy/accepted.json?post_id={post_id}"),
            "",
        )
        .await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn a_post_the_member_cannot_see_is_not_found() {
    let (db, st) = setup().await;
    group(&db.pool, "team", &[3]).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let raw = "[policy group=team]\nI always open **doors**!\n[/policy]";
    let post_id = create_topic(&st, &mut admin, "A policy to accept here", raw).await;
    let private_group = group(&db.pool, "private_group", &[]).await;
    let category: i32 = sqlx::query_scalar(
        "INSERT INTO categories (name, name_lower, slug, user_id, read_restricted, created_at, updated_at) \
         VALUES ('Private', 'private', 'private', 1, TRUE, now(), now()) RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO category_groups (category_id, group_id, permission_type, created_at, updated_at) \
         VALUES ($1, $2, 1, now(), now())",
    )
    .bind(category)
    .bind(private_group)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE topics SET category_id = $1 WHERE id = (SELECT topic_id FROM posts WHERE id = $2)",
    )
    .bind(category)
    .bind(post_id)
    .execute(&db.pool)
    .await
    .unwrap();
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    for (method, path) in [
        (Method::PUT, "/policy/accept.json"),
        (Method::PUT, "/policy/unaccept.json"),
    ] {
        let (status, _) = user1
            .send(method, path, &format!("post_id={post_id}"))
            .await;
        assert_eq!(status, 404, "{path}");
    }
    for path in ["/policy/accepted.json", "/policy/not-accepted.json"] {
        let (status, _) = user1
            .send(Method::GET, &format!("{path}?post_id={post_id}"), "")
            .await;
        assert_eq!(status, 404, "{path}");
    }
}

#[tokio::test]
async fn the_post_serializer_carries_the_policy_keys() {
    let (db, st) = setup().await;
    let team = group(&db.pool, "team", &[1, 3, 4]).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let raw = "[policy group=team]\nI always open **doors**!\n[/policy]";
    let post_id = create_topic(&st, &mut admin, "A policy to accept here", raw).await;

    let mut anonymous = Client::anonymous(st.clone()).await;
    let post = post_json(&mut anonymous, post_id).await;
    assert_eq!(post["policy_accepted_by"], serde_json::json!([]));
    assert_eq!(ids(&post["policy_not_accepted_by"]), [1, 3, 4]);
    assert_eq!(post["policy_not_accepted_by_count"], 3);
    assert_eq!(post["policy_can_accept"], false);
    assert_eq!(post["policy_stats"], true);
    let keys: Vec<&str> = post
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .filter(|k| k.starts_with("policy_"))
        .collect();
    assert_eq!(
        keys,
        [
            "policy_can_accept",
            "policy_can_revoke",
            "policy_accepted",
            "policy_revoked",
            "policy_not_accepted_by",
            "policy_not_accepted_by_count",
            "policy_accepted_by",
            "policy_accepted_by_count",
            "policy_stats"
        ]
    );

    accept(&db.pool, post_id, 3).await;
    let post = post_json(&mut anonymous, post_id).await;
    assert_eq!(ids(&post["policy_not_accepted_by"]), [1, 4]);
    assert_eq!(ids(&post["policy_accepted_by"]), [3]);

    // An admin who has not accepted may accept, not revoke, unless both
    // show at once.
    let post = post_json(&mut admin, post_id).await;
    assert_eq!(post["policy_can_accept"], true);
    assert_eq!(post["policy_can_revoke"], false);
    common::set_setting(&db.pool, "policy_easy_revoke", 5, "t").await;
    let post = post_json(&mut admin, post_id).await;
    assert_eq!(post["policy_can_accept"], true);
    assert_eq!(post["policy_can_revoke"], true);

    // Inactive and suspended accepters are left out.
    sqlx::query("UPDATE users SET suspended_till = now() + interval '1 year' WHERE id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let post = post_json(&mut admin, post_id).await;
    assert_eq!(post["policy_accepted_by"], serde_json::json!([]));
    sqlx::query("UPDATE users SET suspended_till = NULL, active = FALSE WHERE id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let post = post_json(&mut admin, post_id).await;
    assert_eq!(post["policy_accepted_by"], serde_json::json!([]));
    sqlx::query("UPDATE users SET active = TRUE WHERE id = 3")
        .execute(&db.pool)
        .await
        .unwrap();

    // Private: only admins see the lists.
    sqlx::query("UPDATE post_policies SET private = TRUE WHERE post_id = $1")
        .bind(i64::from(post_id))
        .execute(&db.pool)
        .await
        .unwrap();
    let post = post_json(&mut anonymous, post_id).await;
    assert!(post.get("policy_not_accepted_by").is_none());
    assert!(post.get("policy_accepted_by").is_none());
    assert!(post.get("policy_stats").is_none());
    let post = post_json(&mut admin, post_id).await;
    assert_eq!(ids(&post["policy_accepted_by"]), [3]);

    // Members visible to owners: a member sees no lists, an owner does.
    sqlx::query("UPDATE post_policies SET private = FALSE WHERE post_id = $1")
        .bind(i64::from(post_id))
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE groups SET members_visibility_level = 4 WHERE id = $1")
        .bind(team)
        .execute(&db.pool)
        .await
        .unwrap();
    let mut user2 = Client::logged_in(st.clone(), "user2").await;
    let post = post_json(&mut user2, post_id).await;
    assert!(post.get("policy_accepted_by").is_none());
    add_member(&db.pool, team, 4, true).await;
    let post = post_json(&mut user2, post_id).await;
    assert_eq!(ids(&post["policy_accepted_by"]), [3]);

    // The group gone: no lists for anyone but admins, theirs empty.
    sqlx::query("DELETE FROM groups WHERE id = $1")
        .bind(team)
        .execute(&db.pool)
        .await
        .unwrap();
    let post = post_json(&mut anonymous, post_id).await;
    assert!(post.get("policy_not_accepted_by").is_none());
    let post = post_json(&mut admin, post_id).await;
    assert_eq!(post["policy_not_accepted_by"], serde_json::json!([]));

    // Off: no keys.
    common::set_setting(&db.pool, "policy_enabled", 5, "f").await;
    let post = post_json(&mut admin, post_id).await;
    assert!(post.get("policy_can_accept").is_none());
}

#[tokio::test]
async fn the_current_user_and_their_options_carry_the_plugins_keys() {
    let (db, st) = setup().await;
    let creators = group(&db.pool, "creators", &[]).await;
    common::set_setting(
        &db.pool,
        "create_policy_allowed_groups",
        20,
        &format!("1|2|{creators}"),
    )
    .await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (_, current) = user1.json(Method::GET, "/session/current.json", "").await;
    assert_eq!(current["current_user"]["can_create_policy"], false);
    add_member(&db.pool, creators, 3, false).await;
    let (_, current) = user1.json(Method::GET, "/session/current.json", "").await;
    assert_eq!(current["current_user"]["can_create_policy"], true);

    let (status, body) = user1
        .json(
            Method::PUT,
            "/u/user1.json",
            "policy_email_frequency=when_away",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let stored: i32 =
        sqlx::query_scalar("SELECT policy_email_frequency FROM user_options WHERE user_id = 3")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(stored, 1);
    let (_, user) = user1.json(Method::GET, "/u/user1.json", "").await;
    assert_eq!(
        user["user"]["user_option"]["policy_email_frequency"],
        "when_away"
    );
}

/// `Jobs::DiscoursePolicy::CheckPolicy#execute` at a time.
async fn check_at(st: &AppState, at: chrono::DateTime<Utc>) {
    discourse_rs::clock::pin(Some(at));
    discourse_rs::jobs::run_scheduled(st, discourse_rs::plugins::policy::check::JOB)
        .await
        .unwrap();
}

/// A policy on a post by the admin, created at a time (`freeze_time`).
async fn policy_post_at(
    db: &TestDb,
    st: &AppState,
    admin: &mut Client,
    at: chrono::DateTime<Utc>,
    raw: &str,
) -> i32 {
    discourse_rs::clock::pin(Some(at));
    db.pin_clock(Some(at)).await;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM topics")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    create_topic(st, admin, &format!("A policy that renews {n}"), raw).await
}

#[tokio::test]
async fn renewals_expire_acceptances_by_days() {
    let db = TestDb::with_clock().await;
    common::set_setting(&db.pool, "policy_enabled", 5, "t").await;
    common::set_setting(&db.pool, "rate_limit_create_topic", 3, "0").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    group(&db.pool, "team", &[3, 4]).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let y = |year| Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0).unwrap();

    // No renew start: an acceptance older than the period expires.
    let post_id = policy_post_at(
        &db,
        &st,
        &mut admin,
        y(2019),
        "[policy group=team renew=400]\nI always open **doors**!\n[/policy]",
    )
    .await;
    discourse_rs::clock::pin(Some(y(2021)));
    accept(&db.pool, post_id, 3).await;
    accept(&db.pool, post_id, 4).await;
    check_at(&st, y(2022)).await;
    assert_eq!(accepted_by(&db.pool, post_id).await, [3, 4]);
    check_at(&st, y(2023)).await;
    assert!(accepted_by(&db.pool, post_id).await.is_empty());

    // With a renew start: nothing before it, everything accepted before
    // it after, then every 100 days from it.
    let post_id = policy_post_at(
        &db,
        &st,
        &mut admin,
        y(2019),
        "[policy group=team renew=100 renew-start=\"2020-10-17\"]\nI always open **doors**!\n[/policy]",
    )
    .await;
    accept(&db.pool, post_id, 3).await;
    accept(&db.pool, post_id, 4).await;
    check_at(&st, y(2020)).await;
    assert_eq!(accepted_by(&db.pool, post_id).await, [3, 4]);
    check_at(&st, Utc.with_ymd_and_hms(2020, 10, 18, 0, 0, 0).unwrap()).await;
    assert!(accepted_by(&db.pool, post_id).await.is_empty());
    accept(&db.pool, post_id, 3).await;
    accept(&db.pool, post_id, 4).await;
    let later = Utc.with_ymd_and_hms(2020, 10, 17, 0, 0, 0).unwrap() + chrono::Duration::days(101);
    discourse_rs::clock::pin(Some(later));
    accept(&db.pool, post_id, 4).await;
    check_at(&st, later).await;
    assert_eq!(accepted_by(&db.pool, post_id).await, [4]);
    discourse_rs::clock::pin(None);
}

#[tokio::test]
async fn renewals_by_interval_move_the_next_date_on() {
    let db = TestDb::with_clock().await;
    common::set_setting(&db.pool, "policy_enabled", 5, "t").await;
    common::set_setting(&db.pool, "rate_limit_create_topic", 3, "0").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    group(&db.pool, "team", &[3, 4]).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let day = |m, d| Utc.with_ymd_and_hms(2020, m, d, 0, 0, 0).unwrap();
    for (how_often, next) in [
        (
            "monthly",
            Utc.with_ymd_and_hms(2020, 11, 17, 0, 0, 0).unwrap(),
        ),
        (
            "quarterly",
            Utc.with_ymd_and_hms(2021, 1, 17, 0, 0, 0).unwrap(),
        ),
        (
            "yearly",
            Utc.with_ymd_and_hms(2021, 10, 17, 0, 0, 0).unwrap(),
        ),
    ] {
        let raw = format!(
            "[policy group=team renew={how_often} renew-start=\"2020-10-17\"]\nI always open **doors**!\n[/policy]"
        );
        let post_id = policy_post_at(&db, &st, &mut admin, day(10, 16), &raw).await;
        accept(&db.pool, post_id, 3).await;
        accept(&db.pool, post_id, 4).await;
        check_at(&st, day(10, 17)).await;
        assert_eq!(accepted_by(&db.pool, post_id).await, [3, 4], "{how_often}");
        check_at(&st, day(10, 18)).await;
        assert!(
            accepted_by(&db.pool, post_id).await.is_empty(),
            "{how_often}"
        );
        let policy = policy_of(&db.pool, post_id).await.unwrap();
        assert_eq!(policy.next_renew_at, Some(next.naive_utc()), "{how_often}");
    }
    discourse_rs::clock::pin(None);
}

#[tokio::test]
async fn reminders_notify_members_who_can_see_the_post() {
    let db = TestDb::with_clock().await;
    common::set_setting(&db.pool, "policy_enabled", 5, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    group(&db.pool, "team", &[3, 4]).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let now = Utc::now();
    let post_id = policy_post_at(
        &db,
        &st,
        &mut admin,
        now,
        "[policy group=team reminder=weekly]\nI always open **doors**!\n[/policy]",
    )
    .await;
    let reminders = |user_id: i32| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_as::<_, (i64, bool)>(
                "SELECT id, high_priority FROM notifications WHERE user_id = $1 \
                 AND notification_type = 18 AND post_number = 1 \
                 AND topic_id = (SELECT topic_id FROM posts WHERE id = $2) ORDER BY id",
            )
            .bind(user_id)
            .bind(post_id)
            .fetch_all(&pool)
            .await
            .unwrap()
        }
    };
    check_at(&st, now).await;
    assert!(reminders(3).await.is_empty());

    // user2 asked for the email always.
    sqlx::query("UPDATE user_options SET policy_email_frequency = 2 WHERE user_id = 4")
        .execute(&db.pool)
        .await
        .unwrap();
    let later = now + chrono::Duration::weeks(2);
    db.pin_clock(Some(later)).await;
    check_at(&st, later).await;
    check_at(&st, later).await;
    let first = reminders(3).await;
    assert_eq!(first.len(), 1);
    assert!(first[0].1);
    assert_eq!(reminders(4).await.len(), 1);
    let queued: Vec<Value> = sqlx::query_scalar(
        "SELECT args FROM discourse_rs.jobs WHERE name = 'user_email' AND args->>'type' = 'policy_email'",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(queued.len(), 1, "{queued:?}");
    assert_eq!(queued[0]["user_id"], 4);

    // The next reminder replaces the last.
    let later = later + chrono::Duration::weeks(2);
    db.pin_clock(Some(later)).await;
    check_at(&st, later).await;
    let second = reminders(3).await;
    assert_eq!(second.len(), 1);
    assert_ne!(second[0].0, first[0].0);

    // The email.
    discourse_rs::clock::pin(None);
    db.pin_clock(None).await;
    // Enqueued at the pinned time, weeks ahead: due now.
    sqlx::query("UPDATE discourse_rs.jobs SET run_at = now() AT TIME ZONE 'utc'")
        .execute(&db.pool)
        .await
        .unwrap();
    discourse_rs::jobs::drain(&st).await.unwrap();
    let sent = st.mailer.sent();
    let skipped: Vec<(String, i32)> = sqlx::query_as(
        "SELECT email_type, reason_type FROM skipped_email_logs WHERE email_type = 'policy_email'",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert!(skipped.is_empty(), "{skipped:?}");
    let failed: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT name, last_error FROM discourse_rs.jobs WHERE name = 'user_email' AND last_error IS NOT NULL",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert!(failed.is_empty(), "{failed:?}");
    let email = sent
        .iter()
        .find(|m| {
            m.header("Subject")
                .is_some_and(|s| s.starts_with("A policy needs your review"))
        })
        .expect("the policy email");
    let title: String = sqlx::query_scalar(
        "SELECT t.title FROM topics t JOIN posts p ON p.topic_id = t.id WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        email.header("Subject"),
        Some(format!("A policy needs your review: {title}").as_str())
    );
    assert_eq!(
        email
            .header("List-Unsubscribe")
            .map(|v| v.ends_with("/my/preferences>")),
        Some(true)
    );
    assert!(
        email.text.contains("[Visit Topic][2] to respond."),
        "{}",
        email.text
    );
    assert!(email.text.contains("/email/unsubscribe/"), "{}", email.text);
    let key_type: String = sqlx::query_scalar(
        "SELECT unsubscribe_key_type FROM unsubscribe_keys WHERE user_id = 4 ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(key_type, "policy_email");
}

#[tokio::test]
async fn reminders_skip_members_who_cannot_see_the_post() {
    let db = TestDb::with_clock().await;
    common::set_setting(&db.pool, "policy_enabled", 5, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    group(&db.pool, "team", &[3, 4]).await;
    let access = group(&db.pool, "access", &[3]).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let now = Utc::now();
    let post_id = policy_post_at(
        &db,
        &st,
        &mut admin,
        now,
        "[policy group=team reminder=weekly]\nI always open **doors**!\n[/policy]",
    )
    .await;
    let category: i32 = sqlx::query_scalar(
        "INSERT INTO categories (name, name_lower, slug, user_id, read_restricted, created_at, updated_at) \
         VALUES ('Private', 'private', 'private', 1, TRUE, now(), now()) RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO category_groups (category_id, group_id, permission_type, created_at, updated_at) \
         VALUES ($1, $2, 1, now(), now())",
    )
    .bind(category)
    .bind(access)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE topics SET category_id = $1 WHERE id = (SELECT topic_id FROM posts WHERE id = $2)",
    )
    .bind(category)
    .bind(post_id)
    .execute(&db.pool)
    .await
    .unwrap();
    check_at(&st, now + chrono::Duration::weeks(2)).await;
    let notified: Vec<i32> = sqlx::query_scalar(
        "SELECT user_id FROM notifications WHERE notification_type = 18 ORDER BY user_id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(notified, [3]);
    discourse_rs::clock::pin(None);
}

#[tokio::test]
async fn the_policy_is_drawn_in_its_post() {
    let (db, st) = setup().await;
    group(&db.pool, "team", &[1, 3, 4]).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let raw = "[policy group=team accept=\"I agree\"]\nI always open **doors**!\n[/policy]";
    let post_id = create_topic(&st, &mut admin, "A policy to accept here", raw).await;
    accept(&db.pool, post_id, 4).await;
    let topic_id: i32 = sqlx::query_scalar("SELECT topic_id FROM posts WHERE id = $1")
        .bind(post_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let html = user1
        .page(&format!("/t/a-policy-to-accept-here/{topic_id}"))
        .await;
    assert!(
        html.contains("<div class=\"policy\" data-accept=\"I agree\" data-group=\"team\" data-version=\"1\"><div class=\"policy-body\"><p>I always open <strong>doors</strong>!</p>"),
        "{html}"
    );
    assert!(
        html.contains(&format!("<div class=\"policy-footer\" data-post-id=\"{post_id}\"><div class=\"policy-actions\"><button class=\"btn btn-text accept btn-accept-policy btn-primary\" type=\"button\"><span class=\"d-button-label\">I agree</span></button></div>")),
        "{html}"
    );
    assert!(html.contains("<a class=\"toggle toggle-accepted\" href title=\"Users who have accepted this policy\"><span class=\"user-count\">1</span>"), "{html}");
    assert!(
        html.contains("<div class=\"users accepted\"><img loading=\"lazy\""),
        "{html}"
    );
    assert!(html.contains("<a class=\"toggle toggle-not-accepted\" href title=\"Users who have not accepted this policy\"><span class=\"user-count\">2</span>"), "{html}");
    assert!(!html.contains("users not-accepted"), "{html}");
    // Not theirs, not staff: no settings button.
    assert!(!html.contains("edit-policy-settings-btn"), "{html}");
    let html = admin
        .page(&format!("/t/a-policy-to-accept-here/{topic_id}"))
        .await;
    assert!(html.contains("edit-policy-settings-btn"), "{html}");

    // The component drawn again with the other list.
    let html = user1
        .page(&format!("/live/policy/{post_id}?not_accepted=1"))
        .await;
    assert!(html.starts_with("<div class=\"policy-footer\""), "{html}");
    assert!(
        html.contains("<div class=\"users not-accepted\">"),
        "{html}"
    );
    assert!(!html.contains("<div class=\"users accepted\">"), "{html}");
}

#[tokio::test]
async fn the_builder_inserts_and_edits_policies() {
    let (db, st) = setup().await;
    group(&db.pool, "team", &[3]).await;
    group(&db.pool, "target", &[]).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    // The composer's options menu offers it to who may create policies.
    let html = admin.page("/latest").await;
    assert!(
        html.contains("&quot;name&quot;:&quot;policy&quot;,&quot;icon&quot;:&quot;file-signature&quot;,&quot;label&quot;:&quot;Create Policy&quot;"),
        "{html}"
    );
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let html = user1.page("/latest").await;
    assert!(
        !html.contains("&quot;name&quot;:&quot;policy&quot;"),
        "{html}"
    );

    let html = admin.page("/live/policy-builder").await;
    assert!(
        html.starts_with(
            "<div class=\"policy-builder\" data-title=\"Policy\" data-insert=\"true\""
        ),
        "{html}"
    );
    assert!(
        html.contains("data-template=\"I accept this policy\""),
        "{html}"
    );
    assert!(
        html.contains("<option value=\"team\">team</option>"),
        "{html}"
    );
    // Automatic groups only for the audience, not the group to add to.
    assert!(
        html.contains("<option value=\"staff\">staff</option>"),
        "{html}"
    );
    let add_to = html
        .split("policy-builder-form__add-users-to-group")
        .nth(1)
        .unwrap();
    assert!(!add_to.contains("value=\"staff\""), "{add_to}");
    assert!(add_to.contains("value=\"target\""), "{add_to}");
    assert!(
        html.contains("name=\"version\" type=\"number\" value=\"1\""),
        "{html}"
    );
    assert!(
        html.contains("<span class=\"d-button-label\">Insert</span>"),
        "{html}"
    );

    let raw = "[policy groups=\"team\" version=\"3\" reminder=\"weekly\" add-users-to-group=\"target\" private=\"true\"]\nI accept this policy\n[/policy]";
    let post_id = create_topic(&st, &mut admin, "A policy to edit here", raw).await;
    let html = admin
        .page(&format!("/live/policy-builder?post_id={post_id}"))
        .await;
    assert!(html.contains("data-insert=\"false\""), "{html}");
    assert!(html.contains("/posts/by_number/"), "{html}");
    assert!(
        html.contains("<option value=\"team\" selected>team</option>"),
        "{html}"
    );
    assert!(
        html.contains("name=\"version\" type=\"number\" value=\"3\""),
        "{html}"
    );
    assert!(html.contains("value=\"weekly\">weekly</option>"), "{html}");
    assert!(
        html.contains("name=\"accept\" type=\"text\" value=\"Accept Policy\""),
        "{html}"
    );
    assert!(
        html.contains("<input checked=\"\" class=\"form-kit__control-checkbox\""),
        "{html}"
    );
    assert!(
        html.contains("<span class=\"d-button-label\">Save changes</span>"),
        "{html}"
    );
}

#[tokio::test]
async fn the_report_lists_who_has_not_accepted() {
    let (db, st) = setup().await;
    group(&db.pool, "team", &[3, 4]).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let raw = "[policy group=team]\nI always open **doors**!\n[/policy]";
    let post_id = create_topic(&st, &mut admin, "A policy to accept here", raw).await;
    accept(&db.pool, post_id, 3).await;
    let topic_id: i32 = sqlx::query_scalar("SELECT topic_id FROM posts WHERE id = $1")
        .bind(post_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    // Listed with the plugin's reports.
    let (status, body) = admin.json(Method::GET, "/admin/reports.json", "").await;
    assert_eq!(status, 200, "{body}");
    let listed = body["reports"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["type"] == "unaccepted-policies")
        .unwrap_or_else(|| panic!("{body}"));
    assert_eq!(listed["title"], "Unaccepted policies");
    // ReportsController#show refuses a type with a dash, as Rails does.
    let (status, _) = admin
        .send(Method::GET, "/admin/reports/unaccepted-policies.json", "")
        .await;
    assert_eq!(status, 404);
    let mut conn = db.pool.acquire().await.unwrap();
    let rows = discourse_rs::plugins::policy::unaccepted_policies(&mut conn)
        .await
        .unwrap();
    assert_eq!(rows, [(topic_id, 4)]);
}

#[tokio::test]
async fn the_email_preferences_and_unsubscribe_offer_the_frequency() {
    let (db, st) = setup().await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let html = user1.page("/u/user1/preferences/emails").await;
    assert!(
        html.contains("<h3>Policies</h3><div class=\"control-group policy-setting controls-dropdown\"><label for=\"user_policy_email_frequency\">Email me when a policy needs my review</label>"),
        "{html}"
    );

    let key = {
        let mut conn = db.pool.acquire().await.unwrap();
        discourse_rs::email::unsubscribe::create_key_for(&mut conn, 3, "policy_email", None)
            .await
            .unwrap()
    };
    let mut anonymous = Client::anonymous(st.clone()).await;
    let html = anonymous.page(&format!("/email/unsubscribe/{key}")).await;
    assert!(
        html.contains("<label>Set policy alert emails frequency to:</label>\n    <select name=\"policy_email_frequency\" id=\"policy_email_frequency\" class=\"combobox\"><option selected=\"selected\" value=\"never\">Never</option>\n<option value=\"when_away\">Only when away</option>\n<option value=\"always\">Always</option></select>"),
        "{html}"
    );
    let (status, _) = anonymous
        .send(
            Method::POST,
            &format!("/email/unsubscribe/{key}"),
            "policy_email_frequency=always",
        )
        .await;
    assert!(status < 400, "{status}");
    let stored: i32 =
        sqlx::query_scalar("SELECT policy_email_frequency FROM user_options WHERE user_id = 3")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(stored, 2);
}
