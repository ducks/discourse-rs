//! Chat's corner cases that no larger item covers: a DM participant's
//! multi-valued custom fields, a DM whose other participants are all
//! deleted (NullUsers in its title and chatable), and the states Rails
//! raises on (a DM channel without its direct message).

mod common;

use axum::body::Body;
use axum::http::{Request, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn send(&mut self, method: &str, path: &str, body: &str) -> (u16, String) {
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
        let status = response.status().as_u16();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let pair = v.to_str().unwrap().split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client.send("GET", "/session/csrf.json", "").await;
        let csrf: Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
            .send(
                "POST",
                "/session",
                &format!("login={username}&password=password"),
            )
            .await;
        client
    }

    async fn json(&mut self, path: &str) -> Value {
        let (status, body) = self.send("GET", path, "").await;
        assert_eq!(status, 200, "{body}");
        serde_json::from_str(&body).unwrap()
    }
}

/// user1's DM with user2 (channel 3, direct message 1).
async fn chat_dm(db: &TestDb) {
    let day = "now() - interval '1 day'";
    for sql in [
        format!(
            "INSERT INTO direct_message_channels (id, \"group\", created_at, updated_at) VALUES (1, false, {day}, {day})"
        ),
        format!(
            "INSERT INTO direct_message_users (direct_message_channel_id, user_id, created_at, updated_at) VALUES (1, 3, {day}, {day}), (1, 4, {day}, {day})"
        ),
        format!(
            "INSERT INTO chat_channels (id, chatable_type, chatable_id, type, status, user_count, name, messages_count, created_at, updated_at) VALUES (3, 'DirectMessage', 1, 'DirectMessageChannel', 0, 2, NULL, 0, {day}, {day})"
        ),
        format!(
            "INSERT INTO user_chat_channel_memberships (user_id, chat_channel_id, following, muted, notification_level, created_at, updated_at) VALUES (3, 3, true, false, 2, {day}, {day}), (4, 3, true, false, 2, {day}, {day})"
        ),
    ] {
        sqlx::query(&sql).execute(&db.pool).await.unwrap();
    }
}

fn dm(channels: &Value) -> &Value {
    channels["direct_message_channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == 3)
        .unwrap_or_else(|| panic!("{channels}"))
}

#[tokio::test]
async fn a_participants_repeated_custom_field_is_an_array() {
    let db = TestDb::new().await;
    chat_dm(&db).await;
    common::set_setting(&db.pool, "public_user_custom_fields", 8, "favorite|single").await;
    sqlx::query(
        "INSERT INTO user_custom_fields (user_id, name, value, created_at, updated_at) VALUES \
         (4, 'favorite', 'tea', now(), now()), (4, 'single', 'one', now(), now()), \
         (4, 'favorite', 'coffee', now(), now()), (4, 'favorite', 'water', now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let channels = user1.json("/chat/api/me/channels").await;
    let users = &dm(&channels)["chatable"]["users"];
    assert_eq!(users[0]["username"], "user2");
    assert_eq!(
        users[0]["custom_fields"],
        serde_json::json!({"favorite": ["tea", "coffee", "water"], "single": "one"})
    );
}

#[tokio::test]
async fn a_dm_with_its_other_participant_deleted_names_the_null_user() {
    let db = TestDb::new().await;
    chat_dm(&db).await;
    sqlx::query("DELETE FROM users WHERE id = 4")
        .execute(&db.pool)
        .await
        .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let channels = user1.json("/chat/api/me/channels").await;
    let channel = dm(&channels);
    // Chat::NullUser's username, in the title and the chatable.
    assert_eq!(channel["title"], "deleted");
    assert_eq!(channel["chatable"]["users"][0]["username"], "deleted");
    // The channel page slugifies the same title.
    let (status, html) = user1.send("GET", "/chat/c/deleted/3", "").await;
    assert_eq!(status, 200, "{html}");
}

#[tokio::test]
async fn a_dm_channel_without_its_direct_message_raises() {
    let db = TestDb::new().await;
    chat_dm(&db).await;
    sqlx::query("DELETE FROM direct_message_channels WHERE id = 1")
        .execute(&db.pool)
        .await
        .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    // The list joins the direct message and leaves it out; the channel
    // itself is the guardian's nil chatable, a 500 on Rails too.
    let channels = user1.json("/chat/api/me/channels").await;
    assert_eq!(channels["direct_message_channels"], serde_json::json!([]));
    let (status, _) = user1.send("GET", "/chat/api/channels/3", "").await;
    assert_eq!(status, 500);
}
