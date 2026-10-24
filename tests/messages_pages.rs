//! The messages pages under the profile (`/u/:username/messages/...`): the
//! inboxes dropdown, the routed inbox's navigation and list, and the
//! exception and not-found pages a list's error leaves.

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

#[tokio::test]
async fn the_inbox_lists_ones_messages() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/u/user1/messages").await;
    assert!(
        html.contains("<title>Inbox - Messages - user1 - "),
        "{html}"
    );
    assert!(html.contains("user-messages-page"));
    // The Messages tab, then the inbox's own navigation.
    assert!(html.contains(
        "<li aria-current=\"page\" class=\"user-nav__personal-messages\"><a class=\"active\" href=\"/u/user1/messages\">"
    ));
    assert!(html.contains(
        "<li aria-current=\"location\" class=\"user-nav__messages-latest\"><a class=\"active\" href=\"/u/user1/messages\">"
    ));
    assert!(html.contains("class=\"user-nav__messages-new\""));
    assert!(html.contains("data-identifier=\"messages-dropdown\""));
    assert!(html.contains("id=\"new-private-message-btn\""));
    // user1's messages: to user0, and from admin (user1's archived one
    // stays in the archive).
    assert!(html.contains("data-topic-id=\"42\""));
    assert!(html.contains("data-topic-id=\"43\""));
    assert!(!html.contains("data-topic-id=\"44\""));
    let archive = user1.page("/u/user1/messages/archive").await;
    assert!(archive.contains("data-topic-id=\"44\""));
    assert!(archive.contains("<title>Archive - Messages - user1 - "));
}

#[tokio::test]
async fn someone_elses_inbox_is_access_denied() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/u/user2/messages").await;
    assert!(html.contains("<div class=\"error-page\"><div class=\"face\">:(</div><div class=\"reason\">Access Denied</div>"), "{html}");
    assert!(html.contains("href=\"/topics/private-messages/user2.json\""));
    assert!(!html.contains("user-messages-page"));
    // An inbox of a group one is not in is not found.
    let html = user1.page("/u/user1/messages/group/nope").await;
    assert!(html.contains("page-not-found"), "{html}");
}

#[tokio::test]
async fn a_group_inbox_lists_the_groups_messages() {
    let db = TestDb::new().await;
    for sql in [
        "INSERT INTO topics (id, title, fancy_title, slug, user_id, last_post_user_id, archetype, subtype, created_at, updated_at, bumped_at, posts_count, highest_post_number, visible) VALUES (90, 'A group message the moderators received', 'A group message the moderators received', 'a-group-message-the-moderators-received', 3, 3, 'private_message', 'user_to_user', now() - interval '1 hour', now() - interval '1 hour', now() - interval '1 hour', 1, 1, true)",
        "INSERT INTO posts (id, topic_id, user_id, post_number, raw, cooked, created_at, updated_at, last_version_at, sort_order) VALUES (900, 90, 3, 1, 'The first post.', '<p>The first post.</p>', now() - interval '1 hour', now() - interval '1 hour', now() - interval '1 hour', 1)",
        "INSERT INTO topic_allowed_users (topic_id, user_id, created_at, updated_at) VALUES (90, 3, now(), now())",
        "INSERT INTO topic_allowed_groups (topic_id, group_id) VALUES (90, 2)",
        "UPDATE groups SET has_messages = true WHERE id = 2",
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let html = admin.page("/u/admin/messages/group/moderators").await;
    assert!(
        html.contains("<title>Moderators - Messages - admin - "),
        "{html}"
    );
    assert!(html.contains("<div class=\"group-messages group-moderators\">"));
    assert!(html.contains("data-topic-id=\"90\""));
    assert!(html.contains("group-notifications-tracking"));
    assert!(
        html.contains("<li aria-current=\"location\" class=\"user-nav__messages-group-latest\">")
    );
    // The dropdown shows the group, its inbox a row of the menu.
    assert!(html.contains("<span class=\"d-button-label\">moderators</span>"));
    assert!(html.contains("data-href=\"/u/admin/messages/group/moderators\""));
}

/// topic-map/private-message-map.gjs's entry for one of a message's users.
fn pm_map_user(id: i32, username: &str, name: &str, letter_url: &str, remove: &str) -> String {
    format!(
        "<div class=\"user btn-default\" data-id=\"{id}\"><a class=\"user-link trigger-user-card\" title=\"{username}\" aria-label=\"{username}&#x27;s profile\" data-user-card=\"{username}\" href=\"/u/{username}\" tabindex=\"0\"><img alt=\"\" width=\"24\" height=\"24\" src=\"{letter_url}\" class=\"avatar\" title=\"{name}\"><span class=\"username\">{username}</span></a>{remove}</div>"
    )
}

fn remove_button(data: &str) -> String {
    format!(
        "<button class=\"btn no-text btn-icon btn-transparent remove-invited btn-small\"{data} type=\"button\"><svg class=\"fa d-icon d-icon-xmark svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#xmark\"></use></svg><span aria-hidden=\"true\">&#8203;</span></button>"
    )
}

#[tokio::test]
async fn a_message_draws_its_own_topic_chrome() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/t/parity-fixture-pm-user1-to-user0/42").await;
    // topic.gjs: the archetype's body class and DiscourseTopic's class.
    let body = &html[html.find("<body").unwrap()..];
    let body = &body[..body.find('>').unwrap()];
    assert!(body.contains(" archetype-private_message\""), "{body}");
    assert!(html.contains("<div class=\"private_message\">"));
    // The title's envelope, to the message's inbox for who can send them.
    assert!(html.contains(
        "<h1 data-topic-id=\"42\"><a aria-label=\"Inbox\" href=\"/u/user1/messages\" title=\"This topic is a personal message\"><span class=\"private-message-glyph-wrapper\"><svg class=\"fa d-icon d-icon-envelope svg-icon fa-width-auto private-message-glyph svg-string\""
    ), "{html}");
    // The first post's topic map lists the participants: user1 may remove
    // anyone (the message is theirs) and themself, and invite.
    let map_start = html
        .find("<section class=\"topic-map__private-message-map\"")
        .expect("the participants");
    let map = &html[map_start..];
    let map = &map[..map.find("</section>").unwrap()];
    assert!(map.starts_with(
        "<section class=\"topic-map__private-message-map\" data-topic-url=\"/t/42\" data-messages-url=\"/u/user1/messages\" data-remove-self-confirm=\"Are you sure you want to remove yourself from this message? You will no longer be able to see or reply to it.\"><div class=\"participants\">"
    ), "{map}");
    let user0 = pm_map_user(
        2,
        "user0",
        "User0",
        "/letter_avatar_proxy/v4/letter/u/94ad74/24.png",
        &remove_button(" data-remove-user=\"user0\""),
    );
    let me = pm_map_user(
        3,
        "user1",
        "User1",
        "/letter_avatar_proxy/v4/letter/u/5daacb/24.png",
        &remove_button(" data-remove-user=\"user1\" data-remove-self"),
    );
    // In the order the message's users were added.
    assert!(map.contains(&format!("{user0}{me}")), "{map}");
    assert!(map.ends_with(
        "<button class=\"btn no-text btn-icon btn-default btn-small add-participant-btn\" type=\"button\"><svg class=\"fa d-icon d-icon-plus svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#plus\"></use></svg><span aria-hidden=\"true\">&#8203;</span></button></div>"
    ), "{map}");
    // Removing oneself asks first (dialog.js).
    assert!(html.contains("<script type=\"application/json\" id=\"dialog-strings\">"));
    // The footer: share and bookmark without labels, then Archive.
    assert!(html.contains(
        "<button class=\"btn no-text btn-icon btn-default topic-footer-button share-and-invite\" data-share-url=\"/t/parity-fixture-pm-user1-to-user0/42\" id=\"topic-footer-button-share-and-invite\" title=\"share a link to this topic\" type=\"button\">"
    ), "{html}");
    assert!(html.contains(
        "<button class=\"btn no-text btn-icon fk-d-menu__trigger topic-bookmarks-menu-trigger bookmark widget-button bookmark-menu__trigger btn-icon no-text btn-default topic-footer-button bookmarked --post-bookmarked-only\" title=\"You&#x27;ve bookmarked this. Parity fixture: pinned, in a PM\" aria-expanded=\"false\" data-identifier=\"topic-bookmarks-menu\" data-trigger=\"\" data-unported=\"the topic's bookmarks menu (TopicBookmarksMenu)\" type=\"button\">"
    ));
    assert!(html.contains(
        "<button aria-label=\"Archive\" class=\"btn btn-icon-text btn-default topic-footer-button standard archive-topic\" id=\"topic-footer-button-archive\" title=\"Move message to your archive\" data-archive-url=\"/t/42/archive-message\" data-inbox-url=\"/u/user1/messages\" type=\"button\"><svg class=\"fa d-icon d-icon-folder svg-icon fa-width-auto svg-string\""
    ), "{html}");
    // user1 had read the first post: the visited line follows it.
    assert!(html.contains(
        "<div class=\"small-action topic-post-visited\"><div class=\"topic-post-visited-line post-1\"><span aria-level=\"2\" class=\"topic-post-visited-message\" role=\"heading\">last visit</span></div></div>"
    ));
    let archive = html.find("id=\"topic-footer-button-archive\"").unwrap();
    let defer = html.find("id=\"topic-footer-button-defer\"").unwrap();
    assert!(archive < defer);
    assert!(!html.contains("id=\"topic-footer-button-flag\""));
}

#[tokio::test]
async fn an_archived_message_moves_back_to_the_inbox() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1
        .page("/t/parity-fixture-pm-user2-to-user1-archived/44")
        .await;
    assert!(html.contains(
        "<button aria-label=\"Move to Inbox\" class=\"btn btn-icon-text btn-default topic-footer-button standard archive-topic\" id=\"topic-footer-button-archive\" title=\"Move message back to Inbox\" data-archive-url=\"/t/44/move-to-inbox\" data-inbox-url=\"/u/user1/messages\" type=\"button\"><svg class=\"fa d-icon d-icon-envelope svg-icon fa-width-auto svg-string\""
    ), "{html}");
    // user2 started it: user1 may only remove themself.
    assert!(html.contains("<span class=\"username\">user2</span></a></div>"));
    assert!(html.contains(" data-remove-user=\"user1\" data-remove-self type=\"button\">"));
    // A related message, then the suggested ones: tabs, the first shown.
    assert!(html.contains("<div class=\"more-topics__lists\"><div aria-labelledby=\"related-messages-title\" class=\"more-topics__list\" id=\"related-messages\" role=\"complementary\"><h3 class=\"more-topics__list-title\" id=\"related-messages-title\">Related Messages</h3>"), "{html}");
    assert!(html.contains(
        "<th class=\"topic-list-data default\" scope=\"col\"><ul class=\"nav nav-pills\"><li><button class=\"btn btn-text topic-list-header-tab active\" tabindex=\"-1\" data-tab-id=\"related-messages\" title=\"Related Messages\" type=\"button\"><span class=\"d-button-label\">Related Messages</span></button></li><li><button class=\"btn btn-text topic-list-header-tab\" tabindex=\"0\" data-tab-id=\"suggested-topics\" title=\"Suggested\" type=\"button\"><span class=\"d-button-label\">Suggested</span></button></li></ul></th><th class=\"topic-list-data posters\" data-sort-order=\"posters\" scope=\"col\" aria-label=\"Posters\"><span class=\"sr-only\">Posters</span></th>"
    ));
    assert!(html.contains("<template class=\"more-topics__tab\" data-tab-id=\"suggested-topics\"><div aria-labelledby=\"suggested-topics-title\" class=\"more-topics__list\" id=\"suggested-topics\" role=\"complementary\"><h3 class=\"more-topics__list-title\" id=\"suggested-topics-title\">Suggested Messages</h3>"));
    // A message's list shows its posters.
    assert!(html.contains(
        "<td class=\"posters topic-list-data\"><a aria-label=\"admin's profile\" class=\"latest single\" data-user-card=\"admin\" href=\"/u/admin\" tabindex=\"0\">"
    ), "{html}");
    // The browse more line counts what is left in the inbox, this message
    // read: user1's reply to read in the one to user0, and admin's to
    // user1 and user2.
    assert!(html.contains("There is <a href=\"/u/user1/messages/unread\">1 unread</a>"));
    assert!(html.contains(
        "and <a href=\"/u/user1/messages/new\">1 new</a> message remaining, or browse other <a href=\"/u/user1/messages\">personal messages</a>"
    ), "{html}");
}

#[tokio::test]
async fn a_group_message_shows_its_group_and_inbox() {
    let db = TestDb::new().await;
    for sql in [
        "INSERT INTO topics (id, title, fancy_title, slug, user_id, last_post_user_id, archetype, subtype, created_at, updated_at, bumped_at, posts_count, highest_post_number, visible) VALUES (90, 'A group message the moderators received', 'A group message the moderators received', 'a-group-message-the-moderators-received', 3, 3, 'private_message', 'user_to_user', now() - interval '1 hour', now() - interval '1 hour', now() - interval '1 hour', 1, 1, true)",
        "INSERT INTO posts (id, topic_id, user_id, post_number, raw, cooked, created_at, updated_at, last_version_at, sort_order) VALUES (900, 90, 3, 1, 'The first post.', '<p>The first post.</p>', now() - interval '1 hour', now() - interval '1 hour', now() - interval '1 hour', 1)",
        "INSERT INTO topic_allowed_users (topic_id, user_id, created_at, updated_at) VALUES (90, 3, now(), now())",
        "INSERT INTO topic_allowed_groups (topic_id, group_id) VALUES (90, 2)",
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let html = admin
        .page("/t/a-group-message-the-moderators-received/90")
        .await;
    // Seen through the moderators: their inbox.
    assert!(html.contains(
        "<a aria-label=\"Inbox\" href=\"/u/admin/messages/group/moderators\" title=\"This topic is a personal message\">"
    ), "{html}");
    // Staff may remove the group and the user.
    assert!(html.contains(&format!(
        "<div class=\"participants\"><div class=\"user group btn-default\" data-id=\"2\"><a class=\"group-link\" href=\"/g/moderators\"><svg class=\"fa d-icon d-icon-users svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#users\"></use></svg><span class=\"group-name\">moderators</span></a>{}</div>",
        remove_button(" data-remove-group=\"moderators\"")
    )), "{html}");
    assert!(html.contains(&remove_button(" data-remove-user=\"user1\"")));
    assert!(html.contains("data-inbox-url=\"/u/admin/messages\""));
}

#[tokio::test]
async fn a_regular_topic_keeps_its_chrome() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/t/parity-fixture-liked-and-archived/41").await;
    let body = &html[html.find("<body").unwrap()..];
    let body = &body[..body.find('>').unwrap()];
    // AddCategoryTagClasses, then the archetype.
    assert!(
        body.contains(" category category-general tag-guide tag-howto archetype-regular\""),
        "{body}"
    );
    assert!(html.contains("<div class=\"regular\">"));
    assert!(html.contains("<h1 data-topic-id=\"41\"><span class=\"topic-statuses\">"));
    assert!(!html.contains("topic-map__private-message-map"));
    assert!(!html.contains("id=\"topic-footer-button-archive\""));
    // dialog.js comes with every topic page while polls are on (a vote's
    // errors open its dialog), not only a message's.
    assert!(html.contains("id=\"dialog-strings\""));
    assert!(html.contains(
        "<button class=\"btn btn-icon-text btn-default topic-footer-button share-and-invite\" aria-label=\"Share\""
    ));
}

#[tokio::test]
async fn a_warning_has_no_envelope() {
    let db = TestDb::new().await;
    sqlx::query("UPDATE topics SET subtype = 'moderator_warning' WHERE id = 42")
        .execute(&db.pool)
        .await
        .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let json = user1.send(Method::GET, "/t/42.json", "").await;
    let json: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(json["is_warning"], true);
    let html = user1.page("/t/parity-fixture-pm-user1-to-user0/42").await;
    assert!(html.contains("<h1 data-topic-id=\"42\"><span class=\"topic-statuses\">"));
    assert!(!html.contains("private-message-glyph"));
    // A topic is never one.
    let json = user1.send(Method::GET, "/t/41.json", "").await;
    let json: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(json.get("is_warning").is_none());
}

#[tokio::test]
async fn a_messages_docked_header_shows_its_envelope_and_participants() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1
        .page("/t/parity-fixture-pm-admin-to-user1-and-user2/43")
        .await;
    let start = html
        .find("<template id=\"header-topic-info\">")
        .expect("the docked header's template");
    let template = &html[start..];
    let template = &template[..template.find("</template>").unwrap()];
    // Header::Topic::Info: the envelope to the viewer's inbox, the title,
    // no category, and the participants in the order they were added,
    // each opening their user card (header-topic-info.js).
    assert!(template.contains(
        "<h1 class=\"header-title\"><a aria-label=\"Inbox\" class=\"private-message-glyph-wrapper\" href=\"/u/user1/messages\"><svg class=\"fa d-icon d-icon-envelope svg-icon fa-width-auto private-message-glyph svg-string\""
    ), "{template}");
    assert!(template.contains(
        "</svg></a><span class=\"topic-statuses\"></span><a class=\"topic-link\" data-topic-id=\"43\" href=\"/t/parity-fixture-pm-admin-to-user1-and-user2/43\"><span>Parity fixture: PM admin to user1 and user2</span></a>"
    ), "{template}");
    assert!(!template.contains("categories-wrapper"), "{template}");
    let user = |name: &str, letter: &str| {
        format!(
            "<span class=\"trigger-user-card\"><a class=\"icon\" data-auto-route=\"true\" href=\"/u/{name}\" title=\"{name}\"><img alt=\"\" width=\"24\" height=\"24\" src=\"{letter}\" class=\"avatar\" title=\"{name}\"></a></span>"
        )
    };
    assert!(template.contains(&format!(
        "<div class=\"topic-header-extra\"><div class=\"topic-header-participants\">{}{}{}</div></div>",
        user("user1", "/letter_avatar_proxy/v4/letter/u/5daacb/24.png"),
        user("user2", "/letter_avatar_proxy/v4/letter/u/ecb155/24.png"),
        user("admin", "/letter_avatar_proxy/v4/letter/a/3ec8ea/24.png"),
    )), "{template}");
}

/// The page's #add-pm-participants-config, parsed.
fn invite_config(html: &str) -> Option<serde_json::Value> {
    let open = "<script type=\"application/json\" id=\"add-pm-participants-config\">";
    let start = html.find(open)? + open.len();
    let json = &html[start..start + html[start..].find("</script>")?];
    Some(serde_json::from_str(json).unwrap())
}

// The add participant button's AddPmParticipants modal: a member who may
// invite to the message gets its config and script (the instructions for
// an email or a username, InvitePanel's strings); a regular topic and a
// visitor get neither.
#[tokio::test]
async fn a_message_carries_its_add_participants_modal() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let html = user1.page("/t/parity-fixture-pm-user1-to-user0/42").await;
    assert!(html.contains("add-participant-btn"));
    let config = invite_config(&html).expect("the modal's config");
    assert_eq!(config["topicId"], 42);
    assert_eq!(config["currentUsername"], "user1");
    assert_eq!(config["canInviteViaEmail"], true);
    assert_eq!(config["approvalNotice"], false);
    assert_eq!(config["strings"]["title"], "Invite");
    assert_eq!(
        config["strings"]["instructions"],
        "Invitee's Email or Username"
    );
    assert_eq!(
        config["strings"]["placeholder"],
        "email address or username"
    );
    assert_eq!(config["strings"]["send"], "Send Invite");
    assert_eq!(
        config["strings"]["customMessage"],
        "Make your invite a little bit more personal by writing a <a href>custom message</a>."
    );
    assert!(html.contains("/assets/add-pm-participants.js?v="));

    let html = user1.page("/t/parity-fixture-replies-and-posters/35").await;
    assert!(invite_config(&html).is_none());
    assert!(!html.contains("add-pm-participants.js"));
}

/// The member's composer as the page draws it, up to its script.
fn composer(html: &str) -> &str {
    let start = html
        .find("<div id=\"reply-control\"")
        .expect("the composer");
    &html[start..start + html[start..].find("<script").unwrap()]
}

// The message composer (the composer's privateMessage action): its
// recipients chooser (PmUserSelector's EmailGroupUserChooser, closed and
// empty), the strings composer.js swaps in for the mode, and the chooser's
// script ahead of the composer's. Staff get the official warning toggle.
#[tokio::test]
async fn the_composer_carries_the_message_mode() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let html = user1.page("/latest").await;
    let c = composer(&html);
    assert!(c.contains(
        "<div class=\"composer-fields\"><div class=\"user-selector\" hidden><div class=\"users-input\"><details id=\"private-message-users\" class=\"select-kit multi-select user-chooser email-group-user-chooser\" data-options=\""
    ), "{c}");
    assert!(c.contains(
        "<summary data-name=\"\" data-value=\"\" tabindex=\"0\" id=\"private-message-users-header\" class=\"select-kit-header multi-select-header\"><div class=\"select-kit-header-wrapper\"><span class=\"formatted-selection\">Add users or groups</span><svg class=\"fa d-icon d-icon-plus svg-icon fa-width-auto angle-icon svg-string\""
    ));
    assert!(c.contains(
        "<div id=\"private-message-users-body\" class=\"select-kit-body\"></div></details></div></div><div class=\"title-and-category\" hidden>"
    ));
    // can_send_private_email_messages: send_email_messages_allowed_groups
    // is staff.
    assert!(c.contains("&quot;allowEmails&quot;:false"));
    assert!(c.contains("&quot;deleteItem&quot;:&quot;Delete %{name}&quot;"));
    assert!(!c.contains("add-warning"));
    assert!(c.contains(" data-username=\"user1\""));
    assert!(c.contains(
        " data-label-title-message=\"What is this discussion about in one brief sentence?\""
    ));
    assert!(c.contains(" data-label-reference-title=\"RE: %{title}\""));
    // The action title and save button: Create message and Send message,
    // with the envelope; a reply in a message names it.
    assert!(c.contains(
        "data-action-private-message=\"&lt;svg class&#x3D;&quot;fa d-icon d-icon-envelope svg-icon fa-width-auto svg-string&quot;"
    ));
    assert!(c.contains("Create message&lt;/span&gt;\""));
    assert!(c.contains("Send message&lt;/span&gt;\""));
    assert!(c.contains("Message&lt;/span&gt;\" data-label-title-message"));
    let chooser_js = html
        .find("/assets/email-group-user-chooser.js?v=")
        .expect("the chooser's script");
    assert!(chooser_js < html.find("/assets/composer.js?v=").unwrap());

    let mut admin = Client::logged_in(st, "admin").await;
    let html = admin.page("/latest").await;
    assert!(composer(&html).contains("&quot;allowEmails&quot;:true"));
    assert!(composer(&html).contains(
        "</details></div><label class=\"add-warning\" hidden><input class=\"ember-checkbox\" type=\"checkbox\"><span>This is an official warning.</span></label></div>"
    ));
}

// What composer.js sends for a message started from a profile: the draft
// it saved under a new_private_message key, then the post with its title,
// archetype and target_recipients. The message is created to its
// recipient, the draft goes, and the recipient's inbox lists it.
#[tokio::test]
async fn a_message_from_the_composer_reaches_its_recipient() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let draft = user1
        .send(
            Method::POST,
            "/drafts.json",
            "draft_key=new_private_message_1792832483305&sequence=0&data=%7B%22reply%22%3A%22Hello%22%2C%22action%22%3A%22privateMessage%22%2C%22title%22%3A%22A%20message%20from%20the%20composer%22%2C%22recipients%22%3A%22user0%22%7D",
        )
        .await;
    assert!(draft.contains("\"draft_sequence\""), "{draft}");
    let body = user1
        .send(
            Method::POST,
            "/posts",
            "raw=Hello%20there%2C%20this%20is%20a%20message%20from%20the%20composer.&draft_key=new_private_message_1792832483305&title=A%20message%20from%20the%20composer&archetype=private_message&target_recipients=user0",
        )
        .await;
    let post: serde_json::Value = serde_json::from_str(&body).unwrap();
    let topic_id = post["topic_id"].as_i64().expect("the message's topic");
    assert_eq!(post["topic_slug"], "a-message-from-the-composer");
    let (archetype, allowed): (String, Vec<String>) = sqlx::query_as(
        "SELECT t.archetype, ARRAY(SELECT u.username FROM topic_allowed_users tau JOIN users u ON u.id = tau.user_id WHERE tau.topic_id = t.id ORDER BY u.username) FROM topics t WHERE t.id = $1",
    )
    .bind(topic_id as i32)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(archetype, "private_message");
    assert_eq!(allowed, ["user0", "user1"]);
    let drafts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM drafts WHERE draft_key = 'new_private_message_1792832483305'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(drafts, 0);
    let mut user0 = Client::logged_in(st, "user0").await;
    let inbox = user0.page("/u/user0/messages").await;
    assert!(inbox.contains(&format!("data-topic-id=\"{topic_id}\"")));
}

// The profile's Message button that opens the message composer
// (composePrivateMessage), and none on one's own profile.
#[tokio::test]
async fn a_profile_offers_to_message_its_user() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.page("/u/user0/summary").await;
    assert!(html.contains(
        "<li><button class=\"btn btn-icon-text btn-primary compose-pm\" type=\"button\"><svg class=\"fa d-icon d-icon-envelope svg-icon fa-width-auto svg-string\""
    ));
    let own = user1.page("/u/user1/summary").await;
    assert!(!own.contains("compose-pm"));
}
