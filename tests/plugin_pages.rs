//! The bundled plugins' UI on the server-rendered pages. discourse-reactions:
//! the reaction controls in the post menu, with what the browser side reads
//! to draw the picker and the users menu, and the activity Reactions tab.
//! discourse-solved: the solved status, the accepted answers under the
//! first post, the Solved button, "Me too" and the activity Solved tab.

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
