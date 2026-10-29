//! discourse-microsoft-auth: its spec/integration/microsoft_auth_spec.rb
//! and spec/lib/microsoft_auth_revoker_spec.rb, against Microsoft's login
//! and Graph endpoints served by the test on loopback
//! (`Config::auth_provider_hosts`).

mod common;

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Form, State};
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

const CLIENT_ID: &str = "abcdef11223344";
const CLIENT_SECRET: &str = "adddcccdddd99922";
const ACCESS_TOKEN: &str = "microsoft_access_token_448";
const TEMP_CODE: &str = "microsoft_temp_code_544254";

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
    async fn send(&mut self, method: Method, path: &str) -> Reply {
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
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
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

    fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
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
struct Microsoft {
    /// The address /me answers with.
    email: String,
    /// Each token request's Authorization header and form.
    token_requests: Vec<(String, Vec<(String, String)>)>,
}

type Shared = Arc<Mutex<Microsoft>>;

/// The stub's token endpoint: a form encoded token, as the spec's.
async fn token(
    State(p): State<Shared>,
    headers: HeaderMap,
    Form(form): Form<Vec<(String, String)>>,
) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    p.lock().unwrap().token_requests.push((auth, form));
    let body = form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("access_token", ACCESS_TOKEN),
            ("token_type", "Bearer"),
            ("expires_in", "3599"),
            (
                "scope",
                "openid email profile https://graph.microsoft.com/User.Read",
            ),
        ])
        .finish();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/x-www-form-urlencoded")],
        body,
    )
}

async fn me(State(p): State<Shared>, headers: HeaderMap) -> (StatusCode, Json<Value>) {
    let bearer = format!("Bearer {ACCESS_TOKEN}");
    if headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        != Some(bearer.as_str())
    {
        return (StatusCode::UNAUTHORIZED, Json(json!({})));
    }
    let email = p.lock().unwrap().email.clone();
    (
        StatusCode::OK,
        Json(json!({
            "businessPhones": ["+1 425 555 0109"],
            "displayName": "Adele Vance",
            "givenName": "Adele",
            "jobTitle": "Retail Manager",
            "mail": email,
            "mobilePhone": "+1 425 555 0109",
            "officeLocation": "18/2111",
            "preferredLanguage": "en-US",
            "surname": "Vance",
            "userPrincipalName": email,
            "id": "87d349ed-44d7-43e1-9a83-5f2406dee5bd",
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

/// The spec's before block: the plugin configured and on, Microsoft's
/// hosts on the stub, /me answering with user1's email.
async fn site(db: &TestDb) -> (AppState, Shared) {
    let shared: Shared = Arc::new(Mutex::new(Microsoft {
        email: "user2@example.com".into(),
        ..Default::default()
    }));
    let app = Router::new()
        .route("/ms-login/{tenant}/oauth2/v2.0/token", post(token))
        .route("/ms-graph/v1.0/me", get(me))
        .with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut config = config(RailsEnv::Test, &[]);
    config.auth_provider_hosts = vec![
        (
            "https://login.microsoftonline.com".into(),
            format!("{base}/ms-login"),
        ),
        (
            "https://graph.microsoft.com".into(),
            format!("{base}/ms-graph"),
        ),
    ];
    let state = state(db.pool.clone(), config).await;
    set(&state, "allowed_internal_hosts", "127.0.0.1").await;
    set(&state, "auth_require_interaction", "f").await;
    set(&state, "microsoft_auth_client_id", CLIENT_ID).await;
    set(&state, "microsoft_auth_client_secret", CLIENT_SECRET).await;
    set(&state, "microsoft_auth_enabled", "t").await;
    (state, shared)
}

/// POST /auth/microsoft_office365 (GET: interaction isn't required), then
/// the callback with the authorize redirect's state and the code.
async fn log_in(browser: &mut Browser) -> (Reply, Reply) {
    let start = browser.send(Method::GET, "/auth/microsoft_office365").await;
    assert_eq!(start.status, StatusCode::FOUND, "{}", start.body);
    let back = browser
        .send(
            Method::GET,
            &format!(
                "/auth/microsoft_office365/callback?state={}&code={TEMP_CODE}",
                query_param(&start.location, "state")
            ),
        )
        .await;
    (start, back)
}

#[tokio::test]
async fn signs_in_the_user_whose_email_matches_when_emails_are_verified() {
    let db = TestDb::new().await;
    let (state, microsoft) = site(&db).await;
    set(&state, "microsoft_auth_email_verified", "t").await;
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let (start, back) = log_in(&mut browser).await;
    assert!(
        start
            .location
            .starts_with("https://login.microsoftonline.com/common/oauth2/v2.0/authorize?"),
        "{}",
        start.location
    );
    assert_eq!(query_param(&start.location, "client_id"), CLIENT_ID);
    assert_eq!(
        query_param(&start.location, "scope"),
        "openid email profile https://graph.microsoft.com/User.Read"
    );
    assert_eq!(
        query_param(&start.location, "redirect_uri"),
        "http://test.localhost/auth/microsoft_office365/callback"
    );
    assert_eq!(back.status, StatusCode::FOUND, "{}", back.body);
    assert_eq!(back.location, "http://test.localhost/");
    {
        // The token request: the credentials in a Basic header, the code,
        // the grant and the callback in the form.
        use base64::Engine;
        let m = microsoft.lock().unwrap();
        let (auth, form) = &m.token_requests[0];
        let basic = base64::engine::general_purpose::STANDARD
            .encode(format!("{CLIENT_ID}:{CLIENT_SECRET}"));
        assert_eq!(auth, &format!("Basic {basic}"));
        let get = |k: &str| form.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("code"), Some(TEMP_CODE));
        assert_eq!(get("grant_type"), Some("authorization_code"));
        assert_eq!(
            get("redirect_uri"),
            Some("http://test.localhost/auth/microsoft_office365/callback")
        );
    }
    let user1 = user_by_email(&db.pool, "user2@example.com").await;
    assert!(browser.cookie("_t").is_some());
    let logged_on: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_auth_tokens WHERE user_id = $1 AND authenticated_with_oauth)",
    )
    .bind(user1)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(logged_on);
    let (uid, info, extra): (String, Value, Value) = sqlx::query_as(
        "SELECT provider_uid, info, extra FROM user_associated_accounts \
         WHERE provider_name = 'microsoft_office365' AND user_id = $1",
    )
    .bind(user1)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(uid, "87d349ed-44d7-43e1-9a83-5f2406dee5bd");
    assert_eq!(
        info,
        json!({"name": "Adele Vance", "email": "user2@example.com"})
    );
    // Graph's keys as the SnakyHash keeps them.
    assert_eq!(extra["raw_info"]["display_name"], "Adele Vance");
    assert_eq!(
        extra["raw_info"]["user_principal_name"],
        "user2@example.com"
    );
}

#[tokio::test]
async fn does_not_sign_in_the_matching_user_when_emails_are_not_verified() {
    let db = TestDb::new().await;
    let (state, _) = site(&db).await;
    set(&state, "microsoft_auth_email_verified", "f").await;
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let (start, back) = log_in(&mut browser).await;
    assert!(
        start
            .location
            .starts_with("https://login.microsoftonline.com/common/oauth2/v2.0/authorize?")
    );
    assert_eq!(back.status, StatusCode::FOUND, "{}", back.body);
    assert_eq!(back.location, "http://test.localhost/");
    assert!(browser.cookie("_t").is_none());
    // Left to sign up instead.
    assert!(browser.cookie("authentication_data").is_some());
    let associated: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_associated_accounts WHERE provider_name = 'microsoft_office365' AND user_id IS NOT NULL",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(associated, 0);
}

#[tokio::test]
async fn a_single_tenant_uses_the_tenant_id_from_the_site_setting() {
    let db = TestDb::new().await;
    let (state, _) = site(&db).await;
    set(&state, "microsoft_auth_tenant_id", "my-tenant-id").await;
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let start = browser.send(Method::GET, "/auth/microsoft_office365").await;
    assert_eq!(start.status, StatusCode::FOUND);
    assert!(
        start
            .location
            .starts_with("https://login.microsoftonline.com/my-tenant-id/oauth2/v2.0/authorize?"),
        "{}",
        start.location
    );
}

#[tokio::test]
async fn the_login_is_off_while_the_plugin_is() {
    let db = TestDb::new().await;
    let (state, _) = site(&db).await;
    set(&state, "microsoft_auth_enabled", "f").await;
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let start = browser
        .send(Method::POST, "/auth/microsoft_office365")
        .await;
    assert_eq!(start.status, StatusCode::NOT_FOUND);
    let response = browser.send(Method::GET, "/site.json").await;
    let site: Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        !site["auth_providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "microsoft_office365")
    );
    set(&state, "microsoft_auth_enabled", "t").await;
    let response = browser.send(Method::GET, "/site.json").await;
    let site: Value = serde_json::from_str(&response.body).unwrap();
    let provider = site["auth_providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "microsoft_office365")
        .cloned()
        .expect("listed");
    assert_eq!(provider["icon_override"], "fab-microsoft");
    assert_eq!(provider["can_connect"], true);
    assert_eq!(provider["can_revoke"], true);
}

/// The revoker spec's fixtures: three users with an auth token, a user
/// API key and an API key each; Microsoft associations for the first two
/// and a Facebook one for the first.
async fn revoker_fixtures(pool: &PgPool) -> [i32; 3] {
    let mut users = [0; 3];
    for (i, email) in [
        "user0@example.com",
        "user1@example.com",
        "user2@example.com",
    ]
    .iter()
    .enumerate()
    {
        let id = user_by_email(pool, email).await;
        users[i] = id;
        sqlx::query("UPDATE users SET active = true WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO user_auth_tokens (user_id, auth_token, prev_auth_token, rotated_at, created_at, updated_at) \
             VALUES ($1, $2, $2, now(), now(), now())",
        )
        .bind(id)
        .bind(format!("token-{id}"))
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO user_api_keys (user_id, application_name, client_id, created_at, updated_at, key_hash) \
             VALUES ($1, 'app', $2, now(), now(), $2)",
        )
        .bind(id)
        .bind(format!("key-{id}"))
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO api_keys (created_by_id, created_at, updated_at, key_hash, truncated_key) \
             VALUES ($1, now(), now(), $2, 'abcd')",
        )
        .bind(id)
        .bind(format!("api-{id}"))
        .execute(pool)
        .await
        .unwrap();
    }
    for (provider, uid, user) in [
        ("microsoft_office365", "100", users[0]),
        ("microsoft_office365", "200", users[1]),
        ("facebook", "100", users[0]),
    ] {
        sqlx::query(
            "INSERT INTO user_associated_accounts (provider_name, provider_uid, user_id, info, created_at, updated_at) \
             VALUES ($1, $2, $3, '{\"email\":\"someuser@somedomain.tld\"}', now(), now())",
        )
        .bind(provider)
        .bind(uid)
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
    }
    users
}

#[tokio::test]
async fn the_revoker_logs_out_deactivates_and_revokes_microsoft_users() {
    let db = TestDb::new().await;
    let users = revoker_fixtures(&db.pool).await;
    let count = |sql: &'static str| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let accounts = count("SELECT count(*) FROM user_associated_accounts").await;
    let active = count("SELECT count(*) FROM users WHERE active").await;
    let tokens = count("SELECT count(*) FROM user_auth_tokens").await;
    let user_keys = count("SELECT count(*) FROM user_api_keys WHERE revoked_at IS NULL").await;
    let api_keys = count("SELECT count(*) FROM api_keys WHERE revoked_at IS NULL").await;

    let mut conn = db.pool.acquire().await.unwrap();
    discourse_rs::plugins::microsoft_auth::revoke(&mut conn)
        .await
        .unwrap();

    assert_eq!(
        count("SELECT count(*) FROM user_associated_accounts").await,
        accounts - 2
    );
    assert_eq!(
        count("SELECT count(*) FROM user_associated_accounts WHERE provider_name = 'microsoft_office365'")
            .await,
        0
    );
    assert_eq!(
        count("SELECT count(*) FROM users WHERE active").await,
        active - 2
    );
    assert_eq!(
        count("SELECT count(*) FROM user_auth_tokens").await,
        tokens - 2
    );
    assert_eq!(
        count("SELECT count(*) FROM user_api_keys WHERE revoked_at IS NULL").await,
        user_keys - 2
    );
    assert_eq!(
        count("SELECT count(*) FROM api_keys WHERE revoked_at IS NULL").await,
        api_keys - 2
    );
    for (user, revoked) in [(users[0], true), (users[1], true), (users[2], false)] {
        let (active, tokens, user_key_revoked, api_key_revoked): (bool, i64, bool, bool) =
            sqlx::query_as(
                "SELECT u.active, \
                   (SELECT count(*) FROM user_auth_tokens WHERE user_id = u.id), \
                   (SELECT revoked_at IS NOT NULL FROM user_api_keys WHERE user_id = u.id), \
                   (SELECT revoked_at IS NOT NULL FROM api_keys WHERE created_by_id = u.id) \
                 FROM users u WHERE u.id = $1",
            )
            .bind(user)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(active, !revoked, "user {user}");
        assert_eq!(tokens, if revoked { 0 } else { 1 }, "user {user}");
        assert_eq!(user_key_revoked, revoked, "user {user}");
        assert_eq!(api_key_revoked, revoked, "user {user}");
    }
}

#[tokio::test]
async fn log_out_users_only_deletes_their_auth_tokens() {
    let db = TestDb::new().await;
    let users = revoker_fixtures(&db.pool).await;
    let mut conn = db.pool.acquire().await.unwrap();
    discourse_rs::plugins::microsoft_auth::log_out_users(&mut conn)
        .await
        .unwrap();
    for (user, logged_out) in [(users[0], true), (users[1], true), (users[2], false)] {
        let (active, tokens): (bool, i64) = sqlx::query_as(
            "SELECT u.active, (SELECT count(*) FROM user_auth_tokens WHERE user_id = u.id) \
             FROM users u WHERE u.id = $1",
        )
        .bind(user)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert!(active);
        assert_eq!(tokens, if logged_out { 0 } else { 1 });
    }
    let microsoft: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_associated_accounts WHERE provider_name = 'microsoft_office365'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(microsoft, 2);
}
