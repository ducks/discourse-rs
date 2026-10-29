//! discourse-login-with-amazon: the login against Amazon's authorize,
//! token and profile endpoints served by the test on loopback
//! (`Config::auth_provider_hosts`), and its
//! spec/system/associated_account_prefs_spec.rb on the server drawn
//! account preferences.

mod common;

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::routing::{get, post};
use axum::{Json, Router};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const CLIENT_ID: &str = "amzn1.application-oa2-client.test";
const CLIENT_SECRET: &str = "amzn-made-up-secret";
const ACCESS_TOKEN: &str = "Atza|amazon-access-token";

/// A browser: keeps cookies between requests.
struct Browser {
    state: AppState,
    cookies: Vec<(String, String)>,
}

struct Reply {
    status: StatusCode,
    location: String,
    body: String,
}

impl Browser {
    async fn send(
        &mut self,
        method: Method,
        path: &str,
        body: &str,
        headers: &[(&str, &str)],
    ) -> Reply {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        for (k, v) in headers {
            request = request.header(*k, *v);
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
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
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
            location,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    async fn get(&mut self, path: &str) -> Reply {
        self.send(Method::GET, path, "", &[]).await
    }

    fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// Logged in as `username` through POST /session.
    async fn log_in_as(&mut self, username: &str) {
        let csrf = self.get("/session/csrf.json").await;
        let csrf: Value = serde_json::from_str(&csrf.body).unwrap();
        let login = form_urlencoded::Serializer::new(String::new())
            .append_pair("login", username)
            .append_pair("password", "password")
            .append_pair("authenticity_token", csrf["csrf"].as_str().unwrap())
            .finish();
        let reply = self
            .send(
                Method::POST,
                "/session",
                &login,
                &[("x-requested-with", "XMLHttpRequest")],
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    }
}

fn query_param(url: &str, name: &str) -> String {
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or_default();
    form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

#[derive(Default)]
struct Amazon {
    /// The address the profile answers with.
    email: String,
    /// Each token request's Authorization header and form.
    token_requests: Vec<(String, Vec<(String, String)>)>,
    /// Each profile request's query.
    profile_queries: Vec<Vec<(String, String)>>,
}

type Shared = Arc<Mutex<Amazon>>;

async fn token(
    State(p): State<Shared>,
    headers: HeaderMap,
    Form(form): Form<Vec<(String, String)>>,
) -> Json<Value> {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    p.lock().unwrap().token_requests.push((auth, form));
    Json(json!({
        "access_token": ACCESS_TOKEN,
        "token_type": "bearer",
        "expires_in": 3600,
        "refresh_token": "Atzr|refresh",
    }))
}

/// Amazon's profile, its keys as Amazon spells them.
async fn profile(
    State(p): State<Shared>,
    Query(query): Query<Vec<(String, String)>>,
) -> (StatusCode, Json<Value>) {
    let mut p = p.lock().unwrap();
    p.profile_queries.push(query.clone());
    if !query
        .iter()
        .any(|(k, v)| k == "access_token" && v == ACCESS_TOKEN)
    {
        return (StatusCode::UNAUTHORIZED, Json(json!({})));
    }
    (
        StatusCode::OK,
        Json(json!({
            "Profile": {
                "CustomerId": "amzn1.account.AAAAAA",
                "Name": "Amazon Shopper",
                "PrimaryEmail": p.email,
                "PostalCode": "98101",
            },
            "ResponseStatus": "OK",
        })),
    )
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

async fn user_by_email(pool: &PgPool, email: &str) -> i32 {
    sqlx::query_scalar("SELECT user_id FROM user_emails WHERE email = $1")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The plugin configured and on, Amazon's hosts on the stub.
async fn site(db: &TestDb, email: &str) -> (AppState, Shared) {
    let shared: Shared = Arc::new(Mutex::new(Amazon {
        email: email.into(),
        ..Default::default()
    }));
    let app = Router::new()
        .route("/amzn-api/auth/o2/token", post(token))
        .route("/amzn-www/ap/user/profile", get(profile))
        .with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut config = config(RailsEnv::Test, &[]);
    config.auth_provider_hosts = vec![
        ("https://www.amazon.com".into(), format!("{base}/amzn-www")),
        ("https://api.amazon.com".into(), format!("{base}/amzn-api")),
    ];
    let state = state(db.pool.clone(), config).await;
    set(&state, "allowed_internal_hosts", "127.0.0.1").await;
    set(&state, "auth_require_interaction", "f").await;
    set(&state, "login_with_amazon_client_id", CLIENT_ID).await;
    set(&state, "login_with_amazon_client_secret", CLIENT_SECRET).await;
    set(&state, "enable_login_with_amazon", "t").await;
    (state, shared)
}

/// /auth/amazon, then the callback with the authorize redirect's state.
async fn log_in(browser: &mut Browser) -> (Reply, Reply) {
    let start = browser.get("/auth/amazon").await;
    assert_eq!(start.status, StatusCode::FOUND, "{}", start.body);
    let back = browser
        .get(&format!(
            "/auth/amazon/callback?code=amzn-code&state={}",
            query_param(&start.location, "state")
        ))
        .await;
    (start, back)
}

#[tokio::test]
async fn logs_on_the_user_whose_verified_email_matches() {
    let db = TestDb::new().await;
    let (state, amazon) = site(&db, "user2@example.com").await;
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let (start, back) = log_in(&mut browser).await;
    assert!(
        start.location.starts_with("https://www.amazon.com/ap/oa?"),
        "{}",
        start.location
    );
    assert_eq!(query_param(&start.location, "client_id"), CLIENT_ID);
    // login_with_amazon_scope's default replaces the strategy's own.
    assert_eq!(query_param(&start.location, "scope"), "profile");
    assert_eq!(
        query_param(&start.location, "redirect_uri"),
        "http://test.localhost/auth/amazon/callback"
    );
    assert_eq!(back.status, StatusCode::FOUND, "{}", back.body);
    assert_eq!(back.location, "http://test.localhost/");
    assert!(browser.cookie("_t").is_some());
    {
        use base64::Engine;
        let a = amazon.lock().unwrap();
        let (auth, form) = &a.token_requests[0];
        let basic = base64::engine::general_purpose::STANDARD
            .encode(format!("{CLIENT_ID}:{CLIENT_SECRET}"));
        assert_eq!(auth, &format!("Basic {basic}"));
        // build_access_token: the credentials in the form as well, the
        // callback without its query as the redirect.
        let get = |k: &str| form.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("code"), Some("amzn-code"));
        assert_eq!(get("grant_type"), Some("authorization_code"));
        assert_eq!(get("client_id"), Some(CLIENT_ID));
        assert_eq!(get("client_secret"), Some(CLIENT_SECRET));
        assert_eq!(
            get("redirect_uri"),
            Some("http://test.localhost/auth/amazon/callback")
        );
        assert_eq!(a.profile_queries.len(), 1);
    }
    let user = user_by_email(&db.pool, "user2@example.com").await;
    let (uid, info, extra): (String, Value, Value) = sqlx::query_as(
        "SELECT provider_uid, info, extra FROM user_associated_accounts \
         WHERE provider_name = 'amazon' AND user_id = $1",
    )
    .bind(user)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(uid, "amzn1.account.AAAAAA");
    assert_eq!(info["email"], "user2@example.com");
    assert_eq!(info["name"], "Amazon Shopper");
    assert_eq!(extra, json!({"postal_code": "98101"}));
}

#[tokio::test]
async fn the_scope_setting_is_the_one_asked_for() {
    let db = TestDb::new().await;
    let (state, _) = site(&db, "user2@example.com").await;
    set(&state, "login_with_amazon_scope", "profile postal_code").await;
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let start = browser.get("/auth/amazon").await;
    assert_eq!(query_param(&start.location, "scope"), "profile postal_code");
}

#[tokio::test]
async fn unverified_emails_leave_the_login_to_sign_up() {
    let db = TestDb::new().await;
    let (state, _) = site(&db, "user2@example.com").await;
    set(&state, "login_with_amazon_email_verified", "f").await;
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let (_, back) = log_in(&mut browser).await;
    assert_eq!(back.status, StatusCode::FOUND, "{}", back.body);
    assert!(browser.cookie("_t").is_none());
    assert!(browser.cookie("authentication_data").is_some());
}

#[tokio::test]
async fn overriding_the_email_updates_the_associated_user() {
    let db = TestDb::new().await;
    let (state, _) = site(&db, "shopper@example.com").await;
    set(&state, "login_with_amazon_overrides_email", "t").await;
    let user = user_by_email(&db.pool, "user1@example.com").await;
    sqlx::query(
        "INSERT INTO user_associated_accounts (provider_name, provider_uid, user_id, info, created_at, updated_at) \
         VALUES ('amazon', 'amzn1.account.AAAAAA', $1, '{}', now(), now())",
    )
    .bind(user)
    .execute(&db.pool)
    .await
    .unwrap();
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let (_, back) = log_in(&mut browser).await;
    assert_eq!(back.status, StatusCode::FOUND, "{}", back.body);
    assert!(browser.cookie("_t").is_some());
    let email: String =
        sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"")
            .bind(user)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(email, "shopper@example.com");
}

#[tokio::test]
async fn the_login_is_off_while_the_plugin_is() {
    let db = TestDb::new().await;
    let (state, _) = site(&db, "user2@example.com").await;
    set(&state, "enable_login_with_amazon", "f").await;
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let start = browser.send(Method::POST, "/auth/amazon", "", &[]).await;
    assert_eq!(start.status, StatusCode::NOT_FOUND);
    let site: Value = serde_json::from_str(&browser.get("/site.json").await.body).unwrap();
    assert!(
        !site["auth_providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "amazon")
    );
}

/// The account preferences page of user1, as user1 sees it.
async fn account_prefs(state: &AppState) -> String {
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    browser.log_in_as("user1").await;
    let page = browser
        .send(
            Method::GET,
            "/u/user1/preferences/account",
            "",
            &[("accept", "text/html")],
        )
        .await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    page.body
}

/// The `.associated-accounts .amazon` row, if there is one.
fn amazon_row(page: &str) -> Option<String> {
    let start = page.find("<tr class=\"amazon")?;
    let end = page[start..].find("</tr>")? + start;
    Some(page[start..end].to_string())
}

#[tokio::test]
async fn the_connect_button_shows_while_users_can_connect() {
    let db = TestDb::new().await;
    let (state, _) = site(&db, "user2@example.com").await;
    set(&state, "login_with_amazon_user_can_connect", "t").await;
    let row = amazon_row(&account_prefs(&state).await).expect("an amazon row");
    assert!(row.contains("associated-account__actions"), "{row}");
    assert!(row.contains("connect-account"), "{row}");
    assert!(row.contains("Connect"), "{row}");
}

#[tokio::test]
async fn no_row_while_users_cannot_connect() {
    let db = TestDb::new().await;
    let (state, _) = site(&db, "user2@example.com").await;
    set(&state, "login_with_amazon_user_can_connect", "f").await;
    assert_eq!(amazon_row(&account_prefs(&state).await), None);
}

async fn associate_user1(pool: &PgPool) {
    let user = user_by_email(pool, "user1@example.com").await;
    sqlx::query(
        "INSERT INTO user_associated_accounts (provider_name, provider_uid, user_id, info, created_at, updated_at) \
         VALUES ('amazon', '1234', $1, '{}', now(), now())",
    )
    .bind(user)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn the_revoke_button_shows_while_users_can_revoke() {
    let db = TestDb::new().await;
    let (state, _) = site(&db, "user2@example.com").await;
    associate_user1(&db.pool).await;
    set(&state, "login_with_amazon_user_can_revoke", "t").await;
    let row = amazon_row(&account_prefs(&state).await).expect("an amazon row");
    assert!(row.contains("revoke-account"), "{row}");
    assert!(row.contains("d-icon-trash-can"), "{row}");
}

#[tokio::test]
async fn no_revoke_button_while_users_cannot_revoke() {
    let db = TestDb::new().await;
    let (state, _) = site(&db, "user2@example.com").await;
    associate_user1(&db.pool).await;
    set(&state, "login_with_amazon_user_can_revoke", "f").await;
    // Connecting off too: an associated account still shows.
    set(&state, "login_with_amazon_user_can_connect", "f").await;
    let row = amazon_row(&account_prefs(&state).await).expect("an amazon row");
    assert!(!row.contains("<button"), "{row}");
}
