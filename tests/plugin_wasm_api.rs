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

fn encoded(pairs: &[(&str, &str)]) -> String {
    form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

/// A signed-in session and a token its writes pass with.
async fn writer(st: &AppState) -> (String, String) {
    let cookie = login_cookie(st, "user0").await;
    let (status, token) = send(st, "/echo/token", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    (cookie, token)
}

async fn post_form(
    st: &AppState,
    path: &str,
    (cookie, token): &(String, String),
    pairs: &[(&str, &str)],
) -> (StatusCode, String) {
    let builder = Request::post(path)
        .header(header::COOKIE, cookie)
        .header("x-csrf-token", token)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    request(st, builder, &encoded(pairs)).await
}

/// One statement through the probe route, its `{"ok"}` or `{"error"}`.
async fn probe(st: &AppState, write: Option<&(String, String)>, sql: &str) -> Value {
    let (status, body) = match write {
        Some(session) => post_form(st, "/echo/probe", session, &[("sql", sql)]).await,
        None => {
            send(
                st,
                &format!("/echo/probe?{}", encoded(&[("sql", sql)])),
                None,
            )
            .await
        }
    };
    assert_eq!(status, StatusCode::OK, "{sql}: {body}");
    serde_json::from_str::<Value>(&body).unwrap()[0].clone()
}

fn denied(result: &Value, what: &str) {
    let error = result["error"]
        .as_str()
        .unwrap_or_else(|| panic!("{what}: {result}"));
    assert!(error.contains("permission denied"), "{what}: {error}");
}

/// Its migrations ran as its role, in its schema; a route writes its own
/// rows, a failed one leaves none, and a job writes too.
#[tokio::test]
async fn a_guest_migrates_its_schema_and_writes_its_own_rows() {
    install();
    let db = TestDb::new().await;
    set_setting(&db.pool, "echo_enabled", BOOL, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let session = writer(&st).await;
    let user0: i32 = sqlx::query_scalar("SELECT id FROM users WHERE username = 'user0'")
        .fetch_one(&db.pool)
        .await
        .unwrap();

    let (status, body) = post_form(&st, "/echo/echoes", &session, &[("body", "hello there")]).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, r#"{"id":1}"#));
    let (status, body) = post_form(&st, "/echo/echoes", &session, &[("body", "fail")]).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(
        json_of(&st, "/echo/echoes.json").await,
        json!([{"id": 1, "body": "hello there", "user_id": user0}]),
        "the failed route's insert rolled back"
    );

    let mut conn = db.pool.acquire().await.unwrap();
    discourse_rs::jobs::enqueue(&mut conn, "echo_job", json!({"body": "from a job"}))
        .await
        .unwrap();
    assert_eq!(discourse_rs::jobs::drain(&st).await.unwrap(), 1);
    let rows = json_of(&st, "/echo/echoes.json").await;
    assert_eq!(rows[1]["body"], "from a job", "{rows}");

    let database: String = sqlx::query_scalar("SELECT current_database()::text")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let owner: String = sqlx::query_scalar(
        "SELECT nspowner::regrole::text FROM pg_namespace WHERE nspname = 'plugin_echo'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(owner, plugin_wasm::role_name("echo", &database));
    let versions: Vec<String> =
        sqlx::query_scalar("SELECT version FROM plugin_echo.schema_migrations")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(versions, vec!["0001_echoes".to_string()]);
}

/// What the role can't do: read credentials, ungranted core columns, the
/// host's tables or another plugin's schema; write core tables or create
/// in core's schema; write under GET; get the app's role back; keep a
/// session setting; outlast its time limit.
#[tokio::test]
async fn a_guests_sql_stays_in_its_sandbox() {
    install();
    let db = TestDb::new().await;
    set_setting(&db.pool, "echo_enabled", BOOL, "t").await;
    for sql in [
        "CREATE SCHEMA plugin_other",
        "CREATE TABLE plugin_other.secrets (id int)",
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
    let app_user: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let session = writer(&st).await;

    let title = probe(&st, None, "SELECT title FROM public.topics WHERE id = 1").await;
    assert!(
        title["ok"][0]["title"].is_string(),
        "the granted columns: {title}"
    );
    for (sql, what) in [
        (
            "SELECT password_hash FROM public.user_passwords LIMIT 1",
            "credentials",
        ),
        (
            "SELECT username FROM public.users LIMIT 1",
            "an ungranted table",
        ),
        (
            "SELECT user_id FROM public.topics LIMIT 1",
            "an ungranted column",
        ),
        (
            "SELECT * FROM discourse_rs.plugin_roles",
            "the host's tables",
        ),
        (
            "SELECT * FROM plugin_other.secrets",
            "another plugin's schema",
        ),
    ] {
        denied(&probe(&st, None, sql).await, what);
    }
    for (sql, what) in [
        ("!UPDATE public.topics SET title = 'x'", "a core write"),
        ("!CREATE TABLE public.evil (id int)", "core's schema"),
        (&format!("!SET ROLE \"{app_user}\"") as &str, "SET ROLE"),
        (
            &format!("SELECT set_config('role', '{app_user}', false)") as &str,
            "set_config",
        ),
    ] {
        denied(&probe(&st, Some(&session), sql).await, what);
    }
    let read_only = probe(&st, None, "!INSERT INTO echoes (body) VALUES ('x')").await;
    assert!(
        read_only["error"]
            .as_str()
            .unwrap_or("")
            .contains("read-only"),
        "GET is read only: {read_only}"
    );
    // RESET ROLE goes back to the session's user: the plugin's own role.
    let (status, body) = post_form(
        &st,
        "/echo/probe",
        &session,
        &[
            ("sql", "!RESET ROLE"),
            ("sql", "SELECT current_user::text AS u"),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let results: Value = serde_json::from_str(&body).unwrap();
    assert!(
        results[1]["ok"][0]["u"]
            .as_str()
            .unwrap()
            .starts_with("dpl_echo_"),
        "{results}"
    );

    // A session setting doesn't outlive the hook.
    for _ in 0..3 {
        probe(&st, None, "!SET search_path = pg_catalog").await;
        let path = probe(&st, None, "SELECT current_setting('search_path') AS p").await;
        assert!(
            path.to_string().contains("plugin_echo"),
            "the role's search path again: {path}"
        );
    }

    // A query past the time limit (500 ms) stops: by statement_timeout,
    // an error the guest sees, or, where the guest lifted that, by the
    // host cancelling it, which fails the route. The plugin's connections
    // still work after.
    let started = std::time::Instant::now();
    let slept = probe(&st, None, "SELECT pg_sleep(5)").await;
    assert!(
        slept["error"]
            .as_str()
            .unwrap_or("")
            .contains("statement timeout"),
        "{slept}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    let started = std::time::Instant::now();
    let lifted = encoded(&[
        ("sql", "!SET LOCAL statement_timeout = 0"),
        ("sql", "SELECT pg_sleep(5)"),
    ]);
    let (status, _) = send(&st, &format!("/echo/probe?{lifted}"), None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    for _ in 0..5 {
        assert!(json_of(&st, "/echo/echoes.json").await.is_array());
    }
}

/// Rows of the plugin's own table with this body, read as the app.
async fn echoes(pool: &sqlx::PgPool, body: &str) -> i64 {
    let schema = plugin_wasm::schema_name("echo");
    sqlx::query_scalar(&format!(
        "SELECT count(*) FROM \"{schema}\".echoes WHERE body = $1"
    ))
    .bind(body)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A guest's COMMIT or ROLLBACK fails the hook before its next statement,
/// which would otherwise commit on its own (and write under GET); and a
/// connection goes back to the pool with nothing of the guest's session
/// on it.
#[tokio::test]
async fn a_guest_keeps_to_its_transaction_and_leaves_no_session_behind() {
    install();
    let db = TestDb::new().await;
    set_setting(&db.pool, "echo_enabled", BOOL, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let session = writer(&st).await;

    for end in ["!COMMIT", "!ROLLBACK"] {
        let after = format!("after {end}");
        let insert = format!("!INSERT INTO echoes (body) VALUES ('{after}')");
        let (status, _) = post_form(
            &st,
            "/echo/probe",
            &session,
            &[("sql", end), ("sql", &insert)],
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{end}");
        let get = encoded(&[("sql", end), ("sql", &insert)]);
        let (status, _) = send(&st, &format!("/echo/probe?{get}"), None).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{end} under GET");
        assert_eq!(echoes(&db.pool, &after).await, 0, "{end}");
    }

    // An advisory lock taken for the session (RESET ALL keeps it, DISCARD
    // ALL lets it go) is gone once the connection is back in the pool.
    let locked = probe(
        &st,
        Some(&session),
        "SELECT pg_advisory_lock(424242)::text AS l",
    )
    .await;
    assert!(locked["ok"].is_array(), "{locked}");
    let mut held = 1;
    for _ in 0..50 {
        held = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 424242",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        if held == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(held, 0, "the lock went with the session");
    for _ in 0..5 {
        assert!(json_of(&st, "/echo/echoes.json").await.is_array());
    }
}

/// A database where every role may still create in public (one upgraded
/// from PostgreSQL 14 or older) gets no plugin roles: a plugin could
/// shadow what the app's SQL resolves to.
#[tokio::test]
async fn no_plugin_database_where_everyone_creates_in_public() {
    install();
    let db = TestDb::new().await;
    sqlx::query("GRANT CREATE ON SCHEMA public TO PUBLIC")
        .execute(&db.pool)
        .await
        .unwrap();
    let error = plugin_wasm::install_databases(&db.pool)
        .await
        .expect_err("refused");
    assert!(
        error.contains("REVOKE CREATE ON SCHEMA public FROM PUBLIC"),
        "{error}"
    );
    let roles: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_roles WHERE rolname = $1")
        .bind(plugin_wasm::role_name("echo", &db.name()))
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(roles, 0, "nothing was created");
}

/// An app role that is not a superuser, as in production (it owns its
/// database; here it holds the owner's grants), and creates the plugin
/// roles with CREATEROLE. Without it, the boot says what to grant.
#[tokio::test]
async fn an_app_role_with_createrole_installs_plugin_databases() {
    install();
    let db = TestDb::new().await;
    set_setting(&db.pool, "echo_enabled", BOOL, "t").await;
    let app = app_role(&db, true).await;
    plugin_wasm::install_databases(&app).await.unwrap();
    let st = state(app.clone(), config(RailsEnv::Test, &[])).await;
    let session = writer(&st).await;
    let (status, body) = post_form(&st, "/echo/echoes", &session, &[("body", "as the app")]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(echoes(&db.pool, "as the app").await, 1);
    let title = probe(&st, None, "SELECT title FROM public.topics WHERE id = 1").await;
    assert!(title["ok"][0]["title"].is_string(), "{title}");
    denied(
        &probe(&st, None, "SELECT username FROM public.users LIMIT 1").await,
        "an ungranted table",
    );

    let db = TestDb::new().await;
    let app = app_role(&db, false).await;
    let error = plugin_wasm::install_databases(&app)
        .await
        .expect_err("refused");
    assert!(error.contains("CREATEROLE"), "{error}");
}

/// A login role holding what the database's owner would (every schema,
/// table and sequence, with grant option), with or without CREATEROLE.
/// Named for scripts/drop-stale-test-dbs' sweep of `dpl_` roles.
async fn app_role(db: &TestDb, createrole: bool) -> sqlx::PgPool {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let role = format!(
        "dpl_testapp_{}_{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let create = if createrole {
        "CREATEROLE"
    } else {
        "NOCREATEROLE"
    };
    for sql in [
        format!("CREATE ROLE \"{role}\" LOGIN NOSUPERUSER {create} PASSWORD 'test'"),
        format!("GRANT ALL ON DATABASE \"{}\" TO \"{role}\"", db.name()),
        format!(
            "DO $$ DECLARE s text; BEGIN \
               FOR s IN SELECT nspname FROM pg_namespace \
                 WHERE nspname !~ '^pg_' AND nspname <> 'information_schema' LOOP \
                 EXECUTE format('GRANT ALL ON SCHEMA %I TO %I', s, '{role}'); \
                 EXECUTE format('GRANT ALL ON ALL TABLES IN SCHEMA %I TO %I WITH GRANT OPTION', s, '{role}'); \
                 EXECUTE format('GRANT ALL ON ALL SEQUENCES IN SCHEMA %I TO %I WITH GRANT OPTION', s, '{role}'); \
               END LOOP; END $$"
        ),
    ] {
        sqlx::query(&sql).execute(&db.pool).await.unwrap();
    }
    let options = (*db.pool.connect_options())
        .clone()
        .username(&role)
        .password("test");
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap()
}

/// Two sites in one process: the plugin is the process', its database
/// each site's (a role and schema per database), and one site's rows
/// stay there.
#[tokio::test]
async fn each_site_has_its_own_plugin_database() {
    install();
    let one = TestDb::new().await;
    let two = TestDb::new().await;
    for db in [&one, &two] {
        set_setting(&db.pool, "echo_enabled", BOOL, "t").await;
    }
    let st_one = state(one.pool.clone(), config(RailsEnv::Test, &[])).await;
    let st_two = state(two.pool.clone(), config(RailsEnv::Test, &[])).await;
    let session = writer(&st_one).await;
    let (status, _) = post_form(&st_one, "/echo/echoes", &session, &[("body", "site one")]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(echoes(&one.pool, "site one").await, 1);
    assert_eq!(json_of(&st_two, "/echo/echoes.json").await, json!([]));
    assert_ne!(
        plugin_wasm::role_name("echo", &one.name()),
        plugin_wasm::role_name("echo", &two.name())
    );
}
