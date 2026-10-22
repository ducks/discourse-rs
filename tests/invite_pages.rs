//! The invite page (`/invites/:key`) and the profile's Invites tab
//! (`/u/:username/invited/...`). The JSON and the writes behind them are
//! covered by parity/writes/invites; these pin the HTML each viewer gets.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use tower::ServiceExt;

const FIXTURES: &[&str] = &[
    "INSERT INTO groups (id, name, created_at, updated_at) VALUES (9001, 'testers', now(), now())",
    "INSERT INTO group_users (group_id, user_id, owner, created_at, updated_at) VALUES (9001, 3, TRUE, now(), now())",
    "INSERT INTO invites (id, invite_key, email, invited_by_id, created_at, updated_at, emailed_status, \
         max_redemptions_allowed, redemption_count, expires_at, email_token, description) VALUES \
     (900, 'Fixture900', NULL, 3, now() - interval '3 days', now() - interval '3 days', 0, 5, 0, now() + interval '30 days', NULL, 'for the book club'), \
     (901, 'Fixture901', 'friend@example.com', 3, now() - interval '2 days', now() - interval '2 days', 4, 1, 0, now() + interval '60 days', '0123456789abcdef0123456789abcdef', NULL), \
     (902, 'Fixture902', NULL, 3, now() - interval '10 days', now() - interval '5 days', 0, 1, 1, now() + interval '80 days', NULL, NULL), \
     (903, 'Fixture903', 'old@example.com', 3, now() - interval '100 days', now() - interval '100 days', 4, 1, 0, now() - interval '10 days', 'fedcba9876543210fedcba9876543210', NULL)",
    "INSERT INTO invited_users (user_id, invite_id, redeemed_at, created_at, updated_at) VALUES (2, 902, now(), now(), now())",
    "INSERT INTO topic_invites (topic_id, invite_id, created_at, updated_at) VALUES (35, 900, now(), now())",
    "INSERT INTO invited_groups (group_id, invite_id, created_at, updated_at) VALUES (9001, 900, now(), now())",
];

async fn fixtures(db: &TestDb) {
    for sql in FIXTURES {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
}

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    fn anonymous(state: AppState) -> Client {
        Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        }
    }

    async fn send(&mut self, method: Method, path: &str, body: &str) -> (StatusCode, String) {
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
        self.keep_cookies(&response);
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn keep_cookies(&mut self, response: &axum::response::Response) {
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap();
            let pair = v.split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client::anonymous(state);
        let (_, csrf) = client.send(Method::GET, "/session/csrf.json", "").await;
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

    /// The page as a browser asks for it: the status, the Location, the
    /// body.
    async fn page(&mut self, path: &str) -> (StatusCode, String, String) {
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
        self.keep_cookies(&response);
        let status = response.status();
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            location,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }
}

#[tokio::test]
async fn an_invite_link_asks_a_visitor_for_their_email() {
    let db = TestDb::new().await;
    fixtures(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut visitor = Client::anonymous(st);
    let (status, _, html) = visitor.page("/invites/Fixture900").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<title>Invitation - "), "{html}");
    assert!(html.contains("invite-page"));
    assert!(html.contains("<div class=\"container invites-show clearfix\""));
    // The progress bar at signup, the welcome, who invited them.
    assert!(html.contains("<div class=\"signup-progress-bar__segment --active\">"));
    assert!(html.contains("<h1 class=\"login-title\">Welcome to Discourse!</h1>"));
    assert!(html.contains("<p>You were invited by:</p>"));
    assert!(html.contains("data-username=\"user1\""));
    // Login codes are on: CodeLoginForm's email field and Continue.
    assert!(html.contains("code-login-form__email-step"));
    assert!(html.contains("id=\"code-login-email\""));
    assert!(html.contains("code-login-form__continue"));
    // No log in or sign up in the header, no sidebar.
    assert!(!html.contains("login-button"));
    assert!(!html.contains("id=\"d-sidebar\""));
    assert!(html.contains("/assets/invite.js"));
}

#[tokio::test]
async fn an_email_invite_shows_the_email_and_sends_the_code() {
    let db = TestDb::new().await;
    fixtures(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut visitor = Client::anonymous(st);
    let (_, _, html) = visitor.page("/invites/Fixture901").await;
    assert!(
        html.contains("<p class=\"email-message\">Your account email address is <b>f****d@e*****e.com</b>.</p>"),
        "{html}"
    );
    assert!(html.contains("ll send a one-time code to f****d@e*****e.com"));
    assert!(html.contains("Send code"));
}

#[tokio::test]
async fn without_login_codes_the_signup_form_shows() {
    let db = TestDb::new().await;
    fixtures(&db).await;
    common::set_setting(&db.pool, "enable_local_logins_via_code", 5, "f").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut visitor = Client::anonymous(st);
    let (_, _, html) = visitor.page("/invites/Fixture900").await;
    assert!(html.contains("<form class=\"invite-signup\""), "{html}");
    assert!(html.contains("id=\"new-account-email\""));
    assert!(html.contains("id=\"new-account-username\""));
    assert!(html.contains("id=\"new-account-password\""));
    assert!(html.contains("invitation-cta__accept"));
}

#[tokio::test]
async fn a_member_accepts_with_one_button_or_is_told_why_not() {
    let db = TestDb::new().await;
    fixtures(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user2 = Client::logged_in(st, "user2").await;
    let (status, _, html) = user2.page("/invites/Fixture900").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("accept-invitation"), "{html}");
    assert!(!html.contains("signup-progress-bar"));
    let (_, _, html) = user2.page("/invites/Fixture901").await;
    let alert = html
        .split("<div class=\"alert alert-error\">")
        .nth(1)
        .unwrap_or_default();
    assert!(
        alert.contains("available for your email address."),
        "{alert}"
    );
    // Accepting: user2 joins testers and is sent to the topic.
    let (status, body) = user2
        .send(Method::PUT, "/invites/show/Fixture900.json", "timezone=UTC")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"redirect_to\":\"/t/parity-fixture-replies-and-posters/35\""));
    // Now in the invite's group, the link goes straight to the topic.
    let (status, location, _) = user2.page("/invites/Fixture900").await;
    assert_eq!(status, StatusCode::FOUND);
    assert!(location.ends_with("/t/parity-fixture-replies-and-posters/35"));
}

#[tokio::test]
async fn a_spent_invite_shows_the_error_page() {
    let db = TestDb::new().await;
    fixtures(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut visitor = Client::anonymous(st);
    let (status, _, html) = visitor.page("/invites/Fixture903").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<div class='invite-error'>"), "{html}");
    assert!(html.contains("Your invite token has expired."));
    let (_, _, html) = visitor.page("/invites/nope").await;
    assert!(html.contains("Your invite token is invalid."));
}

#[tokio::test]
async fn the_invites_tab_lists_ones_invites() {
    let db = TestDb::new().await;
    fixtures(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    // The index goes to the pending invites.
    let (status, location, _) = user1.page("/u/user1/invited").await;
    assert_eq!(status, StatusCode::FOUND);
    assert!(location.ends_with("/u/user1/invited/pending"));
    let (status, _, html) = user1.page("/u/user1/invited/pending").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<title>Pending - user1 - "), "{html}");
    assert!(html.contains("user-invites-page"));
    assert!(html.contains("<li aria-current=\"page\" class=\"user-nav__invites\">"));
    assert!(html.contains(">Pending (2)</a>"));
    assert!(html.contains(">Expired (1)</a>"));
    assert!(html.contains(">Redeemed (1)</a>"));
    assert!(html.contains("<table class=\"d-table user-invite-list\">"));
    assert!(html.contains("friend@example.com"));
    assert!(html.contains("link Fixt... (0 / 5 redeemed)"));
    assert!(html.contains("<div class=\"invite-description\">for the book club</div>"));
    assert!(html.contains("href=\"/g/testers\""));
    assert!(html.contains("href=\"/t/parity-fixture-replies-and-posters/35\""));
    assert!(html.contains("invite-button"));
    assert!(html.contains("/assets/user-invited.js"));
    let (_, _, html) = user1.page("/u/user1/invited/redeemed").await;
    assert!(html.contains(">user0</a>"), "{html}");
    // dAvatar at tiny, as Ember draws it (avatars.js scales the source).
    assert!(
        html.contains(
            r#"<img alt="" width="24" height="24" src="/letter_avatar_proxy/v4/letter/u/94ad74/24.png" class="avatar" title="user0">"#
        ),
        "{html}"
    );
    assert!(html.contains("<th class=\"d-table__header-cell\">Days Visited</th>"));
}

#[tokio::test]
async fn another_member_sees_only_the_redeemed_invites() {
    let db = TestDb::new().await;
    fixtures(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user2 = Client::logged_in(st, "user2").await;
    let (status, location, _) = user2.page("/u/user1/invited/pending").await;
    assert_eq!(status, StatusCode::FOUND);
    assert!(location.ends_with("/u/user1/invited/redeemed"));
    let (_, _, html) = user2.page("/u/user1/invited/redeemed").await;
    assert!(
        !html.contains("user-navigation-secondary\"><nav aria-label=\"User secondary - invites\"")
    );
    assert!(html.contains(">user0</a>"), "{html}");
    assert!(!html.contains("<th class=\"d-table__header-cell\">Days Visited</th>"));
    assert!(!html.contains("btn-default invite-button"));
}
