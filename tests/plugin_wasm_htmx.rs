//! A WebAssembly plugin's UI as htmx (plugins/helpful): its button in
//! each post's menu (`outlet.post_menu`), a click on it posting to the
//! guest's route, which answers with the button redrawn, and
//! `publish_post_change` redrawing the post on every other page showing
//! it, through /live, for that page's viewer.

mod common;

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, next_sse_event, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use discourse_rs::plugin_wasm::{self, Plugin, Registry};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

const BOOL: i32 = 5;
/// The public fixture topic, its second post (user1's).
const TOPIC: &str = "/t/parity-fixture-replies-and-posters/35";
const POST: i32 = 36;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn install() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = std::process::Command::new(cargo)
        .args([
            "build",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
            "-p",
            "helpful-wasm",
        ])
        .current_dir(root().join("plugins"))
        .env_remove("RUSTFLAGS")
        .status()
        .expect("cargo runs");
    assert!(status.success(), "building helpful-wasm");
    let path: PathBuf =
        root().join("plugins/target/wasm32-unknown-unknown/release/helpful_wasm.wasm");
    let plugin = Plugin::load_dir(
        plugin_wasm::runtime().unwrap(),
        &root().join("plugins/helpful"),
        &std::fs::read(path).unwrap(),
    )
    .unwrap();
    let mut registry = Registry::default();
    registry.add(plugin);
    discourse_rs::plugin_api::install(registry.api_plugins()).unwrap();
    plugin_wasm::install(registry);
}

async fn send(
    st: &AppState,
    request: axum::http::request::Builder,
    cookies: &[(String, String)],
    body: String,
) -> axum::response::Response {
    let jar: Vec<String> = cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
    discourse_rs::app(st.clone())
        .oneshot(
            request
                .header(header::HOST, "test.localhost")
                .header(header::COOKIE, jar.join("; "))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A member signed in: their cookies and the token their writes carry.
struct Member {
    cookies: Vec<(String, String)>,
    token: String,
}

impl Member {
    async fn login(st: &AppState, username: &str) -> Member {
        let mut member = Member {
            cookies: Vec::new(),
            token: String::new(),
        };
        member.token = member.csrf(st).await;
        let response = send(
            st,
            Request::post("/session")
                .header("x-requested-with", "XMLHttpRequest")
                .header("x-csrf-token", &member.token)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded"),
            &member.cookies,
            format!("login={username}&password=password"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        member.keep(&response);
        member.token = member.csrf(st).await;
        member
    }

    async fn csrf(&mut self, st: &AppState) -> String {
        let response = send(
            st,
            Request::get("/session/csrf.json").header("x-requested-with", "XMLHttpRequest"),
            &self.cookies,
            String::new(),
        )
        .await;
        self.keep(&response);
        let csrf: Value = serde_json::from_str(&text(response).await).unwrap();
        csrf["csrf"].as_str().unwrap().to_string()
    }

    fn keep(&mut self, response: &axum::response::Response) {
        for v in response.headers().get_all(header::SET_COOKIE) {
            let pair = v.to_str().unwrap().split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
    }

    async fn page(&self, st: &AppState) -> String {
        let response = send(st, Request::get(TOPIC), &self.cookies, String::new()).await;
        assert_eq!(response.status(), StatusCode::OK);
        text(response).await
    }

    /// The button's click: htmx's POST, with the token from the page's
    /// `hx-headers`.
    async fn click(&self, st: &AppState, post_id: i32) -> (StatusCode, String) {
        let response = send(
            st,
            Request::post(format!("/helpful/posts/{post_id}"))
                .header("hx-request", "true")
                .header("x-csrf-token", &self.token),
            &self.cookies,
            String::new(),
        )
        .await;
        (response.status(), text(response).await)
    }
}

async fn anonymous_page(st: &AppState) -> String {
    let response = send(st, Request::get(TOPIC), &[], String::new()).await;
    assert_eq!(response.status(), StatusCode::OK);
    text(response).await
}

/// The button the guest drew for `post_id`, from a page or fragment.
fn button(html: &str, post_id: i32) -> Option<String> {
    let marker = format!("id=\"post_{}\"", post_id - 35 + 1);
    let start = html.find(&marker)? + marker.len();
    let rest = &html[start..];
    // Up to the next post.
    let rest = &rest[..rest.find("id=\"post_").unwrap_or(rest.len())];
    let at = rest.find("<button class=\"btn btn-icon-text btn-flat post-action-menu__helpful")?;
    let end = rest[at..].find("</button>")? + at + "</button>".len();
    Some(rest[at..end].to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn each_viewer_gets_the_guests_button_in_the_post_menu() {
    install();
    let db = TestDb::new().await;
    set_setting(&db.pool, "helpful_enabled", BOOL, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;

    let page = Member::login(&st, "user0").await.page(&st).await;
    let theirs = button(&page, POST).expect("user0 can mark user1's post");
    assert!(
        theirs.contains(r#"hx-post="/helpful/posts/36""#)
            && theirs.contains(r#"hx-swap="outerHTML""#),
        "{theirs}"
    );
    assert!(theirs.contains(r##"<use href="#thumbs-up">"##), "{theirs}");
    assert!(
        page.contains("data-target=\"helpful\""),
        "the guest's stylesheet"
    );
    assert!(button(&page, 35).is_none(), "nothing on user0's own post");

    assert!(
        button(&anonymous_page(&st).await, POST).is_none(),
        "nothing for an anonymous reader while no one has marked it"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_click_redraws_the_button_and_every_other_page_showing_the_post() {
    install();
    let db = TestDb::new().await;
    set_setting(&db.pool, "helpful_enabled", BOOL, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let member = Member::login(&st, "user0").await;

    // An anonymous reader's page, listening.
    let from = st.bus.now().await.unwrap();
    let live = send(
        &st,
        Request::get(format!("/live?topic=35&posts=1-5&position={from}")),
        &[],
        String::new(),
    )
    .await;
    assert_eq!(live.status(), StatusCode::OK);
    let mut body = live.into_body();
    let mut buffer = String::new();

    let (status, fragment) = member.click(&st, POST).await;
    assert_eq!(status, StatusCode::OK, "{fragment}");
    assert!(
        fragment.starts_with(
            "<button class=\"btn btn-icon-text btn-flat post-action-menu__helpful --marked\""
        ) && fragment.contains(r#"aria-pressed="true""#)
            && fragment.contains(r#"<span class="d-button-label">1</span>"#),
        "the clicker's button, marked: {fragment}"
    );

    // The reader's page redraws the post as they see it: the count, no
    // toggle.
    let html = next_sse_event(&mut body, &mut buffer, "post").await;
    assert!(
        html.contains(r#"hx-swap-oob="outerHTML:#posts > [data-post-number='2']""#),
        "{html}"
    );
    let theirs = button(&html, POST).expect("the count for an anonymous reader");
    assert!(
        theirs.contains(" disabled ")
            && !theirs.contains("hx-post")
            && !theirs.contains("--marked")
            && theirs.contains(r#"<span class="d-button-label">1</span>"#),
        "{theirs}"
    );
    let page = anonymous_page(&st).await;
    assert_eq!(button(&page, POST).as_deref(), Some(theirs.as_str()));

    // A second click takes the mark back.
    let (status, fragment) = member.click(&st, POST).await;
    assert_eq!(status, StatusCode::OK, "{fragment}");
    assert!(
        !fragment.contains("--marked") && !fragment.contains("d-button-label"),
        "{fragment}"
    );
    let html = next_sse_event(&mut body, &mut buffer, "post").await;
    assert!(button(&html, POST).is_none(), "{html}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_guests_route_refuses_what_its_button_never_offers() {
    install();
    let db = TestDb::new().await;
    set_setting(&db.pool, "helpful_enabled", BOOL, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let member = Member::login(&st, "user1").await;
    assert_eq!(
        member.click(&st, POST).await.0,
        StatusCode::FORBIDDEN,
        "own post"
    );
    assert_eq!(member.click(&st, 999_999).await.0, StatusCode::NOT_FOUND);

    // Without the session's token the host answers before the guest runs.
    let response = send(
        &st,
        Request::post(format!("/helpful/posts/{POST}")).header("hx-request", "true"),
        &member.cookies,
        String::new(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(text(response).await, r#"["BAD CSRF"]"#);

    // Turned off, the route is gone and the menu has no button.
    set_setting(&db.pool, "helpful_enabled", BOOL, "f").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let other = Member::login(&st, "user0").await;
    assert_eq!(other.click(&st, POST).await.0, StatusCode::NOT_FOUND);
    assert!(button(&other.page(&st).await, POST).is_none());
}
