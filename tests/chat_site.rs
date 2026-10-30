//! Chat's keys on core: its markdown options and hashtag sources on
//! /site.json, chat_notifications on /notifications/totals, chat_quoted
//! notifications from posts quoting messages, the chat_summary email
//! unsubscriber, and the chat API key scope following the plugin.

mod common;

use axum::body::Body;
use axum::http::{Request, header};
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
    headers: Vec<(&'static str, String)>,
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
        for (name, value) in &self.headers {
            request = request.header(*name, value.as_str());
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

    async fn anonymous(state: AppState) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
            headers: Vec::new(),
        };
        let (_, csrf) = client.send("GET", "/session/csrf.json", "").await;
        let csrf: Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client::anonymous(state).await;
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

#[tokio::test]
async fn site_json_carries_chats_hashtag_sources_and_markdown_options() {
    let db = TestDb::new().await;
    common::set_setting(&db.pool, "tagging_enabled", 5, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut anon = Client::anonymous(st.clone()).await;
    let site = anon.json("/site.json").await;
    let contexts = json!({
        "topic-composer": ["category", "tag", "channel"],
        "chat-composer": ["channel", "category", "tag"],
    });
    assert_eq!(site["hashtag_configurations"], contexts);
    assert_eq!(
        site["hashtag_icons"],
        json!({"category": "folder", "tag": "tag", "channel": "comment"})
    );
    let chat = &site["markdown_additional_options"]["chat"];
    assert_eq!(chat["hashtag_configurations"], contexts);
    assert_eq!(chat["limited_pretty_text_features"][6], "chat-transcript");
    assert_eq!(chat["limited_pretty_text_markdown_rules"][15], "entity");

    // Without public channels the channel source is off; the contexts
    // stay.
    common::set_setting(&db.pool, "enable_public_channels", 5, "f").await;
    let site = anon.json("/site.json").await;
    assert_eq!(
        site["hashtag_configurations"],
        json!({"topic-composer": ["category", "tag"], "chat-composer": ["category", "tag"]})
    );
    assert_eq!(
        site["hashtag_icons"],
        json!({"category": "folder", "tag": "tag"})
    );

    // With chat off its registrations go; the markdown options stay, set
    // once whether or not the plugin is enabled.
    common::set_setting(&db.pool, "chat_enabled", 5, "f").await;
    let site = anon.json("/site.json").await;
    assert_eq!(
        site["hashtag_configurations"],
        json!({"topic-composer": ["category", "tag"]})
    );
    assert_eq!(
        site["markdown_additional_options"]["chat"]["hashtag_configurations"],
        json!({"topic-composer": ["category", "tag"]})
    );
}

/// user1's DM with user2 (channel 3) holding two messages from user2 that
/// user1 hasn't read.
async fn unread_dm(db: &TestDb) {
    let day = "now() - interval '1 day'";
    for sql in [
        format!(
            "INSERT INTO direct_message_channels (id, \"group\", created_at, updated_at) VALUES (1, false, {day}, {day})"
        ),
        format!(
            "INSERT INTO direct_message_users (direct_message_channel_id, user_id, created_at, updated_at) VALUES (1, 3, {day}, {day}), (1, 4, {day}, {day})"
        ),
        format!(
            "INSERT INTO chat_channels (id, chatable_type, chatable_id, type, status, user_count, messages_count, created_at, updated_at) VALUES (3, 'DirectMessage', 1, 'DirectMessageChannel', 0, 2, 2, {day}, {day})"
        ),
        format!(
            "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id, excerpt) VALUES \
             (1, 3, 4, {day}, {day}, 'hi', '<p>hi</p>', 1, 4, 'hi'), (2, 3, 4, {day}, {day}, 'there', '<p>there</p>', 1, 4, 'there')"
        ),
        "UPDATE chat_channels SET last_message_id = 2 WHERE id = 3".into(),
        format!(
            "INSERT INTO user_chat_channel_memberships (user_id, chat_channel_id, following, muted, notification_level, created_at, updated_at) VALUES (3, 3, true, false, 2, {day}, {day}), (4, 3, true, false, 2, {day}, {day})"
        ),
    ] {
        sqlx::query(&sql).execute(&db.pool).await.unwrap();
    }
}

#[tokio::test]
async fn notification_totals_count_chat_for_those_who_chat() {
    let db = TestDb::new().await;
    unread_dm(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (status, _) = Client::anonymous(st.clone())
        .await
        .send("GET", "/notifications/totals.json", "")
        .await;
    assert_eq!(status, 403);

    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let totals = user1.json("/notifications/totals.json").await;
    let keys: Vec<&str> = totals
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "username",
            "unread_notifications",
            "unread_personal_messages",
            "topic_tracking",
            "group_inboxes",
            "chat_notifications"
        ],
        "{totals}"
    );
    assert_eq!(totals["username"], "user1");
    assert_eq!(totals["group_inboxes"], json!([]));
    // The DM's two unread messages.
    assert_eq!(totals["chat_notifications"], 2);

    // Staff see their unseen reviewables.
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let totals = admin.json("/notifications/totals").await;
    assert!(totals.get("unseen_reviewables").is_some(), "{totals}");
    assert_eq!(totals["chat_notifications"], 0);

    // Chat turned off in the user's preferences drops the key.
    sqlx::query("UPDATE user_options SET chat_enabled = false WHERE user_id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let totals = user1.json("/notifications/totals.json").await;
    assert!(totals.get("chat_notifications").is_none(), "{totals}");
}

async fn run_post_alert(st: &AppState, post_id: i32, new_record: bool) {
    let job: i64 = sqlx::query_scalar(
        "INSERT INTO discourse_rs.jobs (name, args) VALUES ('post_alert', $1) RETURNING id",
    )
    .bind(json!({ "post_id": post_id, "new_record": new_record, "options": null }))
    .fetch_one(&st.pool)
    .await
    .unwrap();
    discourse_rs::jobs::perform_now(st, job).await.unwrap();
    let error: Option<String> =
        sqlx::query_scalar("SELECT last_error FROM discourse_rs.jobs WHERE id = $1")
            .bind(job)
            .fetch_optional(&st.pool)
            .await
            .unwrap()
            .flatten();
    assert_eq!(error, None, "post_alert ran");
}

/// A regular reply by someone other than user2 and admin in a public
/// topic, its raw quoting messages of user2, admin and its own author.
async fn quoting_post(st: &AppState) -> (i32, String) {
    let (post_id, username): (i32, String) = sqlx::query_as(
        "SELECT p.id, u.username FROM posts p JOIN users u ON u.id = p.user_id \
         JOIN topics t ON t.id = p.topic_id JOIN categories c ON c.id = t.category_id \
         WHERE t.archetype = 'regular' AND p.post_number > 1 AND p.post_type = 1 \
           AND p.deleted_at IS NULL AND t.deleted_at IS NULL AND NOT c.read_restricted \
           AND p.user_id > 0 AND p.user_id NOT IN (1, 4) \
         ORDER BY p.id LIMIT 1",
    )
    .fetch_one(&st.pool)
    .await
    .unwrap();
    let raw = format!(
        "[chat quote=\"User2;1;2026-10-01T00:00:00Z\" channel=\"General\" channelId=\"2\"]\nhi\n[/chat]\n\n\
         [chat quote=\"{username};2;2026-10-01T00:00:00Z\" channel=\"General\" channelId=\"2\"]\nme\n[/chat]\n\n\
         [chat quote=\"user2;3;2026-10-01T00:00:00Z\" channel=\"General\" channelId=\"2\"]\nagain\n[/chat]\n\n\
         [chat quote=\"admin\"]\nno id, no match\n[/chat]"
    );
    sqlx::query("UPDATE posts SET raw = $2 WHERE id = $1")
        .bind(post_id)
        .bind(&raw)
        .execute(&st.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM notifications")
        .execute(&st.pool)
        .await
        .unwrap();
    (post_id, username)
}

async fn chat_quoted(st: &AppState) -> Vec<(i32, Value)> {
    let rows: Vec<(i32, String)> = sqlx::query_as(
        "SELECT user_id, data FROM notifications WHERE notification_type = 33 ORDER BY id",
    )
    .fetch_all(&st.pool)
    .await
    .unwrap();
    rows.into_iter()
        .map(|(u, d)| (u, serde_json::from_str(&d).unwrap()))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_post_quoting_chat_messages_notifies_their_authors() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (post_id, username) = quoting_post(&st).await;
    run_post_alert(&st, post_id, true).await;
    // user2 once (named twice, any case); not the author, nor a tag
    // without its ";".
    let notified = chat_quoted(&st).await;
    assert_eq!(notified.len(), 1, "{notified:?}");
    assert_eq!(notified[0].0, 4);
    assert_eq!(notified[0].1["display_username"], username.as_str());

    // An edit (not a new record) notifies nobody.
    sqlx::query("DELETE FROM notifications")
        .execute(&db.pool)
        .await
        .unwrap();
    run_post_alert(&st, post_id, false).await;
    assert!(chat_quoted(&st).await.is_empty());

    // Nor does a whisper, nor anything with chat off.
    sqlx::query("UPDATE posts SET post_type = 4 WHERE id = $1")
        .bind(post_id)
        .execute(&db.pool)
        .await
        .unwrap();
    run_post_alert(&st, post_id, true).await;
    assert!(chat_quoted(&st).await.is_empty());
    sqlx::query("UPDATE posts SET post_type = 1 WHERE id = $1")
        .bind(post_id)
        .execute(&db.pool)
        .await
        .unwrap();
    common::set_setting(&db.pool, "chat_enabled", 5, "f").await;
    run_post_alert(&st, post_id, true).await;
    assert!(chat_quoted(&st).await.is_empty());
}

#[tokio::test]
async fn the_chat_summary_unsubscribe_offers_its_frequency() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let key = {
        let mut conn = db.pool.acquire().await.unwrap();
        discourse_rs::email::unsubscribe::create_key_for(&mut conn, 3, "chat_summary", None)
            .await
            .unwrap()
    };
    let mut anonymous = Client::anonymous(st.clone()).await;
    let (status, html) = anonymous
        .send("GET", &format!("/email/unsubscribe/{key}"), "")
        .await;
    assert_eq!(status, 200, "{html}");
    assert!(
        html.contains("  <p>\n    <label>Set chat summary emails to:</label>\n    <select name=\"chat_email_frequency\" id=\"chat_email_frequency\" class=\"combobox\"><option value=\"never\">Never</option>\n<option selected=\"selected\" value=\"when_away\">Only when away</option></select>\n  </p>\n"),
        "{html}"
    );
    let (status, _) = anonymous
        .send(
            "POST",
            &format!("/email/unsubscribe/{key}"),
            "chat_email_frequency=never",
        )
        .await;
    assert!(status < 400, "{status}");
    let stored: i32 =
        sqlx::query_scalar("SELECT chat_email_frequency FROM user_options WHERE user_id = 3")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(stored, 0);
}

#[tokio::test]
async fn the_chat_api_key_scope_follows_the_plugin() {
    use sha2::{Digest, Sha256};
    let db = TestDb::new().await;
    sqlx::query(
        "INSERT INTO api_keys (id, user_id, created_by_id, created_at, updated_at, key_hash, truncated_key, scope_mode) \
         VALUES (9701, 1, 1, now(), now(), $1, 'abcd', 2)",
    )
    .bind(format!("{:x}", Sha256::digest(b"chat-scope-key")))
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO api_key_scopes (api_key_id, resource, action, created_at, updated_at) \
         VALUES (9701, 'chat', 'create_message', now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let scopes = admin.json("/admin/api/keys/scopes.json").await;
    assert_eq!(
        scopes["scopes"]["chat"][0]["scope_id"], "chat:create_message",
        "{scopes}"
    );
    let key = admin.json("/admin/api/keys/9701.json").await;
    assert_eq!(
        key["key"]["api_key_scopes"][0]["urls"],
        json!(["/chat/:chat_channel_id (POST)"])
    );

    let mut api = Client {
        state: st.clone(),
        cookies: Vec::new(),
        csrf: None,
        headers: vec![
            ("api-key", "chat-scope-key".into()),
            ("api-username", "admin".into()),
        ],
    };
    let credentials = "The API username or key is invalid.";
    let (_, body) = api.send("POST", "/chat/2", "message=hello").await;
    assert!(!body.contains(credentials), "{body}");

    // With chat off the scope is gone: listed nowhere, permitting nothing.
    common::set_setting(&db.pool, "chat_enabled", 5, "f").await;
    let scopes = admin.json("/admin/api/keys/scopes.json").await;
    assert!(scopes["scopes"].get("chat").is_none(), "{scopes}");
    let key = admin.json("/admin/api/keys/9701.json").await;
    assert_eq!(key["key"]["api_key_scopes"][0]["urls"], json!([]));
    let (status, body) = api.send("POST", "/chat/2", "message=hello").await;
    assert_eq!(status, 403);
    assert!(body.contains(credentials), "{body}");
}
