//! discourse-captcha: its spec/lib/discourse_captcha/
//! create_users_controller_patch_spec.rb (users#create gated on the
//! provider's verdict), the hCaptcha and reCAPTCHA controller and provider
//! specs, the problem check specs, and the signup form's widget
//! (spec/system/signup_with_captcha_spec.rb's server side), against the
//! providers' siteverify endpoints served by the test on loopback
//! (`Config::auth_provider_hosts`).

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::{Form, State};
use axum::http::{Method, Request, StatusCode, header};
use axum::routing::post;
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

/// A browser: keeps cookies between requests, sends the CSRF token.
struct Browser {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

struct Reply {
    status: StatusCode,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|_| panic!("JSON: {}", self.body))
    }
}

impl Browser {
    fn new(state: &AppState) -> Browser {
        Browser {
            state: state.clone(),
            cookies: Vec::new(),
            csrf: None,
        }
    }

    async fn send(&mut self, method: Method, path: &str, body: &str) -> Reply {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest");
        if let Some(csrf) = &self.csrf {
            request = request.header("x-csrf-token", csrf.as_str());
        }
        if !self.cookies.is_empty() {
            let cookie: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookie.join("; "));
        }
        if !body.is_empty() {
            request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap();
            let (pair, attrs) = v.split_once(';').unwrap_or((v, ""));
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !attrs.contains("max-age=0") && !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    async fn get(&mut self, path: &str) -> Reply {
        self.send(Method::GET, path, "").await
    }

    async fn post(&mut self, path: &str, form: &[(&str, &str)]) -> Reply {
        if self.csrf.is_none() {
            let csrf = self.get("/session/csrf.json").await.json();
            self.csrf = Some(csrf["csrf"].as_str().unwrap().to_string());
        }
        let body = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        self.send(Method::POST, path, &body).await
    }

    /// The spec's `honeypot_magic`, then POST /u.json.
    async fn sign_up(&mut self) -> Reply {
        let hp = self.get("/session/hp.json").await.json();
        let challenge: String = hp["challenge"].as_str().unwrap().chars().rev().collect();
        let value = hp["value"].as_str().unwrap().to_string();
        self.post(
            "/u.json",
            &[
                ("email", "awesomeunicorn@discourse.org"),
                ("username", "awesomeunicorn"),
                ("password", "P4ssw0rd$$"),
                ("password_confirmation", &value),
                ("challenge", &challenge),
            ],
        )
        .await
    }
}

/// What the stubbed siteverify answers, and the forms it was sent.
struct Verify {
    status: u16,
    body: String,
    forms: Vec<(String, Vec<(String, String)>)>,
}

type Shared = Arc<Mutex<Verify>>;

async fn siteverify(
    State(p): State<Shared>,
    uri: axum::http::Uri,
    Form(form): Form<Vec<(String, String)>>,
) -> (StatusCode, String) {
    let mut p = p.lock().unwrap();
    p.forms.push((uri.path().to_string(), form));
    (StatusCode::from_u16(p.status).unwrap(), p.body.clone())
}

/// A setting as SiteSetting.set stores it, typed by its definition.
async fn set(state: &AppState, name: &str, value: &str) {
    let data_type = state
        .site_setting_defs
        .get(name)
        .unwrap_or_else(|| panic!("unknown setting {name}"))
        .data_type as i32;
    common::set_setting(&state.pool, name, data_type, value).await;
}

/// The spec's before block: local logins, the plugin on with provider
/// none, both providers' keys, siteverify accepting.
async fn site(db: &TestDb) -> (AppState, Shared) {
    let shared: Shared = Arc::new(Mutex::new(Verify {
        status: 200,
        body: "{\"success\":true}".into(),
        forms: Vec::new(),
    }));
    let app = Router::new()
        .route("/hc/siteverify", post(siteverify))
        .route("/g/recaptcha/api/siteverify", post(siteverify))
        .with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut config = config(RailsEnv::Test, &[]);
    config.auth_provider_hosts = vec![
        ("https://hcaptcha.com".into(), format!("{base}/hc")),
        ("https://www.google.com".into(), format!("{base}/g")),
    ];
    let state = state(db.pool.clone(), config).await;
    set(&state, "allowed_internal_hosts", "127.0.0.1").await;
    set(&state, "enable_local_logins", "t").await;
    set(&state, "discourse_captcha_enabled", "t").await;
    set(&state, "discourse_captcha_provider", "none").await;
    set(&state, "hcaptcha_site_key", "site-key").await;
    set(&state, "hcaptcha_secret_key", "secret-key").await;
    set(&state, "recaptcha_site_key", "site-key").await;
    set(&state, "recaptcha_secret_key", "secret-key").await;
    (state, shared)
}

fn answer(verify: &Shared, status: u16, body: &str) {
    let mut v = verify.lock().unwrap();
    v.status = status;
    v.body = body.to_string();
}

async fn users(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM users WHERE username = 'awesomeunicorn'")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The token handed over, then the signup.
async fn sign_up_with(state: &AppState, provider: &str, token: &str) -> Reply {
    let mut browser = Browser::new(state);
    let kept = browser
        .post(
            &format!("/captcha/{provider}/create.json"),
            &[("token", token)],
        )
        .await;
    assert_eq!(kept.status, StatusCode::OK, "{}", kept.body);
    browser.sign_up().await
}

#[tokio::test]
async fn a_refused_hcaptcha_fails_registration() {
    let db = TestDb::new().await;
    let (state, verify) = site(&db).await;
    set(&state, "discourse_captcha_provider", "hcaptcha").await;
    answer(&verify, 200, "{\"success\":false}");
    let reply = sign_up_with(&state, "hcaptcha", "token-from-hCaptcha").await;
    assert_eq!(reply.json()["success"], false);
    assert_eq!(reply.json()["message"], "Captcha verification failed");
    assert_eq!(users(&db.pool).await, 0);
    // The form send_verification posts.
    let v = verify.lock().unwrap();
    assert_eq!(v.forms[0].0, "/hc/siteverify");
    assert_eq!(
        v.forms[0].1,
        vec![
            ("secret".to_string(), "secret-key".to_string()),
            ("response".to_string(), "token-from-hCaptcha".to_string()),
        ]
    );
}

#[tokio::test]
async fn a_refused_recaptcha_fails_registration() {
    let db = TestDb::new().await;
    let (state, verify) = site(&db).await;
    set(&state, "discourse_captcha_provider", "recaptcha").await;
    answer(&verify, 200, "{\"success\":false}");
    let reply = sign_up_with(&state, "recaptcha", "token-from-reCaptcha").await;
    assert_eq!(reply.json()["success"], false);
    assert_eq!(users(&db.pool).await, 0);
    assert_eq!(
        verify.lock().unwrap().forms[0].0,
        "/g/recaptcha/api/siteverify"
    );
}

#[tokio::test]
async fn a_missing_token_fails_registration_for_either_provider() {
    let db = TestDb::new().await;
    let (state, verify) = site(&db).await;
    for provider in ["hcaptcha", "recaptcha"] {
        set(&state, "discourse_captcha_provider", provider).await;
        let reply = Browser::new(&state).sign_up().await;
        assert_eq!(reply.json()["success"], false, "{provider}");
        assert_eq!(reply.json()["message"], "Captcha verification failed");
    }
    assert!(verify.lock().unwrap().forms.is_empty());
    assert_eq!(users(&db.pool).await, 0);
}

#[tokio::test]
async fn a_verified_hcaptcha_lets_registration_through() {
    let db = TestDb::new().await;
    let (state, _) = site(&db).await;
    set(&state, "discourse_captcha_provider", "hcaptcha").await;
    let reply = sign_up_with(&state, "hcaptcha", "token-from-hCaptcha").await;
    assert_eq!(reply.json()["success"], true, "{}", reply.body);
    assert_eq!(users(&db.pool).await, 1);
}

#[tokio::test]
async fn a_verified_recaptcha_lets_registration_through() {
    let db = TestDb::new().await;
    let (state, _) = site(&db).await;
    set(&state, "discourse_captcha_provider", "recaptcha").await;
    let reply = sign_up_with(&state, "recaptcha", "token-from-reCaptcha").await;
    assert_eq!(reply.json()["success"], true, "{}", reply.body);
}

#[tokio::test]
async fn a_login_required_site_still_takes_the_token() {
    let db = TestDb::new().await;
    let (state, _) = site(&db).await;
    set(&state, "login_required", "t").await;
    set(&state, "discourse_captcha_provider", "hcaptcha").await;
    let reply = sign_up_with(&state, "hcaptcha", "token-from-hCaptcha").await;
    assert_eq!(reply.json()["success"], true, "{}", reply.body);
}

#[tokio::test]
async fn registration_is_ungated_while_off_or_without_a_provider() {
    let db = TestDb::new().await;
    let (state, verify) = site(&db).await;
    set(&state, "discourse_captcha_provider", "hcaptcha").await;
    set(&state, "discourse_captcha_enabled", "f").await;
    let reply = Browser::new(&state).sign_up().await;
    assert_eq!(reply.json()["success"], true, "{}", reply.body);
    let db = TestDb::new().await;
    let (state, _) = site(&db).await;
    let reply = Browser::new(&state).sign_up().await;
    assert_eq!(reply.json()["success"], true, "{}", reply.body);
    assert!(verify.lock().unwrap().forms.is_empty());
}

#[tokio::test]
async fn a_provider_without_its_keys_blocks_registration() {
    let db = TestDb::new().await;
    let (state, _) = site(&db).await;
    for provider in ["hcaptcha", "recaptcha"] {
        set(&state, "discourse_captcha_provider", provider).await;
        set(&state, &format!("{provider}_site_key"), "").await;
        set(&state, &format!("{provider}_secret_key"), "").await;
        let reply = Browser::new(&state).sign_up().await;
        assert_eq!(reply.json()["success"], false);
        assert!(
            reply.json()["message"]
                .as_str()
                .unwrap()
                .contains("not properly configured"),
            "{}",
            reply.body
        );
    }
}

#[tokio::test]
async fn provider_errors_and_bad_answers_fail_registration() {
    for (provider, status, body) in [
        ("hcaptcha", 500, "Internal Server Error"),
        ("recaptcha", 503, "Service Unavailable"),
        ("hcaptcha", 200, "not valid json"),
        ("recaptcha", 200, "not valid json"),
        (
            "hcaptcha",
            200,
            "{\"error-codes\":[\"missing-input-secret\"]}",
        ),
    ] {
        let db = TestDb::new().await;
        let (state, verify) = site(&db).await;
        set(&state, "discourse_captcha_provider", provider).await;
        answer(&verify, status, body);
        let reply = sign_up_with(&state, provider, "a-token").await;
        assert_eq!(reply.json()["success"], false, "{provider} {status} {body}");
        assert_eq!(users(&db.pool).await, 0);
    }
}

#[tokio::test]
async fn a_token_is_used_once() {
    let db = TestDb::new().await;
    let (state, verify) = site(&db).await;
    set(&state, "discourse_captcha_provider", "hcaptcha").await;
    answer(&verify, 200, "{\"success\":false}");
    let mut browser = Browser::new(&state);
    browser
        .post("/captcha/hcaptcha/create.json", &[("token", "t1")])
        .await;
    assert_eq!(browser.sign_up().await.json()["success"], false);
    answer(&verify, 200, "{\"success\":true}");
    // fetch_captcha_token deleted it: nothing to verify the second time.
    assert_eq!(browser.sign_up().await.json()["success"], false);
    assert_eq!(verify.lock().unwrap().forms.len(), 1);
}

#[tokio::test]
async fn create_keeps_the_token_for_two_minutes() {
    let db = TestDb::new().await;
    let (state, _) = site(&db).await;
    for provider in ["hcaptcha", "recaptcha"] {
        set(&state, "discourse_captcha_provider", provider).await;
        let mut browser = Browser::new(&state);
        let reply = browser
            .post(
                &format!("/captcha/{provider}/create.json"),
                &[("token", "test-token")],
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.json(), json!({"success": "OK"}));
        let (value, ttl): (Value, f64) = sqlx::query_as(
            "SELECT value, extract(epoch FROM expires_at - clock_timestamp())::float8 \
             FROM discourse_rs.server_sessions WHERE key LIKE '%' || $1",
        )
        .bind(format!("{provider}_token"))
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(value, json!("test-token"));
        assert!((1.0..=120.0).contains(&ttl), "{ttl}");
        sqlx::query("DELETE FROM discourse_rs.server_sessions")
            .execute(&db.pool)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn create_wants_a_token() {
    let db = TestDb::new().await;
    let (state, _) = site(&db).await;
    for provider in ["hcaptcha", "recaptcha"] {
        set(&state, "discourse_captcha_provider", provider).await;
        let path = format!("/captcha/{provider}/create.json");
        let mut browser = Browser::new(&state);
        assert_eq!(
            browser.post(&path, &[]).await.status,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            browser.post(&path, &[("token", "")]).await.status,
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn create_is_not_found_for_another_provider_none_or_the_plugin_off() {
    let db = TestDb::new().await;
    let (state, _) = site(&db).await;
    let status = |provider: &'static str| {
        let state = state.clone();
        async move {
            Browser::new(&state)
                .post(
                    &format!("/captcha/{provider}/create.json"),
                    &[("token", "test-token")],
                )
                .await
                .status
        }
    };
    set(&state, "discourse_captcha_provider", "recaptcha").await;
    assert_eq!(status("hcaptcha").await, StatusCode::NOT_FOUND);
    set(&state, "discourse_captcha_provider", "hcaptcha").await;
    assert_eq!(status("recaptcha").await, StatusCode::NOT_FOUND);
    set(&state, "discourse_captcha_provider", "none").await;
    assert_eq!(status("hcaptcha").await, StatusCode::NOT_FOUND);
    assert_eq!(status("recaptcha").await, StatusCode::NOT_FOUND);
    set(&state, "discourse_captcha_provider", "hcaptcha").await;
    set(&state, "discourse_captcha_enabled", "f").await;
    assert_eq!(status("hcaptcha").await, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_checks_the_csrf_token() {
    let db = TestDb::new().await;
    let (state, _) = site(&db).await;
    set(&state, "discourse_captcha_provider", "hcaptcha").await;
    let mut browser = Browser::new(&state);
    let reply = browser
        .send(Method::POST, "/captcha/hcaptcha/create.json", "token=t")
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
}

/// The problem check's verdict, through the admin dashboard's run.
async fn problems(state: &AppState) -> Vec<String> {
    let mut conn = state.pool.acquire().await.unwrap();
    let settings = discourse_rs::site_settings::SiteSettings::load(
        &mut conn,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await
    .unwrap();
    let env = discourse_rs::problem_checks::Env {
        settings: &settings,
        defs: &state.site_setting_defs,
        config: &state.config,
        i18n: &state.i18n,
    };
    discourse_rs::problem_checks::run_realtime(&mut conn, &env)
        .await
        .unwrap();
    sqlx::query_scalar(
        "SELECT identifier || ':' || priority FROM admin_notices \
         WHERE identifier LIKE '%captcha%' ORDER BY id",
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap()
}

#[tokio::test]
async fn the_problem_checks_report_a_provider_without_its_keys() {
    for provider in ["hcaptcha", "recaptcha"] {
        let db = TestDb::new().await;
        let (state, _) = site(&db).await;
        set(&state, "discourse_captcha_provider", provider).await;
        let site_key = format!("{provider}_site_key");
        let secret_key = format!("{provider}_secret_key");
        // Both set: no problem.
        assert!(problems(&state).await.is_empty());
        for (site, secret) in [("", "just a string"), ("just a string", ""), ("", "")] {
            set(&state, &site_key, site).await;
            set(&state, &secret_key, secret).await;
            assert_eq!(
                problems(&state).await,
                vec![format!("{provider}_configuration:1")],
                "{provider} site {site:?} secret {secret:?}"
            );
        }
        // The plugin off: its checks are not registered, and are chill.
        set(&state, "discourse_captcha_enabled", "f").await;
        let mut conn = state.pool.acquire().await.unwrap();
        let settings = discourse_rs::site_settings::SiteSettings::load(
            &mut conn,
            &state.site_setting_defs,
            &state.config.globals,
        )
        .await
        .unwrap();
        assert!(
            discourse_rs::plugins::problem_checks(&settings)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            discourse_rs::plugins::problem_check(&settings, &format!("{provider}_configuration"))
                .unwrap(),
            Some(false)
        );
    }
}

#[tokio::test]
async fn the_problem_message_is_the_plugins() {
    let db = TestDb::new().await;
    let (state, _) = site(&db).await;
    set(&state, "discourse_captcha_provider", "hcaptcha").await;
    set(&state, "hcaptcha_site_key", "").await;
    assert_eq!(problems(&state).await.len(), 1);
    let message: String = {
        let mut conn = state.pool.acquire().await.unwrap();
        let settings = discourse_rs::site_settings::SiteSettings::load(
            &mut conn,
            &state.site_setting_defs,
            &state.config.globals,
        )
        .await
        .unwrap();
        let env = discourse_rs::problem_checks::Env {
            settings: &settings,
            defs: &state.site_setting_defs,
            config: &state.config,
            i18n: &state.i18n,
        };
        let urls = discourse_rs::url::Urls {
            config: &state.config,
            settings: &settings,
        };
        discourse_rs::problem_checks::message(&env, &urls, "hcaptcha_configuration", &json!({}))
            .unwrap()
    };
    assert_eq!(
        message,
        "There is a problem with your hCaptcha `site_key` or `secret_key` configuration"
    );
}

/// The signup page, as a browser asks for it.
async fn signup_page(state: &AppState) -> String {
    let request = Request::get("/signup")
        .header(header::HOST, "test.localhost")
        .header(header::ACCEPT, "text/html")
        .body(Body::empty())
        .unwrap();
    let response = discourse_rs::app(state.clone())
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn the_signup_form_carries_the_selected_providers_widget() {
    let db = TestDb::new().await;
    let (state, _) = site(&db).await;
    set(&state, "enable_local_logins_via_code", "f").await;
    set(&state, "discourse_captcha_provider", "hcaptcha").await;
    set(
        &state,
        "hcaptcha_site_key",
        "10000000-ffff-ffff-ffff-000000000001",
    )
    .await;
    let page = signup_page(&state).await;
    assert!(
        page.contains(
            "<div class=\"create-account-after-user-fields-outlet captcha-fields-connector\" data-provider=\"hcaptcha\" data-site-key=\"10000000-ffff-ffff-ffff-000000000001\""
        ),
        "{page}"
    );
    assert!(page.contains("discourse-captcha.js"));
    // After the form's fields, inside it.
    let connector = page.find("captcha-fields-connector").unwrap();
    assert!(page[..connector].contains("new-account-password"));
    assert!(page[connector..].contains("</form>"));
    assert!(!page.contains("data-provider=\"recaptcha\""));

    set(&state, "discourse_captcha_provider", "recaptcha").await;
    let page = signup_page(&state).await;
    assert!(page.contains("data-provider=\"recaptcha\" data-site-key=\"site-key\""));
    assert!(!page.contains("data-provider=\"hcaptcha\""));

    set(&state, "login_required", "t").await;
    let page = signup_page(&state).await;
    assert!(page.contains("captcha-fields-connector"));
}

#[tokio::test]
async fn no_widget_while_off_or_without_a_provider() {
    let db = TestDb::new().await;
    let (state, _) = site(&db).await;
    set(&state, "enable_local_logins_via_code", "f").await;
    let page = signup_page(&state).await;
    assert!(!page.contains("captcha-fields-connector"));
    assert!(!page.contains("discourse-captcha.js"));
    set(&state, "discourse_captcha_provider", "hcaptcha").await;
    set(&state, "discourse_captcha_enabled", "f").await;
    let page = signup_page(&state).await;
    assert!(!page.contains("captcha-fields-connector"));
}
