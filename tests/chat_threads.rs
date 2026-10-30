//! Chat threads beyond what the write cases record: what the bus carries
//! (a reply to its thread's channel, the thread's preview to the channel,
//! a new thread announced, the member's first thread), the watched thread
//! notifications as they consolidate, and the pages: the thread
//! indicator under an original message, the threads list button, the
//! channel's threads, a thread and the member's threads.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, header};
use common::{TestDb, bus_messages, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use discourse_rs::site_settings::SiteSettings;
use http_body_util::BodyExt;
use serde_json::{Value, json};
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

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client.send(Method::GET, "/session/csrf.json", "").await;
        let csrf: Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
            .send(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
            )
            .await;
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

/// The write cases' threaded General channel: thread 1 on user0's message
/// 1, replies by user1 (2) and admin (3), and user0's channel message 4.
async fn threaded(db: &TestDb) {
    for sql in [
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) VALUES (1, 2, 2, now() - interval '50 minutes', now() - interval '50 minutes', 'Hello **world**', '<p>Hello <strong>world</strong></p>', 1, 2)",
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id, in_reply_to_id) VALUES (2, 2, 3, now() - interval '40 minutes', now() - interval '40 minutes', 'hi back', '<p>hi back</p>', 1, 3, 1)",
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) VALUES (3, 2, 1, now() - interval '35 minutes', now() - interval '35 minutes', 'thread reply two', '<p>thread reply two</p>', 1, 1)",
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) VALUES (4, 2, 2, now() - interval '30 minutes', now() - interval '30 minutes', 'channel chatter', '<p>channel chatter</p>', 1, 2)",
        "SELECT setval('chat_messages_id_seq', 4)",
        "UPDATE chat_channels SET last_message_id = 4, messages_count = 4, threading_enabled = true WHERE id = 2",
        "INSERT INTO chat_threads (id, channel_id, original_message_id, original_message_user_id, status, replies_count, force, last_message_id, created_at, updated_at) VALUES (1, 2, 1, 2, 0, 2, false, 3, now() - interval '40 minutes', now() - interval '35 minutes')",
        "SELECT setval('chat_threads_id_seq', 1)",
        "UPDATE chat_messages SET thread_id = 1 WHERE id IN (1, 2, 3)",
        "INSERT INTO user_chat_thread_memberships (user_id, thread_id, notification_level, last_read_message_id, created_at, updated_at) VALUES (2, 1, 2, NULL, now(), now()), (3, 1, 2, 2, now(), now()), (1, 1, 2, 3, now(), now())",
        "UPDATE user_chat_channel_memberships SET last_read_message_id = 4 WHERE chat_channel_id = 2",
        "UPDATE site_settings SET value = 't' WHERE name = 'chat_threads_enabled'",
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
    sqlx::query(
        "INSERT INTO site_settings (name, data_type, value, created_at, updated_at) \
         SELECT 'chat_threads_enabled', 5, 't', now(), now() \
         WHERE NOT EXISTS (SELECT 1 FROM site_settings WHERE name = 'chat_threads_enabled')",
    )
    .execute(&db.pool)
    .await
    .unwrap();
}

fn of_type<'a>(messages: &'a [pg_bus::Message], channel: &str, kind: &str) -> Vec<&'a Value> {
    messages
        .iter()
        .filter(|m| m.channel == channel && m.data["type"] == kind)
        .map(|m| &m.data)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_in_a_thread_goes_to_the_thread() {
    let db = TestDb::new().await;
    threaded(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let from = st.bus.now().await.unwrap();
    let (status, body) = user1
        .send(
            Method::POST,
            "/chat/2.json",
            "message=more+in+the+thread&thread_id=1&staged_id=s",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let messages = bus_messages(
        &st,
        from,
        &[
            "/chat/2",
            "/chat/2/thread/1",
            "/chat/2/new-messages",
            "/chat/user-tracking-state/3",
        ],
    )
    .await;
    // The reply goes to the thread, not the channel.
    let sent = of_type(&messages, "/chat/2/thread/1", "sent");
    assert_eq!(sent.len(), 1, "{messages:?}");
    assert_eq!(sent[0]["staged_id"], "s");
    assert_eq!(sent[0]["chat_message"]["thread_id"], 1);
    assert!(of_type(&messages, "/chat/2", "sent").is_empty());
    let new = of_type(&messages, "/chat/2/new-messages", "thread");
    assert_eq!(new.len(), 1);
    assert_eq!(new[0]["thread_id"], 1);
    assert_eq!(new[0]["force_thread"], false);
    // The thread's preview follows, on the channel.
    let preview = of_type(&messages, "/chat/2", "update_thread_original_message");
    assert_eq!(preview.len(), 1);
    assert_eq!(preview[0]["original_message_id"], 1);
    assert_eq!(preview[0]["preview"]["reply_count"], 3);
    assert_eq!(
        preview[0]["preview"]["last_reply_excerpt"],
        "more in the thread"
    );
    assert_eq!(preview[0]["preview"]["participant_count"], 3);
    // The member's tracking state names their last read channel message.
    let tracking: Vec<&Value> = messages
        .iter()
        .filter(|m| m.channel == "/chat/user-tracking-state/3")
        .map(|m| &m.data)
        .collect();
    assert_eq!(tracking.len(), 1);
    assert_eq!(tracking[0]["last_read_message_id"], 4);
    // The channel's last message stays the channel's.
    let last: Option<i64> =
        sqlx::query_scalar("SELECT last_message_id FROM chat_channels WHERE id = 2")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(last, Some(4));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_to_a_channel_message_starts_a_thread() {
    let db = TestDb::new().await;
    threaded(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let from = st.bus.now().await.unwrap();
    let (status, body) = user1
        .send(
            Method::POST,
            "/chat/2.json",
            "message=starting+a+thread&in_reply_to_id=4",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let messages = bus_messages(&st, from, &["/chat/2", "/chat/user-has-threads/2"]).await;
    let created = of_type(&messages, "/chat/2", "thread_created");
    assert_eq!(created.len(), 1, "{messages:?}");
    assert_eq!(created[0]["thread_id"], 2);
    assert_eq!(created[0]["chat_message"]["id"], 4);
    let thread: (i64, i64) = sqlx::query_as(
        "SELECT original_message_id, original_message_user_id FROM chat_threads WHERE id = 2",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(thread, (4, 2));
}

#[tokio::test(flavor = "multi_thread")]
async fn watched_thread_notifications_consolidate() {
    let db = TestDb::new().await;
    threaded(&db).await;
    sqlx::query("UPDATE user_chat_thread_memberships SET notification_level = 3 WHERE user_id = 2")
        .execute(&db.pool)
        .await
        .unwrap();
    for sql in [
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id, thread_id) VALUES (5, 2, 3, now(), now(), 'one', '<p>one</p>', 1, 3, 1)",
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id, thread_id) VALUES (6, 2, 1, now(), now(), 'two', '<p>two</p>', 1, 1, 1)",
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut conn = st.pool.acquire().await.unwrap();
    let settings = SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap();
    for (id, author) in [(5, 3), (6, 1)] {
        discourse_rs::plugins::chat::notify::notify_watching(
            &st,
            &mut conn,
            &settings,
            &json!({ "chat_message_id": id, "except_user_ids": [author] }),
        )
        .await
        .unwrap();
    }
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT data FROM notifications WHERE user_id = 2 AND notification_type = 40 ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    let data: Value = serde_json::from_str(&rows[0]).unwrap();
    // The member never read the thread: its first unread reply stays.
    assert_eq!(data["chat_message_id"], 5);
    assert_eq!(data["username"], "admin");
    assert_eq!(data["username2"], "user1");
    assert_eq!(data["user_ids"], json!([3, 1]));
    assert_eq!(data["count"], 2);
    assert_eq!(data["description"], "Hello **world**");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_threaded_channel_page_shows_the_thread_indicator() {
    let db = TestDb::new().await;
    threaded(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let page = user1.page("/chat/c/general/2").await;
    // The original message with its indicator, the replies left out.
    assert!(
        page.contains("has-thread-indicator") && page.contains("data-thread-id=\"1\""),
        "{page}"
    );
    assert!(page.contains(
        "<a class=\"chat-message-thread-indicator\" tabindex=\"0\" title=\"Open Thread\" href=\"/chat/c/general/2/t/1/3\">"
    ));
    assert!(
        page.contains(
            "<div class=\"chat-message-thread-indicator__replies-count\">2 replies</div>"
        )
    );
    assert!(page.contains("chat-thread-participants__avatar-group"));
    assert!(!page.contains("thread reply two</p>"));
    // The threads list button, with the unread thread.
    assert!(page.contains(
        "<a class=\"c-navbar__threads-list-button btn no-text btn-transparent has-unreads\" title=\"Threads\" href=\"/chat/c/general/2/t\">"
    ));
    assert!(page.contains("chat-thread-header-unread-indicator__number\">1</div>"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_opens_beside_its_channel() {
    let db = TestDb::new().await;
    threaded(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let page = user1.page("/chat/c/general/2/t/1").await;
    assert!(
        page.contains("main-chat-outlet has-side-panel-expanded chat-view"),
        "{page}"
    );
    // The channel route stays, without its threads list button.
    assert!(page.contains("c-routes --channel"));
    assert!(!page.contains("c-navbar__threads-list-button"));
    // The thread, with its messages (no indicator there) and composer.
    assert!(
        page.contains("<div class=\"chat-side-panel\"><div class=\"c-routes --channel-thread\">")
    );
    assert!(
        page.contains("<div class=\"chat-thread --loaded\" data-id=\"1\" data-channel-id=\"2\"")
    );
    assert!(page.contains("thread reply two</p>"));
    assert!(page.contains("id=\"thread-composer\""));
    assert!(page.contains("placeholder=\"Chat in thread\""));
    // The header: back to the threads while one is unread, the title, the
    // tracking dropdown at tracking, and the close button.
    assert!(page.contains("title=\"Return to threads list\" href=\"/chat/c/general/2/t\""));
    assert!(page.contains("<div class=\"c-navbar__title\" title=\"Thread\"><span class=\"c-navbar__title-text\">Thread</span></div>"));
    assert!(
        page.contains("c-navbar__thread-tracking-dropdown -persisted\" title=\"")
            && page.contains("data-level-name=\"tracking\"")
    );
    assert!(page.contains("c-navbar__close-thread-button btn-transparent btn btn-icon no-text\" title=\"Close Thread\" href=\"/chat/c/general/2\""));

    // A thread that isn't the channel's goes back to the channel.
    let (status, _) = user1.send(Method::GET, "/chat/c/general/2/t/99", "").await;
    assert_eq!(status, 302);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_channel_threads_list_its_threads() {
    let db = TestDb::new().await;
    threaded(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let page = user1.page("/chat/c/general/2/t").await;
    assert!(
        page.contains("<div class=\"c-routes --channel-threads\">"),
        "{page}"
    );
    assert!(page.contains("<div class=\"chat-thread-list-item -is-unread\" data-thread-id=\"1\">"));
    assert!(page.contains("<div class=\"chat-thread-list-item__title\">Hello world</div>"));
    assert!(page.contains("data-href=\"/chat/c/general/2/t/1\""));
    assert!(page.contains("chat-thread-list-item-unread-indicator__number\">1</span>"));
    // The member's threads.
    let page = user1.page("/chat/threads").await;
    assert!(
        page.contains("<div class=\"c-user-thread\" data-id=\"1\">"),
        "{page}"
    );
    assert!(page.contains(
        "<a class=\"chat__thread-title__name\" href=\"/chat/c/general/2/t/1\">Hello world</a>"
    ));
    assert!(page.contains("chat-message-thread-indicator\" tabindex=\"-1\""));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_message_is_drawn_for_the_thread_pane() {
    let db = TestDb::new().await;
    threaded(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    // The original message: with its indicator in the channel, without it
    // in the thread.
    let channel = user1.page("/live/chat/2/1").await;
    assert!(
        channel.contains("chat-message-thread-indicator"),
        "{channel}"
    );
    let thread = user1.page("/live/chat/2/1?context=thread").await;
    assert!(
        !thread.contains("chat-message-thread-indicator"),
        "{thread}"
    );
    // A reply says nothing of what it replies to in a thread.
    let reply = user1.page("/live/chat/2/2?previous=1").await;
    assert!(!reply.contains("chat-reply is-direct-reply"), "{reply}");
}
