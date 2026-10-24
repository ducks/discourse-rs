//! The bundled plugins' UI on the server-rendered pages. discourse-reactions:
//! the reaction controls in the post menu, with what the browser side reads
//! to draw the picker and the users menu, and the activity Reactions tab.
//! discourse-solved: the solved status, the accepted answers under the
//! first post, the Solved button, "Me too" and the activity Solved tab.
//! discourse-poll: each cooked poll drawn as its widget, and drawn again
//! for poll.js in the state a click brings it to. checklist: what
//! static/js/checklist.js reads of a post with boxes, and a toggle's token
//! on its revised message.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use tower::ServiceExt;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn send(&mut self, method: Method, path: &str, body: &str) -> String {
        self.send_as(method, path, body, "application/x-www-form-urlencoded")
            .await
    }

    async fn send_as(
        &mut self,
        method: Method,
        path: &str,
        body: &str,
        content_type: &str,
    ) -> String {
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
            request = request.header(header::CONTENT_TYPE, content_type);
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
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
        String::from_utf8_lossy(&bytes).into_owned()
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let csrf = client.send(Method::GET, "/session/csrf.json", "").await;
        let csrf: serde_json::Value = serde_json::from_str(&csrf).unwrap();
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
        let mut request = Request::get(path).header(header::HOST, "test.localhost");
        let cookie: Vec<String> = self
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        request = request.header(header::COOKIE, cookie.join("; "));
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// user1's clap on user0's post 35, with its shadow like.
async fn clap(db: &TestDb) {
    for sql in [
        "INSERT INTO discourse_reactions_reactions (post_id, reaction_type, reaction_value, reaction_users_count, created_at, updated_at) \
         VALUES (35, 0, 'clap', 1, '2026-10-01 04:00:00', '2026-10-01 04:00:00')",
        "INSERT INTO discourse_reactions_reaction_users (reaction_id, user_id, post_id, created_at, updated_at) \
         SELECT id, 3, 35, '2026-10-01 04:00:00', '2026-10-01 04:00:00' FROM discourse_reactions_reactions WHERE post_id = 35",
        "INSERT INTO post_actions (post_id, user_id, post_action_type_id, created_at, updated_at) \
         VALUES (35, 3, 2, '2026-10-01 04:00:00', '2026-10-01 04:00:00')",
        "UPDATE posts SET like_count = 1 WHERE id = 35",
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
}

#[tokio::test]
async fn a_members_controls_carry_the_picker_and_the_users_menu() {
    let db = TestDb::new().await;
    clap(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user2 = Client::logged_in(st, "user2").await;
    let html = user2.page("/t/parity-fixture-replies-and-posters/35").await;

    // Someone else's clap: user2 may react, so the picker is there to draw,
    // seven reactions in seven columns, the main one first.
    let right = html
        .split("id=\"discourse-reactions-actions-35-right\"")
        .nth(1)
        .expect("the reaction button");
    let picker = right
        .split("data-picker=\"")
        .nth(1)
        .and_then(|p| p.split('"').next())
        .expect("the picker's data")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#x3D;", "=")
        .replace("&#x27;", "'")
        .replace("&#39;", "'");
    let picker: serde_json::Value = serde_json::from_str(&picker).unwrap();
    assert_eq!(picker["cols"], 7);
    let ids: Vec<&str> = picker["reactions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "heart",
            "+1",
            "laughing",
            "open_mouth",
            "clap",
            "confetti_ball",
            "hugs"
        ]
    );
    assert_eq!(
        picker["reactions"][0]["title"],
        "React to this post with: heart"
    );
    assert_eq!(picker["reactions"][0]["canUndo"], true);
    assert!(right.contains("title=\"Like this post\""));

    // The counter: one reaction, from the users list.
    assert!(html.contains(
        "aria-label=\"1 reaction\" class=\"discourse-reactions-counter\" id=\"discourse-reactions-counter-35-left\""
    ));
    assert!(html.contains("/discourse-reactions/posts/35/reactions-users-list.json"));

    // user1 sees their own reaction on the button.
    let mut user1 = Client::logged_in(
        state(db.pool.clone(), config(RailsEnv::Test, &[])).await,
        "user1",
    )
    .await;
    let html = user1.page("/t/parity-fixture-replies-and-posters/35").await;
    assert!(html.contains(
        "<img alt=\":clap\" class=\"btn-toggle-reaction-emoji reaction-button\" src=\"/images/emoji/twitter/clap.png?v=15\">"
    ));
    assert!(html.contains(
        "class=\"discourse-reactions-actions custom-reaction-used has-reactions has-reacted\""
    ));
}

#[tokio::test]
async fn the_activity_reactions_tab_lists_the_users_reactions() {
    let db = TestDb::new().await;
    clap(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/u/user1/activity/reactions").await;
    assert!(html.contains(
        "<li aria-current=\"location\" class=\"user-activity-bottom-outlet discourse-reactions-user-activity-reactions\"><a class=\"active\" href=\"/u/user1/activity/reactions\">"
    ), "{html}");
    assert!(html.contains(
        "<div class=\"discourse-reactions-my-reaction\"><img width=\"20\" height=\"20\" src=\"/images/emoji/twitter/clap.png?v=15\" title=\"clap\" alt=\"clap\" class=\"emoji reaction-emoji\"><a class=\"avatar-link\" data-user-card=\"user1\">"
    ));
    assert!(
        html.contains(
            "<span class=\"title\"><a href=\"/t/parity-fixture-replies-and-posters/35\">"
        )
    );
}

/// General takes accepted answers; user0, topic 35's author, accepted
/// user2's reply (post 37).
async fn solved(db: &TestDb) {
    for sql in [
        "INSERT INTO category_custom_fields (category_id, name, value, created_at, updated_at) \
         VALUES (4, 'enable_accepted_answers', 'true', now(), now())",
        "INSERT INTO discourse_solved_solved_topics (topic_id, answer_post_id, accepter_user_id, created_at, updated_at) \
         VALUES (35, 37, 2, now(), now())",
        "INSERT INTO discourse_solved_topic_answers (solved_topic_id, answer_post_id, accepter_user_id, created_at, updated_at) \
         SELECT id, 37, 2, now(), now() FROM discourse_solved_solved_topics WHERE topic_id = 35",
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
}

#[tokio::test]
async fn a_solved_topic_shows_its_answer() {
    let db = TestDb::new().await;
    solved(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut anonymous = Client {
        state: st.clone(),
        cookies: Vec::new(),
        csrf: None,
    };
    let html = anonymous
        .page("/t/parity-fixture-replies-and-posters/35")
        .await;
    // The title's status, after the core ones.
    assert!(html.contains(
        "<span class=\"topic-statuses\"><span class=\"topic-status --solved\" title=\"This topic has a solution\">"
    ), "{html}");
    // The accordion under the first post, its one answer expanded.
    assert!(html.contains("<aside class=\"d-post-accordion accepted-answers\""));
    assert!(html.contains(
        "<div class=\"quote d-post-accordion-item d-post-accordion-item--has-content\" data-expanded=\"\" data-overflowing=\"true\" data-post=\"3\" data-topic=\"35\" data-username=\"user2\" style=\"--max-lines-displayed: 8\">"
    ));
    assert!(html.contains(
        "<blockquote class=\"d-post-accordion-item__content\" id=\"post-accordion-item-35-3\">"
    ));
    // The answer's menu: the solution, not a button, for a reader.
    assert!(html.contains(
        "<span class=\"extra-buttons\"><span class=\"accepted-text\" title=\"This is the accepted solution to this topic\">"
    ));
    assert!(html.contains("/assets/discourse-solved.js"));

    // The topic's author unaccepts from the answer's menu, and the other
    // replies' Solved buttons collapse behind show more, labelled for them.
    let mut user0 = Client::logged_in(st.clone(), "user0").await;
    let html = user0.page("/t/parity-fixture-replies-and-posters/35").await;
    assert!(html.contains(
        "<span class=\"extra-buttons\"><button class=\"btn btn-icon-text post-action-menu__solved-accepted accepted fade-out btn-flat\" title=\"Unselect if this reply no longer solves the problem\" type=\"button\">"
    ));
    assert!(html.contains(
        "<button hidden class=\"btn btn-icon-text post-action-menu__solved-unaccepted unaccepted btn-flat\" title=\"Select if this reply solves the problem\" type=\"button\">"
    ));

    // The list marks it solved.
    let html = anonymous.page("/latest").await;
    assert!(
        html.contains("status-solved\" data-topic-id=\"35\""),
        "{html}"
    );
}

#[tokio::test]
async fn an_unsolved_topic_offers_me_too() {
    let db = TestDb::new().await;
    sqlx::query(
        "INSERT INTO category_custom_fields (category_id, name, value, created_at, updated_at) \
         VALUES (4, 'enable_accepted_answers', 'true', now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/t/parity-fixture-replies-and-posters/35").await;
    assert!(html.contains(
        "<div class=\"solved-shared-issue-row\"><button class=\"btn btn-icon-text btn-default post-action-menu__solved-shared-issue\" title=\"I am also experiencing this issue\" type=\"button\">"
    ), "{html}");
    // Not on a reply: user1 may not accept answers here.
    assert!(!html.contains("post-action-menu__solved-unaccepted"));
    assert!(!html.contains("accepted-answers"));
}

#[tokio::test]
async fn the_activity_solved_tab_lists_the_users_answers() {
    let db = TestDb::new().await;
    solved(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user0 = Client::logged_in(st, "user0").await;
    let html = user0.page("/u/user2/activity/solved").await;
    assert!(html.contains(
        "<li aria-current=\"location\" class=\"user-activity-bottom-outlet solved-list\"><a class=\"active\" href=\"/u/user2/activity/solved\">"
    ), "{html}");
    assert!(html.contains("<a aria-label=\"Parity fixture: replies and posters - post #3\" href=\"http://test.localhost/t/parity-fixture-replies-and-posters/35/3\">"));
    let html = user0.page("/u/user0/activity/solved").await;
    assert!(html.contains("You haven’t solved any topics yet"));
    let html = user0.page("/u/user2/summary").await;
    assert!(html.contains("<li class=\"user-summary-stat-outlet solved-count linked-stat\"><a href=\"/u/user2/activity/solved\">"));
}

#[tokio::test]
async fn a_members_header_and_sidebar_show_chat() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/latest").await;
    assert!(
        html.contains("<body hx-boost=\"true\" class=\"chat-enabled "),
        "{html}"
    );
    assert!(html.contains(
        "<li class=\"header-dropdown-toggle chat-header-icon\"><a class=\"btn no-text icon btn-flat\" href=\"/chat\" tabindex=\"0\" title=\"Chat\"><svg class=\"fa d-icon d-icon-d-chat svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#comment\"></use></svg></a></li>"
    ));
    assert!(html.contains("data-section-name=\"chat-search\""));
    assert!(html.contains(
        "<li class=\"sidebar-section-link-wrapper\" data-list-item-name=\"general\"><a class=\"sidebar-section-link sidebar-row channel-2\" title=\"General chat\" data-link-name=\"general\" href=\"/chat/c/general/2\"><span class=\"sidebar-section-link-prefix icon\" style=\"color: #25AAE2\">"
    ), "{html}");
    assert!(html.contains("data-sidebar-action-id=\"channelListOptions\""));
    assert!(html.contains("data-link-name=\"new-chat-dm\" href=\"/chat/new-message\""));
    // user1 follows General only; the Staff channel is admin's.
    assert!(!html.contains("/chat/c/staff/1"));
}

#[tokio::test]
async fn unread_chat_shows_in_the_sidebar_and_the_header() {
    let db = TestDb::new().await;
    sqlx::query(
        "INSERT INTO chat_messages (chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         VALUES (2, 2, now(), now(), 'hi', '<p>hi</p>', 1, 2)",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/latest").await;
    assert!(html.contains(
        "<span class=\"sidebar-section-link-content-badge icon unread\"><svg class=\"fa d-icon d-icon-circle"
    ), "{html}");
    assert!(html.contains("<div class=\"chat-channel-unread-indicator\"></div></a></li>"));
}

#[tokio::test]
async fn chat_is_hidden_from_a_member_who_turned_it_off() {
    let db = TestDb::new().await;
    sqlx::query("UPDATE user_options SET chat_enabled = FALSE WHERE user_id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/latest").await;
    assert!(!html.contains("chat-header-icon"));
    assert!(!html.contains("chat-channels"));
    assert!(!html.contains("chat-enabled"));
}

#[tokio::test]
async fn a_profile_offers_to_chat() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/u/user0/summary").await;
    assert!(html.contains(
        "<li class=\"user-card-below-message-button chat-button\"><button class=\"btn btn-icon-text btn-primary chat-direct-message-btn\" type=\"button\">"
    ), "{html}");
    let own = user1.page("/u/user1/summary").await;
    assert!(!own.contains("chat-direct-message-btn"));
}

/// General's messages, as parity/writes/chat/messages.case.json seeds
/// them: user0's message, user1's reply, admin's edited message with
/// reactions, a deleted one, user1's /me and a link; user1 has read to
/// the second and bookmarked the first.
async fn chat_messages(db: &TestDb) {
    let m = |id: i32, user: i32, mins: i32, msg: &str, cooked: &str, extra: &str, values: &str| {
        format!(
            "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id{extra}) \
             VALUES ({id}, 2, {user}, now() - interval '{mins} minutes', now() - interval '{mins} minutes', '{msg}', '{cooked}', 1, {user}{values})"
        )
    };
    for sql in [
        m(1, 2, 50, "Hello **world**", "<p>Hello <strong>world</strong></p>", "", ""),
        m(2, 3, 40, "@user0 hi back", "<p><a class=\"mention\" href=\"/u/user0\">@user0</a> hi back</p>", ", in_reply_to_id", ", 1"),
        m(3, 1, 30, "edited", "<p>edited</p>", "", ""),
        m(4, 2, 20, "gone", "<p>gone</p>", ", deleted_at, deleted_by_id", ", now() - interval '10 minutes', 2"),
        m(5, 3, 15, "/me waves", "<p>/me waves</p>", "", ""),
        m(6, 2, 5, "https://example.com", "<p><a href=\"https://example.com\">https://example.com</a></p>", "", ""),
        "SELECT setval('chat_messages_id_seq', 6)".into(),
        "UPDATE chat_channels SET last_message_id = 6 WHERE id = 2".into(),
        "INSERT INTO chat_message_revisions (chat_message_id, old_message, new_message, created_at, updated_at, user_id) VALUES (3, 'x', 'edited', now(), now(), 1)".into(),
        "INSERT INTO chat_message_reactions (chat_message_id, user_id, emoji, created_at, updated_at) VALUES (3, 3, 'heart', now(), now()), (3, 2, 'heart', now(), now())".into(),
        "INSERT INTO bookmarks (user_id, bookmarkable_id, bookmarkable_type, name, auto_delete_preference, created_at, updated_at) VALUES (3, 1, 'ChatMessage', 'later', 3, now(), now())".into(),
        "UPDATE user_chat_channel_memberships SET last_read_message_id = 2 WHERE chat_channel_id = 2 AND user_id = 3".into(),
    ] {
        sqlx::query(&sql).execute(&db.pool).await.unwrap();
    }
}

impl Client {
    /// A page request's status and Location, without following it.
    async fn head(&mut self, path: &str) -> (u16, Option<String>) {
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
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|v| v.to_str().unwrap().to_string());
        (response.status().as_u16(), location)
    }
}

#[tokio::test]
async fn chat_goes_to_the_last_channel_or_the_browser() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    assert_eq!(
        user1.head("/chat").await,
        (302, Some("/chat/browse/open".into()))
    );
    assert_eq!(
        user1.head("/chat/browse").await,
        (302, Some("/chat/browse/open".into()))
    );
    // Archiving is off: its tab goes back to the browser.
    assert_eq!(
        user1.head("/chat/browse/archived").await,
        (302, Some("/chat/browse".into()))
    );
    // A channel the member can't open sends them back to /chat; a wrong
    // slug to the channel's own.
    assert_eq!(
        user1.head("/chat/c/staff/1").await,
        (302, Some("/chat".into()))
    );
    assert_eq!(
        user1.head("/chat/c/-/2").await,
        (302, Some("/chat/c/general/2".into()))
    );
    assert_eq!(
        user1.head("/chat/c/-/2/5").await,
        (302, Some("/chat/c/general/2/5".into()))
    );
    // Reading a channel makes it the last one.
    user1.page("/chat/c/general/2").await;
    assert_eq!(
        user1.head("/chat").await,
        (302, Some("/chat/c/general/2".into()))
    );
}

#[tokio::test]
async fn chat_off_in_preferences_goes_to_the_disabled_page() {
    let db = TestDb::new().await;
    sqlx::query("UPDATE user_options SET chat_enabled = FALSE WHERE user_id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    assert_eq!(
        user1.head("/chat/c/general/2").await,
        (302, Some("/chat/disabled".into()))
    );
    let html = user1.page("/chat/disabled").await;
    assert!(html.contains("<div class=\"chat-disabled\">"), "{html}");
    assert!(html.contains("href=\"/my/preferences/chat\""));
}

#[tokio::test]
async fn a_channel_page_shows_its_messages() {
    let db = TestDb::new().await;
    chat_messages(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/chat/c/general/2").await;
    assert!(
        html.contains("desktop-view not-mobile-device has-chat has-full-page-chat\">"),
        "{html}"
    );
    assert!(html.contains("<title>#General - Chat - "));
    // The bookmarked first message, and the reply to it saying nothing of
    // what it replies to.
    assert!(html.contains(
        "<div class=\"chat-message-container -persisted -processed -bookmarked\" data-id=\"1\""
    ));
    assert!(html.contains("<div class=\"chat-message-container is-by-current-user -last-read -persisted -processed\" data-id=\"2\""));
    // The first unread message carries the last visit line; the deleted
    // one is left out for a member.
    assert!(html.contains("data-id=\"3\" data-created-at=") && html.contains("data-newest=\"\""));
    assert!(!html.contains("data-id=\"4\""));
    // /me hides its author.
    assert!(html.contains("-persisted -processed -user-info-hidden\" data-id=\"5\""));
    assert!(html.contains("<span class=\"chat-message-edited\">(edited)</span>"));
    assert!(html.contains("<button aria-describedby=\"chat-message-reaction-description-3-0\" aria-label=\"heart reaction, 2 people\" aria-pressed=\"true\" class=\"chat-message-reaction reacted\" data-emoji-name=\"heart\""));
    assert!(html.contains("<span class=\"sr-only\" id=\"chat-message-reaction-description-3-0\"><span>You and user0 reacted with </span><img"));
    assert!(html.contains("placeholder=\"Chat in #General\""));
    assert!(html.contains("<div class=\"all-loaded-message\">Showing all messages</div>"));
    // The sidebar marks the channel.
    assert!(html.contains("<a class=\"active sidebar-section-link sidebar-row sidebar-section-link--active channel-2\""));
}

#[tokio::test]
async fn staff_see_deleted_messages_collapsed() {
    let db = TestDb::new().await;
    chat_messages(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let html = admin.page("/chat/c/general/2").await;
    assert!(
        html.contains("-persisted -processed -deleted\" data-id=\"4\""),
        "{html}"
    );
    assert!(html.contains("<span class=\"d-button-label\">A message was deleted. [view]</span>"));
}

#[tokio::test]
async fn an_empty_channel_shows_its_members() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/chat/c/general/2").await;
    assert!(html.contains("<div class=\"empty-state__title\" data-test-title=\"\">You&#x27;re the first in #General</div>")
        || html.contains("<div class=\"empty-state__title\" data-test-title=\"\">You're the first in #General</div>"), "{html}");
    assert!(html.contains(
        "<span class=\"empty-state__members-count\"><strong>2 members</strong> here</span>"
    ));
    assert!(html.contains("title=\"admin\"") && html.contains("title=\"user0\""));
}

#[tokio::test]
async fn the_browser_lists_channels() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let html = admin.page("/chat/browse/all").await;
    assert!(html.contains("<a class=\"active chat-browse-view__filter-link -all\" href=\"/chat/browse/all\">All</a>"), "{html}");
    assert!(html.contains("<div class=\"chat-channel-card\" data-channel-id=\"2\" data-following=\"true\" style=\"--chat-channel-card-border: #25AAE2\">"));
    assert!(html.contains("data-channel-id=\"1\""));
    assert!(html.contains(">3 members</a>"));
    let filtered = admin.page("/chat/browse/all?filter=staff").await;
    assert!(
        filtered.contains("data-channel-id=\"1\"") && !filtered.contains("data-channel-id=\"2\"")
    );
}

#[tokio::test]
async fn membership_controls_follow_the_membership() {
    let db = TestDb::new().await;
    sqlx::query("UPDATE user_chat_channel_memberships SET starred = TRUE WHERE user_id = 3 AND chat_channel_id = 2")
        .execute(&db.pool)
        .await
        .unwrap();
    // user2 was seen before, so their first page doesn't auto-join them.
    sqlx::query("UPDATE users SET first_seen_at = now() - interval '1 day', last_seen_at = now() - interval '1 day' WHERE id = 4")
        .execute(&db.pool)
        .await
        .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let html = user1.page("/chat/c/general/2").await;
    assert!(
        html.contains("c-navbar__star-channel-button --starred"),
        "{html}"
    );
    assert!(html.contains("data-section-name=\"chat-starred-channels\""));
    assert!(html.contains("data-last-read=\"0\" data-following=\"true\""));
    // user2 follows nothing: the browser offers to join, the channel
    // shows its preview card.
    let mut user2 = Client::logged_in(st, "user2").await;
    let browse = user2.page("/chat/browse/open").await;
    assert!(browse.contains("toggle-channel-membership-button -join btn-primary btn-small chat-channel-card__join-btn"), "{browse}");
    let channel = user2.page("/chat/c/general/2").await;
    assert!(
        channel.contains("<div class=\"chat-channel-preview-card --logged-in\">"),
        "{channel}"
    );
    assert!(!channel.contains("chat-composer__wrapper"));
}

#[tokio::test]
async fn a_channel_page_carries_the_message_actions() {
    let db = TestDb::new().await;
    chat_messages(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let html = user1.page("/chat/c/general/2").await;
    // The toolbar's template, with the channel's permissions for chat.js.
    assert!(html.contains("<template class=\"chat-message-actions-template\" data-viewer-id=\"3\" data-staff=\"false\" data-can-modify=\"true\" data-following=\"true\" data-can-moderate=\"false\" data-can-delete-self=\"true\" data-can-delete-others=\"false\""), "{html}");
    assert!(html.contains("data-quick-defaults=\"+1|heart|tada\""));
    assert!(html.contains(
        "<li aria-checked=\"false\" role=\"menuitemradio\" data-name=\"Edit\" data-value=\"edit\""
    ));
    assert!(html.contains("<template class=\"chat-toast-template\">"));
    // What the actions read of each message.
    assert!(html.contains("data-user-id=\"2\" data-username=\"user0\""));
    assert!(html.contains("data-message=\"Hello **world**\""));
    assert!(html.contains("data-bookmark-name=\"later\""));

    // A silenced member can't interact: no toolbar.
    sqlx::query("UPDATE users SET silenced_till = now() + interval '1 day' WHERE id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let html = user1.page("/chat/c/general/2").await;
    assert!(html.contains("data-id=\"1\""));
    assert!(!html.contains("chat-message-actions-template"));
}

#[tokio::test]
async fn deleted_messages_collapse_to_the_last_of_their_run() {
    let db = TestDb::new().await;
    chat_messages(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let html = admin.page("/chat/c/general/2").await;
    assert!(html.contains("<div class=\"chat-message-text -deleted\"><button class=\"btn btn-flat chat-message-expand\" type=\"button\"><span class=\"d-button-label\">A message was deleted. [view]</span></button></div></div><template class=\"chat-message-expanded\" data-id=\"4\">"), "{html}");
    // Expanded, it is drawn whole.
    assert!(html.contains("<div class=\"chat-cooked\"><p>gone</p></div>"));

    // Two in a row: only the last shows, counting both.
    sqlx::query("UPDATE chat_messages SET deleted_at = now(), deleted_by_id = 1 WHERE id = 5")
        .execute(&db.pool)
        .await
        .unwrap();
    let html = admin.page("/chat/c/general/2").await;
    assert_eq!(html.matches("chat-message-expand\"").count(), 1, "{html}");
    assert!(html.contains("2 messages were deleted. [view all]"));
    assert!(html.contains("<template class=\"chat-message-expanded\" data-id=\"4\">"));
    assert!(html.contains("<template class=\"chat-message-expanded\" data-id=\"5\">"));
}

#[tokio::test]
async fn a_draft_comes_back_with_its_reply_or_edit() {
    let db = TestDb::new().await;
    chat_messages(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let draft = |data: &str| {
        format!(
            "INSERT INTO chat_drafts (user_id, chat_channel_id, data, created_at, updated_at) VALUES (3, 2, '{data}', now(), now())"
        )
    };
    sqlx::query(&draft(
        r#"{"message":"half","replyToMsg":{"id":1,"excerpt":"Hello :heart:","user":{"id":2,"avatar_template":"/a/{size}.png","username":"user0"}}}"#,
    ))
    .execute(&db.pool)
    .await
    .unwrap();
    let html = user1.page("/chat/c/general/2").await;
    assert!(
        html.contains(
            "<div class=\"chat-composer-message-details\" data-action=\"reply\" data-id=\"1\""
        ),
        "{html}"
    );
    assert!(html.contains("<span class=\"chat-reply__username\">user0</span><span class=\"chat-reply__excerpt\">Hello <img"));
    assert!(html.contains(">half</textarea>"));

    sqlx::query("DELETE FROM chat_drafts")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(&draft(
        r#"{"message":"/me waves more","editing":true,"id":5,"excerpt":"/me waves"}"#,
    ))
    .execute(&db.pool)
    .await
    .unwrap();
    // user1 is in /chat/online, so their avatar is drawn online.
    sqlx::query(
        "INSERT INTO discourse_rs.presence_clients (channel, user_id, client_id, expires_at) \
         VALUES ('/chat/online', 3, 'tab', now() + interval '1 minute')",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let html = user1.page("/chat/c/general/2").await;
    assert!(
        html.contains(
            "<div class=\"chat-composer-message-details\" data-action=\"edit\" data-id=\"5\""
        ),
        "{html}"
    );
    assert!(html.contains("<div class=\"chat-user-avatar is-online\" data-username=\"user1\">"));
    assert!(html.contains(">/me waves more</textarea>"));

    // What Rails stores of a hash (Ruby's inspect) the client can't read:
    // no draft.
    sqlx::query("DELETE FROM chat_drafts")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(&draft(r#"{message: "x"}"#))
        .execute(&db.pool)
        .await
        .unwrap();
    let html = user1.page("/chat/c/general/2").await;
    assert!(!html.contains("chat-composer-message-details\" data-action=\"reply"));
    assert!(html.contains("type=\"text\"></textarea>"), "{html}");
}

#[tokio::test]
async fn a_live_chat_message_is_drawn_for_its_viewer() {
    let db = TestDb::new().await;
    chat_messages(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    // After user1's own message, admin's shows its author.
    let html = user1.page("/live/chat/2/3?previous=2").await;
    assert!(
        html.starts_with(
            "<div class=\"chat-message-container -persisted -processed\" data-id=\"3\""
        ),
        "{html}"
    );
    // user0's /me after user1's own: the author shows.
    let html = user1.page("/live/chat/2/5?previous=1").await;
    assert!(html.contains("data-id=\"5\""));
    // Another's deleted message, a channel they can't see, nothing there.
    assert_eq!(user1.head("/live/chat/2/4").await.0, 404);
    assert_eq!(user1.head("/live/chat/1/3").await.0, 404);
    assert_eq!(user1.head("/live/chat/2/99").await.0, 404);
    // Its author sees their deleted message, expanded.
    let mut user0 = Client::logged_in(st, "user0").await;
    let html = user0.page("/live/chat/2/4?previous=3").await;
    assert!(html.contains("-deleted"), "{html}");
    assert!(html.contains("<p>gone</p>"));
}

#[tokio::test]
async fn a_skin_toned_chat_reaction_is_drawn() {
    let db = TestDb::new().await;
    chat_messages(&db).await;
    sqlx::query(
        "INSERT INTO chat_message_reactions (chat_message_id, user_id, emoji, created_at, updated_at) VALUES (1, 3, 'waving_hand:t3', now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/chat/c/general/2").await;
    assert!(
        html.contains("data-emoji-name=\"waving_hand:t3\""),
        "{html}"
    );
    assert!(html.contains("/images/emoji/twitter/waving_hand/3.png"));
}

#[tokio::test]
async fn the_emoji_data_carries_names_aliases_and_unicode() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut anon = Client {
        state: st,
        cookies: Vec::new(),
        csrf: None,
    };
    let body = anon.page("/assets/emoji-data.json").await;
    let data: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        data["names"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n == "heart")
    );
    assert_eq!(data["aliases"]["thumbsup"], "+1");
    assert_eq!(data["unicode"]["👍"], "+1");
}

#[tokio::test]
async fn a_channel_page_carries_what_stages_a_message() {
    let db = TestDb::new().await;
    chat_messages(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/chat/c/general/2").await;
    // The member's own message, with and without its author line, and the
    // chat renderer to cook it with.
    assert!(html.contains("<template class=\"chat-staged-template\" data-variant=\"info\" data-wasm=\"/assets/markdown.wasm?v="), "{html}");
    assert!(html.contains("data-settings=\"/assets/markdown-settings.json?chat=1\""));
    assert!(html.contains(
        "data-label-network-error=\"Network error\" data-label-send-again=\"Send again?\""
    ));
    assert!(html.contains("<template class=\"chat-staged-template\" data-variant=\"hidden\"><div class=\"chat-message-container is-by-current-user -persisted -processed -user-info-hidden\" data-id=\"0\""));

    let settings = user1.page("/assets/markdown-settings.json?chat=1").await;
    let settings: serde_json::Value = serde_json::from_str(&settings).unwrap();
    assert_eq!(settings["chat"], true);
    assert_eq!(settings["poll"], false);
    let plain = user1.page("/assets/markdown-settings.json").await;
    let plain: serde_json::Value = serde_json::from_str(&plain).unwrap();
    assert_eq!(plain["chat"], false);

    // After their own staged message, their next one hides its author;
    // after another's, it shows.
    sqlx::query(
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         VALUES (7, 2, 3, now(), now(), 'again', '<p>again</p>', 1, 3)",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let html = user1.page("/live/chat/2/7?previous=staged").await;
    assert!(html.contains("-user-info-hidden\" data-id=\"7\""), "{html}");
    let html = user1.page("/live/chat/2/7?previous=6").await;
    assert!(html.contains("-processed\" data-id=\"7\""), "{html}");
}

#[tokio::test]
async fn the_user_menu_shows_chat_mentions() {
    let db = TestDb::new().await;
    let data = r#"{"chat_message_id":7,"chat_channel_id":2,"mentioned_by_username":"user1","is_direct_message_channel":false,"chat_channel_title":"General","chat_channel_slug":"general"}"#;
    let group = r#"{"chat_message_id":8,"chat_channel_id":2,"mentioned_by_username":"user1","is_direct_message_channel":false,"chat_channel_title":"General","chat_channel_slug":null,"identifier":"squad","is_group_mention":true}"#;
    for d in [data, group] {
        sqlx::query(
            "INSERT INTO notifications (notification_type, user_id, data, read, high_priority, created_at, updated_at) \
             VALUES (29, 2, $1, false, true, now(), now())",
        )
        .bind(d)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user0 = Client::logged_in(st, "user0").await;
    let html = user0.page("/user-menu").await;
    assert!(
        html.contains(r#"<span class="item-label ">user1</span><span class="item-description ">mentioned you in &quot;General&quot;</span>"#)
            && html.contains(r#"<a href="/chat/c/general/2/7" title="#),
        "{html}"
    );
    // No slug: the title slugified.
    assert!(
        html.contains(r#"<a href="/chat/c/general/2/8" title="#),
        "{html}"
    );
}
#[tokio::test]
async fn the_composer_carries_its_autocompletes() {
    let db = TestDb::new().await;
    common::set_setting(&db.pool, "emoji_deny_list", 1, "poop").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/chat/c/general/2").await;
    // ChatComposer#setupAutocomplete's settings, the denied emojis (with
    // their aliases), the chat-composer context's hashtag types in order
    // and the "more" item's label.
    let start = html
        .find("data-autocomplete=\"")
        .expect("the composer's autocompletes")
        + 19;
    let end = start + html[start..].find('"').unwrap();
    let cfg: serde_json::Value =
        serde_json::from_str(&html[start..end].replace("&quot;", "\"")).unwrap();
    assert_eq!(cfg["mentions"], true);
    assert_eq!(cfg["emoji"], true);
    assert_eq!(cfg["emojiMinChars"], 0);
    assert_eq!(cfg["deniedEmojis"][0], "poop");
    assert_eq!(
        cfg["hashtagOrder"],
        serde_json::json!(["channel", "category", "tag"])
    );
    assert_eq!(cfg["moreEmoji"], "more…");
    assert!(html.contains("/assets/autocomplete.js"));
}

#[tokio::test]
async fn the_mobile_view_lists_chat_channels() {
    let db = TestDb::new().await;
    chat_messages(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    user1.cookies.push(("mobile_view".into(), "1".into()));
    // ChatIndexRoute replaces /chat with the channels list, which the
    // mobile view keeps (no last channel redirect).
    user1.page("/chat/c/general/2").await;
    assert_eq!(
        user1.head("/chat").await,
        (302, Some("/chat/channels".into()))
    );
    let html = user1.page("/chat/channels").await;
    assert!(
        html.contains("<div class=\"c-routes --channels\">"),
        "{html}"
    );
    assert!(html.contains("<div class=\"chat-channel-row__content is-category\"><div class=\"chat-channel-icon --icon\" style=\"color: #25AAE2\">"));
    assert!(html.contains("data-chat-channel-id=\"2\" href=\"/chat/c/general/2\""));
    assert!(html.contains("<div class=\"chat-channel-name__label\">General</div>"));
    assert!(html.contains("<div class=\"chat-channel__metadata-date\" data-at=\""));
    // The chat header in the logo's place, and the footer on channels.
    assert!(html.contains(
        "<div class=\"c-header\"><a class=\"btn-flat back-to-forum\" href=\"/\" title=\"Forum\">"
    ));
    assert!(html.contains(
        "class=\"btn btn-icon-text btn-transparent c-footer__item --active\" id=\"c-footer-channels\""
    ));
    assert!(!html.contains("chat-header-icon"));
    // A channel's navbar goes back to the list.
    let channel = user1.page("/chat/c/general/2").await;
    assert!(
        channel.contains("<a class=\"c-navbar__back-button no-text btn-transparent btn\" title=\"Back\" href=\"/chat/channels\">"),
        "{channel}"
    );
}

/// user1's DM with user2: user2's greeting, user1's answer and user2's
/// question, which user1 hasn't read.
async fn chat_dm(db: &TestDb) {
    let day = "now() - interval '1 day'";
    let m = |id: i32, user: i32, mins: i32, text: &str| {
        format!(
            "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id, excerpt) \
             VALUES ({id}, 3, {user}, now() - interval '{mins} minutes', now() - interval '{mins} minutes', '{text}', '<p>{text}</p>', 1, {user}, '{text}')"
        )
    };
    for sql in [
        format!(
            "INSERT INTO direct_message_channels (id, \"group\", created_at, updated_at) VALUES (1, false, {day}, {day})"
        ),
        format!(
            "INSERT INTO direct_message_users (direct_message_channel_id, user_id, created_at, updated_at) VALUES (1, 3, {day}, {day}), (1, 4, {day}, {day})"
        ),
        format!(
            "INSERT INTO chat_channels (id, chatable_type, chatable_id, type, status, user_count, name, messages_count, created_at, updated_at) VALUES (3, 'DirectMessage', 1, 'DirectMessageChannel', 0, 2, NULL, 3, {day}, {day})"
        ),
        m(1, 4, 30, "hello there"),
        m(2, 3, 20, "hi"),
        m(3, 4, 10, "are you around later today?"),
        "UPDATE chat_channels SET last_message_id = 3 WHERE id = 3".into(),
        format!(
            "INSERT INTO user_chat_channel_memberships (user_id, chat_channel_id, following, muted, notification_level, last_read_message_id, created_at, updated_at) VALUES (3, 3, true, false, 2, 2, {day}, {day}), (4, 3, true, false, 2, 3, {day}, {day})"
        ),
        "SELECT setval('chat_messages_id_seq', 3)".into(),
        "SELECT setval('chat_channels_id_seq', 3)".into(),
        "SELECT setval('direct_message_channels_id_seq', 1)".into(),
    ] {
        sqlx::query(&sql).execute(&db.pool).await.unwrap();
    }
}

#[tokio::test]
async fn the_direct_messages_page_lists_a_members_dms() {
    let db = TestDb::new().await;
    chat_dm(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    // Desktop stays on the list on a page load, a last channel or not.
    user1.page("/chat/c/user2/3").await;
    let html = user1.page("/chat/direct-messages").await;
    assert!(
        html.contains("<div class=\"c-routes --direct-messages\"><div class=\"c-navbar-container\"><nav class=\"c-navbar\"><div class=\"c-navbar__title\" title=\"DMs\">"),
        "{html}"
    );
    assert!(html.contains("<title>Chat - "));
    assert!(html.contains("<div class=\"chat-channel-divider direct-message-channels-section\"><span class=\"channel-title\">DMs</span><div class=\"chat-channel-divider__actions\"><button class=\"btn no-text btn-icon btn-transparent chat-channel-list-options-button title-action\""));
    assert!(html.contains("<a class=\"chat-channel-row can-leave has-unread\" tabindex=\"0\" data-chat-channel-id=\"3\" href=\"/chat/c/user2/3\"><div class=\"chat-channel-row__content is-dm\"><div class=\"chat-channel-icon --avatar\"><div class=\"chat-user-avatar\" data-username=\"user2\">"));
    // A DM's unread message is urgent.
    assert!(html.contains("<div class=\"chat-channel-name__label\">user2</div><div class=\"chat-channel-unread-indicator -urgent\"><div class=\"chat-channel-unread-indicator__number\">1</div></div>"));
    assert!(html.contains("<button class=\"btn no-text btn-icon toggle-channel-membership-button -leave btn-flat chat-channel-leave-btn\" title=\"Close channel\" type=\"button\">"));
    assert!(!html.contains("chat-channel__last-message"));
    assert!(!html.contains("c-footer"));
    // /chat goes on to the last channel, a DM too.
    assert_eq!(user1.head("/chat").await, (302, Some("/chat/c/-/3".into())));

    // Without DMs: the empty state offering one.
    let mut user0 = Client::logged_in(st, "user0").await;
    let html = user0.page("/chat/direct-messages").await;
    assert!(
        html.contains("<div class=\"channels-list-container direct-message-channels center-empty-channels-list\" id=\"direct-message-channels\"><div class=\"empty-state__container --empty-channels-list --with-image\">"),
        "{html}"
    );
    assert!(html.contains("<div class=\"empty-state__title\" data-test-title=\"\">You have not joined any direct messages yet!</div><div class=\"empty-state__cta\"><button class=\"btn btn-primary\" type=\"button\"><span class=\"d-button-label\">Start a conversation</span></button></div>"));
}

#[tokio::test]
async fn the_mobile_direct_messages_list_shows_last_messages() {
    let db = TestDb::new().await;
    chat_dm(&db).await;
    common::set_setting(&db.pool, "chat_preferred_index", 7, "direct_messages").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    user1.cookies.push(("mobile_view".into(), "1".into()));
    // The preferred index.
    assert_eq!(
        user1.head("/chat").await,
        (302, Some("/chat/direct-messages".into()))
    );
    let html = user1.page("/chat/direct-messages").await;
    // The section's buttons in the navbar, no divider, no leave button.
    assert!(
        html.contains("<nav class=\"c-navbar__actions\"><button class=\"btn no-text btn-icon btn-transparent chat-channel-list-options-button title-action\""),
        "{html}"
    );
    assert!(!html.contains("direct-message-channels-section"));
    assert!(!html.contains("chat-channel-leave-btn"));
    assert!(
        html.contains(
            "<div class=\"chat-channel__last-message\">are you around later today?</div>"
        )
    );
    assert!(html.contains(
        "class=\"btn btn-icon-text btn-transparent c-footer__item --active\" id=\"c-footer-direct-messages\""
    ));
    // A DM's navbar goes back to the list.
    let channel = user1.page("/chat/c/user2/3").await;
    assert!(
        channel.contains("href=\"/chat/direct-messages\">"),
        "{channel}"
    );
}

#[tokio::test]
async fn a_dm_draws_the_last_visit_line_on_its_last_message() {
    let db = TestDb::new().await;
    chat_dm(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/chat/c/user2/3").await;
    // The unread message is the channel's last: Ember draws the line.
    assert!(
        html.contains("data-id=\"3\" data-created-at=") && html.contains("data-newest=\"\""),
        "{html}"
    );
    assert_eq!(html.matches("data-newest=\"\"").count(), 1);
}

#[tokio::test]
async fn the_threads_page_has_none_to_list() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let html = user1.page("/chat/threads").await;
    assert!(
        html.contains("<div class=\"c-routes --threads\"><div class=\"c-navbar-container\"><nav class=\"c-navbar\"><div class=\"c-navbar__title\" title=\"My Threads\">"),
        "{html}"
    );
    assert!(html.contains("<div class=\"c-list\"><div class=\"--loaded c-user-threads\"><div class=\"c-list-empty-state\"><div class=\"empty-state-threads\">You don"));
    assert!(html.contains("c-navbar__open-drawer-button"));
    user1.cookies.push(("mobile_view".into(), "1".into()));
    let html = user1.page("/chat/threads").await;
    assert!(!html.contains("c-navbar__open-drawer-button"), "{html}");
    assert!(html.contains("<nav class=\"c-footer\">"));
    assert!(!html.contains("c-footer__item --active"));

    // A thread the member follows in a channel with threading: refused.
    chat_messages(&db).await;
    for sql in [
        "UPDATE chat_channels SET threading_enabled = true WHERE id = 2",
        "INSERT INTO chat_threads (id, channel_id, original_message_id, original_message_user_id, last_message_id, replies_count, created_at, updated_at) VALUES (1, 2, 1, 2, 2, 1, now(), now())",
        "UPDATE chat_messages SET thread_id = 1 WHERE id IN (1, 2)",
        "INSERT INTO user_chat_thread_memberships (user_id, thread_id, notification_level, created_at, updated_at) VALUES (3, 1, 2, now(), now())",
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
    let mut desktop = Client::logged_in(st, "user1").await;
    assert_eq!(desktop.head("/chat/threads").await.0, 500);
}

impl Client {
    /// GET /live/chat/drawer for a URL: the status, the routes visited,
    /// the channel shown and the markup.
    async fn drawer(&mut self, query: &str) -> (u16, String, Option<String>, String) {
        let cookie: Vec<String> = self
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let request = Request::get(format!("/live/chat/drawer?{query}"))
            .header(header::HOST, "test.localhost")
            .header(header::COOKIE, cookie.join("; "));
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .map(|v| v.to_str().unwrap().to_string())
        };
        let routes = header("x-chat-drawer-routes").unwrap_or_default();
        let channel = header("x-chat-drawer-channel");
        let status = response.status().as_u16();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            routes,
            channel,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }
}

/// The drawer's buttons, expanded.
const DRAWER_BUTTONS: &str = "<button class=\"btn no-text btn-icon btn-transparent no-text c-navbar__toggle-drawer-button\" title=\"Collapse Chat Drawer\" type=\"button\"><svg class=\"fa d-icon d-icon-minus svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#minus\"></use></svg><span aria-hidden=\"true\">&#8203;</span></button><button class=\"btn no-text btn-icon btn-transparent no-text c-navbar__full-page-button\" title=\"Open full-screen chat\" type=\"button\"><svg class=\"fa d-icon d-icon-discourse-expand svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#discourse-expand\"></use></svg><span aria-hidden=\"true\">&#8203;</span></button><button class=\"btn no-text btn-icon btn-transparent no-text c-navbar__close-drawer-button\" title=\"Close\" type=\"button\"><svg class=\"fa d-icon d-icon-xmark svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#xmark\"></use></svg><span aria-hidden=\"true\">&#8203;</span></button>";

#[tokio::test]
async fn every_page_has_the_chat_drawer_outlet() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let outlet = "<div class=\"below-footer-outlet chat-drawer-outlet\" id=\"chat-drawer-outlet\" hx-preserve><div class=\"chat-drawer-outlet-container\"></div></div>";
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    for path in ["/latest", "/categories", "/chat/browse/open"] {
        let html = user1.page(path).await;
        assert!(html.contains(outlet), "{path}: {html}");
        // After the theme's footer, before the composer.
        assert!(
            html.find(outlet) < html.find("id=\"reply-control\""),
            "{path}"
        );
        assert!(
            html.contains("/chat-drawer.js?v=") && html.contains(" data-homepage=\"/latest\" data-hide-presence=\"false\" data-starred=\"false\" data-scripts=\"/assets/emoji-picker.js?v&#x3D;"),
            "{path}: {html}"
        );
        // The new message modal's errors open Ember's dialog, not an alert.
        assert!(
            html.contains("/dialog.js?v=") && html.contains("id=\"dialog-strings\""),
            "{path}"
        );
    }
    assert!(
        user1
            .page("/assets/chat-drawer.js")
            .await
            .contains("ChatStateManager")
    );
    // A visitor gets the outlet, which stays empty: no drawer opens.
    let html = Client {
        state: st.clone(),
        cookies: Vec::new(),
        csrf: None,
    }
    .page("/latest")
    .await;
    assert!(html.contains(outlet), "{html}");
    assert!(!html.contains("chat-drawer.js"));
    // With the plugin off, no outlet.
    common::set_setting(&db.pool, "chat_enabled", 5, "f").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/latest").await;
    assert!(!html.contains("chat-drawer-outlet"), "{html}");
}

#[tokio::test]
async fn the_chat_drawer_lists_the_channels() {
    let db = TestDb::new().await;
    chat_messages(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, routes, channel, html) = user1.drawer("url=/chat&expanded=true").await;
    assert_eq!((status, routes.as_str(), channel), (200, "chat", None));
    assert!(
        html.starts_with(&format!("<div class=\"c-drawer-routes --channels\"><div class=\"c-navbar-container -clickable\"><nav class=\"c-navbar\"><div class=\"c-navbar__title\" title=\"Chat\"><span class=\"c-navbar__title-text\">Chat</span></div><nav class=\"c-navbar__actions\">{DRAWER_BUTTONS}</nav></nav></div><div class=\"chat-drawer-content\"><div class=\"chat-channel-divider public-channels-section\"><span class=\"channel-title\">Channels</span><div class=\"chat-channel-divider__actions\"><button class=\"btn no-text btn-icon btn-transparent chat-channel-list-options-button title-action\"")),
        "{html}"
    );
    assert!(html.contains("<div class=\"channels-list-container public-channels\" id=\"public-channels\"><a class=\"chat-channel-row has-unread\" tabindex=\"0\" data-chat-channel-id=\"2\" href=\"/chat/c/general/2\">"), "{html}");
    // The footer while expanded: channels and search (no DMs for user1
    // without the setting's groups), so more than one route.
    assert!(html.ends_with("</nav></div>"), "{html}");
    assert!(html.contains("<nav class=\"c-footer\">"), "{html}");

    // Collapsed: the navbar alone, its toggle expanding.
    let (_, _, _, html) = user1.drawer("url=/chat/channels&expanded=false").await;
    assert_eq!(
        html,
        "<div class=\"c-drawer-routes --channels\"><div class=\"c-navbar-container -clickable\"><nav class=\"c-navbar\"><div class=\"c-navbar__title\" title=\"Chat\"><span class=\"c-navbar__title-text\">Chat</span></div><nav class=\"c-navbar__actions\"><button class=\"btn no-text btn-icon btn-transparent no-text c-navbar__toggle-drawer-button\" title=\"Expand Chat Drawer\" type=\"button\"><svg class=\"fa d-icon d-icon-angles-up svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#angles-up\"></use></svg><span aria-hidden=\"true\">&#8203;</span></button><button class=\"btn no-text btn-icon btn-transparent no-text c-navbar__close-drawer-button\" title=\"Close\" type=\"button\"><svg class=\"fa d-icon d-icon-xmark svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#xmark\"></use></svg><span aria-hidden=\"true\">&#8203;</span></button></nav></nav></div></div>"
    );

    // No starred channels: the starred route goes on to the channels.
    let (_, routes, _, html) = user1.drawer("url=/chat/starred-channels").await;
    assert_eq!(routes, "chat.starred-channels chat.channels");
    assert!(html.starts_with("<div class=\"c-drawer-routes --channels\">"));
    // A starred one: listed under its divider, the footer's starred item.
    sqlx::query("UPDATE user_chat_channel_memberships SET starred = true WHERE chat_channel_id = 2 AND user_id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, routes, _, html) = user1.drawer("url=/chat/starred-channels").await;
    assert_eq!(routes, "chat.starred-channels");
    assert!(
        html.contains("<div class=\"c-navbar__title\" title=\"Starred\">")
            && html.contains("<div class=\"chat-drawer-content\"><div class=\"chat-channel-divider starred-channels-section\"><span class=\"channel-title\">Starred channels</span>")
            && html.contains("<div class=\"channels-list-container starred-channels\"><a class=\"chat-channel-row has-unread\" tabindex=\"0\" data-chat-channel-id=\"2\" href=\"/chat/c/general/2\">"),
        "{html}"
    );
    assert!(
        html.contains("c-footer__item --active\" id=\"c-footer-starred\""),
        "{html}"
    );
    // Pages say so, for the drawer's default URL.
    assert!(
        user1
            .page("/latest")
            .await
            .contains("data-starred=\"true\"")
    );
}

#[tokio::test]
async fn the_chat_drawer_shows_a_channel() {
    let db = TestDb::new().await;
    chat_messages(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, routes, channel, html) = user1.drawer("url=/chat/c/general/2").await;
    assert_eq!(
        (status, routes.as_str(), channel.as_deref()),
        (200, "chat.channel", Some("2"))
    );
    assert!(
        html.starts_with("<div class=\"c-drawer-routes --channel\"><div class=\"c-navbar-container -clickable\"><nav class=\"c-navbar\"><a class=\"c-navbar__back-button no-text btn-transparent btn\" title=\"Back\" href=\"/chat/channels\"><svg class=\"fa d-icon d-icon-chevron-left svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#chevron-left\"></use></svg></a><a class=\"c-navbar__channel-title\" href=\"/chat/c/general/2/info/settings\">"),
        "{html}"
    );
    assert!(html.contains("c-navbar__star-channel-button"));
    assert!(html.contains(&format!("{DRAWER_BUTTONS}</nav></nav></div><div class=\"chat-drawer-content\"><div class=\"chat-channel --loaded\" data-id=\"2\"")), "{html}");
    assert!(!html.contains("c-navbar__open-drawer-button"));
    // The messages and the composer, as full-page chat draws them.
    assert!(html.contains("<p>Hello <strong>world</strong></p>"));
    assert!(html.contains("id=\"channel-composer\""));
    assert!(html.ends_with("</div></div></div>"));
    // The slug isn't checked; a near message is the channel around it.
    let (_, routes, channel, html) = user1.drawer("url=/chat/c/-/2/3").await;
    assert_eq!(
        (routes.as_str(), channel.as_deref()),
        ("chat.channel.near-message", Some("2"))
    );
    assert!(html.contains("data-id=\"3\""));
    // From the browser, back goes there.
    let (_, _, _, html) = user1
        .drawer("url=/chat/c/general/2&previous=chat.browse")
        .await;
    assert!(
        html.contains("title=\"Back\" href=\"/chat/browse\">"),
        "{html}"
    );
    // Collapsed: the navbar without the back button, the title unlinked,
    // none of the channel's buttons, no full page button.
    let (_, _, _, html) = user1.drawer("url=/chat/c/general/2&expanded=false").await;
    assert!(html.starts_with("<div class=\"c-drawer-routes --channel\"><div class=\"c-navbar-container -clickable\"><nav class=\"c-navbar\"><div class=\"c-navbar__channel-title\"><div class=\"chat-channel-title is-category\">"), "{html}");
    for absent in [
        "chat-drawer-content",
        "c-navbar__full-page-button",
        "c-navbar__star-channel-button",
        "c-navbar__filter",
        "/info/settings",
    ] {
        assert!(!html.contains(absent), "{absent}: {html}");
    }
    assert!(html.contains("title=\"Expand Chat Drawer\""));
    // A channel the member can't open is refused, as is the mobile view.
    assert_eq!(user1.drawer("url=/chat/c/staff/1").await.0, 500);
    user1.cookies.push(("mobile_view".into(), "1".into()));
    assert_eq!(user1.drawer("url=/chat/c/general/2").await.0, 404);
}

#[tokio::test]
async fn the_chat_drawer_browses_and_lists_dms() {
    let db = TestDb::new().await;
    chat_dm(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (_, routes, _, html) = user1.drawer("url=/chat/browse").await;
    assert_eq!(routes, "chat.browse");
    assert!(
        html.starts_with("<div class=\"c-drawer-routes --browse\"><div class=\"c-navbar-container -clickable\"><nav class=\"c-navbar\"><a class=\"c-navbar__back-button no-text btn-transparent btn\" title=\"Back\" href=\"/chat/channels\">"),
        "{html}"
    );
    assert!(html.contains(&format!("<div class=\"c-navbar__title\" title=\"Channels\"><span class=\"c-navbar__title-text\">Channels</span></div><nav class=\"c-navbar__actions\">{DRAWER_BUTTONS}</nav></nav></div><div class=\"chat-drawer-content\"><div class=\"chat-browse-view\">")), "{html}");
    assert!(html.contains(
        "<a class=\"active chat-browse-view__filter-link -open\" href=\"/chat/browse/open\">"
    ));
    let (_, routes, _, html) = user1.drawer("url=/chat/browse/all").await;
    assert_eq!(routes, "chat.browse.all");
    assert!(html.contains("<a class=\"active chat-browse-view__filter-link -all\""));

    // The DMs, titled Chat, with their footer.
    let (_, routes, _, html) = user1.drawer("url=/chat/direct-messages").await;
    assert_eq!(routes, "chat.direct-messages");
    assert!(
        html.starts_with("<div class=\"c-drawer-routes --direct-messages\"><div class=\"c-navbar-container -clickable\"><nav class=\"c-navbar\"><div class=\"c-navbar__title\" title=\"Chat\">"),
        "{html}"
    );
    assert!(html.contains("<div class=\"chat-drawer-content\"><div class=\"chat-channel-divider direct-message-channels-section\"><span class=\"channel-title\">DMs</span>"));
    assert!(html.contains("data-chat-channel-id=\"3\" href=\"/chat/c/user2/3\""));
    assert!(
        html.contains("c-footer__item --active\" id=\"c-footer-direct-messages\""),
        "{html}"
    );
    // A DM's back button goes to the DMs.
    let (_, _, channel, html) = user1.drawer("url=/chat/c/user2/3").await;
    assert_eq!(channel.as_deref(), Some("3"));
    assert!(
        html.contains("title=\"Back\" href=\"/chat/direct-messages\">"),
        "{html}"
    );
    // The DMs index: /chat redirects there.
    common::set_setting(&db.pool, "chat_preferred_index", 7, "direct_messages").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let (_, routes, _, html) = user1.drawer("url=/chat").await;
    assert_eq!(routes, "chat chat.direct-messages");
    assert!(html.starts_with("<div class=\"c-drawer-routes --direct-messages\">"));
}

#[tokio::test]
async fn the_chat_drawer_refuses_what_it_doesnt_draw_yet() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    for url in [
        "/chat/threads",
        "/chat/search",
        "/chat/c/general/2/pins",
        "/chat/c/general/2/t",
        "/chat/c/general/2/t/1",
        "/chat/c/general/2/info/settings",
        "/chat/c/general/2/info/members",
    ] {
        assert_eq!(user1.drawer(&format!("url={url}")).await.0, 500, "{url}");
    }
    // What the drawer doesn't handle goes to full-page chat: not found here.
    for url in [
        "/chat/new-message",
        "/chat/disabled",
        "/chat/message/1",
        "/latest",
    ] {
        assert_eq!(user1.drawer(&format!("url={url}")).await.0, 404, "{url}");
    }
    // Nor for a visitor.
    let mut anon = Client {
        state: st,
        cookies: Vec::new(),
        csrf: None,
    };
    assert_eq!(anon.drawer("url=/chat").await.0, 404);
}

/// user0's topic with three polls: a public single choice, a multiple
/// choice whose results wait for a vote and which closes in 2099, and one
/// whose results wait for its close. Its first post's id.
async fn poll_topic(st: &AppState) -> i64 {
    let raw = "Which ones?\n\n\
        [poll name=single public=true]\n* Yes\n* No\n[/poll]\n\n\
        [poll name=multi type=multiple min=1 max=2 results=on_vote close=2099-01-01T00:00:00.000Z]\n# Pick some\n* Red\n* Green\n* Blue\n[/poll]\n\n\
        [poll name=later results=on_close]\n* Now\n* Never\n[/poll]";
    let mut user0 = Client::logged_in(st.clone(), "user0").await;
    let body = format!(
        "title={}&raw={}&category=4",
        urlencode("Three polls to vote in"),
        urlencode(raw)
    );
    let created = user0.send(Method::POST, "/posts", &body).await;
    let created: serde_json::Value = serde_json::from_str(&created).expect(&created);
    created["id"]
        .as_i64()
        .unwrap_or_else(|| panic!("{created}"))
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

/// The poll widget of that name in the page, `div.poll-outer` on.
fn poll_widget<'a>(html: &'a str, name: &str) -> &'a str {
    let start = html
        .find(&format!("<div data-poll-name=\"{name}\""))
        .unwrap_or_else(|| panic!("the {name} widget: {html}"));
    let rest = &html[start..];
    // Up to the next widget or the end of the cooked content.
    let end = rest[1..]
        .find("<div data-poll-name=")
        .map(|i| i + 1)
        .or_else(|| rest.find("<div class=\"cooked-selection-barrier\""))
        .unwrap_or(rest.len());
    &rest[..end]
}

/// The option digests a widget offers, in order.
fn poll_digests(widget: &str) -> Vec<String> {
    widget
        .split("<li data-poll-option-id=\"")
        .skip(1)
        .map(|s| s.split('"').next().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn polls_are_drawn_as_their_widgets() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let post_id = poll_topic(&st).await;
    let topic_id: i32 = sqlx::query_scalar("SELECT topic_id FROM posts WHERE id = $1")
        .bind(post_id as i32)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let path = format!("/t/three-polls-to-vote-in/{topic_id}");
    let mut anonymous = Client {
        state: st.clone(),
        cookies: Vec::new(),
        csrf: None,
    };
    let html = anonymous.page(&path).await;
    // Each cooked poll is replaced; none is left to draw a "0 voters" stub.
    assert_eq!(html.matches("class=\"poll-outer\"").count(), 3, "{html}");
    assert!(!html.contains("<div class=\"poll\" data-poll"), "{html}");
    assert!(html.contains("/assets/poll.js"));

    let single = poll_widget(&html, "single");
    assert!(single.starts_with(
        "<div data-poll-name=\"single\" data-poll-type=\"regular\" class=\"poll-outer\" data-poll-state=\""
    ), "{single}");
    assert_eq!(poll_digests(single).len(), 2);
    assert!(
        single.contains("<button> <svg class=\"fa d-icon d-icon-far-circle svg-icon"),
        "{single}"
    );
    assert!(single.contains("<span class=\"option-text\">Yes</span>"));
    assert!(single.contains(
        "<div class=\"poll-info_counts-count\"> <span class=\"info-number\">0</span> <span class=\"info-label\">voters</span> </div>"
    ), "{single}");
    assert!(single.contains("<li class=\"is-public\">"));
    // A single choice is cast by its click: no Cast Votes, and nothing to
    // remove or show yet.
    assert!(!single.contains("cast-votes"));
    assert!(!single.contains("remove-vote"));
    assert!(!single.contains("toggle-results"));
    assert!(
        single.contains(
            "<div class=\"poll-buttons\"> <div class=\"poll-buttons-dropdown\">  </div> </div> </div></div>"
        ),
        "{single}"
    );

    let multi = poll_widget(&html, "multi");
    assert!(multi.contains("data-poll-type=\"multiple\""));
    assert!(
        multi.contains("<div class=\"poll-title\">Pick some</div>"),
        "{multi}"
    );
    assert!(multi.contains("d-icon-far-square svg-icon"));
    assert!(
        multi.contains("<li class=\"multiple-help-text\"> <svg class=\"fa d-icon d-icon-list-ul"),
        "{multi}"
    );
    assert!(
        multi.contains("Choose up to <strong>2 options</strong>."),
        "{multi}"
    );
    // Closing in 2099: the countdown, kept current by dates.js.
    assert!(multi.contains("d-icon-far-clock"), "{multi}");
    assert!(multi.contains("Closes in"), "{multi}");
    assert!(multi.contains("<li class=\"results-on-vote\">"));
    assert!(
        multi.contains(
            "<button class=\"btn cast-votes btn-default\" disabled=\"\" title=\"Cast your votes\">"
        ),
        "{multi}"
    );

    let later = poll_widget(&html, "later");
    assert!(later.contains("<li class=\"results-on-close\">"), "{later}");

    // The poll's author may close it: the dropdown's one action, drawn as
    // a button (its action is not ported).
    let mut user0 = Client::logged_in(st.clone(), "user0").await;
    let html = user0.page(&path).await;
    let single = poll_widget(&html, "single");
    assert!(single.contains(
        "<button class=\"btn btn-icon-text widget-button btn-default toggle-status\" type=\"button\" data-poll-action=\"toggleStatus\">"
    ), "{single}");
}

#[tokio::test]
async fn a_poll_vote_draws_the_widget_again() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let post_id = poll_topic(&st).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let live =
        |name: &str, state: &str| format!("/live/poll/{post_id}/{name}?state={}", urlencode(state));

    // Picking options in a multiple choice: the vote in progress, which
    // Cast Votes sends once it is enough.
    let html = user1.page(&live("multi", "{}")).await;
    let digests = poll_digests(&html);
    assert_eq!(digests.len(), 3, "{html}");
    let html = user1
        .page(&live(
            "multi",
            &format!("{{\"vote\":[\"{}\"]}}", digests[0]),
        ))
        .await;
    assert!(html.contains("d-icon-far-square-check svg-icon"), "{html}");
    assert!(
        html.contains("<button class=\"btn cast-votes btn-primary\" title=\"Cast your votes\">"),
        "{html}"
    );
    let three = format!(
        "{{\"vote\":[\"{}\",\"{}\",\"{}\"]}}",
        digests[0], digests[1], digests[2]
    );
    let html = user1.page(&live("multi", &three)).await;
    assert!(
        html.contains("<button class=\"btn cast-votes btn-default\" disabled=\"\""),
        "{html}"
    );

    // A single choice voted: its results show, with the voter; Hide
    // results goes back to the options with the vote and Remove vote.
    let single = user1.page(&live("single", "{}")).await;
    let digests = poll_digests(&single);
    let voted = user1
        .send(
            Method::PUT,
            "/polls/vote",
            &format!(
                "post_id={post_id}&poll_name=single&options%5B%5D={}",
                digests[0]
            ),
        )
        .await;
    assert!(voted.contains("\"voters\":1"), "{voted}");
    let html = user1
        .page(&live("single", "{\"results\":true,\"toggle\":true}"))
        .await;
    assert!(
        html.contains(
            "<div class=\"discourse-poll-regular-results\"> <div class=\"tab-container\">"
        ),
        "{html}"
    );
    assert!(
        html.contains("<li class=\"chosen\"> <div class=\"option\"> <p> <span class=\"percentage\">100%</span> <span class=\"option-text\">Yes</span>"),
        "{html}"
    );
    assert!(
        html.contains("<a data-user-card=\"user1\" title=\"user1\">"),
        "{html}"
    );
    assert!(!html.contains("Not ported"), "{html}");
    assert!(
        html.contains(
            "<span class=\"info-number\">1</span> <span class=\"info-label\">voter</span>"
        ),
        "{html}"
    );
    assert!(
        html.contains("btn btn-default toggle-results\" title=\"Back to your votes\""),
        "{html}"
    );
    let html = user1
        .page(&live("single", "{\"results\":false,\"toggle\":false}"))
        .await;
    assert!(html.contains("d-icon-circle svg-icon"), "{html}");
    assert!(
        html.contains("btn btn-default remove-vote\" title=\"Remove your vote\""),
        "{html}"
    );
    assert!(
        html.contains("btn btn-default toggle-results\" title=\"Display the poll results\""),
        "{html}"
    );

    // The page draws it as Ember starts it: a saved vote shows the results.
    let topic_id: i32 = sqlx::query_scalar("SELECT topic_id FROM posts WHERE id = $1")
        .bind(post_id as i32)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let page = user1
        .page(&format!("/t/three-polls-to-vote-in/{topic_id}"))
        .await;
    assert!(poll_widget(&page, "single").contains("discourse-poll-regular-results"));

    // Results that wait for the close: the vote is recorded, to amend.
    let later = user1.page(&live("later", "{}")).await;
    let digests = poll_digests(&later);
    user1
        .send(
            Method::PUT,
            "/polls/vote",
            &format!(
                "post_id={post_id}&poll_name=later&options%5B%5D={}",
                digests[1]
            ),
        )
        .await;
    let html = user1.page(&live("later", "{}")).await;
    assert!(
        html.contains("<div class=\"poll-voted-choices\">"),
        "{html}"
    );
    assert!(html.contains("<li class=\"vote-recorded\">"), "{html}");
    assert!(
        html.contains("<span class=\"option-text\">Never</span>"),
        "{html}"
    );
    assert!(
        html.contains("btn btn-default amend-vote\" title=\"Change your vote\""),
        "{html}"
    );
    let html = user1.page(&live("later", "{\"amending\":true}")).await;
    assert!(html.contains("btn btn-default keep-vote\""), "{html}");
    assert_eq!(poll_digests(&html).len(), 2, "{html}");

    // Removed, the options are back as they were.
    user1
        .send(
            Method::DELETE,
            "/polls/vote",
            &format!("post_id={post_id}&poll_name=single"),
        )
        .await;
    let html = user1.page(&live("single", "{}")).await;
    assert!(!html.contains("d-icon-circle svg-icon"), "{html}");
    assert!(
        html.contains("<span class=\"info-number\">0</span>"),
        "{html}"
    );

    // Not a poll the post has, or not a post the viewer sees: not found.
    assert!(user1.page(&live("nope", "{}")).await.is_empty());
}

/// The `data-poll-state` a widget was drawn in.
fn poll_state(widget: &str) -> serde_json::Value {
    let raw = widget
        .split("data-poll-state=\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap_or_else(|| panic!("no state: {widget}"));
    serde_json::from_str(&raw.replace("&quot;", "\"").replace("&amp;", "&")).unwrap()
}

#[tokio::test]
async fn poll_results_draw_as_ember_draws_them() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let raw = "Results:\n\n\
        [poll name=reg public=true]\n* Alpha\n* Beta\n[/poll]\n\n\
        [poll name=num type=number min=1 max=5 step=1]\n[/poll]\n\n\
        [poll name=pie chartType=pie]\n* Cats\n* Dogs\n[/poll]\n\n\
        [poll name=rc type=ranked_choice public=true]\n* Xeno\n* Yarn\n[/poll]";
    let mut user0 = Client::logged_in(st.clone(), "user0").await;
    let created = user0
        .send(
            Method::POST,
            "/posts",
            &format!(
                "title={}&raw={}&category=4",
                urlencode("Poll results to draw"),
                urlencode(raw)
            ),
        )
        .await;
    let created: serde_json::Value = serde_json::from_str(&created).expect(&created);
    let post_id = created["id"].as_i64().unwrap();
    // 27 more members, voters of Alpha, the first 25 preloaded; three of
    // them rate the number poll 1, 4 and 4, two of them pick Cats.
    let mut conn = db.pool.acquire().await.unwrap();
    for sql in [
        "CREATE TEMP TABLE tu AS SELECT * FROM users WHERE id = 5",
        "INSERT INTO users SELECT (jsonb_populate_record(tu, jsonb_build_object('id', g, 'username', 'voter' || g, 'username_lower', 'voter' || g, 'name', 'Voter ' || g))).* FROM tu, generate_series(100, 126) g",
        "INSERT INTO poll_votes (poll_id, poll_option_id, user_id, created_at, updated_at, rank) \
         SELECT p.id, po.id, g, timestamp '2026-10-20 10:00' + g * interval '1 minute', now(), 0 \
         FROM polls p JOIN poll_options po ON po.poll_id = p.id, generate_series(100, 126) g \
         WHERE p.name = 'reg' AND po.html = 'Alpha'",
        "INSERT INTO poll_votes (poll_id, poll_option_id, user_id, created_at, updated_at, rank) \
         SELECT p.id, po.id, v.u, now(), now(), 0 FROM polls p JOIN poll_options po ON po.poll_id = p.id \
         JOIN (VALUES (100, '1'), (101, '4'), (102, '4')) v(u, h) ON po.html = v.h WHERE p.name = 'num'",
        "INSERT INTO poll_votes (poll_id, poll_option_id, user_id, created_at, updated_at, rank) \
         SELECT p.id, po.id, g, now(), now(), 0 FROM polls p JOIN poll_options po ON po.poll_id = p.id, \
         generate_series(100, 101) g WHERE p.name = 'pie' AND po.html = 'Cats'",
    ] {
        // One connection: the temporary table lives on it.
        sqlx::query(sql)
            .execute(&mut *conn)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    drop(conn);
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let live =
        |name: &str, state: &str| format!("/live/poll/{post_id}/{name}?state={}", urlencode(state));
    let shown = "{\"results\":true}";

    // Standard results: even-round percentages, the bars, the preloaded
    // voters and a load more for the rest.
    let html = user1.page(&live("reg", shown)).await;
    assert!(
        html.contains("<span class=\"percentage\">100%</span> <span class=\"option-text\">Alpha</span> </p> <div class=\"bar-back\"> <div class=\"bar\" style=\"width:100%\"></div> </div>"),
        "{html}"
    );
    assert!(
        html.contains(
            "<span class=\"percentage\">0%</span> <span class=\"option-text\">Beta</span>"
        ),
        "{html}"
    );
    assert_eq!(html.matches("data-user-card=\"voter").count(), 25, "{html}");
    assert!(html.contains("<button class=\"poll-voters-toggle-expand btn no-text btn-icon\" type=\"button\" data-poll-voters=\""), "{html}");
    let state = poll_state(&html);
    assert_eq!(state["expanded"], false);
    let alpha = state["voters"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, v)| v.as_array().unwrap().len() == 25)
        .map(|(k, _)| k.clone())
        .unwrap();
    // The gear's one action is the tally: drawn as counts.
    assert!(
        html.contains("data-poll-action=\"toggleDisplayMode\""),
        "{html}"
    );
    let html = user1
        .page(&live("reg", "{\"results\":true,\"tally\":true}"))
        .await;
    assert!(
        html.contains("<span class=\"absolute\">27 votes</span>"),
        "{html}"
    );
    assert!(
        html.contains("<span class=\"absolute\">0 votes</span>"),
        "{html}"
    );
    assert!(html.contains("btn-default show-percentage"), "{html}");

    // Load more: voters.json's next page, kept in the state and drawn.
    let page = user1
        .send(
            Method::GET,
            &format!("/polls/voters.json?post_id={post_id}&poll_name=reg&option_id={alpha}&page=2&limit=25"),
            "",
        )
        .await;
    let page: serde_json::Value = serde_json::from_str(&page).expect(&page);
    let more = page["voters"][&alpha].as_array().unwrap();
    assert_eq!(more.len(), 2, "{page}");
    let mut kept = state["voters"][&alpha].as_array().unwrap().clone();
    for v in more {
        kept.push(serde_json::json!([v["username"], v["avatar_template"]]));
    }
    let expanded = serde_json::json!({"results": true, "voters": {alpha.clone(): kept}});
    let html = user1.page(&live("reg", &expanded.to_string())).await;
    assert_eq!(html.matches("data-user-card=\"voter").count(), 27, "{html}");
    assert_eq!(poll_state(&html)["expanded"], true);
    // All 27 drawn: no load more for Alpha; Beta has no votes to load.
    assert!(!html.contains("poll-voters-toggle-expand"), "{html}");

    // The number poll's average, to two decimals.
    let html = user1.page(&live("num", shown)).await;
    assert!(
        html.contains("<div class=\"discourse-poll-number-results\"> <span>Average rating: <strong>3</strong>.</span> </div>"),
        "{html}"
    );

    // The pie's canvas and legend, filled by poll.js from the state.
    let html = user1.page(&live("pie", shown)).await;
    assert!(
        html.contains(&format!("<canvas class=\"poll-results-canvas\" id=\"poll-results-chart-pie-{post_id}\"></canvas>")),
        "{html}"
    );
    assert_eq!(poll_state(&html)["pie"]["data"], serde_json::json!([2, 0]));

    // Ranked choice: the options with their rank dropdowns, the vote in
    // progress, cast, then the outcome.
    let html = user1.page(&live("rc", "{}")).await;
    assert!(
        html.contains("<ul class=\"ranked-choice-poll-options options\">"),
        "{html}"
    );
    let digests: Vec<String> = poll_state(&html)["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_str().unwrap().to_string())
        .collect();
    let vote = serde_json::json!({"vote": [{"digest": digests[0], "rank": 2}, {"digest": digests[1], "rank": 1}]});
    let html = user1.page(&live("rc", &vote.to_string())).await;
    assert!(
        html.contains("<span class=\"d-button-label\"> 2 </span>"),
        "{html}"
    );
    assert!(
        html.contains("<button class=\"btn cast-votes btn-primary\""),
        "{html}"
    );
    let voted = user1
        .send(
            Method::PUT,
            "/polls/vote",
            &format!(
                "post_id={post_id}&poll_name=rc&options%5B0%5D%5Bdigest%5D={}&options%5B0%5D%5Brank%5D=2&options%5B1%5D%5Bdigest%5D={}&options%5B1%5D%5Brank%5D=1",
                digests[0], digests[1]
            ),
        )
        .await;
    assert!(voted.contains("\"voters\":1"), "{voted}");
    let html = user1.page(&live("rc", shown)).await;
    assert!(html.contains("<li class=\"tab nav-item active\"> <button class=\"nav-btn btn no-text\" type=\"button\" data-poll-tab=\"outcome\"> Outcome </button>"), "{html}");
    assert!(
        html.contains("<span class=\"poll-results-ranked-choice-info\">Winner was Yarn after one round.</span>"),
        "{html}"
    );
    let html = user1
        .page(&live("rc", "{\"results\":true,\"tab\":\"votes\"}"))
        .await;
    assert!(html.contains("<span class=\"rank\">1</span>"), "{html}");
    assert!(html.contains("<span class=\"rank\">2</span>"), "{html}");

    // Anonymous readers are asked to log in instead of ranking.
    let mut anon = Client {
        state: st.clone(),
        cookies: Vec::new(),
        csrf: None,
    };
    let html = anon.page(&live("rc", "{}")).await;
    assert!(
        html.contains("<span class=\"d-button-label\">Login to vote!</span>"),
        "{html}"
    );

    // The topic page hands poll.js where Chart.js is.
    let topic_id = created["topic_id"].as_i64().unwrap();
    let page = user1
        .page(&format!("/t/poll-results-to-draw/{topic_id}"))
        .await;
    assert!(page.contains("data-chart-js=\""), "{page}");
}

/// The `data-checklist-post` of the post with that id's `div.cooked`, as
/// JSON; None when it has none.
fn checklist_model(html: &str, post_id: i64) -> Option<serde_json::Value> {
    let article = html.find(&format!("data-post-id=\"{post_id}\""))?;
    let rest = &html[article..];
    let rest = &rest[..rest.find("</article>").unwrap_or(rest.len())];
    let start = rest.find("<div class=\"cooked\"")?;
    let tag = &rest[start..start + rest[start..].find('>')?];
    let value = tag
        .split("data-checklist-post=\"")
        .nth(1)?
        .split('"')
        .next()?;
    serde_json::from_str(&value.replace("&quot;", "\"")).ok()
}

#[tokio::test]
async fn checklist_posts_carry_what_the_decorator_reads() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let raw = "Checklist to toggle:\n\n- [ ] first\n- [x] second\n\n[X] fixed and [] legacy";
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let created = user1
        .send(
            Method::POST,
            "/posts",
            &format!("topic_id=35&raw={}", urlencode(raw)),
        )
        .await;
    let created: serde_json::Value = serde_json::from_str(&created).expect(&created);
    let post_id = created["id"].as_i64().unwrap();
    let slug: String = sqlx::query_scalar("SELECT slug FROM topics WHERE id = 35")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let path = format!("/t/{slug}/35");

    let html = user1.page(&path).await;
    // The decorator's script, its setting and its label.
    assert!(html.contains("/assets/checklist.js"), "{html}");
    assert!(html.contains("\"checklist_enabled\":true"), "{html}");
    assert!(
        html.contains("\"checklist.item\":\"Checklist item\""),
        "{html}"
    );
    // The model it reads, on the post with boxes only.
    let model = checklist_model(&html, post_id).unwrap_or_else(|| panic!("{html}"));
    assert_eq!(
        model,
        serde_json::json!({
            "topic_id": 35,
            "post_number": created["post_number"],
            "can_edit": true,
            "updated_at": created["updated_at"],
            "version": 1,
        })
    );
    assert_eq!(html.matches("data-checklist-post=").count(), 1, "{html}");
    // The boxes as cooked, the permanent one without a source.
    assert!(html.contains(
        "<li><span class=\"chcklst-box fa fa-square-o\" data-chk-src=\"2:0\"></span> first</li>"
    ));
    assert!(html.contains(
        "<span class=\"chcklst-box checked permanent fa fa-square-check\"></span> fixed"
    ));

    // Read only for those who may not edit it.
    let mut anonymous = Client {
        state: st.clone(),
        cookies: Vec::new(),
        csrf: None,
    };
    let html = anonymous.page(&path).await;
    assert_eq!(checklist_model(&html, post_id).unwrap()["can_edit"], false);

    // A toggle's revised message carries its token, for the page that
    // made it to keep its drawing.
    let from = st.bus.now().await.unwrap();
    let body = serde_json::json!({
        "post_id": post_id,
        "expected_raw": raw,
        "expected_updated_at": created["updated_at"],
        "mutation_id": "mutation-1",
        "toggles": [{"checkbox_index": 0, "checkbox_source": "2:0", "checked": true}],
    });
    let toggled = user1
        .send_as(
            Method::PUT,
            "/checklist/toggle",
            &body.to_string(),
            "application/json",
        )
        .await;
    let toggled: serde_json::Value = serde_json::from_str(&toggled).expect(&toggled);
    assert_eq!(toggled["revised"], true, "{toggled}");
    assert!(toggled["raw"].as_str().unwrap().contains("- [x] first"));
    let messages = common::bus_messages(&st, from, &["/topic/35"]).await;
    let revised: Vec<&serde_json::Value> = messages
        .iter()
        .map(|m| &m.data)
        .filter(|d| d["type"] == "revised")
        .collect();
    assert_eq!(revised.len(), 1, "{messages:?}");
    assert_eq!(revised[0]["preserve_cooked_token"], "mutation-1");

    // The page drawn again has the new state and update time.
    let html = user1.page(&path).await;
    let model = checklist_model(&html, post_id).unwrap();
    assert_eq!(model["updated_at"], toggled["updated_at"]);
    assert!(html.contains(
        "<li><span class=\"chcklst-box checked fa fa-square-check-o\" data-chk-src=\"2:0\"></span> first</li>"
    ));
}

/// The system user's toggle is saved and answered as anyone's: revise!
/// returns successfully_saved_post_and_topic for it too (its
/// SYSTEM_USER_ID return is inside alert_users, not revise!'s result).
#[tokio::test]
async fn the_system_user_toggles_a_box() {
    use sha2::{Digest, Sha256};
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let raw = "- [ ] first\n- [x] second";
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let created = user1
        .send(
            Method::POST,
            "/posts",
            &format!("topic_id=35&raw={}", urlencode(raw)),
        )
        .await;
    let created: serde_json::Value = serde_json::from_str(&created).expect(&created);
    // A global key (no user), acting as the system user.
    sqlx::query(
        "INSERT INTO api_keys (id, user_id, created_by_id, created_at, updated_at, key_hash, truncated_key, scope_mode) \
         VALUES (9701, NULL, 1, now(), now(), $1, 'sysk', 0)",
    )
    .bind(format!("{:x}", Sha256::digest(b"system-toggle-key")))
    .execute(&db.pool)
    .await
    .unwrap();
    let body = serde_json::json!({
        "post_id": created["id"],
        "expected_raw": raw,
        "expected_updated_at": created["updated_at"],
        "mutation_id": "mutation-system",
        "toggles": [{"checkbox_index": 0, "checkbox_source": "0:0", "checked": true}],
    });
    let request = Request::builder()
        .method(Method::PUT)
        .uri("/checklist/toggle")
        .header(header::HOST, "test.localhost")
        .header(header::CONTENT_TYPE, "application/json")
        .header("api-key", "system-toggle-key")
        .header("api-username", "system")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = discourse_rs::app(st.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let answer = String::from_utf8_lossy(&bytes);
    assert_eq!(status, axum::http::StatusCode::OK, "{answer}");
    let answer: serde_json::Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(answer["revised"], true, "{answer}");
    let (saved, editor): (String, Option<i32>) =
        sqlx::query_as("SELECT raw, last_editor_id FROM posts WHERE id = $1")
            .bind(created["id"].as_i64().unwrap() as i32)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(saved, "- [x] first\n- [x] second");
    assert_eq!(editor, Some(-1));
}

// Chat's new message modal (static/js/chat-new-message.js) rides on every
// page for a member who can chat, with what the modal reads of the site
// and the member; a visitor gets neither.
#[tokio::test]
async fn every_page_carries_the_new_message_modal() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    for path in ["/latest", "/u/user0/summary", "/chat/browse/open"] {
        let html = user1.page(path).await;
        let at = html
            .find("/assets/chat-new-message.js?v=")
            .unwrap_or_else(|| panic!("{path}: {html}"));
        assert!(
            html.find("/assets/chat-drawer.js?v=").unwrap() < at,
            "{path}"
        );
        let config = &html[at..at + html[at..].find(" defer>").unwrap()];
        for want in [
            "&quot;filter&quot;:&quot;Filter&quot;",
            "&quot;newGroupChat&quot;:&quot;New group chat&quot;",
            "&quot;addMembers&quot;:&quot;...add more members&quot;",
            "&quot;groupName&quot;:&quot;Group chat name (optional)&quot;",
            "&quot;membersOther&quot;:&quot;%{count}/%{max} members&quot;",
            "&quot;createGroup&quot;:&quot;Create group chat&quot;",
            "&quot;canGroupChat&quot;:true",
            "&quot;maxDmUsers&quot;:20",
            "&quot;canDirectMessage&quot;:true",
            "&quot;publicChannels&quot;:true",
            "&quot;prioritizeUsername&quot;:true",
            "&quot;username&quot;:&quot;user1&quot;",
        ] {
            assert!(config.contains(want), "{path}: {want} in {config}");
        }
    }
    assert!(
        user1
            .page("/assets/chat-new-message.js")
            .await
            .contains("ChatModalNewMessage")
    );
    let html = Client {
        state: st,
        cookies: Vec::new(),
        csrf: None,
    }
    .page("/latest")
    .await;
    assert!(!html.contains("chat-new-message.js"));
}

// GET /chat/new-message, as ChatNewMessageRoute leaves a page load: to
// /chat without params, to the channel asked for (by id or slug) or /chat
// when it can't be opened. With recipients the page is served and nothing
// is written; the browser finds or creates the direct message with the
// POST chat-new-message.js sends (recorded as
// chat/dm_create_from_new_message_modal) and opens it. A seeded draft is
// refused; a visitor goes where the chat route sends them.
#[tokio::test]
async fn chat_new_message_goes_where_the_route_goes() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    assert_eq!(
        user1.head("/chat/new-message").await,
        (302, Some("/chat".into()))
    );
    assert_eq!(
        user1.head("/chat/new-message?channel_id=2").await,
        (302, Some("/chat/c/general/2".into()))
    );
    assert_eq!(
        user1.head("/chat/new-message?channel=general").await,
        (302, Some("/chat/c/general/2".into()))
    );
    assert_eq!(
        user1.head("/chat/new-message?channel_id=1").await,
        (302, Some("/chat".into()))
    );
    assert_eq!(
        user1.head("/chat/new-message?channel=nope").await,
        (302, Some("/chat".into()))
    );
    let channels = |pool: sqlx::PgPool| async move {
        sqlx::query_scalar::<_, i64>(
            "SELECT (SELECT count(*) FROM chat_channels) + (SELECT count(*) FROM direct_message_channels) \
             + (SELECT count(*) FROM direct_message_users) + (SELECT count(*) FROM user_chat_channel_memberships)",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let before = channels(db.pool.clone()).await;
    for path in [
        "/chat/new-message?recipients=user0",
        "/chat/new-message?recipients=user0,user2",
        "/chat/new-message?recipients=",
    ] {
        assert_eq!(user1.head(path).await, (200, None), "{path}");
    }
    let html = user1.page("/chat/new-message?recipients=user0").await;
    assert!(
        html.contains("<div class=\"main-chat-outlet chat-view\" id=\"main-chat-outlet\"></div>")
    );
    assert!(html.contains("/assets/chat-new-message.js?v="));
    assert_eq!(channels(db.pool.clone()).await, before, "a GET wrote");
    // chat.upsertDmChannel's request, then the channel it opens.
    let body = "target_usernames%5B%5D=user0&target_groups=&upsert=true&name=";
    let created: serde_json::Value = serde_json::from_str(
        &user1
            .send(Method::POST, "/chat/api/direct-message-channels.json", body)
            .await,
    )
    .unwrap();
    let id = created["channel"]["id"].as_i64().unwrap();
    assert_eq!(created["channel"]["title"], "user0");
    let members: Vec<String> = sqlx::query_scalar(
        "SELECT u.username FROM chat_channels c \
         JOIN direct_message_users d ON d.direct_message_channel_id = c.chatable_id \
         JOIN users u ON u.id = d.user_id WHERE c.id = $1 ORDER BY u.username",
    )
    .bind(id)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(members, ["user0", "user1"]);
    assert_eq!(
        user1.head(&format!("/chat/c/user0/{id}")).await,
        (200, None)
    );
    // Found again, not created twice.
    let again: serde_json::Value = serde_json::from_str(
        &user1
            .send(Method::POST, "/chat/api/direct-message-channels.json", body)
            .await,
    )
    .unwrap();
    assert_eq!(again["channel"]["id"].as_i64(), Some(id));
    assert_eq!(
        user1
            .head("/chat/new-message?recipients=user0&message=hello")
            .await
            .0,
        500
    );
    let mut visitor = Client {
        state: st,
        cookies: Vec::new(),
        csrf: None,
    };
    assert_eq!(
        visitor.head("/chat/new-message?recipients=user0").await,
        (302, Some("/latest".into()))
    );
}
