//! The chat channel pane's paging and jumps (ROADMAP 224): the pages of
//! messages static/js/chat.js loads as the member scrolls or jumps
//! (GET /live/chat/:channel_id/messages), a near-message url's target,
//! a target that can't load, the `/channel -> /c` redirects and a
//! channel without a slug.

mod common;

use axum::body::Body;
use axum::http::{Request, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use tower::ServiceExt;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
}

struct Answer {
    status: u16,
    headers: axum::http::HeaderMap,
    body: String,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

impl Client {
    async fn get(&mut self, path: &str) -> Answer {
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
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Answer {
            status,
            headers,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
        };
        let mut request = Request::get("/session/csrf.json")
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest");
        let response = discourse_rs::app(client.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        client.keep_cookies(&response);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let csrf: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        request = Request::post("/session")
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest")
            .header("x-csrf-token", csrf["csrf"].as_str().unwrap())
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::COOKIE, client.cookie());
        let response = discourse_rs::app(client.state.clone())
            .oneshot(
                request
                    .body(Body::from(format!("login={username}&password=password")))
                    .unwrap(),
            )
            .await
            .unwrap();
        client.keep_cookies(&response);
        client
    }

    fn cookie(&self) -> String {
        self.cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn keep_cookies(&mut self, response: &axum::response::Response) {
        for v in response.headers().get_all(header::SET_COOKIE) {
            let pair = v.to_str().unwrap().split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
    }
}

/// 120 messages in General (ids 1 to 120), user0's, a minute apart,
/// user1 having read none.
async fn many_messages(db: &TestDb) {
    for sql in [
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         SELECT n, 2, 2, now() - (121 - n) * interval '1 minute', now() - (121 - n) * interval '1 minute', 'message ' || n, '<p>message ' || n || '</p>', 1, 2 \
         FROM generate_series(1, 120) n",
        "SELECT setval('chat_messages_id_seq', 120)",
        "UPDATE chat_channels SET last_message_id = 120 WHERE id = 2",
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
}

/// The ids of the messages a page draws, in order (not the staged
/// message templates' 0 and -1).
fn ids(html: &str) -> Vec<i64> {
    html.split("<div class=\"chat-message-container")
        .skip(1)
        .filter_map(|part| {
            let at = part.find("data-id=\"")? + 9;
            part[at..].split('"').next()?.parse().ok()
        })
        .filter(|id| *id > 0)
        .collect()
}

#[tokio::test]
async fn the_channel_pages_through_its_messages() {
    let db = TestDb::new().await;
    many_messages(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;

    // The latest page, more to load before it.
    let page = user1.get("/chat/c/general/2").await.body;
    assert!(
        page.contains(" data-can-load-more-past=\"true\" data-can-load-more-future=\"false\" data-label-all-loaded=\"Showing all messages\" data-label-not-found=\"The requested URL or resource could not be found.\""),
        "{page}"
    );
    assert!(!page.contains("data-target="));
    assert!(!page.contains("all-loaded-message"));
    let shown = ids(&page);
    assert_eq!(shown.first(), Some(&71));
    assert_eq!(shown.last(), Some(&120));

    // Older: the 50 before the first, which is drawn again after them.
    let older = user1
        .get("/live/chat/2/messages?target_message_id=71&direction=past")
        .await;
    assert_eq!(older.status, 200);
    assert_eq!(older.header("x-chat-can-load-more-past"), Some("true"));
    // A page past says nothing of newer ones, as the API answers.
    assert_eq!(older.header("x-chat-can-load-more-future"), Some("null"));
    let (batch, anchor) = older
        .body
        .split_once("<template class=\"chat-message-anchor\" data-id=\"71\">")
        .unwrap();
    assert_eq!(ids(batch), (21..=70).collect::<Vec<_>>());
    assert_eq!(ids(anchor), vec![71]);
    // The first of a page shows its author; the anchor too (it was the
    // first of its own).
    assert!(
        batch.starts_with(
            "<div class=\"chat-message-container -persisted -processed\" data-id=\"21\""
        )
    );
    assert!(
        anchor.starts_with(
            "<div class=\"chat-message-container -persisted -processed\" data-id=\"71\""
        )
    );
    assert!(batch.contains("-user-info-hidden\" data-id=\"22\""));

    // The oldest: nothing more past.
    let oldest = user1
        .get("/live/chat/2/messages?target_message_id=21&direction=past")
        .await;
    assert_eq!(oldest.header("x-chat-can-load-more-past"), Some("false"));
    assert_eq!(
        ids(oldest.body.split("<template").next().unwrap()),
        (1..=20).collect::<Vec<_>>()
    );

    // Newer, after a message: its first shows its author.
    let newer = user1
        .get("/live/chat/2/messages?target_message_id=10&direction=future")
        .await;
    assert_eq!(newer.header("x-chat-can-load-more-future"), Some("true"));
    assert_eq!(ids(&newer.body), (11..=60).collect::<Vec<_>>());
    assert!(
        newer.body.starts_with(
            "<div class=\"chat-message-container -persisted -processed\" data-id=\"11\""
        )
    );
    assert!(!newer.body.contains("chat-message-anchor"));

    // Around a message, and the latest.
    let around = user1
        .get("/live/chat/2/messages?target_message_id=60")
        .await;
    assert_eq!(around.header("x-chat-can-load-more-past"), Some("true"));
    assert_eq!(around.header("x-chat-can-load-more-future"), Some("true"));
    assert!(ids(&around.body).contains(&60));
    let latest = user1.get("/live/chat/2/messages").await;
    assert_eq!(ids(&latest.body), (71..=120).collect::<Vec<_>>());
    // The "last visit" line only when asked for (the page has none).
    assert!(!latest.body.contains("data-newest"));
    let latest = user1.get("/live/chat/2/messages?newest=true").await;
    assert!(latest.body.contains("data-newest"));

    // As the API refuses: a message that isn't there, a channel they
    // can't see.
    assert_eq!(
        user1
            .get("/live/chat/2/messages?target_message_id=999")
            .await
            .status,
        404
    );
    assert_eq!(user1.get("/live/chat/1/messages").await.status, 404);
    assert_eq!(
        user1
            .get("/live/chat/2/messages?target_message_id=x")
            .await
            .status,
        404
    );
}

#[tokio::test]
async fn a_near_message_url_targets_its_message() {
    let db = TestDb::new().await;
    many_messages(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let page = user1.get("/chat/c/general/2/30").await.body;
    assert!(
        page.contains("<div class=\"chat-channel --loaded\" data-id=\"2\""),
        "{page}"
    );
    assert!(page.contains(" data-can-load-more-past=\"true\" data-can-load-more-future=\"true\""));
    assert!(page.contains(" data-target=\"30\">"));
    assert!(ids(&page).contains(&30));

    // One that can't load: the skeleton, unloaded, for chat.js to pop
    // the 404's error over.
    let page = user1.get("/chat/c/general/2/999").await;
    assert_eq!(page.status, 200);
    assert!(
        page.body
            .contains("<div class=\"chat-channel\" data-id=\"2\""),
        "{}",
        page.body
    );
    assert!(
        page.body.contains(
            "<div class=\"chat-skeleton --animation\"><div class=\"chat-skeleton__body\">"
        )
    );
    assert!(!page.body.contains("data-target="));
    assert!(!page.body.contains("empty-state"));
}

#[tokio::test]
async fn the_channel_routes_redirect_to_the_c_routes() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut visitor = Client {
        state: st,
        cookies: Vec::new(),
    };
    for (from, to) in [
        ("/chat/channel/2", "/chat/c/-/2"),
        ("/chat/channel/2/general", "/chat/c/general/2"),
        ("/chat/channel/2/a%20b", "/chat/c/a%20b/2"),
        ("/chat/channel/2/general/info", "/chat/c/general/2/info"),
        (
            "/chat/channel/2/general/info/members",
            "/chat/c/general/2/info/members",
        ),
        (
            "/chat/channel/2/general/info/settings",
            "/chat/c/general/2/info/settings",
        ),
    ] {
        let answer = visitor.get(from).await;
        assert_eq!(answer.status, 301, "{from}");
        assert_eq!(
            answer.header("location"),
            Some(format!("http://test.localhost{to}").as_str()),
            "{from}"
        );
    }
    // An info route the engine doesn't redirect, and /chat/message/:id,
    // which only Ember's router knows: not found, as Rails answers a page
    // load of either.
    for path in ["/chat/channel/2/general/info/nope", "/chat/message/3"] {
        assert_eq!(visitor.get(path).await.status, 404, "{path}");
    }
}

#[tokio::test]
async fn a_channel_without_a_slug_takes_its_title() {
    let db = TestDb::new().await;
    sqlx::query("UPDATE chat_channels SET slug = NULL WHERE id = 2")
        .execute(&db.pool)
        .await
        .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let answer = user1.get("/chat/c/-/2").await;
    assert_eq!(answer.status, 302);
    assert_eq!(answer.header("location"), Some("/chat/c/general/2"));
    let page = user1.get("/chat/c/general/2").await;
    assert_eq!(page.status, 200);
    assert!(page.body.contains("<title>#General - Chat - "));
}

#[tokio::test]
async fn a_reaction_without_an_emoji_image_draws_nothing() {
    let db = TestDb::new().await;
    many_messages(&db).await;
    sqlx::query(
        "INSERT INTO chat_message_reactions (chat_message_id, user_id, emoji, created_at, updated_at) \
         VALUES (120, 3, 'not_an_emoji_at_all', now(), now()), (120, 2, 'heart', now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let page = user1.get("/chat/c/general/2").await;
    assert_eq!(page.status, 200);
    assert!(
        !page
            .body
            .contains("data-emoji-name=\"not_an_emoji_at_all\"")
    );
    assert!(page.body.contains("data-emoji-name=\"heart\""));
}
