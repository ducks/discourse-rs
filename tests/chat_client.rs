//! What chat's client-side shortcuts read off the page: the composer's
//! example texts for Ctrl/Cmd+B, I and E (modifySelection), and the
//! sidebar links Alt+Up and Alt+Down switch between in order.

mod common;

use axum::body::Body;
use axum::http::{Request, header};
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
    async fn send(&mut self, method: &str, path: &str, body: &str) -> String {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        if !self.cookies.is_empty() {
            let cookie: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookie.join("; "));
        }
        if let Some(token) = &self.csrf {
            request = request
                .header("x-csrf-token", token.as_str())
                .header("x-requested-with", "XMLHttpRequest");
        }
        if !body.is_empty() {
            request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let pair = v.to_str().unwrap().split(';').next().unwrap();
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
            csrf: Some(String::new()),
        };
        let csrf = client.send("GET", "/session/csrf.json", "").await;
        let csrf: Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
            .send(
                "POST",
                "/session",
                &format!("login={username}&password=password"),
            )
            .await;
        client.csrf = None;
        client
    }
}

#[tokio::test]
async fn the_composer_and_sidebar_carry_what_the_shortcuts_read() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let html = user1.send("GET", "/chat/c/general/2", "").await;
    // applySurround's example texts, as Rails' composer.*_text.
    assert!(
        html.contains("data-surround=\"{&quot;bold&quot;:&quot;strong text&quot;,&quot;italic&quot;:&quot;emphasized text&quot;,&quot;code&quot;:&quot;indent preformatted text by 4 spaces&quot;}\""),
        "{html}"
    );
    // The active channel and the sidebar link the switch finds it by.
    assert!(
        html.contains("class=\"chat-channel --loaded is-empty\" data-id=\"2\""),
        "{html}"
    );
    assert!(html.contains("data-section-name=\"chat-channels\""));
    assert!(html.contains("sidebar-section-link--active channel-2\" title=\"General chat\" data-link-name=\"general\" href=\"/chat/c/general/2\""), "{html}");

    // The script with the shortcuts.
    let js = user1.send("GET", "/assets/chat-drawer.js", "").await;
    assert!(
        js.contains("function switchChannel(up, unreadOnly)"),
        "{js}"
    );
}
