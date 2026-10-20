//! The basics a new member and the staff around them hit in the first
//! minutes, driven end to end through the app as a browser would. Each
//! step is its own test. A step the port still refuses is ignored with
//! the ROADMAP item that fixes it; that item's branch removes the ignore.
//! The seeded users' password is "password"; user1 is trust level 2.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

/// A topic in the seed with replies by several members.
const TOPIC: i32 = 35;
/// The seed's General category.
const CATEGORY: i32 = 4;
/// The composer's typing_duration_msecs for a post typed at a human pace:
/// a first post under fast_typing_threshold (3s) goes to the review queue.
const TYPING_MSECS: &str = "12000";

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

struct Reply {
    status: StatusCode,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

impl Client {
    fn new(state: AppState) -> Client {
        Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        }
    }

    async fn send(&mut self, method: Method, path: &str, body: &str) -> Reply {
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
        if !body.is_empty() {
            request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap_or("");
            let (pair, attrs) = v.split_once(';').unwrap_or((v, ""));
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            self.cookies.retain(|(n, _)| n != name);
            if !attrs.contains("max-age=0") && !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    async fn get(&mut self, path: &str) -> Reply {
        self.send(Method::GET, path, "").await
    }

    async fn post(&mut self, path: &str, form: &[(&str, &str)]) -> Reply {
        let body = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        self.send(Method::POST, path, &body).await
    }

    async fn put(&mut self, path: &str, form: &[(&str, &str)]) -> Reply {
        let body = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        self.send(Method::PUT, path, &body).await
    }

    async fn csrf(&mut self) {
        let reply = self.get("/session/csrf.json").await;
        self.csrf = reply.json()["csrf"].as_str().map(str::to_string);
    }

    /// The honeypot answer and reversed challenge signup and activation send.
    async fn honeypot(&mut self) -> (String, String) {
        let hp = self.get("/session/hp.json").await.json();
        (
            hp["value"].as_str().unwrap().to_string(),
            hp["challenge"].as_str().unwrap().chars().rev().collect(),
        )
    }

    async fn login(&mut self, login: &str, password: &str) {
        self.csrf().await;
        let reply = self
            .post("/session", &[("login", login), ("password", password)])
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert!(reply.json()["error"].is_null(), "{}", reply.body);
    }
}

async fn app() -> (TestDb, AppState) {
    let db = TestDb::new().await;
    let state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    (db, state)
}

/// Signs up `username`, activates the account from the emailed token and
/// logs in: a brand-new trust level 0 member.
async fn new_member(db: &TestDb, state: &AppState, username: &str) -> Client {
    let mut client = Client::new(state.clone());
    client.csrf().await;
    let (value, challenge) = client.honeypot().await;
    let email = format!("{username}@example.com");
    let created = client
        .post(
            "/u.json",
            &[
                ("name", "New Member"),
                ("email", &email),
                ("password", "a-long-enough-pass-phrase"),
                ("username", username),
                ("password_confirmation", &value),
                ("challenge", &challenge),
            ],
        )
        .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.body);
    assert_eq!(created.json()["success"], true, "{}", created.body);

    let token: String = sqlx::query_scalar(
        "SELECT args->>'email_token' FROM discourse_rs.jobs \
         WHERE name = 'critical_user_email' AND args->>'type' = 'signup' \
         ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .expect("signup enqueues the activation email");
    let (value, challenge) = client.honeypot().await;
    let activated = client
        .put(
            &format!("/u/activate-account/{token}.json"),
            &[("password_confirmation", &value), ("challenge", &challenge)],
        )
        .await;
    assert_eq!(activated.status, StatusCode::OK, "{}", activated.body);

    let mut client = Client::new(state.clone());
    client.login(username, "a-long-enough-pass-phrase").await;
    let trust_level: i32 = sqlx::query_scalar("SELECT trust_level FROM users WHERE username = $1")
        .bind(username)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(trust_level, 0, "a new account starts at trust level 0");
    client
}

async fn member(state: &AppState, username: &str) -> Client {
    let mut client = Client::new(state.clone());
    client.login(username, "password").await;
    client
}

/// The id of the post the reply made.
fn created_post(reply: &Reply) -> i64 {
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let json = reply.json();
    json["post"]["id"]
        .as_i64()
        .or_else(|| json["id"].as_i64())
        .unwrap_or_else(|| panic!("no post id in {}", reply.body))
}

#[tokio::test]
async fn signs_up_activates_and_logs_in() {
    let (db, state) = app().await;
    let mut client = new_member(&db, &state, "smokenew").await;
    let me = client.get("/session/current.json").await;
    assert_eq!(me.status, StatusCode::OK, "{}", me.body);
    assert_eq!(me.json()["current_user"]["username"], "smokenew");
}

#[tokio::test]
async fn new_member_creates_a_topic() {
    let (db, state) = app().await;
    let mut client = new_member(&db, &state, "smoketopic").await;
    let category = CATEGORY.to_string();
    let reply = client
        .post(
            "/posts.json",
            &[
                ("title", "A first topic from a brand new member"),
                ("raw", "Hello everyone, this is my very first topic here."),
                ("category", &category),
                ("typing_duration_msecs", TYPING_MSECS),
            ],
        )
        .await;
    created_post(&reply);
}

#[tokio::test]
async fn new_member_replies() {
    let (db, state) = app().await;
    let mut client = new_member(&db, &state, "smokereply").await;
    let topic = TOPIC.to_string();
    let reply = client
        .post(
            "/posts.json",
            &[
                ("topic_id", &topic),
                ("raw", "A plain reply from a brand new member account."),
                ("typing_duration_msecs", TYPING_MSECS),
            ],
        )
        .await;
    created_post(&reply);
}

/// A first post the composer says was typed faster than
/// fast_typing_threshold waits for review, and its author is silenced.
#[tokio::test]
async fn new_member_typing_too_fast_is_queued() {
    let (db, state) = app().await;
    let mut client = new_member(&db, &state, "smokefast").await;
    let topic = TOPIC.to_string();
    let reply = client
        .post(
            "/posts.json",
            &[
                ("topic_id", &topic),
                (
                    "raw",
                    "A reply pasted in far faster than anyone could type it.",
                ),
                ("typing_duration_msecs", "200"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["action"], "enqueued", "{}", reply.body);
    let silenced: bool = sqlx::query_scalar(
        "SELECT silenced_till IS NOT NULL FROM users WHERE username = 'smokefast'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(silenced, "a fast typer's first post silences them");
}

#[tokio::test]
async fn member_replies() {
    let (_db, state) = app().await;
    let mut client = member(&state, "user1").await;
    let topic = TOPIC.to_string();
    let reply = client
        .post(
            "/posts.json",
            &[
                ("topic_id", &topic),
                ("raw", "A plain reply from a member who has been around."),
            ],
        )
        .await;
    created_post(&reply);
}

#[tokio::test]
#[ignore = "ROADMAP 135: posts with quotes (QuotedPost) are refused"]
async fn member_replies_with_a_quote() {
    let (_db, state) = app().await;
    let mut client = member(&state, "user1").await;
    let topic = TOPIC.to_string();
    let raw = format!(
        "[quote=\"system, post:1, topic:{TOPIC}\"]\nthe first post\n[/quote]\n\nReplying to that quote."
    );
    let reply = client
        .post("/posts.json", &[("topic_id", &topic), ("raw", &raw)])
        .await;
    created_post(&reply);
}

#[tokio::test]
#[ignore = "ROADMAP 132: topic titles with a unicode emoji are refused"]
async fn member_titles_a_topic_with_an_emoji() {
    let (_db, state) = app().await;
    let mut client = member(&state, "user1").await;
    let category = CATEGORY.to_string();
    let reply = client
        .post(
            "/posts.json",
            &[
                ("title", "Weekend plans and a party \u{1f389} for everyone"),
                ("raw", "Who is coming along this weekend? Let me know."),
                ("category", &category),
            ],
        )
        .await;
    created_post(&reply);
}

#[tokio::test]
async fn member_edits_their_reply() {
    let (_db, state) = app().await;
    let mut client = member(&state, "user1").await;
    let topic = TOPIC.to_string();
    let reply = client
        .post(
            "/posts.json",
            &[
                ("topic_id", &topic),
                ("raw", "A reply that is about to be edited by its author."),
            ],
        )
        .await;
    let id = created_post(&reply);
    let edited = client
        .put(
            &format!("/posts/{id}.json"),
            &[("post[raw]", "A reply that its author has now edited once.")],
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.body);
    assert_eq!(
        edited.json()["post"]["raw"],
        "A reply that its author has now edited once.",
        "{}",
        edited.body
    );
}

#[tokio::test]
async fn member_likes_a_post() {
    let (db, state) = app().await;
    let mut client = member(&state, "user1").await;
    let post_id: i32 = sqlx::query_scalar(
        "SELECT p.id FROM posts p JOIN users u ON u.id = p.user_id \
         WHERE p.topic_id = $1 AND p.deleted_at IS NULL AND u.username <> 'user1' \
           AND NOT EXISTS (SELECT 1 FROM post_actions a JOIN users v ON v.id = a.user_id \
                           WHERE a.post_id = p.id AND v.username = 'user1' \
                             AND a.post_action_type_id = 2 AND a.deleted_at IS NULL) \
         ORDER BY p.post_number LIMIT 1",
    )
    .bind(TOPIC)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let id = post_id.to_string();
    let liked = client
        .post(
            "/post_actions.json",
            &[("id", &id), ("post_action_type_id", "2")],
        )
        .await;
    assert_eq!(liked.status, StatusCode::OK, "{}", liked.body);
}

#[tokio::test]
#[ignore = "ROADMAP 124: viewing a topic with deleted posts as staff is refused"]
async fn staff_open_a_topic_with_a_deleted_reply() {
    let (db, state) = app().await;
    sqlx::query(
        "UPDATE posts SET deleted_at = now() WHERE id = \
         (SELECT id FROM posts WHERE topic_id = $1 AND post_number > 1 \
          ORDER BY post_number LIMIT 1)",
    )
    .bind(TOPIC)
    .execute(&db.pool)
    .await
    .unwrap();
    let mut admin = member(&state, "admin").await;
    let json = admin.get(&format!("/t/{TOPIC}.json")).await;
    assert_eq!(json.status, StatusCode::OK, "{}", json.body);
    let page = admin.get(&format!("/t/{TOPIC}")).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
}

#[tokio::test]
async fn members_open_a_topic_with_a_deleted_reply() {
    let (db, state) = app().await;
    sqlx::query(
        "UPDATE posts SET deleted_at = now() WHERE id = \
         (SELECT id FROM posts WHERE topic_id = $1 AND post_number > 1 \
          ORDER BY post_number LIMIT 1)",
    )
    .bind(TOPIC)
    .execute(&db.pool)
    .await
    .unwrap();
    let mut client = member(&state, "user1").await;
    let json = client.get(&format!("/t/{TOPIC}.json")).await;
    assert_eq!(json.status, StatusCode::OK, "{}", json.body);
}
