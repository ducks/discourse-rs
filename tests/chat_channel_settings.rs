//! What a chat channel's info pages change, ported from chat's request
//! specs: the member's notification settings, the channel's status,
//! deleting it (TrashChannel and Jobs::Chat::ChannelDelete), invites,
//! the slug edit and its validation errors, chat on or off for the
//! member, and the live page hearing channels change.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn request(&mut self, method: Method, path: &str, body: Option<Value>) -> (u16, String) {
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
            Some(Value::String(form)) => {
                request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
                Body::from(form)
            }
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

    async fn json(&mut self, method: Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let (status, text) = self.request(method, path, body).await;
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client.json(Method::GET, "/session/csrf.json", None).await;
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
            .request(
                Method::POST,
                "/session",
                Some(Value::String(format!("login={username}&password=password"))),
            )
            .await;
        client
    }

    /// A page as a browser asks for it: the status, the Location and the
    /// body.
    async fn page(&self, path: &str) -> (u16, Option<String>, String) {
        let cookie: Vec<String> = self
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let response = discourse_rs::app(self.state.clone())
            .oneshot(
                Request::get(path)
                    .header(header::HOST, "test.localhost")
                    .header(header::COOKIE, cookie.join("; "))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|l| l.to_str().unwrap().to_string());
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            location,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    /// A stream the page keeps open.
    async fn open(&self, path: &str) -> Body {
        let cookie: Vec<String> = self
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let response = discourse_rs::app(self.state.clone())
            .oneshot(
                Request::get(path)
                    .header(header::HOST, "test.localhost")
                    .header(header::COOKIE, cookie.join("; "))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200, "{path}");
        response.into_body()
    }
}

async fn scalar<T>(db: &TestDb, sql: &str) -> T
where
    T: Send + Unpin + for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    sqlx::query_scalar(sql).fetch_one(&db.pool).await.unwrap()
}

async fn exec(db: &TestDb, sql: &str) {
    sqlx::query(sql).execute(&db.pool).await.unwrap();
}

#[tokio::test]
async fn a_member_changes_their_notification_settings() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, body) = user1
        .json(
            Method::PUT,
            "/chat/api/channels/2/notifications-settings/me",
            Some(json!({"notifications_settings": {"muted": true, "notification_level": "always", "foo": 1}})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["membership"]["muted"], true);
    assert_eq!(body["membership"]["notification_level"], "always");
    assert_eq!(body["membership"]["chat_channel_id"], 2);
    assert_eq!(body["membership"]["user"]["username"], "user1");
    let (muted, level): (bool, i32) = sqlx::query_as(
        "SELECT muted, notification_level FROM user_chat_channel_memberships WHERE user_id = 3 AND chat_channel_id = 2",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(muted);
    assert_eq!(level, 2);

    // No settings: params.require.
    let (status, _) = user1
        .json(
            Method::PUT,
            "/chat/api/channels/2/notifications-settings/me",
            Some(json!({})),
        )
        .await;
    assert_eq!(status, 400);
    // Staff only: can't join it.
    let (status, _) = user1
        .json(
            Method::PUT,
            "/chat/api/channels/1/notifications-settings/me",
            Some(json!({"notifications_settings": {"muted": true}})),
        )
        .await;
    assert_eq!(status, 403);
    // No membership.
    let mut user2 = Client::logged_in(st, "user2").await;
    let (status, _) = user2
        .json(
            Method::PUT,
            "/chat/api/channels/2/notifications-settings/me",
            Some(json!({"notifications_settings": {"muted": true}})),
        )
        .await;
    assert_eq!(status, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn staff_close_and_reopen_a_channel() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, _) = user1
        .json(
            Method::PUT,
            "/chat/api/channels/2/status",
            Some(json!({"status": "closed"})),
        )
        .await;
    assert_eq!(status, 403);

    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let from = st.bus.now().await.unwrap();
    let (status, body) = admin
        .json(
            Method::PUT,
            "/chat/api/channels/2/status",
            Some(json!({"status": "closed"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["channel"]["id"], 2);
    assert_eq!(body["channel"]["status"], "closed");
    assert!(body["channel"]["current_user_membership"].is_null());
    assert_eq!(
        scalar::<i32>(&db, "SELECT status FROM chat_channels WHERE id = 2").await,
        2
    );
    let (details, previous, new): (String, String, String) = sqlx::query_as(
        "SELECT details, previous_value, new_value FROM user_histories WHERE custom_type = 'chat_channel_status_change'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        details,
        "chat_channel_id: 2\nchat_channel_name: General\ncategory_id: 4"
    );
    assert_eq!((previous.as_str(), new.as_str()), ("open", "closed"));
    let heard = common::heard(&st, from, "/chat/channel-status", 3, 1).await;
    assert_eq!(
        heard,
        vec![json!({"chat_channel_id": 2, "status": "closed"})]
    );

    // Already closed: can't close again.
    let (status, _) = admin
        .json(
            Method::PUT,
            "/chat/api/channels/2/status",
            Some(json!({"status": "closed"})),
        )
        .await;
    assert_eq!(status, 403);
    let (status, body) = admin
        .json(
            Method::PUT,
            "/chat/api/channels/2/status",
            Some(json!({"status": "open"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["channel"]["status"], "open");

    // Not an editable status, a read only channel, a missing one.
    let (status, body) = admin
        .json(
            Method::PUT,
            "/chat/api/channels/2/status",
            Some(json!({"status": "archived"})),
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(
        body["errors"],
        json!(["Status is not included in the list"])
    );
    exec(&db, "UPDATE chat_channels SET status = 1 WHERE id = 2").await;
    let (status, _) = admin
        .json(
            Method::PUT,
            "/chat/api/channels/2/status",
            Some(json!({"status": "closed"})),
        )
        .await;
    assert_eq!(status, 403);
    let (status, _) = admin
        .json(
            Method::PUT,
            "/chat/api/channels/99/status",
            Some(json!({"status": "closed"})),
        )
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn staff_delete_a_channel_and_the_job_clears_it() {
    let db = TestDb::new().await;
    exec(
        &db,
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         VALUES (1, 2, 3, now(), now(), 'hi', '<p>hi</p>', 1, 3)",
    )
    .await;
    exec(
        &db,
        "INSERT INTO chat_message_reactions (chat_message_id, user_id, emoji, created_at, updated_at) VALUES (1, 2, 'heart', now(), now())",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, _) = user1
        .json(Method::DELETE, "/chat/api/channels/2", None)
        .await;
    assert_eq!(status, 403);

    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, body) = admin
        .json(Method::DELETE, "/chat/api/channels/2", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"success": "OK"}));
    let (slug, deleted_by): (String, Option<i32>) = sqlx::query_as(
        "SELECT slug, deleted_by_id FROM chat_channels WHERE id = 2 AND deleted_at IS NOT NULL",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(deleted_by, Some(1));
    let pattern = regex::Regex::new(r"^\d{8}-\d{4}-general-deleted$").unwrap();
    assert!(pattern.is_match(&slug), "{slug}");
    let details: String = scalar(
        &db,
        "SELECT details FROM user_histories WHERE custom_type = 'chat_channel_delete' AND acting_user_id = 1",
    )
    .await;
    assert_eq!(
        details,
        "chat_channel_id: 2\nchat_channel_name: General\ncategory_id: 4"
    );
    let jobs: Vec<(String, Value)> =
        sqlx::query_as("SELECT name, args FROM discourse_rs.jobs ORDER BY id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert!(
        jobs.iter()
            .any(|(n, a)| n == "Jobs::Chat::ChannelDelete" && a["chat_channel_id"] == 2),
        "{jobs:?}"
    );
    assert!(
        jobs.iter().any(|(n, a)| n == "remap_hashtag"
            && a["remaps"] == json!([{"type": "channel", "id": 2, "old_ref": "general"}])),
        "{jobs:?}"
    );
    // Gone for everyone now.
    let (status, _) = admin
        .json(Method::DELETE, "/chat/api/channels/2", None)
        .await;
    assert_eq!(status, 404);
    // An edit of one of its messages fails as Rails' does.
    let (status, _) = user1
        .json(
            Method::PUT,
            "/chat/api/channels/2/messages/1",
            Some(json!({"message": "edited"})),
        )
        .await;
    assert_eq!(status, 500);

    discourse_rs::plugins::chat::channel_settings::channel_delete(
        &st,
        &json!({"chat_channel_id": 2}),
    )
    .await
    .unwrap();
    assert_eq!(
        scalar::<i64>(
            &db,
            "SELECT COUNT(*) FROM user_chat_channel_memberships WHERE chat_channel_id = 2"
        )
        .await,
        0
    );
    assert_eq!(
        scalar::<i64>(
            &db,
            "SELECT COUNT(*) FROM chat_message_reactions WHERE chat_message_id = 1"
        )
        .await,
        0
    );
    assert_eq!(
        scalar::<Option<i32>>(&db, "SELECT deleted_by_id FROM chat_messages WHERE id = 1").await,
        Some(1)
    );
}

#[tokio::test]
async fn the_deleted_slug_is_truncated() {
    let db = TestDb::new().await;
    common::set_setting(&db.pool, "max_topic_title_length", 3, "20").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, body) = admin
        .json(Method::DELETE, "/chat/api/channels/2", None)
        .await;
    assert_eq!(status, 200, "{body}");
    let slug: String = scalar(&db, "SELECT slug FROM chat_channels WHERE id = 2").await;
    // "<yyyymmdd>-<hhmm>-general-deleted" cut to 20 characters.
    assert_eq!(slug.len(), 20, "{slug}");
    assert!(slug.ends_with("-genera"), "{slug}");
}

#[tokio::test]
async fn a_member_invites_users_to_a_channel() {
    let db = TestDb::new().await;
    exec(
        &db,
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         VALUES (1, 2, 3, now(), now(), 'hi', '<p>hi</p>', 1, 3)",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let (status, body) = user1
        .json(
            Method::POST,
            "/chat/api/channels/2/invites?user_ids=4,5&message_id=1",
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"success": "OK"}));
    let rows: Vec<(i32, String, bool)> = sqlx::query_as(
        "SELECT user_id, data, high_priority FROM notifications WHERE notification_type = 31 ORDER BY user_id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, 4);
    assert!(rows[0].2);
    let data: Value = serde_json::from_str(&rows[0].1).unwrap();
    assert_eq!(
        data,
        json!({
            "message": "chat.invitation_notification",
            "chat_channel_id": 2,
            "chat_channel_title": "General",
            "chat_channel_slug": "general",
            "invited_by_username": "user1",
            "chat_message_id": 1,
        })
    );

    let (status, body) = user1
        .json(Method::POST, "/chat/api/channels/2/invites", None)
        .await;
    assert_eq!(status, 400);
    assert_eq!(body["errors"], json!(["User ids can't be blank"]));
    let (status, _) = user1
        .json(
            Method::POST,
            "/chat/api/channels/1/invites?user_ids=4",
            None,
        )
        .await;
    assert_eq!(status, 403);
    let (status, _) = user1
        .json(
            Method::POST,
            "/chat/api/channels/-1/invites?user_ids=4",
            None,
        )
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn a_member_turns_chat_off_and_on() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    // The id in the path is not looked at.
    let (status, body) = user1
        .json(
            Method::PUT,
            "/chat/user_chat_enabled/1",
            Some(json!({"chat_enabled": false})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"chat_enabled": false}));
    assert!(
        !scalar::<bool>(
            &db,
            "SELECT chat_enabled FROM user_options WHERE user_id = 3"
        )
        .await
    );
    assert!(
        scalar::<bool>(
            &db,
            "SELECT chat_enabled FROM user_options WHERE user_id = 1"
        )
        .await
    );
    let (status, _) = user1
        .json(Method::PUT, "/chat/user_chat_enabled/3", Some(json!({})))
        .await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn staff_change_a_channels_slug() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, body) = admin
        .json(
            Method::PUT,
            "/chat/api/channels/2",
            Some(json!({"channel": {"slug": "The Lobby"}})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["channel"]["slug"], "the-lobby");
    let remap: Value = scalar(
        &db,
        "SELECT args FROM discourse_rs.jobs WHERE name = 'remap_hashtag'",
    )
    .await;
    assert_eq!(
        remap["remaps"],
        json!([{"type": "channel", "id": 2, "old_ref": "general"}])
    );

    let (status, body) = admin
        .json(
            Method::PUT,
            "/chat/api/channels/2",
            Some(json!({"channel": {"slug": "staff"}})),
        )
        .await;
    assert_eq!(status, 422);
    assert_eq!(
        body,
        json!({"errors": ["Slug is already in use"], "error_type": "record_invalid"})
    );
    let (status, body) = admin
        .json(
            Method::PUT,
            "/chat/api/channels/2",
            Some(json!({"channel": {"name": "x".repeat(300), "description": "y".repeat(501)}})),
        )
        .await;
    assert_eq!(status, 422);
    assert_eq!(
        body["errors"],
        json!([
            "Name is too long (maximum is 255 characters)",
            "Description is too long (maximum is 500 characters)"
        ])
    );
    // A blank slug comes back from the title.
    let (status, body) = admin
        .json(
            Method::PUT,
            "/chat/api/channels/2",
            Some(json!({"channel": {"slug": "", "name": "Big Room"}})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["channel"]["slug"], "big-room");
}

#[tokio::test]
async fn an_empty_channels_category_can_be_deleted() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, body) = admin
        .json(
            Method::POST,
            "/categories.json",
            Some(json!({"name": "Empty", "color": "0088CC", "text_color": "FFFFFF"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let id = body["category"]["id"].as_i64().unwrap();
    exec(
        &db,
        &format!(
            "INSERT INTO chat_channels (id, chatable_id, chatable_type, type, name, slug, created_at, updated_at) \
             VALUES (3, {id}, 'Category', 'CategoryChannel', 'Empty', 'empty', now(), now())"
        ),
    )
    .await;
    // A message keeps it.
    exec(
        &db,
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         VALUES (1, 3, 1, now(), now(), 'hi', '<p>hi</p>', 1, 1)",
    )
    .await;
    let (status, _) = admin
        .json(Method::DELETE, &format!("/categories/{id}"), None)
        .await;
    assert_eq!(status, 403);
    exec(
        &db,
        "UPDATE chat_messages SET deleted_at = now() WHERE id = 1",
    )
    .await;
    let (status, body) = admin
        .json(Method::DELETE, &format!("/categories/{id}"), None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        scalar::<i64>(&db, "SELECT COUNT(*) FROM chat_channels WHERE id = 3").await,
        0
    );
}

/// ChatSubscriptionsManager's global subscriptions: a channel renamed
/// reaches its members' sidebars, and a channel they are added to is
/// followed from then on.
#[tokio::test(flavor = "multi_thread")]
async fn the_live_page_hears_channels_change() {
    let db = TestDb::new().await;
    exec(&db, "UPDATE users SET last_seen_at = now() WHERE id = 3").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let user1 = Client::logged_in(st.clone(), "user1").await;
    let (_, _, page) = user1.page("/latest").await;
    assert!(page.contains("&#38;chat=discovery"), "{page}");
    let from = st.bus.now().await.unwrap();
    let mut body = user1
        .open(&format!("/live?chat=discovery&position={from}"))
        .await;
    let mut buffer = String::new();
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, reply) = admin
        .json(
            Method::PUT,
            "/chat/api/channels/2",
            Some(json!({"channel": {"name": "Lobby"}})),
        )
        .await;
    assert_eq!(status, 200, "{reply}");
    let heard = common::heard(&st, from, "/chat/channel-edits", 3, 1).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    let html = common::next_sse_event(&mut body, &mut buffer, "chat").await;
    assert!(html.contains("Lobby"), "{html}");

    // Added to a new channel: its new messages are heard.
    exec(
        &db,
        "INSERT INTO chat_channels (id, chatable_id, chatable_type, type, name, slug, created_at, updated_at) \
         VALUES (3, 2, 'Category', 'CategoryChannel', 'Feedback', 'feedback', now(), now())",
    )
    .await;
    exec(
        &db,
        "INSERT INTO user_chat_channel_memberships (user_id, chat_channel_id, following, created_at, updated_at, last_viewed_at) \
         VALUES (3, 3, TRUE, now(), now(), now())",
    )
    .await;
    let mut conn = db.pool.acquire().await.unwrap();
    st.bus
        .publish(
            &mut conn,
            "/chat/new-channel",
            &json!({"channel": {"id": 3}}),
            Some(&[discourse_rs::bus::user_tag(3)]),
        )
        .await
        .unwrap();
    common::next_sse_event(&mut body, &mut buffer, "chat").await;
    // The stream ends for the page to reconnect, following it now.
    let ended = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while let Some(frame) = body.frame().await {
            frame.unwrap();
        }
    })
    .await;
    assert!(ended.is_ok(), "the stream ends");
    let from = st.bus.now().await.unwrap();
    let mut body = user1
        .open(&format!("/live?chat=discovery&position={from}"))
        .await;
    let mut buffer = String::new();
    exec(
        &db,
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         VALUES (1, 3, 1, now(), now(), 'hi', '<p>hi</p>', 1, 1)",
    )
    .await;
    exec(
        &db,
        "UPDATE chat_channels SET last_message_id = 1 WHERE id = 3",
    )
    .await;
    st.bus
        .publish(
            &mut conn,
            "/chat/3/new-messages",
            &json!({"type": "channel", "channel_id": 3}),
            Some(&[discourse_rs::bus::user_tag(3)]),
        )
        .await
        .unwrap();
    let html = common::next_sse_event(&mut body, &mut buffer, "chat").await;
    assert!(html.contains("channel-3"), "{html}");
}

#[tokio::test]
async fn the_info_routes_go_to_the_settings() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let user1 = Client::logged_in(st, "user1").await;
    let (status, location, _) = user1.page("/chat/c/general/2/info").await;
    assert_eq!(
        (status, location.as_deref()),
        (302, Some("/chat/c/general/2/info/settings"))
    );
    // The channel's own slug, and /chat for one the member can't open.
    let (status, location, _) = user1.page("/chat/c/wrong/2/info/members").await;
    assert_eq!(
        (status, location.as_deref()),
        (302, Some("/chat/c/general/2/info/members"))
    );
    let (status, location, _) = user1.page("/chat/c/staff/1/info/settings").await;
    assert_eq!((status, location.as_deref()), (302, Some("/chat")));
    let (status, _, _) = user1.page("/chat/c/general/2/info/other").await;
    assert_eq!(status, 404);
    // Members of a channel that isn't open go to the settings.
    exec(&db, "UPDATE chat_channels SET status = 2 WHERE id = 2").await;
    let (status, location, _) = user1.page("/chat/c/general/2/info/members").await;
    assert_eq!(
        (status, location.as_deref()),
        (302, Some("/chat/c/general/2/info/settings"))
    );
}

#[tokio::test]
async fn a_members_settings_show_what_they_may_change() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, _, html) = user1.page("/chat/c/general/2/info/settings").await;
    assert_eq!(status, 200, "{html}");
    assert!(html.contains("<title>#General - Chat - "), "{html}");
    assert!(html.contains("<div class=\"c-routes --channel-info\">"));
    assert!(html.contains(
        "<a class=\"c-navbar__back-button no-text btn-transparent btn\" title=\"Back\" href=\"/chat/c/general/2\">"
    ));
    assert!(html.contains(
        "<li><a class=\"active\" href=\"/chat/c/general/2/info/settings\">Settings</a></li><li><a href=\"/chat/c/general/2/info/members\">Members<span class=\"c-channel-info__member-count\">(3)</span></a></li>"
    ));
    assert!(html.contains("<div class=\"c-channel-settings__name\">General</div>"));
    assert!(html.contains(
        "<div class=\"c-channel-settings__slug\"><a href=\"/chat/c/general/2\">/chat/c/general/2</a></div>"
    ));
    assert!(html.contains("c-channel-settings__mute-switch"));
    assert!(html.contains("c-channel-settings__notifications-selector"));
    assert!(html.contains("<span class=\"name\">Only for mentions</span>"));
    assert!(html.contains("<span class=\"chat-retention-reminder-text\">"));
    assert!(html.contains("data-category-id=\"4\""));
    assert!(html.contains("toggle-channel-membership-button -leave btn-default"));
    // Staff only: editing and the admin section.
    assert!(!html.contains("edit-name-slug-btn"));
    assert!(!html.contains("data-section=\"admin\""));
    assert!(html.contains("/assets/chat-channel-info.js"));

    let admin = Client::logged_in(st, "admin").await;
    let (status, _, html) = admin.page("/chat/c/general/2/info/settings").await;
    assert_eq!(status, 200, "{html}");
    assert!(html.contains("edit-name-slug-btn btn-flat"));
    assert!(html.contains("edit-description-btn btn-flat"));
    assert!(html.contains("data-section=\"admin\""));
    assert!(html.contains("c-channel-settings__auto-join-switch"));
    assert!(html.contains("c-channel-settings__channel-wide-mentions"));
    assert!(html.contains("close-btn chat-form__btn btn-transparent"));
    assert!(html.contains("delete-btn chat-form__btn btn-transparent"));
}

#[tokio::test]
async fn the_members_tab_lists_and_filters_members() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let admin = Client::logged_in(st, "admin").await;
    let (status, _, html) = admin.page("/chat/c/general/2/info/members").await;
    assert_eq!(status, 200, "{html}");
    assert!(
        html.contains("<div class=\"c-routes --channel-info-members\">"),
        "{html}"
    );
    assert!(html.contains("data-username=\"user0\""));
    assert!(html.contains("data-username=\"user1\""));
    // Admins remove others, not themselves.
    assert_eq!(html.matches("-remove-member").count(), 2, "{html}");
    let (status, _, items) = admin
        .page("/chat/c/general/2/info/members/page?username=user0")
        .await;
    assert_eq!(status, 200);
    assert!(
        items.contains("data-username=\"user0\"") && !items.contains("data-username=\"user1\""),
        "{items}"
    );
    let (_, _, none) = admin
        .page("/chat/c/general/2/info/members/page?username=nobody")
        .await;
    assert!(none.contains("-no-results"), "{none}");
}
