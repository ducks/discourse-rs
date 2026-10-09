//! discourse-reactions on the server-rendered pages for a member: the
//! reaction controls in the post menu, with what the browser side reads to
//! draw the picker and the users menu, and the activity Reactions tab.

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
