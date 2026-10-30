//! What ROADMAP 162 ports of chat: an archive's progress for staff,
//! changing a channel's slug, webhook messages and bot blocks, bots'
//! edits, the mobile starred and empty channels routes, and notifying of
//! a deleted author's message with nobody to notify.

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
    async fn send(
        &mut self,
        method: Method,
        path: &str,
        body: &str,
    ) -> (u16, Option<String>, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        if !body.is_empty() || path.starts_with("/chat/api") || path.starts_with("/session") {
            request = request.header("x-requested-with", "XMLHttpRequest");
        }
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
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|v| v.to_str().unwrap().to_string());
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
        (
            status,
            location,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, _, csrf) = client.send(Method::GET, "/session/csrf.json", "").await;
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

    async fn page(&mut self, path: &str) -> (u16, Option<String>, String) {
        self.send(Method::GET, path, "").await
    }

    async fn json(&mut self, path: &str) -> Value {
        let (status, _, body) = self.send(Method::GET, path, "").await;
        assert_eq!(status, 200, "{body}");
        serde_json::from_str(&body).unwrap()
    }
}

async fn exec(db: &TestDb, sql: &str) {
    sqlx::query(sql).execute(&db.pool).await.unwrap();
}

#[tokio::test]
async fn staff_see_an_archives_progress() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    exec(&db, "UPDATE chat_channels SET status = 3 WHERE id = 2").await;
    exec(
        &db,
        "INSERT INTO chat_channel_archives (chat_channel_id, archived_by_id, total_messages, archived_messages, \
           archive_error, destination_topic_id, created_at, updated_at) \
         VALUES (2, 1, 10, 4, 'boom', 7, now(), now())",
    )
    .await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let channel = admin.json("/chat/api/channels/2.json").await;
    let c = &channel["channel"];
    assert_eq!(c["archive_failed"], true, "{c}");
    assert_eq!(c["archive_completed"], false);
    assert_eq!(c["archived_messages"], 4);
    assert_eq!(c["total_messages"], 10);
    assert_eq!(c["archive_topic_id"], 7);
    // The channel page tells an admin, with the retry button.
    let (_, _, html) = admin.page("/chat/c/general/2").await;
    assert!(
        html.contains("<div class=\"alert alert-warn chat-channel-retry-archive archived\"><div class=\"chat-channel-archive-failed-message\">Archive channel failed. 4/10 messages have been archived. <a target=\"_blank\" href=\"/t/-/7\">The destination topic</a> was created."),
        "{html}"
    );

    // Every message archived and none left: complete.
    exec(
        &db,
        "UPDATE chat_channel_archives SET archived_messages = 10",
    )
    .await;
    let channel = admin.json("/chat/api/channels/2.json").await;
    assert_eq!(channel["channel"]["archive_completed"], true);
    assert_eq!(channel["channel"]["archive_failed"], false);

    // Not for others.
    let mut user1 = Client::logged_in(st, "user1").await;
    let channel = user1.json("/chat/api/channels/2.json").await;
    assert!(channel["channel"].get("archive_failed").is_none());
}

async fn slug_of(db: &TestDb) -> Option<String> {
    sqlx::query_scalar("SELECT slug FROM chat_channels WHERE id = 2")
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn remaps(db: &TestDb) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT args FROM discourse_rs.jobs WHERE name = 'remap_hashtag' ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn staff_change_a_channels_slug() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, _, body) = admin
        .send(
            Method::PUT,
            "/chat/api/channels/2",
            "channel%5Bslug%5D=Water+Cooler",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(slug_of(&db).await.as_deref(), Some("water-cooler"));
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["channel"]["slug"], "water-cooler");
    assert_eq!(
        remaps(&db).await,
        vec![json!({ "remaps": [{ "type": "channel", "id": 2, "old_ref": "general" }] })]
    );

    // Only the case changed: no remap.
    let (status, _, _) = admin
        .send(
            Method::PUT,
            "/chat/api/channels/2",
            "channel%5Bslug%5D=WATER-COOLER",
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(slug_of(&db).await.as_deref(), Some("water-cooler"));

    // Blank: the title's (generate_auto_slug).
    let (status, _, _) = admin
        .send(Method::PUT, "/chat/api/channels/2", "channel%5Bslug%5D=")
        .await;
    assert_eq!(status, 200);
    assert_eq!(slug_of(&db).await.as_deref(), Some("general"));

    // Another channel's: refused, unchanged.
    let (status, _, _) = admin
        .send(
            Method::PUT,
            "/chat/api/channels/2",
            "channel%5Bslug%5D=staff",
        )
        .await;
    assert_ne!(status, 200);
    assert_eq!(slug_of(&db).await.as_deref(), Some("general"));
}

/// A webhook's message, and a bot's with blocks, in General.
async fn hook_and_bot_messages(db: &TestDb) {
    for sql in [
        "INSERT INTO incoming_chat_webhooks (id, name, key, chat_channel_id, username, emoji, created_at, updated_at) \
         VALUES (1, 'CI', 'abc', 2, 'Build bot', ':robot:', now(), now())",
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         VALUES (1, 2, -1, now() - interval '5 minutes', now(), 'built', '<p>built</p>', 2, -1)",
        "INSERT INTO chat_webhook_events (chat_message_id, incoming_chat_webhook_id, created_at, updated_at) VALUES (1, 1, now(), now())",
        r#"INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id, blocks)
         VALUES (2, 2, -1, now(), now(), 'pick', '<p>pick</p>', 2, -1,
           '[{"type":"actions","elements":[{"type":"button","action_id":"go","text":{"type":"plain_text","text":"Go :+1:"},"style":"primary"}]}]')"#,
        "UPDATE chat_channels SET last_message_id = 2 WHERE id = 2",
    ] {
        exec(db, sql).await;
    }
}

#[tokio::test]
async fn webhook_messages_and_blocks_are_served_and_drawn() {
    let db = TestDb::new().await;
    hook_and_bot_messages(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let list = user1.json("/chat/api/channels/2/messages.json").await;
    let messages = list["messages"].as_array().unwrap();
    let hook = messages.iter().find(|m| m["id"] == 1).unwrap();
    assert_eq!(
        hook["chat_webhook_event"],
        json!({ "username": "Build bot", "emoji": ":robot:" })
    );
    let bot = messages.iter().find(|m| m["id"] == 2).unwrap();
    assert_eq!(bot["chat_webhook_event"], Value::Null);
    assert_eq!(
        bot["blocks"],
        json!([{ "type": "actions", "elements": [{
            "action_id": "go", "type": "button",
            "text": { "text": "Go :+1:", "type": "plain_text" },
            "style": "primary", "icon": null,
        }]}])
    );

    let (_, _, html) = user1.page("/chat/c/general/2").await;
    assert!(
        html.contains("<div class=\"chat-message-avatar\"><div class=\"chat-emoji-avatar\"><div class=\"chat-emoji-avatar-container\"><img"),
        "{html}"
    );
    assert!(html.contains(
        "\">Build bot</span><span class=\"chat-message-info__bot-indicator\">bot</span>"
    ));
    assert!(html.contains("<div class=\"chat-message__blocks-wrapper\"><div class=\"chat-message__blocks\"><div class=\"chat-message__block-wrapper\"><div class=\"chat-message__block\"><div class=\"block__actions-wrapper\"><div class=\"block__actions\"><div class=\"block__action-wrapper\"><div class=\"block__action\"><button class=\"btn btn-text block__button btn-primary\" id=\"go\" type=\"button\" data-message-id=\"2\"><span class=\"d-button-label\">Go <img"));
}

#[tokio::test]
async fn the_mobile_view_has_the_starred_route() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    // The desktop view goes to /chat.
    let (status, location, _) = user1.page("/chat/starred-channels").await;
    assert_eq!((status, location.as_deref()), (302, Some("/chat")));
    user1.cookies.push(("mobile_view".into(), "1".into()));
    // Nothing starred: the channels.
    let (status, location, _) = user1.page("/chat/starred-channels").await;
    assert_eq!((status, location.as_deref()), (302, Some("/chat/channels")));
    exec(
        &db,
        "UPDATE user_chat_channel_memberships SET starred = TRUE WHERE user_id = 3 AND chat_channel_id = 2",
    )
    .await;
    let (status, _, html) = user1.page("/chat/starred-channels").await;
    assert_eq!(status, 200);
    assert!(
        html.contains("<div class=\"c-routes --starred-channels\"><div class=\"c-navbar-container\"><nav class=\"c-navbar\"><div class=\"c-navbar__title\" title=\"Starred\">"),
        "{html}"
    );
    assert!(html.contains(
        "<div class=\"channels-list-container starred-channels\"><a class=\"chat-channel-row"
    ));
    assert!(!html.contains("starred-channels-section"));
    assert!(html.contains("id=\"c-footer-starred\""));
}

#[tokio::test]
async fn the_mobile_channels_list_without_channels_offers_the_browser() {
    let db = TestDb::new().await;
    exec(
        &db,
        "UPDATE user_chat_channel_memberships SET following = FALSE WHERE user_id = 3",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    user1.cookies.push(("mobile_view".into(), "1".into()));
    let (status, _, html) = user1.page("/chat/channels").await;
    assert_eq!(status, 200, "{html}");
    assert!(html.contains("<div class=\"channels-list-container public-channels\" id=\"public-channels\"><div class=\"empty-state__container --empty-channels-list --with-image\">"), "{html}");
    assert!(html.contains(
        "data-href=\"/chat/browse\"><span class=\"d-button-label\">Browse channels</span>"
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bot_edits_and_joins_the_channel() {
    let db = TestDb::new().await;
    exec(
        &db,
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         VALUES (5, 2, -1, now() - interval '1 hour', now(), 'old', '<p>old</p>', 2, -1)",
    )
    .await;
    exec(
        &db,
        "DELETE FROM user_chat_channel_memberships WHERE user_id = -1",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut conn = st.pool.acquire().await.unwrap();
    let settings = discourse_rs::site_settings::SiteSettings::load(
        &mut conn,
        &st.site_setting_defs,
        &st.config.globals,
    )
    .await
    .unwrap();
    let system = discourse_rs::session::current::SessionUser::load(&mut conn, -1)
        .await
        .unwrap()
        .unwrap();
    let guardian = discourse_rs::guardian::Guardian::for_user(&mut conn, &system)
        .await
        .unwrap();
    let host = discourse_rs::pretty_text::Host::from_state(&st);
    let mut cx = discourse_rs::plugins::chat::channels::Context {
        conn: &mut conn,
        settings: &settings,
        i18n: &st.i18n,
        guardian: &guardian,
        base_path: "",
        config: &st.config,
    };
    let outcome = cx
        .update_message(&host, &st.bus, Some(2), "5", Some("new".into()), &[])
        .await
        .unwrap();
    assert!(
        matches!(
            outcome,
            discourse_rs::plugins::chat::modify::Outcome::Done(_)
        ),
        "the edit went through"
    );
    let message: String = sqlx::query_scalar("SELECT message FROM chat_messages WHERE id = 5")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(message, "new");
    let following: bool = sqlx::query_scalar(
        "SELECT following FROM user_chat_channel_memberships WHERE user_id = -1 AND chat_channel_id = 2",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(following);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_authors_message_notifies_nobody_without_failing() {
    let db = TestDb::new().await;
    exec(
        &db,
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         VALUES (7, 2, 999999, now(), now(), 'orphan', '<p>orphan</p>', 2, 999999)",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut conn = st.pool.acquire().await.unwrap();
    let settings = discourse_rs::site_settings::SiteSettings::load(
        &mut conn,
        &st.site_setting_defs,
        &st.config.globals,
    )
    .await
    .unwrap();
    // Nobody watches the channel, and the mention map is empty.
    exec(
        &db,
        "UPDATE user_chat_channel_memberships SET notification_level = 1",
    )
    .await;
    discourse_rs::plugins::chat::notify::notify_watching(
        &st,
        &mut conn,
        &settings,
        &json!({ "chat_message_id": 7, "except_user_ids": [] }),
    )
    .await
    .unwrap();
    discourse_rs::plugins::chat::notify::notify_mentioned(
        &st,
        &mut conn,
        &settings,
        &json!({ "chat_message_id": 7, "to_notify_ids_map": {}, "timestamp": "2026-10-10 00:00:00 UTC" }),
    )
    .await
    .unwrap();
}
