//! The chat drawer's channel and list routes beyond the channel
//! (GET /live/chat/drawer, plugins/chat/drawer.rs): a channel's settings
//! and members, its pins, its threads and a thread, the member's threads
//! and search, each with the drawer's navbar and the content its
//! full-page counterpart draws; and GET /live/chat/search, the search's
//! results.

#[path = "common/chat_client.rs"]
mod chat_client;
mod common;

use chat_client::Client;
use common::{TestDb, config, set_setting, state};
use discourse_rs::config::RailsEnv;

/// The drawer's buttons, expanded.
const DRAWER_BUTTONS: &str = "<button class=\"btn no-text btn-icon btn-transparent no-text c-navbar__toggle-drawer-button\" title=\"Collapse Chat Drawer\" type=\"button\"><svg class=\"fa d-icon d-icon-minus svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#minus\"></use></svg><span aria-hidden=\"true\">&#8203;</span></button><button class=\"btn no-text btn-icon btn-transparent no-text c-navbar__full-page-button\" title=\"Open full-screen chat\" type=\"button\"><svg class=\"fa d-icon d-icon-discourse-expand svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#discourse-expand\"></use></svg><span aria-hidden=\"true\">&#8203;</span></button><button class=\"btn no-text btn-icon btn-transparent no-text c-navbar__close-drawer-button\" title=\"Close\" type=\"button\"><svg class=\"fa d-icon d-icon-xmark svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#xmark\"></use></svg><span aria-hidden=\"true\">&#8203;</span></button>";

fn icon(name: &str) -> String {
    format!(
        "<svg class=\"fa d-icon d-icon-{name} svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#{name}\"></use></svg>"
    )
}

/// The drawer's route for a query: the status, the routes visited, the
/// channel shown and the markup.
async fn drawer(c: &mut Client, query: &str) -> (u16, String, Option<String>, String) {
    let (status, headers, html) = c
        .page_with_headers(&format!("/live/chat/drawer?{query}"))
        .await;
    let header = |name: &str| headers.get(name).map(|v| v.to_str().unwrap().to_string());
    (
        status,
        header("x-chat-drawer-routes").unwrap_or_default(),
        header("x-chat-drawer-channel"),
        html,
    )
}

/// General (2) with threading and a thread (1) of message 1 with two
/// replies, which user1 (3) follows; message 4 in the channel.
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
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
    set_setting(&db.pool, "chat_threads_enabled", 5, "t").await;
}

#[tokio::test]
async fn the_drawer_draws_a_channels_settings_and_members() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let (status, routes, channel, html) =
        drawer(&mut user1, "url=/chat/c/general/2/info/settings").await;
    assert_eq!(
        (status, routes.as_str(), channel.as_deref()),
        (200, "chat.channel.info.settings", Some("2"))
    );
    // The navbar: back to the channel, the title linking to the settings,
    // the star for a member following it, the drawer's buttons.
    assert!(
        html.starts_with(&format!(
            "<div class=\"c-drawer-routes --channel-info-settings\"><div class=\"c-navbar-container -clickable\"><nav class=\"c-navbar\"><a class=\"c-navbar__back-button no-text btn-transparent btn\" title=\"Return to channel\" href=\"/chat/c/general/2\">{}</a><a class=\"c-navbar__channel-title\" href=\"/chat/c/general/2/info/settings\"><div class=\"chat-channel-title is-category\">",
            icon("chevron-left")
        )),
        "{html}"
    );
    assert!(html.contains("c-navbar__star-channel-button"), "{html}");
    // The nav and the tab, as full-page chat draws them.
    assert!(
        html.contains(&format!("{DRAWER_BUTTONS}</nav></nav></div><div class=\"chat-drawer-content\"><div class=\"c-routes --channel-info-nav\"><nav class=\"c-channel-info__nav\"><ul class=\"nav nav-pills\"><li><a class=\"active\" href=\"/chat/c/general/2/info/settings\">Settings</a></li><li><a href=\"/chat/c/general/2/info/members\">Members<span class=\"c-channel-info__member-count\">")),
        "{html}"
    );
    assert!(
        html.contains("<div class=\"c-channel-settings\" data-channel-id=\"2\""),
        "{html}"
    );
    // The toasts and emoji picker chat-channel-info.js shows; its script
    // comes with the drawer's.
    assert!(html.contains("fk-d-toast"));
    assert!(!html.contains("<script"));
    let (_, _, page) = user1.page_with_headers("/latest").await;
    assert!(page.contains("chat-channel-info.js?v"), "{page}");

    let (_, routes, channel, html) = drawer(
        &mut user1,
        "url=/chat/c/-/2/info/members&previous=chat.channel",
    )
    .await;
    assert_eq!(
        (routes.as_str(), channel.as_deref()),
        ("chat.channel.info.members", Some("2"))
    );
    assert!(
        html.starts_with("<div class=\"c-drawer-routes --channel-info-members\">"),
        "{html}"
    );
    assert!(html.contains("<li><a href=\"/chat/c/general/2/info/settings\">Settings</a></li><li><a class=\"active\" href=\"/chat/c/general/2/info/members\">"));
    assert!(
        html.contains("<div class=\"c-channel-members\" data-channel-id=\"2\" data-page-url=\"/chat/c/general/2/info/members/page\">"),
        "{html}"
    );

    // Collapsed: the navbar alone, the title unlinked, no star or back.
    let (_, _, _, html) = drawer(
        &mut user1,
        "url=/chat/c/general/2/info/settings&expanded=false",
    )
    .await;
    assert!(html.starts_with("<div class=\"c-drawer-routes --channel-info-settings\"><div class=\"c-navbar-container -clickable\"><nav class=\"c-navbar\"><div class=\"c-navbar__channel-title\">"), "{html}");
    for absent in [
        "chat-drawer-content",
        "c-navbar__back-button",
        "c-navbar__star-channel-button",
        "c-navbar__full-page-button",
    ] {
        assert!(!html.contains(absent), "{absent}: {html}");
    }
    assert!(html.ends_with("</nav></nav></div></div>"), "{html}");
    // A channel the member can't open is refused.
    assert_eq!(
        drawer(&mut user1, "url=/chat/c/staff/1/info/settings")
            .await
            .0,
        500
    );
}

#[tokio::test]
async fn the_drawer_draws_a_channels_pins() {
    let db = TestDb::new().await;
    threaded(&db).await;
    // Refused while pinned messages are off (the API's 404).
    set_setting(&db.pool, "chat_pinned_messages", 5, "f").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    assert_eq!(
        drawer(&mut user1, "url=/chat/c/general/2/pins").await.0,
        500
    );
    set_setting(&db.pool, "chat_pinned_messages", 5, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, _) = admin
        .request("POST", "/chat/api/channels/2/messages/4/pin", "")
        .await;
    assert_eq!(status, 200);
    let mut user1 = Client::logged_in(st, "user1").await;
    let (status, routes, channel, html) = drawer(&mut user1, "url=/chat/c/general/2/pins").await;
    assert_eq!(
        (status, routes.as_str(), channel.as_deref()),
        (200, "chat.channel.pins", Some("2"))
    );
    assert_eq!(
        html.split("<div class=\"chat-drawer-content\">")
            .next()
            .unwrap(),
        format!(
            "<div class=\"c-drawer-routes --channel-pins\"><div class=\"c-navbar-container -clickable\"><nav class=\"c-navbar\"><a class=\"c-navbar__back-button no-text btn-transparent btn\" title=\"Return to channels list\" href=\"/chat/c/general/2\">{}</a><div class=\"c-navbar__title\" title=\"Pinned messages - General\"><span class=\"c-navbar__title-text\">{}Pinned messages - General</span></div><nav class=\"c-navbar__actions\">{DRAWER_BUTTONS}</nav></nav></div>",
            icon("chevron-left"),
            icon("thumbtack")
        )
    );
    // ChatPinnedMessagesList, as the side panel draws it.
    assert!(
        html.contains("<div class=\"chat-drawer-content\"><div class=\"chat-pinned-messages-list chat-messages-scroller\" data-channel-id=\"2\"><div class=\"chat-pinned-messages-list__items\"><div class=\"chat-pinned-message\"><a class=\"chat-pinned-message__link\" href=\"/chat/c/general/2/4\"><template class=\"chat-pinned-message__content\">"),
        "{html}"
    );
    assert!(html.contains("<p>channel chatter</p>"));
    assert!(html.ends_with("</div></div></div></div>"), "{html}");
    // The full page's side panel draws the same list.
    let (_, _, page) = user1.page_with_headers("/chat/c/general/2/pins").await;
    let list = |html: &str| {
        let start = html.find("<div class=\"chat-pinned-messages-list").unwrap();
        let rest = &html[start..];
        rest[..rest.find("</div></div>").unwrap()].to_string()
    };
    assert_eq!(list(&page), list(&html));
    // Collapsed: the navbar alone.
    let (_, _, _, html) = drawer(&mut user1, "url=/chat/c/general/2/pins&expanded=false").await;
    assert!(!html.contains("chat-pinned-messages-list"), "{html}");
    assert!(!html.contains("c-navbar__back-button"), "{html}");
}

#[tokio::test]
async fn the_drawer_draws_a_channels_threads_and_a_thread() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    threaded(&db).await;
    let mut user1 = Client::logged_in(st, "user1").await;

    // The channel's threads list button leads here.
    let (_, _, _, html) = drawer(&mut user1, "url=/chat/c/general/2").await;
    assert!(
        html.contains("href=\"/chat/c/general/2/t\""),
        "the threads list button: {html}"
    );
    let (status, routes, channel, html) = drawer(&mut user1, "url=/chat/c/general/2/t").await;
    assert_eq!(
        (status, routes.as_str(), channel.as_deref()),
        (200, "chat.channel.threads", Some("2"))
    );
    assert!(
        html.starts_with(&format!(
            "<div class=\"c-drawer-routes --channel-threads\"><div class=\"c-navbar-container -clickable\"><nav class=\"c-navbar\"><a class=\"c-navbar__back-button no-text btn-transparent btn\" title=\"Return to channels list\" href=\"/chat/c/general/2\">{}</a><div class=\"c-navbar__title\" title=\"Threads - General\"><span class=\"c-navbar__title-text\">{}Threads - General</span></div><nav class=\"c-navbar__actions\">{DRAWER_BUTTONS}</nav></nav></div><div class=\"chat-drawer-content\"><div class=\"chat-thread-list\"><div class=\"chat-thread-list__items\"><div class=\"chat-thread-list-item",
            icon("chevron-left"),
            icon("discourse-threads")
        )),
        "{html}"
    );
    assert!(
        html.contains("data-href=\"/chat/c/general/2/t/1\""),
        "{html}"
    );
    assert!(html.ends_with("</div></div></div></div>"), "{html}");

    // A thread, back to the threads list it came from.
    let (status, routes, channel, html) = drawer(
        &mut user1,
        "url=/chat/c/general/2/t/1&previous=chat.channel.threads",
    )
    .await;
    assert_eq!(
        (status, routes.as_str(), channel.as_deref()),
        (200, "chat.channel.thread", Some("2"))
    );
    assert!(
        html.starts_with(&format!(
            "<div class=\"c-drawer-routes --channel-thread\"><div class=\"c-navbar-container -clickable\"><nav class=\"c-navbar\"><a class=\"c-navbar__back-button no-text btn-transparent btn\" title=\"Return to threads list\" href=\"/chat/c/general/2/t\">{}</a><div class=\"c-navbar__title\" title=\"Thread\"><span class=\"c-navbar__title-text\">{}Thread</span></div><nav class=\"c-navbar__actions\">{DRAWER_BUTTONS}</nav></nav></div><div class=\"chat-drawer-content\"><div class=\"chat-thread --loaded\" data-id=\"1\" data-channel-id=\"2\"",
            icon("chevron-left"),
            icon("discourse-threads")
        )),
        "{html}"
    );
    // ChatThread as the full page draws it: its messages and composer,
    // no ThreadHeader.
    assert!(html.contains("<p>thread reply two</p>"));
    assert!(html.contains("id=\"thread-composer\""));
    assert!(!html.contains("c-navbar__close-thread-button"));
    assert!(html.ends_with("</div></div></div>"), "{html}");
    // From the member's threads, back to them; from elsewhere, back to
    // the channel.
    let (_, _, _, html) = drawer(
        &mut user1,
        "url=/chat/c/general/2/t/1&previous=chat.threads",
    )
    .await;
    assert!(
        html.contains("title=\"My Threads\" href=\"/chat/threads\">"),
        "{html}"
    );
    let (_, _, _, html) = drawer(&mut user1, "url=/chat/c/-/2/t/1&previous=chat.channel").await;
    assert!(
        html.contains("title=\"Return to channel\" href=\"/chat/c/general/2\">"),
        "{html}"
    );
    // A titled thread's title.
    sqlx::query("UPDATE chat_threads SET title = 'Big & news' WHERE id = 1")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, _, _, html) = drawer(&mut user1, "url=/chat/c/general/2/t/1").await;
    assert!(
        html.contains("<div class=\"c-navbar__title\" title=\"Big &amp; news\">"),
        "{html}"
    );
    // A message in the thread: the thread around it.
    let (_, routes, _, html) = drawer(&mut user1, "url=/chat/c/general/2/t/1/2").await;
    assert_eq!(routes, "chat.channel.thread.near-message");
    assert!(html.contains("<div class=\"chat-thread --loaded\" data-id=\"1\""));
    // A message with its thread: the channel around the message.
    let (_, routes, channel, html) = drawer(&mut user1, "url=/chat/c/general/2/1/t/1").await;
    assert_eq!(
        (routes.as_str(), channel.as_deref()),
        ("chat.channel.near-message-with-thread", Some("2"))
    );
    assert!(
        html.starts_with("<div class=\"c-drawer-routes --channel\">"),
        "{html}"
    );
    // Collapsed: the navbar alone.
    let (_, _, _, html) = drawer(&mut user1, "url=/chat/c/general/2/t/1&expanded=false").await;
    assert!(!html.contains("chat-thread "), "{html}");
    assert!(html.ends_with("</nav></nav></div></div>"), "{html}");
    let (_, _, _, html) = drawer(&mut user1, "url=/chat/c/general/2/t&expanded=false").await;
    assert!(!html.contains("chat-thread-list"), "{html}");
    // A thread that isn't the channel's is refused.
    assert_eq!(drawer(&mut user1, "url=/chat/c/general/2/t/9").await.0, 500);
    assert_eq!(drawer(&mut user1, "url=/chat/c/general/2/t/x").await.0, 500);
}

#[tokio::test]
async fn the_drawer_lists_the_members_threads() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    // None: the empty state, and no threads item in the footer.
    let (status, routes, channel, html) = drawer(&mut user1, "url=/chat/threads").await;
    assert_eq!(
        (status, routes.as_str(), channel),
        (200, "chat.threads", None)
    );
    assert!(
        html.starts_with(&format!("<div class=\"c-drawer-routes --threads\"><div class=\"c-navbar-container -clickable\"><nav class=\"c-navbar\"><div class=\"c-navbar__title\" title=\"Chat\"><span class=\"c-navbar__title-text\">Chat</span></div><nav class=\"c-navbar__actions\">{DRAWER_BUTTONS}</nav></nav></div><div class=\"chat-drawer-content\"><div class=\"c-list\"><div class=\"--loaded c-user-threads\"><div class=\"c-list-empty-state\">")),
        "{html}"
    );
    assert!(!html.contains("c-footer-threads"), "{html}");

    threaded(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (_, _, _, html) = drawer(&mut user1, "url=/chat/threads").await;
    assert!(
        html.contains("<div class=\"c-user-thread\" data-id=\"1\">"),
        "{html}"
    );
    assert!(html.contains("href=\"/chat/c/general/2/t/1\""), "{html}");
    // The footer, its threads item active and marked unread (user1 hasn't
    // read the last reply).
    assert!(
        html.contains(&format!("<button aria-label=\"Switch to my threads list\" class=\"btn btn-icon-text btn-transparent c-footer__item --active\" id=\"c-footer-threads\" type=\"button\" data-href=\"/chat/threads\">{}<span class=\"d-button-label\">My Threads</span><div class=\"c-unread-indicator\"></div></button>", icon("discourse-threads"))),
        "{html}"
    );
    // The other lists' footers offer it too.
    let (_, _, _, html) = drawer(&mut user1, "url=/chat/channels").await;
    assert!(
        html.contains("c-footer__item\" id=\"c-footer-threads\""),
        "{html}"
    );
    // Collapsed: the navbar alone.
    let (_, _, _, html) = drawer(&mut user1, "url=/chat/threads&expanded=false").await;
    assert!(!html.contains("c-user-threads"), "{html}");
    assert!(!html.contains("c-footer"), "{html}");
    // ROUTES.chat's redirect for my_threads.
    set_setting(&db.pool, "chat_preferred_index", 7, "my_threads").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let (_, routes, _, html) = drawer(&mut user1, "url=/chat").await;
    assert_eq!(routes, "chat chat.threads");
    assert!(html.starts_with("<div class=\"c-drawer-routes --threads\">"));
}

#[tokio::test]
async fn the_drawer_searches_chat() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    threaded(&db).await;
    for (id, text) in [
        (1, "Hello world"),
        (2, "hi back"),
        (3, "thread reply two"),
        (4, "channel chatter"),
    ] {
        sqlx::query(&format!(
            "INSERT INTO chat_message_search_data (chat_message_id, search_data, raw_data, locale, version) \
             VALUES ({id}, setweight(to_tsvector('english', '{text}'), 'A'), '{text}', 'en', 1)"
        ))
        .execute(&db.pool)
        .await
        .unwrap();
    }
    let mut user1 = Client::logged_in(st, "user1").await;
    let (status, routes, channel, html) = drawer(&mut user1, "url=/chat/search").await;
    assert_eq!(
        (status, routes.as_str(), channel),
        (200, "chat.search", None)
    );
    assert!(
        html.starts_with(&format!("<div class=\"c-drawer-routes --search\"><div class=\"c-navbar-container -clickable\"><nav class=\"c-navbar\"><div class=\"c-navbar__title\" title=\"Search chat\"><span class=\"c-navbar__title-text\">Search chat</span></div><nav class=\"c-navbar__actions\">{DRAWER_BUTTONS}</nav></nav></div><div class=\"chat-drawer-content\"><div class=\"c-search\" data-url=\"/live/chat/search\" data-limit=\"20\"><div class=\"chat-search__filters\"><div class=\"filter-input-container\">")),
        "{html}"
    );
    assert!(html.contains("placeholder=\"Search messages\""));
    assert!(html.contains("data-identifier=\"search-sort-options\""));
    assert!(html.contains("<span class=\"d-button-label\">Relevance</span>"));
    // The footer, its search item active.
    assert!(
        html.contains("c-footer__item --active\" id=\"c-footer-search\""),
        "{html}"
    );
    let (_, _, _, html) = drawer(&mut user1, "url=/chat/search&expanded=false").await;
    assert!(!html.contains("c-search"), "{html}");

    // The results: each message with its channel's title, its thread's
    // heading, the message not interactive and in the long date mode.
    sqlx::query("UPDATE chat_threads SET title = 'Big news' WHERE id = 1")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, headers, html) = user1
        .page_with_headers("/live/chat/search?q=thread&sort=latest&offset=0")
        .await;
    assert_eq!(status, 200, "{html}");
    assert_eq!(headers["x-chat-search-has-more"], "false");
    assert_eq!(html.matches("<li ").count(), 1, "{html}");
    assert!(
        html.starts_with("<li aria-describedby=\"chat-search-instructions\" aria-label=\"Chat message from admin: thread reply two in thread Big news\" class=\"chat-message-search-entry\" role=\"option\" tabindex=\"0\" data-url=\"/chat/c/-/2/t/1/3\"><div class=\"chat-message-search-entry__info\"><div class=\"chat-channel-title is-category\">"),
        "{html}"
    );
    assert!(
        html.contains(
            "<div class=\"chat-thread__heading\"><div class=\"chat-thread__heading-icon\">"
        ),
        "{html}"
    );
    assert!(html.contains("-not-interactive"), "{html}");
    assert!(html.contains("data-tz-text=\"%{"), "{html}");
    assert!(html.contains(":MMM D, h:mm a}\""), "{html}");
    assert!(!html.contains("data-mode="), "{html}");
    assert!(html.contains("<p>thread reply two</p>"));
    // A channel's message opens the channel around it; scoped to the
    // channel, no channel title.
    let (_, _, html) = user1
        .page_with_headers("/live/chat/search?q=chatter&channel_id=2")
        .await;
    assert!(html.contains("data-url=\"/chat/c/-/2/4\""), "{html}");
    assert!(!html.contains("chat-channel-title"), "{html}");
    // Paging: has_more while there are more.
    let (_, headers, html) = user1
        .page_with_headers("/live/chat/search?q=thread%20OR%20chatter&offset=0")
        .await;
    assert_eq!(headers["x-chat-search-has-more"], "false", "{html}");
    // A blank query is the contract's 400; with search off, not found.
    assert_eq!(user1.page_with_headers("/live/chat/search?q=").await.0, 400);
    set_setting(&db.pool, "chat_search_enabled", 5, "f").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    assert_eq!(
        user1
            .page_with_headers("/live/chat/search?q=thread")
            .await
            .0,
        404
    );
}
