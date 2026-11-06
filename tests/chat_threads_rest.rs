//! The thread pane and the threads lists beyond tests/chat_threads.rs: a
//! thread's pages of messages for its pane, the lists' next pages, the
//! settings modal and the title prompt as the thread route draws them,
//! the header's back link data, do not disturb on the thread indicators,
//! and the unread threads in the sidebar, the header and the footer.

use crate::common;

use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, header};
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
    async fn request(
        &mut self,
        method: Method,
        path: &str,
        body: &str,
        xhr: bool,
    ) -> (u16, HeaderMap, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        if xhr {
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
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap();
            let pair = v.split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            headers,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    async fn send(&mut self, method: Method, path: &str, body: &str) -> (u16, String) {
        let (status, _, body) = self.request(method, path, body, true).await;
        (status, body)
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
        self.request(Method::GET, path, "", false).await.2
    }

    /// A live fragment, with its status and headers.
    async fn live(&mut self, path: &str) -> (u16, HeaderMap, String) {
        self.request(Method::GET, path, "", false).await
    }

    fn mobile(&mut self) {
        self.cookies.push(("mobile_view".into(), "1".into()));
    }
}

async fn run(db: &TestDb, sqls: &[&str]) {
    for sql in sqls {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
}

/// tests/chat_threads.rs' threaded General channel: thread 1 on user0's
/// message 1 (user0 is 2, user1 3, admin 1), replies by user1 (2) and
/// admin (3), user0's channel message 4; user1 has read the thread to 2.
async fn threaded(db: &TestDb) {
    run(db, &[
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
        "INSERT INTO site_settings (name, data_type, value, created_at, updated_at) \
         SELECT 'chat_threads_enabled', 5, 't', now(), now() \
         WHERE NOT EXISTS (SELECT 1 FROM site_settings WHERE name = 'chat_threads_enabled')",
    ])
    .await;
}

/// Sixty more replies in thread 1 (5 to 64), a minute apart.
async fn long_thread(db: &TestDb) {
    run(db, &[
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id, thread_id) \
         SELECT n, 2, CASE WHEN n % 2 = 0 THEN 3 ELSE 1 END, now() - interval '30 minutes' + n * interval '10 seconds', now(), 'reply ' || n, '<p>reply ' || n || '</p>', 1, CASE WHEN n % 2 = 0 THEN 3 ELSE 1 END, 1 \
         FROM generate_series(5, 64) AS n",
        "SELECT setval('chat_messages_id_seq', 64)",
        "UPDATE chat_threads SET replies_count = 62, last_message_id = 64 WHERE id = 1",
    ])
    .await;
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_pane_pages_through_its_messages() {
    let db = TestDb::new().await;
    threaded(&db).await;
    long_thread(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;

    // Around the member's last read message, which the pane highlights;
    // newer pages still to load, the latest message named for the arrow.
    let page = user1.page("/chat/c/general/2/t/1").await;
    assert!(
        page.contains("data-can-load-more-past=\"false\" data-can-load-more-future=\"true\""),
        "{page}"
    );
    assert!(page.contains("data-target=\"2\""));
    assert!(page.contains("data-last-message-id=\"64\""));
    assert!(page.contains("data-label-deleted-one=\""));
    assert!(!page.contains("reply 64</p>"));

    // The next page after the last message shown, the end reached.
    let (status, headers, html) = user1
        .live("/live/chat/2/messages?thread_id=1&target_message_id=54&direction=future")
        .await;
    assert_eq!(status, 200, "{html}");
    assert_eq!(header(&headers, "x-chat-can-load-more-future"), "false");
    assert!(html.contains("reply 64</p>"));
    assert!(!html.contains("reply 54</p>"), "the anchor stays out");
    // Drawn for the thread's pane: no reply line, no thread indicator.
    assert!(!html.contains("chat-reply is-direct-reply"));
    assert!(!html.contains("chat-message-thread-indicator"));

    // The page before the first message shown, with the anchor drawn again
    // after it.
    let (status, headers, html) = user1
        .live("/live/chat/2/messages?thread_id=1&target_message_id=64&direction=past")
        .await;
    assert_eq!(status, 200);
    assert_eq!(header(&headers, "x-chat-can-load-more-past"), "true");
    assert!(html.contains("reply 63</p>"));
    assert!(html.contains("<template class=\"chat-message-anchor\" data-id=\"64\">"));

    // Around a message (a jump), and from the first without a target.
    let (_, headers, html) = user1
        .live("/live/chat/2/messages?thread_id=1&target_message_id=40")
        .await;
    assert!(html.contains("reply 40</p>"));
    assert_eq!(header(&headers, "x-chat-can-load-more-past"), "true");
    let (_, headers, html) = user1.live("/live/chat/2/messages?thread_id=1").await;
    assert!(html.contains("Hello <strong>world</strong>"), "{html}");
    assert_eq!(header(&headers, "x-chat-can-load-more-future"), "true");

    // A thread that isn't there, or isn't the channel's.
    let (status, _, _) = user1.live("/live/chat/2/messages?thread_id=99").await;
    assert_eq!(status, 404);
    let (status, _, _) = user1.live("/live/chat/2/messages?thread_id=x").await;
    assert_eq!(status, 404);
}

/// Twelve more threads in General (2 to 13), each on its own message
/// with a reply, user1 a member of each.
async fn many_threads(db: &TestDb) {
    run(db, &[
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         SELECT 100 + n, 2, 2, now() - interval '20 minutes' + n * interval '1 minute', now(), 'topic ' || n, '<p>topic ' || n || '</p>', 1, 2 \
         FROM generate_series(2, 13) AS n",
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         SELECT 200 + n, 2, 1, now() - interval '19 minutes' + n * interval '1 minute', now(), 'answer ' || n, '<p>answer ' || n || '</p>', 1, 1 \
         FROM generate_series(2, 13) AS n",
        "INSERT INTO chat_threads (id, channel_id, original_message_id, original_message_user_id, status, replies_count, force, last_message_id, created_at, updated_at) \
         SELECT n, 2, 100 + n, 2, 0, 1, false, 200 + n, now(), now() FROM generate_series(2, 13) AS n",
        "UPDATE chat_messages SET thread_id = id - 100 WHERE id BETWEEN 102 AND 113",
        "UPDATE chat_messages SET thread_id = id - 200 WHERE id BETWEEN 202 AND 213",
        "SELECT setval('chat_messages_id_seq', 213)",
        "SELECT setval('chat_threads_id_seq', 13)",
        "INSERT INTO user_chat_thread_memberships (user_id, thread_id, notification_level, last_read_message_id, created_at, updated_at) \
         SELECT 3, n, 2, 200 + n, now(), now() FROM generate_series(2, 13) AS n",
    ])
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_threads_lists_load_their_next_pages() {
    let db = TestDb::new().await;
    threaded(&db).await;
    many_threads(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;

    // The channel's first ten, then the rest from the load more.
    let page = user1.page("/chat/c/general/2/t").await;
    assert_eq!(
        page.matches("chat-thread-list-item__open-button").count(),
        10,
        "{page}"
    );
    assert!(page.contains("<div class=\"chat-thread-list\" data-channel-id=\"2\">"));
    assert!(page.contains(
        "data-load-more-url=\"/live/chat/2/threads\" data-offset=\"10\" data-done=\"false\""
    ));
    let (status, headers, html) = user1.live("/live/chat/2/threads?offset=10").await;
    assert_eq!(status, 200);
    assert_eq!(header(&headers, "x-chat-count"), "3");
    assert_eq!(
        html.matches("chat-thread-list-item__open-button").count(),
        3
    );
    let (_, headers, html) = user1.live("/live/chat/2/threads?offset=20").await;
    assert_eq!(header(&headers, "x-chat-count"), "0");
    assert!(html.is_empty());
    // Not a channel the viewer may see, or without threads.
    let (status, _, _) = user1.live("/live/chat/999/threads").await;
    assert_eq!(status, 404);

    // The member's threads: the first ten, each with its channel, then
    // the rest.
    let page = user1.page("/chat/threads").await;
    assert_eq!(page.matches("<div class=\"c-user-thread\" ").count(), 10);
    assert!(page.contains("data-channel-id=\"2\""));
    assert!(page.contains("data-load-more-url=\"/live/chat/threads\" data-offset=\"10\""));
    let (_, headers, html) = user1.live("/live/chat/threads?offset=10").await;
    assert_eq!(header(&headers, "x-chat-count"), "3");
    assert_eq!(html.matches("<div class=\"c-user-thread\" ").count(), 3);

    // Visitors get none.
    let mut visitor = Client {
        state: st.clone(),
        cookies: Vec::new(),
        csrf: None,
    };
    let (status, _, _) = visitor.live("/live/chat/threads").await;
    assert_eq!(status, 404);
}

/// The data-tracking of the first list item from `marker` on.
fn tracking_after(page: &str, marker: &str) -> Value {
    let start = page
        .find(marker)
        .unwrap_or_else(|| panic!("{marker}: {page}"));
    let rest = &page[start..];
    let at = rest.find("data-tracking=\"").expect("data-tracking") + "data-tracking=\"".len();
    let end = rest[at..].find('"').unwrap();
    serde_json::from_str(&rest[at..at + end].replace("&quot;", "\"")).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_threads_lists_carry_what_their_tracking_follows() {
    let db = TestDb::new().await;
    threaded(&db).await;
    // user1 ignores admin; user0 watches the thread.
    run(&db, &[
        "INSERT INTO ignored_users (user_id, ignored_user_id, created_at, updated_at, expiring_at) VALUES (3, 1, now(), now(), now() + interval '1 day')",
        "UPDATE user_chat_thread_memberships SET notification_level = 3 WHERE user_id = 2",
    ])
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let followed: Vec<i64> = sqlx::query_scalar(
        "SELECT chat_channel_id FROM user_chat_channel_memberships WHERE user_id = 3 AND following ORDER BY chat_channel_id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert!(followed.contains(&2));
    let last_reply: String = sqlx::query_scalar(
        "SELECT to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS') FROM chat_messages WHERE id = 3",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();

    // The channel's list: the thread's counts, its preview's last reply
    // and the member's membership; the viewer on the load more.
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let page = user1.page("/chat/c/general/2/t").await;
    let tracking = tracking_after(&page, "data-thread-id=\"1\"");
    assert_eq!(tracking["unread_count"], 1, "{tracking}");
    assert_eq!(tracking["mention_count"], 0);
    assert_eq!(tracking["watched_threads_unread_count"], 0);
    assert!(
        tracking["last_reply_created_at"]
            .as_str()
            .unwrap()
            .starts_with(&last_reply),
        "{tracking} {last_reply}"
    );
    assert_eq!(
        tracking["membership"],
        serde_json::json!({"notification_level": 2, "last_read_message_id": 2})
    );
    let viewer = format!(
        " data-viewer-id=\"3\" data-ignored-users=\"[&quot;admin&quot;]\" data-followed=\"{}\"><br></div>",
        serde_json::Value::from(followed.clone())
            .to_string()
            .replace('"', "&quot;")
    );
    assert!(
        page.contains(&format!("data-done=\"false\"{viewer}")),
        "{page}"
    );

    // The member's threads: the same on each thread and the load more.
    let page = user1.page("/chat/threads").await;
    assert_eq!(
        tracking_after(&page, "<div class=\"c-user-thread\" data-id=\"1\""),
        tracking
    );
    assert!(
        page.contains(&format!("data-offset=\"1\"{viewer}")),
        "{page}"
    );

    // A watcher's unread replies count as watched.
    let mut user0 = Client::logged_in(st.clone(), "user0").await;
    let page = user0.page("/chat/c/general/2/t").await;
    let tracking = tracking_after(&page, "data-thread-id=\"1\"");
    assert_eq!(
        tracking["membership"],
        serde_json::json!({"notification_level": 3, "last_read_message_id": null})
    );
    assert!(
        tracking["watched_threads_unread_count"].as_i64().unwrap() > 0,
        "{tracking}"
    );
    assert!(page.contains("data-viewer-id=\"2\" data-ignored-users=\"[]\""));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_thread_settings_are_offered_to_who_may_change_them() {
    let db = TestDb::new().await;
    threaded(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;

    // The original message's author: the title clickable, the gear, the
    // modal's template; no Discourse AI call to action.
    let mut user0 = Client::logged_in(st.clone(), "user0").await;
    let page = user0.page("/chat/c/general/2/t/1").await;
    assert!(
        page.contains("c-navbar__title clickable\" title=\"Thread\" role=\"button\""),
        "{page}"
    );
    assert!(page.contains("c-navbar__thread-settings-button"));
    assert!(page.contains("<template class=\"chat-modal-thread-settings-template\" data-title=\"Set thread title\"><div class=\"chat-modal-thread-settings__header-primary-action\"><button class=\"btn btn-text btn-transparent --primary\" type=\"button\"><span class=\"d-button-label\">Save</span></button></div><div class=\"chat-modal-thread-settings__body\">"));
    assert!(page.contains("<div class=\"chat-modal-thread-settings__footer\"><button class=\"btn btn-text btn-primary\" type=\"button\"><span class=\"d-button-label\">Save Changes</span></button></div></template>"));
    assert!(page.contains("class=\"chat-modal-thread-settings__title-input\" maxlength=\"100\" name=\"thread-title\" placeholder=\"Tell others what this conversation is about...\" type=\"text\" value=\"\""));
    assert!(page.contains("<div class=\"thread-title-length\"><span>0</span>/100</div>"));
    assert!(!page.contains("discourse-ai-cta"));

    // Another member: neither.
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let page = user1.page("/chat/c/general/2/t/1").await;
    assert!(!page.contains("chat-modal-thread-settings-template"));
    assert!(!page.contains("c-navbar__title clickable"));

    // An admin: the call to action too.
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let page = admin.page("/chat/c/general/2/t/1").await;
    assert!(page.contains("chat-modal-thread-settings-template"));
    assert!(page.contains("<p class=\"discourse-ai-cta__title\">"));
    assert!(page.contains("Check out <a href='https://www.discourse.org/ai' rel='noopener noreferrer' target='_blank'>Discourse AI</a> to learn more"));

    // Saved as the modal does: the header and the heading follow.
    let (status, body) = user0
        .send(
            Method::PUT,
            "/chat/api/channels/2/threads/1",
            "title=Our+plans",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let page = user0.page("/chat/c/general/2/t/1").await;
    assert!(page.contains("<span class=\"c-navbar__title-text\">Our plans</span>"));
    assert!(page.contains("<h2 class=\"chat-thread__heading-title\">Our plans</h2>"));
    assert!(page.contains("value=\"Our plans\""));
    assert!(page.contains("<span>9</span>/100"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_title_prompt_shows_on_a_phone_until_seen() {
    let db = TestDb::new().await;
    threaded(&db).await;
    // Five replies: the threshold.
    run(&db, &[
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id, thread_id) \
         SELECT n, 2, 3, now() - interval '20 minutes' + n * interval '1 minute', now(), 'more ' || n, '<p>more ' || n || '</p>', 1, 3, 1 \
         FROM generate_series(5, 7) AS n",
        "SELECT setval('chat_messages_id_seq', 7)",
        "UPDATE chat_threads SET replies_count = 5, last_message_id = 7 WHERE id = 1",
    ])
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user0 = Client::logged_in(st.clone(), "user0").await;

    // Not on desktop.
    let page = user0.page("/chat/c/general/2/t/1").await;
    assert!(page.contains("data-title-prompt=\"false\""), "{page}");
    assert!(!page.contains("chat-thread-title-toast-template"));

    user0.mobile();
    let page = user0.page("/chat/c/general/2/t/1").await;
    assert!(page.contains("data-title-prompt=\"true\""), "{page}");
    assert!(
        page.contains(
            "<template class=\"chat-thread-title-toast-template\" data-username=\"user0\">"
        )
    );
    assert!(page.contains("<div class=\"fk-d-default-toast__title\">Set a thread title</div>"));
    assert!(page.contains("toast-hide\" type=\"button\"><span class=\"d-button-label\">Don&#x27;t show again</span>") || page.contains("Don't show again"));

    // Another member never sees it.
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    user1.mobile();
    let page = user1.page("/chat/c/general/2/t/1").await;
    assert!(page.contains("data-title-prompt=\"false\""));

    // Seen once, it's gone.
    let (status, body) = user0
        .send(
            Method::POST,
            "/chat/api/channels/2/threads/1/mark-thread-title-prompt-seen/me",
            "",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let page = user0.page("/chat/c/general/2/t/1").await;
    assert!(page.contains("data-title-prompt=\"false\""));

    // Nor for a member who turned the prompts off.
    run(&db, &[
        "UPDATE user_chat_thread_memberships SET thread_title_prompt_seen = false WHERE user_id = 2",
        "UPDATE user_options SET show_thread_title_prompts = false WHERE user_id = 2",
    ])
    .await;
    let page = user0.page("/chat/c/general/2/t/1").await;
    assert!(page.contains("data-title-prompt=\"false\""));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_back_link_and_do_not_disturb() {
    let db = TestDb::new().await;
    threaded(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;

    // What chat-thread.js picks the back link from (ChatHistory).
    let page = user1.page("/chat/c/general/2/t/1").await;
    assert!(page.contains("data-channel-href=\"/chat/c/general/2\" data-threads-href=\"/chat/c/general/2/t\" data-my-threads-href=\"/chat/threads\" data-label-channel=\"Return to channel\" data-label-threads=\"Return to threads list\" data-label-my-threads=\"My Threads\""), "{page}");
    assert!(page.contains(
        "data-unread-indicator=\"&lt;div class&#x3D;&quot;chat-thread-header-unread-indicator&quot;"
    ));

    // In do not disturb: the counts hide (the threads button keeps its
    // has-unreads), and the back link goes to the channel.
    run(&db, &[
        "INSERT INTO do_not_disturb_timings (user_id, starts_at, ends_at, scheduled) VALUES (3, now() - interval '1 hour', now() + interval '1 hour', false)",
    ])
    .await;
    let page = user1.page("/chat/c/general/2").await;
    assert!(page.contains("c-navbar__threads-list-button btn no-text btn-transparent has-unreads"));
    // The indicator's last replier without a link of its own: a link in
    // the indicator's link would be split out of it by the parser.
    assert!(page.contains("<div class=\"chat-message-thread-indicator__last-reply-avatar\"><div class=\"chat-user-avatar\" data-username=\"admin\"><span class=\"chat-user-avatar__container\">"));
    assert!(!page.contains("chat-thread-header-unread-indicator"));
    let page = user1.page("/chat/c/general/2/t/1").await;
    assert!(page.contains("title=\"Return to channel\" href=\"/chat/c/general/2\""));
    assert!(page.contains("data-unread-indicator=\"\""));
    assert!(!page.contains("chat-thread-header-unread-indicator__number"));
}

#[tokio::test(flavor = "multi_thread")]
async fn unread_threads_show_in_the_sidebar_header_and_footer() {
    let db = TestDb::new().await;
    threaded(&db).await;
    // user1 viewed the channel after the thread's last reply.
    run(&db, &[
        "UPDATE user_chat_channel_memberships SET last_viewed_at = now() WHERE chat_channel_id = 2 AND user_id = 3",
    ])
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;

    // My threads, with the unread dot; the header icon's dot from the
    // unread thread alone; the channel's link without one (no reply since
    // it was viewed).
    let page = user1.page("/latest").await;
    let my_threads = page
        .find("data-list-item-name=\"user-threads\"")
        .map(|at| &page[at..at + 1500])
        .unwrap_or_else(|| panic!("no My threads link: {page}"));
    assert!(
        my_threads.contains("href=\"/chat/threads\""),
        "{my_threads}"
    );
    assert!(my_threads.contains("sidebar-section-link-content-badge icon unread"));
    let icon = page
        .find("chat-header-icon")
        .map(|at| &page[at..at + 800])
        .unwrap();
    assert!(
        icon.contains("<div class=\"chat-channel-unread-indicator\"></div>"),
        "{icon}"
    );
    assert_eq!(
        page.matches("sidebar-section-link-content-badge icon")
            .count(),
        1,
        "only My threads"
    );

    // A reply since the channel was viewed: its link has the dot too, and
    // a watched thread makes My threads urgent.
    run(&db, &[
        "UPDATE user_chat_channel_memberships SET last_viewed_at = now() - interval '1 day' WHERE chat_channel_id = 2 AND user_id = 3",
    ])
    .await;
    let page = user1.page("/latest").await;
    assert_eq!(
        page.matches("sidebar-section-link-content-badge icon unread")
            .count(),
        2,
        "{page}"
    );

    // The My threads route marks its link active.
    let page = user1.page("/chat/threads").await;
    let my_threads = page
        .find("data-list-item-name=\"user-threads\"")
        .map(|at| &page[at..at + 1500])
        .unwrap();
    assert!(my_threads.contains("active"), "{my_threads}");

    // The phone's footer: My threads with its dot, and the channel row's
    // unread state.
    user1.mobile();
    let page = user1.page("/chat/channels").await;
    let footer = page
        .find("id=\"c-footer-threads\"")
        .map(|at| &page[at..at + 1000])
        .unwrap_or_else(|| panic!("no threads footer item: {page}"));
    assert!(
        footer.contains("<div class=\"c-unread-indicator\"></div>"),
        "{footer}"
    );
    assert!(page.contains("class=\"chat-channel-row has-unread\""));

    // Read, nothing is left.
    run(
        &db,
        &["UPDATE user_chat_thread_memberships SET last_read_message_id = 3 WHERE user_id = 3"],
    )
    .await;
    let mut desktop = Client::logged_in(st.clone(), "user1").await;
    let page = desktop.page("/latest").await;
    assert_eq!(
        page.matches("sidebar-section-link-content-badge icon")
            .count(),
        0
    );
    let icon = page
        .find("chat-header-icon")
        .map(|at| &page[at..at + 800])
        .unwrap();
    assert!(!icon.contains("chat-channel-unread-indicator"), "{icon}");
}
