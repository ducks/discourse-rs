//! A WebAssembly plugin on every plugin_api hook (src/plugin_wasm/api.rs):
//! plugins/echo, tests/plugin_api.rs' echo plugin as a guest, loaded from
//! its directory (plugin.toml, settings.yml, locales/, public/) and held
//! to the same checks as the compiled-in one. Each test installs it
//! first, as cli::run does from PLUGINS_DIR (nextest runs a test per
//! process).

mod common;

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use discourse_rs::plugin_wasm::{self, Plugin, Registry};
use discourse_rs::site_settings::SiteSettings;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const STRING: i32 = 1;
const BOOL: i32 = 5;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// plugins/echo built for wasm32-unknown-unknown (cargo serializes
/// concurrent builds).
fn echo_wasm() -> Vec<u8> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = std::process::Command::new(cargo)
        .args([
            "build",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
            "-p",
            "echo-wasm",
        ])
        .current_dir(root().join("plugins"))
        .env_remove("RUSTFLAGS")
        .status()
        .expect("cargo runs");
    assert!(status.success(), "building echo-wasm");
    let path: PathBuf = root().join("plugins/target/wasm32-unknown-unknown/release/echo_wasm.wasm");
    std::fs::read(path).unwrap()
}

fn install() {
    let plugin = Plugin::load_dir(
        plugin_wasm::runtime().unwrap(),
        &root().join("plugins/echo"),
        &echo_wasm(),
    )
    .unwrap();
    let mut registry = Registry::default();
    registry.add(plugin);
    discourse_rs::plugin_api::install(registry.api_plugins()).unwrap();
    plugin_wasm::install(registry);
}

async fn request(
    st: &AppState,
    request: axum::http::request::Builder,
    body: &str,
) -> (StatusCode, String) {
    let response = discourse_rs::app(st.clone())
        .oneshot(
            request
                .header(header::HOST, "test.localhost")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn send(st: &AppState, path: &str, cookie: Option<&str>) -> (StatusCode, String) {
    let mut builder = Request::get(path);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if path.ends_with(".json") {
        builder = builder.header("x-requested-with", "XMLHttpRequest");
    }
    request(st, builder, "").await
}

/// A member's session cookies, logged in as the seed's `username`.
async fn login_cookie(st: &AppState, username: &str) -> String {
    let mut cookies: Vec<(String, String)> = Vec::new();
    let mut post = async |request: axum::http::request::Builder, body: String| {
        let jar: Vec<String> = cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let request = request
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest")
            .header(header::COOKIE, jar.join("; "));
        let response = discourse_rs::app(st.clone())
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let pair = v.to_str().unwrap().split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let csrf: Value =
        serde_json::from_str(&post(Request::get("/session/csrf.json"), String::new()).await)
            .unwrap();
    let token = csrf["csrf"].as_str().unwrap().to_string();
    post(
        Request::post("/session")
            .header("x-csrf-token", token)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded"),
        format!("login={username}&password=password"),
    )
    .await;
    drop(post);
    cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

async fn current_user(st: &AppState, cookie: &str) -> Value {
    let (status, body) = send(st, "/session/current.json", Some(cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_str::<Value>(&body).unwrap()["current_user"].clone()
}

async fn json_of(st: &AppState, path: &str) -> Value {
    let (status, body) = send(st, path, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_str(&body).unwrap()
}

#[tokio::test]
async fn a_disabled_guest_declares_its_settings_and_does_nothing_else() {
    install();
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut conn = db.pool.acquire().await.unwrap();
    let settings = SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap();
    assert!(!settings.get("echo_enabled").unwrap().truthy());
    assert_eq!(settings.get("echo_greeting").unwrap().to_s(), "hi");
    assert_eq!(st.i18n.t("echo.hello"), Some("Hello from echo"));
    assert_eq!(
        st.i18n.locale("fr").t("echo.hello"),
        Some("Bonjour de echo")
    );
    assert_eq!(
        st.i18n.locale("fr").t("site_settings.echo_enabled"),
        Some("Enable the echo plugin"),
        "English where French has none"
    );

    assert_eq!(send(&st, "/echo.json", None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        send(&st, "/plugins/echo/echo.css", None).await.0,
        StatusCode::NOT_FOUND
    );
    let cookie = login_cookie(&st, "user0").await;
    assert!(current_user(&st, &cookie).await.get("echo_user").is_none());
    assert!(json_of(&st, "/t/1.json").await.get("echo_topic").is_none());
    let (_, page) = send(&st, "/t/about-the-site-feedback-category/1", None).await;
    assert!(!page.contains("echo-card"), "{page}");
    assert!(
        !page.contains("data-target=\"echo\""),
        "no stylesheet while disabled"
    );
}

#[tokio::test]
async fn an_enabled_guest_serves_its_routes_and_adds_its_keys_and_outlet() {
    install();
    let db = TestDb::new().await;
    set_setting(&db.pool, "echo_enabled", BOOL, "t").await;
    set_setting(&db.pool, "echo_greeting", STRING, "bonjour").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;

    assert_eq!(
        json_of(&st, "/echo.json").await,
        json!({"greeting": "bonjour", "hello": "Hello from echo"})
    );

    let cookie = login_cookie(&st, "user0").await;
    let user = current_user(&st, &cookie).await;
    assert_eq!(user["echo_user"], user["id"]);

    let topic = json_of(&st, "/t/1.json").await;
    assert_eq!(topic["echo_topic"]["id"], json!(1));
    assert_eq!(topic["echo_topic"]["title"], topic["title"]);

    let (status, page) = send(&st, "/t/about-the-site-feedback-category/1", None).await;
    assert_eq!(status, StatusCode::OK);
    let card = page
        .find("<div class=\"echo-card\">1</div>")
        .expect("the outlet");
    assert!(card < page.find("class=\"posts-wrapper\"").unwrap());
    assert!(
        page.contains(
            r#"<link href="/plugins/echo/echo.css" media="all" rel="stylesheet" data-target="echo"/>"#
        ),
        "the stylesheet on core's page"
    );
    let (status, css) = send(&st, "/plugins/echo/echo.css", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(css.contains(".echo-card"), "public/ served: {css}");
    let start = page.find("/svg-sprite/").expect("the sprite script");
    let path = &page[start..start + page[start..].find('"').unwrap()];
    let (_, sprite) = send(&st, path, None).await;
    assert!(
        sprite.contains(r#"id=\"record-vinyl\""#),
        "the guest's icon in the sprite"
    );
}

#[tokio::test]
async fn the_worker_runs_a_guests_jobs() {
    install();
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut conn = db.pool.acquire().await.unwrap();
    discourse_rs::jobs::enqueue(&mut conn, "echo_job", json!({"n": 1}))
        .await
        .unwrap();
    discourse_rs::jobs::enqueue(&mut conn, "echo_job", json!({"fail": true}))
        .await
        .unwrap();
    assert_eq!(discourse_rs::jobs::drain(&st).await.unwrap(), 2);
    // The one that ran is gone; the one the guest failed waits for its
    // retry with the guest's message.
    let left: Vec<(Value, Option<String>)> =
        sqlx::query_as("SELECT args, last_error FROM discourse_rs.jobs WHERE name = 'echo_job'")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0].0, json!({"fail": true}));
    assert!(
        left[0]
            .1
            .as_deref()
            .unwrap_or("")
            .contains("echo_job told to fail"),
        "{left:?}"
    );

    let plugin = &discourse_rs::plugin_api::registered()[0];
    assert_eq!(
        plugin.scheduled(),
        vec![("echo_scheduled", std::time::Duration::from_secs(3600))]
    );
    plugin
        .run_job(&st, "echo_scheduled", &Value::Null)
        .await
        .unwrap();
}

/// A guest's write is checked against the session's token before the
/// guest runs, and a form route gets a token that passes.
#[tokio::test]
async fn a_guests_write_is_checked_against_the_sessions_token() {
    install();
    let db = TestDb::new().await;
    set_setting(&db.pool, "echo_enabled", BOOL, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let cookie = login_cookie(&st, "user0").await;
    let post = async |token: Option<&str>| {
        let mut builder = Request::post("/echo/write").header(header::COOKIE, &cookie);
        if let Some(token) = token {
            builder = builder.header("x-csrf-token", token);
        }
        request(&st, builder, "").await.0
    };
    let (status, token) = send(&st, "/echo/token", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!token.is_empty());
    assert_eq!(post(Some(&token)).await, StatusCode::OK);
    assert_eq!(post(Some("forged")).await, StatusCode::FORBIDDEN);
    assert_eq!(post(None).await, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_guests_homepage_answers_the_root_while_it_is_the_homepage() {
    install();
    let db = TestDb::new().await;
    set_setting(&db.pool, "echo_enabled", BOOL, "t").await;
    set_setting(&db.pool, "default_homepage", 7, "echo").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;

    let site = json_of(&st, "/site.json").await;
    assert_eq!(
        site["homepage_options"],
        json!([{"id": "echo", "path": "/echo", "server_side": true}])
    );
    assert_eq!(
        send(&st, "/?year=2026", None).await,
        (StatusCode::OK, "echo home year=2026".to_string())
    );
    let cookie = login_cookie(&st, "user0").await;
    assert_eq!(send(&st, "/", Some(&cookie)).await.1, "echo home ");
    assert_eq!(send(&st, "/echo", None).await.1, "echo home ");
}

#[tokio::test]
async fn an_unavailable_guest_homepage_falls_back_to_the_top_menu() {
    install();
    let db = TestDb::new().await;
    set_setting(&db.pool, "echo_enabled", BOOL, "t").await;
    set_setting(&db.pool, "default_homepage", 7, "echo").await;
    set_setting(&db.pool, "echo_greeting", STRING, "closed").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (status, body) = send(&st, "/", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("echo home"));
    assert!(body.contains("topic-list"), "the latest list");
}

/// Per-request times of the plugin's route and topic view hook
/// (docs/plugin-wasm.md): `cargo nextest run --release --test <this>
/// --run-ignored only --no-capture measure`.
#[tokio::test]
#[ignore]
async fn measure() {
    install();
    let db = TestDb::new().await;
    set_setting(&db.pool, "echo_enabled", BOOL, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    // Signed in: past the anonymous cache.
    let cookie = login_cookie(&st, "user0").await;
    let app = discourse_rs::app(st);
    for path in ["/echo.json", "/t/1.json"] {
        let mut times = Vec::new();
        for i in 0..2200 {
            let started = std::time::Instant::now();
            let response = app
                .clone()
                .oneshot(
                    Request::get(path)
                        .header(header::HOST, "test.localhost")
                        .header("x-requested-with", "XMLHttpRequest")
                        .header(header::COOKIE, &cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            response.into_body().collect().await.unwrap();
            if i >= 200 {
                times.push(started.elapsed().as_micros());
            }
        }
        times.sort();
        println!(
            "{}: {path}: p50 {} us, p90 {} us",
            env!("CARGO_CRATE_NAME"),
            times[times.len() / 2],
            times[times.len() * 9 / 10]
        );
    }
}
