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
