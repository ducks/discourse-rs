//! The rest of the chat channel pane (ROADMAP 224a), its server side: the
//! extras static/js/chat.js draws from (the notices' invite, the in-channel
//! filter, the mobile arrow's unread count, the "+" menu's emoji label,
//! the code block buttons' setting), the page around a day's start that a
//! date separator loads (target_date), and the channels list drawn for no
//! member or a member without chat options.

use crate::common;

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
        let request = Request::get(path)
            .header(header::HOST, "test.localhost")
            .header(header::COOKIE, self.cookie());
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

async fn exec(db: &TestDb, sql: &str) {
    sqlx::query(sql).execute(&db.pool).await.unwrap();
}

/// The ids of the messages a page draws, in order.
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

fn extras(page: &str) -> &str {
    let at = page
        .find("<template class=\"chat-channel-extras\"")
        .expect("the channel's extras");
    let end = page[at..].find("></template>").unwrap();
    &page[at..at + end]
}

#[tokio::test]
async fn the_channel_draws_its_extras_for_chat_js() {
    let db = TestDb::new().await;
    // Three of user0's messages user1 hasn't read.
    exec(
        &db,
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         SELECT n, 2, 2, now() - (4 - n) * interval '1 minute', now(), 'message ' || n, '<p>message ' || n || '</p>', 1, 2 \
         FROM generate_series(1, 3) n",
    )
    .await;
    exec(&db, "SELECT setval('chat_messages_id_seq', 3)").await;
    exec(
        &db,
        "UPDATE user_chat_channel_memberships SET last_read_message_id = NULL \
         WHERE chat_channel_id = 2 AND user_id = (SELECT id FROM users WHERE username = 'user1')",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let page = user1.get("/chat/c/general/2").await.body;
    let x = extras(&page);
    for attr in [
        " data-label-invite=\"Invite to channel\"",
        " data-label-invitations-sent-one=\"Invitation sent\"",
        " data-label-invitations-sent-other=\"Invitations sent\"",
        " data-label-filter=\"Search chat\"",
        " data-label-done=\"Done\"",
        " data-label-no-results=\"No results found\"",
        " data-label-new-messages-one=\"%{count} new message\"",
        " data-label-new-messages-other=\"%{count} new messages\"",
        " data-label-emoji=\"Insert emoji\"",
        " data-unread-count=\"3\"",
        " data-codeblock-buttons=\"true\"",
    ] {
        assert!(x.contains(attr), "{attr} in {x}");
    }
    // After the notices, inside the channel, which the filter bar follows.
    let notices = page.find("<div class=\"chat-notices\">").unwrap();
    assert!(
        page.find("<template class=\"chat-channel-extras\"")
            .unwrap()
            > notices
    );

    // Read up to the latest: nothing unread. Code block buttons follow
    // chat_show_copy_button_on_codeblocks.
    exec(
        &db,
        "UPDATE user_chat_channel_memberships SET last_read_message_id = 3 \
         WHERE chat_channel_id = 2 AND user_id = (SELECT id FROM users WHERE username = 'user1')",
    )
    .await;
    exec(
        &db,
        "INSERT INTO site_settings (name, data_type, value, created_at, updated_at) \
         VALUES ('chat_show_copy_button_on_codeblocks', 5, 'f', now(), now())",
    )
    .await;
    let page = user1.get("/chat/c/general/2").await.body;
    let x = extras(&page);
    assert!(x.contains(" data-unread-count=\"0\""), "{x}");
    assert!(x.contains(" data-codeblock-buttons=\"false\""), "{x}");

    // The drawer's channel route draws them too.
    let drawer = user1
        .get("/live/chat/drawer?url=%2Fchat%2Fc%2Fgeneral%2F2")
        .await;
    assert_eq!(drawer.status, 200);
    assert!(
        drawer
            .body
            .contains("<template class=\"chat-channel-extras\"")
    );
}

#[tokio::test]
async fn a_date_separator_loads_the_page_around_its_day() {
    let db = TestDb::new().await;
    // 30 messages on each of three days, at noon (UTC) a minute apart:
    // ids 1 to 30 three days ago, 31 to 60 two days ago, 61 to 90
    // yesterday.
    exec(
        &db,
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         SELECT n, 2, 2, \
                date_trunc('day', now()) - (3 - (n - 1) / 30) * interval '1 day' + interval '12 hours' + ((n - 1) % 30) * interval '1 minute', \
                now(), 'message ' || n, '<p>message ' || n || '</p>', 1, 2 \
         FROM generate_series(1, 90) n",
    )
    .await;
    exec(&db, "SELECT setval('chat_messages_id_seq', 90)").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let day: String = sqlx::query_scalar(
        "SELECT to_char(date_trunc('day', now()) - interval '2 days', 'YYYY-MM-DD\"T\"HH24:MI:SS\"+00:00\"')",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();

    // fetchMessages({ target_date, direction: FUTURE }): query_by_date's
    // 25 up to the day's start and 25 after, whatever the direction.
    let page = user1
        .get(&format!(
            "/live/chat/2/messages?target_date={}&direction=future&newest=true",
            day.replace('+', "%2B")
        ))
        .await;
    assert_eq!(page.status, 200, "{}", page.body);
    assert_eq!(ids(&page.body), (6..=55).collect::<Vec<_>>());
    assert_eq!(page.header("x-chat-can-load-more-past"), Some("true"));
    assert_eq!(page.header("x-chat-can-load-more-future"), Some("true"));
    // Drawn alone: the first shows its author, no anchor after them.
    assert!(
        page.body.starts_with(
            "<div class=\"chat-message-container -persisted -processed\" data-id=\"6\""
        )
    );
    assert!(!page.body.contains("chat-message-anchor"));

    // A date in another zone, as moment formats the day's start.
    let page = user1
        .get("/live/chat/2/messages?target_date=2000-01-01T00:00:00-05:00")
        .await;
    assert_eq!(page.status, 200);
    assert_eq!(ids(&page.body), (1..=25).collect::<Vec<_>>());
    assert_eq!(page.header("x-chat-can-load-more-past"), Some("false"));
}

/// Draws a channels route as the Context draws it for `guardian`.
async fn channels_lists(
    st: &AppState,
    guardian: &discourse_rs::guardian::Guardian,
) -> (String, String) {
    let mut conn = st.pool.acquire().await.unwrap();
    let settings = discourse_rs::site_settings::SiteSettings::load(
        &mut conn,
        &st.site_setting_defs,
        &st.config.globals,
    )
    .await
    .unwrap();
    let mut cx = discourse_rs::plugins::chat::channels::Context {
        conn: &mut conn,
        settings: &settings,
        i18n: &st.i18n,
        guardian,
        base_path: "",
        config: &st.config,
    };
    let mobile = cx.channels_route().await.unwrap();
    let nav = discourse_rs::plugins::chat::drawer::Nav {
        expanded: true,
        back: "",
        i18n: &st.i18n,
        base_path: "",
    };
    let drawer = cx.drawer_channels_route(&nav).await.unwrap();
    (mobile, drawer)
}

#[tokio::test]
async fn the_channels_list_draws_without_a_member_or_chat_options() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;

    // No member: chatChannelsManager holds no channels, and the empty
    // state offers no browser (canBrowseChannels), nor does the drawer's
    // divider show (displayPublicChannels).
    let anonymous = discourse_rs::guardian::Guardian::anonymous();
    let (mobile, drawer) = channels_lists(&st, &anonymous).await;
    for html in [&mobile, &drawer] {
        assert!(html.contains("--empty-channels-list"), "{html}");
        assert!(!html.contains("empty-state__cta"), "{html}");
        assert!(!html.contains("chat-channel-row"), "{html}");
    }
    assert!(!drawer.contains("public-channels-section"), "{drawer}");

    // A member without chat options: their channels by the columns'
    // defaults (all, alphabetical).
    exec(
        &db,
        "DELETE FROM user_options WHERE user_id = (SELECT id FROM users WHERE username = 'user1')",
    )
    .await;
    let mut conn = st.pool.acquire().await.unwrap();
    let user = discourse_rs::session::current::SessionUser::load(&mut conn, 3)
        .await
        .unwrap()
        .unwrap();
    let guardian = discourse_rs::guardian::Guardian::for_user(&mut conn, &user)
        .await
        .unwrap();
    drop(conn);
    let (mobile, drawer) = channels_lists(&st, &guardian).await;
    for html in [&mobile, &drawer] {
        assert!(
            html.contains("chat-channel-row") && html.contains("General"),
            "{html}"
        );
    }
    assert!(drawer.contains("public-channels-section"), "{drawer}");
}
