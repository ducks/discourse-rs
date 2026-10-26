//! An installed plugin (plugin_api) on every hook: its settings and
//! translations, its routes, its jobs, its keys on the current user and
//! the topic view, and its outlet above a topic's posts; all of them off
//! while its enabled setting is. Each test installs the plugin first, as a
//! plugin's binary does before it serves (nextest runs a test per process).

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode, header};
use axum::routing::get;
use common::{TestDb, config, set_setting, state};
use discourse_rs::config::RailsEnv;
use discourse_rs::guardian::Guardian;
use discourse_rs::plugin_api::{Ctx, Outlet, Plugin};
use discourse_rs::site_settings::SiteSettings;
use discourse_rs::{AppError, AppState};
use http_body_util::BodyExt;
use serde_json::{Map, Value, json};
use tower::ServiceExt;

const STRING: i32 = 1;
const BOOL: i32 = 5;

static SCHEDULED_RUNS: AtomicUsize = AtomicUsize::new(0);
static JOB_ARGS: Mutex<Vec<Value>> = Mutex::new(Vec::new());

struct Echo;

const SETTINGS: &str = "\
plugin: echo
enabled_site_setting: echo_enabled
settings:
  plugins:
    echo_enabled:
      default: false
      client: true
    echo_greeting:
      default: hi
";

const SERVER_EN: &str = "\
en:
  site_settings:
    echo_enabled: \"Enable the echo plugin\"
  echo:
    hello: \"Hello from echo\"
";

async fn echo_json(State(state): State<AppState>) -> Result<axum::Json<Value>, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    Ok(axum::Json(json!({
        "greeting": settings.get("echo_greeting")?.to_s(),
        "hello": state.i18n.t("echo.hello"),
    })))
}

#[async_trait]
impl Plugin for Echo {
    fn name(&self) -> &'static str {
        "echo"
    }
    fn enabled_setting(&self) -> Option<&'static str> {
        Some("echo_enabled")
    }
    fn settings_yml(&self) -> Option<&'static str> {
        Some(SETTINGS)
    }
    fn locale_ymls(&self) -> &[&'static str] {
        &[SERVER_EN]
    }
    fn stylesheet_url(&self, base_path: &str) -> Option<String> {
        Some(format!("{base_path}/echo.css"))
    }
    fn svg_icons(&self) -> &[&'static str] {
        &["record-vinyl"]
    }
    fn routes(&self) -> Router<AppState> {
        Router::new().route("/echo.json", get(echo_json))
    }
    fn jobs(&self) -> &[&'static str] {
        &["echo_job"]
    }
    fn scheduled(&self) -> Vec<(&'static str, Duration)> {
        vec![("echo_scheduled", Duration::from_secs(3600))]
    }
    async fn run_job(&self, _state: &AppState, name: &str, args: &Value) -> Result<(), AppError> {
        match name {
            "echo_scheduled" => {
                SCHEDULED_RUNS.fetch_add(1, Ordering::SeqCst);
            }
            _ => JOB_ARGS.lock().unwrap().push(args.clone()),
        }
        Ok(())
    }
    async fn current_user_fields(
        &self,
        _ctx: &mut Ctx<'_>,
        guardian: &Guardian,
    ) -> Result<Map<String, Value>, AppError> {
        Ok(Map::from_iter([(
            "echo_user".into(),
            json!(guardian.user_id()),
        )]))
    }
    async fn topic_view_fields(
        &self,
        ctx: &mut Ctx<'_>,
        _guardian: &Guardian,
        topic_id: i32,
    ) -> Result<Map<String, Value>, AppError> {
        let title: String = sqlx::query_scalar("SELECT title FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(&mut *ctx.conn)
            .await?;
        Ok(Map::from_iter([(
            "echo_topic".into(),
            json!({"id": topic_id, "title": title}),
        )]))
    }
    async fn outlet_html(
        &self,
        _ctx: &mut Ctx<'_>,
        outlet: &Outlet<'_>,
    ) -> Result<Option<String>, AppError> {
        let Outlet::TopicAbovePosts { view } = outlet;
        Ok(Some(format!(
            "<div class=\"echo-card\">{}</div>",
            view["echo_topic"]["id"]
        )))
    }
}

fn install() {
    discourse_rs::plugin_api::install(vec![Arc::new(Echo)]).unwrap();
}

async fn send(st: &AppState, path: &str, cookie: Option<&str>) -> (StatusCode, String) {
    let mut request = Request::get(path).header(header::HOST, "test.localhost");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    if path.ends_with(".json") {
        request = request.header("x-requested-with", "XMLHttpRequest");
    }
    let response = discourse_rs::app(st.clone())
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
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

async fn topic_json(st: &AppState) -> Value {
    let (status, body) = send(st, "/t/1.json", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_str(&body).unwrap()
}

#[tokio::test]
async fn a_disabled_plugin_declares_its_settings_and_does_nothing_else() {
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

    assert_eq!(send(&st, "/echo.json", None).await.0, StatusCode::NOT_FOUND);
    let cookie = login_cookie(&st, "user0").await;
    assert!(current_user(&st, &cookie).await.get("echo_user").is_none());
    assert!(topic_json(&st).await.get("echo_topic").is_none());
    let (_, page) = send(&st, "/t/about-the-site-feedback-category/1", None).await;
    assert!(!page.contains("echo-card"), "{page}");
    assert!(
        !page.contains("data-target=\"echo\""),
        "no stylesheet while disabled"
    );
}

#[tokio::test]
async fn an_enabled_plugin_serves_its_routes_and_adds_its_keys_and_outlet() {
    install();
    let db = TestDb::new().await;
    set_setting(&db.pool, "echo_enabled", BOOL, "t").await;
    set_setting(&db.pool, "echo_greeting", STRING, "bonjour").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;

    let (status, body) = send(&st, "/echo.json", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({"greeting": "bonjour", "hello": "Hello from echo"})
    );

    let cookie = login_cookie(&st, "user0").await;
    let user = current_user(&st, &cookie).await;
    assert_eq!(user["echo_user"], user["id"]);

    let topic = topic_json(&st).await;
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
            r#"<link href="/echo.css" media="all" rel="stylesheet" data-target="echo"/>"#
        ),
        "the stylesheet on core's page"
    );
    // The icon is in the sprite, enabled or not (register_svg_icon).
    let start = page.find("/svg-sprite/").expect("the sprite script");
    let path = &page[start..start + page[start..].find('"').unwrap()];
    let (_, sprite) = send(&st, path, None).await;
    assert!(
        sprite.contains(r#"id=\"record-vinyl\""#),
        "the plugin's icon in the sprite"
    );
}

#[tokio::test]
async fn the_worker_runs_a_plugins_jobs() {
    install();
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut conn = db.pool.acquire().await.unwrap();
    discourse_rs::jobs::enqueue(&mut conn, "echo_job", json!({"n": 1}))
        .await
        .unwrap();
    // A regular job runs whether the plugin is on or not: the job decides,
    // as a Rails job checks the setting itself.
    assert_eq!(discourse_rs::jobs::drain(&st).await.unwrap(), 1);
    assert_eq!(*JOB_ARGS.lock().unwrap(), vec![json!({"n": 1})]);

    // Scheduled ones only while it is on.
    let run_worker = |st: AppState| async move {
        let stop = tokio::time::sleep(Duration::from_millis(1500));
        discourse_rs::jobs::work(st, stop, Duration::ZERO).await;
    };
    run_worker(st.clone()).await;
    assert_eq!(SCHEDULED_RUNS.load(Ordering::SeqCst), 0);
    set_setting(&db.pool, "echo_enabled", BOOL, "t").await;
    run_worker(st).await;
    assert_eq!(SCHEDULED_RUNS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn plugins_install_once() {
    install();
    assert!(discourse_rs::plugin_api::install(Vec::new()).is_err());
    assert_eq!(discourse_rs::plugin_api::registered().len(), 1);
}

/// A plugin page's head: core's icons, color schemes and stylesheet, the
/// default theme's stylesheets and head_tag, and its body_tag.
#[tokio::test]
async fn a_plugin_page_gets_cores_head() {
    install();
    let db = TestDb::new().await;
    let theme: i32 =
        sqlx::query_scalar("SELECT value::int FROM site_settings WHERE name = 'default_theme_id'")
            .fetch_optional(&db.pool)
            .await
            .unwrap()
            .unwrap_or(-1);
    for (name, value) in [
        ("head_tag", "<meta name=\"echo-head\">"),
        ("body_tag", "<div id=\"echo-body\"></div>"),
    ] {
        sqlx::query(
            "INSERT INTO theme_fields (theme_id, target_id, name, value, type_id, created_at, updated_at) \
             VALUES ($1, 0, $2, $3, 0, now(), now())",
        )
        .bind(theme)
        .bind(name)
        .bind(value)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut conn = db.pool.acquire().await.unwrap();
    let settings = SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap();
    let head =
        discourse_rs::plugin_api::PageHead::load(&mut conn, &st, &settings, &Guardian::anonymous())
            .await
            .unwrap();
    assert_eq!(head.lang, "en");
    assert!(head.head.contains("color_definitions"), "{}", head.head);
    assert!(head.head.contains("/assets/discourse.css"), "{}", head.head);
    assert!(
        head.head.contains("<meta name=\"echo-head\">"),
        "{}",
        head.head
    );
    assert!(head.body_tag.contains("echo-body"), "{}", head.body_tag);
}
