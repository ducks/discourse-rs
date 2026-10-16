//! External logins end to end against a provider served by the test: an
//! OpenID Connect provider (discovery, authorize, token, userinfo) and a
//! plain OAuth2 one (token and user JSON), on loopback. The browser's side
//! is played by hand: /auth/:provider's redirect is read for its state and
//! nonce, and the provider's answer is the callback the browser would
//! follow.

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
use tower::ServiceExt;

/// A browser: keeps cookies between requests.
struct Browser {
    state: AppState,
    cookies: Vec<(String, String)>,
}

struct Reply {
    status: StatusCode,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn location(&self) -> String {
        self.headers
            .iter()
            .find(|(k, _)| k == "location")
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }
}

impl Browser {
    async fn send(&mut self, method: Method, path: &str, body: &str) -> Reply {
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
        if !body.is_empty() {
            request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        for (k, v) in &headers {
            if k != "set-cookie" {
                continue;
            }
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
            headers,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    async fn get(&mut self, path: &str) -> Reply {
        self.send(Method::GET, path, "").await
    }

    fn cookie(&self, name: &str) -> Option<String> {
        self.cookies
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    }

    /// The authentication_data cookie, decoded.
    fn authentication_data(&self) -> Value {
        let raw = self
            .cookie("authentication_data")
            .expect("authentication_data");
        let decoded = form_urlencoded::parse(format!("v={raw}").as_bytes())
            .next()
            .unwrap()
            .1
            .into_owned();
        serde_json::from_str(&decoded).unwrap()
    }
}

fn query_param(url: &str, name: &str) -> String {
    url.split_once('?')
        .and_then(|(_, q)| {
            form_urlencoded::parse(q.as_bytes())
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
        })
        .unwrap_or_default()
}

/// What the fake provider saw and answers with.
#[derive(Default)]
struct Provider {
    base: String,
    /// The nonce the authorize request carried, put in the ID token.
    nonce: String,
    token_requests: Vec<(HeaderMap, Vec<(String, String)>)>,
    userinfo_auths: Vec<String>,
    email: String,
}

type Shared = Arc<Mutex<Provider>>;

fn jwt(claims: Value) -> String {
    use base64::Engine;
    let e = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    format!(
        "{}.{}.unsigned",
        e.encode(json!({"alg": "RS256", "typ": "JWT"}).to_string()),
        e.encode(claims.to_string())
    )
}

async fn discovery(State(p): State<Shared>) -> Json<Value> {
    let base = p.lock().unwrap().base.clone();
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "userinfo_endpoint": format!("{base}/userinfo"),
        "end_session_endpoint": format!("{base}/logout"),
        "token_endpoint_auth_methods_supported": ["client_secret_basic"],
    }))
}

async fn token(
    State(p): State<Shared>,
    headers: HeaderMap,
    Form(form): Form<Vec<(String, String)>>,
) -> (StatusCode, Json<Value>) {
    let mut p = p.lock().unwrap();
    p.token_requests.push((headers, form.clone()));
    let code = form
        .iter()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.as_str());
    if code != Some("good-code") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant"})),
        );
    }
    let now = chrono::Utc::now().timestamp();
    (
        StatusCode::OK,
        Json(json!({
            "access_token": "access-1",
            "token_type": "Bearer",
            "expires_in": 3600,
            "id_token": jwt(json!({
                "iss": p.base,
                "aud": "oidc-client",
                "sub": "sub-1",
                "exp": now + 600,
                "iat": now,
                "nonce": p.nonce,
            })),
        })),
    )
}

async fn userinfo(State(p): State<Shared>, headers: HeaderMap) -> Json<Value> {
    let mut p = p.lock().unwrap();
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    p.userinfo_auths.push(auth);
    Json(json!({
        "sub": "sub-1",
        "email": p.email,
        "email_verified": true,
        "name": "Oidc Person",
        "preferred_username": "oidcperson",
    }))
}

async fn user_json(State(p): State<Shared>, headers: HeaderMap) -> Json<Value> {
    let mut p = p.lock().unwrap();
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    p.userinfo_auths.push(auth);
    Json(json!({
        "user": {"id": 4242, "login": "oauthperson", "name": "Oauth Person", "mail": p.email},
        "verified": true,
    }))
}

async fn serve_provider(email: &str) -> Shared {
    let shared: Shared = Arc::new(Mutex::new(Provider {
        email: email.to_string(),
        ..Default::default()
    }));
    let app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/token", post(token))
        .route("/userinfo", get(userinfo))
        .route("/user.json", get(user_json))
        .with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    shared.lock().unwrap().base = format!("http://{addr}");
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    shared
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

async fn oidc_site(db: &TestDb, email: &str) -> (AppState, Shared) {
    let provider = serve_provider(email).await;
    let state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let base = provider.lock().unwrap().base.clone();
    set(
        &state,
        "openid_connect_discovery_document",
        &format!("{base}/.well-known/openid-configuration"),
    )
    .await;
    set(&state, "openid_connect_client_id", "oidc-client").await;
    set(&state, "openid_connect_client_secret", "oidc-secret").await;
    set(&state, "openid_connect_enabled", "t").await;
    set(&state, "allowed_internal_hosts", "127.0.0.1").await;
    set(&state, "auth_require_interaction", "f").await;
    (state, provider)
}

/// The provider's half: the authorize redirect's state and nonce read,
/// the browser sent back with a code.
async fn round_trip(browser: &mut Browser, provider: &Shared, path: &str, code: &str) -> Reply {
    let start = browser.get(path).await;
    assert_eq!(start.status, StatusCode::FOUND, "{}", start.body);
    let authorize = start.location();
    let base = provider.lock().unwrap().base.clone();
    assert!(authorize.starts_with(&base), "{authorize}");
    provider.lock().unwrap().nonce = query_param(&authorize, "nonce");
    let state_param = query_param(&authorize, "state");
    let name = path.trim_start_matches("/auth/");
    browser
        .get(&format!(
            "/auth/{name}/callback?code={code}&state={state_param}"
        ))
        .await
}

#[tokio::test]
async fn oidc_login_of_a_new_user_leaves_signup_data() {
    let db = TestDb::new().await;
    let (state, provider) = oidc_site(&db, "Oidc.Person@Example.com").await;
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let start = browser.get("/auth/oidc").await;
    let authorize = start.location();
    // Faraday's sorted query: the client, the nonce, the callback, the
    // response type, the scope and the state.
    assert_eq!(query_param(&authorize, "client_id"), "oidc-client");
    assert_eq!(
        query_param(&authorize, "redirect_uri"),
        "http://test.localhost/auth/oidc/callback"
    );
    assert_eq!(query_param(&authorize, "response_type"), "code");
    assert_eq!(query_param(&authorize, "scope"), "openid");
    assert_eq!(query_param(&authorize, "state").len(), 48);
    assert_eq!(query_param(&authorize, "nonce").len(), 64);
    let keys: Vec<String> = form_urlencoded::parse(authorize.split_once('?').unwrap().1.as_bytes())
        .map(|(k, _)| k.into_owned())
        .collect();
    assert_eq!(
        keys,
        [
            "client_id",
            "nonce",
            "redirect_uri",
            "response_type",
            "scope",
            "state"
        ]
    );
    provider.lock().unwrap().nonce = query_param(&authorize, "nonce");
    let back = browser
        .get(&format!(
            "/auth/oidc/callback?code=good-code&state={}",
            query_param(&authorize, "state")
        ))
        .await;
    assert_eq!(back.status, StatusCode::FOUND, "{}", back.body);
    assert_eq!(back.location(), "http://test.localhost/");
    // The token request: Basic auth, the code and the callback.
    {
        let p = provider.lock().unwrap();
        let (headers, form) = &p.token_requests[0];
        assert_eq!(
            headers.get(header::AUTHORIZATION).unwrap(),
            "Basic b2lkYy1jbGllbnQ6b2lkYy1zZWNyZXQ="
        );
        let get = |k: &str| form.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get("grant_type").as_deref(), Some("authorization_code"));
        assert_eq!(
            get("redirect_uri").as_deref(),
            Some("http://test.localhost/auth/oidc/callback")
        );
        assert_eq!(p.userinfo_auths, ["Bearer access-1"]);
    }
    let data = browser.authentication_data();
    assert_eq!(data["email"], "oidc.person@example.com");
    assert_eq!(data["username"], "oidcperson");
    assert_eq!(data["auth_provider"], "oidc");
    assert_eq!(data["email_valid"], true);
    assert_eq!(data["name"], "Oidc Person");
    assert_eq!(data["destination_url"], "/");
    assert!(browser.cookie("_t").is_none());
    let (user_id, info): (Option<i32>, Value) = sqlx::query_as(
        "SELECT user_id, info FROM user_associated_accounts WHERE provider_name = 'oidc' AND provider_uid = 'sub-1'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(user_id, None);
    assert_eq!(info["nickname"], "oidcperson");
}

#[tokio::test]
async fn oidc_login_matches_a_verified_email_and_logs_on() {
    let db = TestDb::new().await;
    let (state, provider) = oidc_site(&db, "user1@example.com").await;
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let back = round_trip(&mut browser, &provider, "/auth/oidc", "good-code").await;
    assert_eq!(back.status, StatusCode::FOUND, "{}", back.body);
    let data = browser.authentication_data();
    assert_eq!(data["authenticated"], true, "{data}");
    assert!(browser.cookie("_t").is_some());
    let (user_id,): (Option<i32>,) = sqlx::query_as(
        "SELECT user_id FROM user_associated_accounts WHERE provider_name = 'oidc' AND provider_uid = 'sub-1'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(user_id, Some(3));
    let oauth: bool = sqlx::query_scalar(
        "SELECT authenticated_with_oauth FROM user_auth_tokens WHERE user_id = 3 ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(oauth);
    let current = browser.get("/session/current.json").await;
    assert_eq!(current.status, StatusCode::OK, "{}", current.body);
}

#[tokio::test]
async fn oidc_refuses_a_bad_state_and_a_bad_code() {
    let db = TestDb::new().await;
    let (state, provider) = oidc_site(&db, "user1@example.com").await;
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let start = browser.get("/auth/oidc").await;
    provider.lock().unwrap().nonce = query_param(&start.location(), "nonce");
    let back = browser
        .get("/auth/oidc/callback?code=good-code&state=not-the-state")
        .await;
    assert_eq!(back.status, StatusCode::FOUND);
    assert_eq!(
        back.location(),
        "/auth/failure?message=csrf_detected&strategy=oidc"
    );
    let failure = browser.get(&back.location()).await;
    assert!(
        failure.body.contains("the authorization timed out"),
        "{}",
        failure.body
    );
    let back = round_trip(&mut browser, &provider, "/auth/oidc", "bad-code").await;
    assert_eq!(
        back.location(),
        "/auth/failure?message=invalid_credentials&strategy=oidc"
    );
    assert!(browser.cookie("_t").is_none());
}

#[tokio::test]
async fn oidc_refuses_a_replayed_nonce() {
    let db = TestDb::new().await;
    let (state, provider) = oidc_site(&db, "user1@example.com").await;
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let start = browser.get("/auth/oidc").await;
    // The provider signs a nonce other than the one sent.
    provider.lock().unwrap().nonce = "someone-elses".into();
    let back = browser
        .get(&format!(
            "/auth/oidc/callback?code=good-code&state={}",
            query_param(&start.location(), "state")
        ))
        .await;
    assert_eq!(
        back.location(),
        "/auth/failure?message=jwt_nonce_verify_failed&strategy=oidc"
    );
}

#[tokio::test]
async fn oauth2_basic_fetches_the_user_json() {
    let db = TestDb::new().await;
    let provider = serve_provider("user2@example.com").await;
    let state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let base = provider.lock().unwrap().base.clone();
    for (k, v) in [
        ("oauth2_client_id", "basic-client"),
        ("oauth2_client_secret", "basic-secret"),
        ("oauth2_authorize_url", &format!("{base}/authorize")),
        ("oauth2_token_url", &format!("{base}/token")),
        ("oauth2_user_json_url", &format!("{base}/user.json")),
        ("oauth2_json_user_id_path", "user.id"),
        ("oauth2_json_username_path", "user.login"),
        ("oauth2_json_name_path", "user.name"),
        ("oauth2_json_email_path", "user.mail"),
        ("oauth2_json_email_verified_path", "verified"),
        ("oauth2_scope", "read"),
        ("oauth2_enabled", "t"),
        ("allowed_internal_hosts", "127.0.0.1"),
        ("auth_require_interaction", "f"),
    ] {
        set(&state, k, v).await;
    }
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let start = browser.get("/auth/oauth2_basic").await;
    assert_eq!(start.status, StatusCode::FOUND, "{}", start.body);
    assert_eq!(query_param(&start.location(), "scope"), "read");
    assert!(query_param(&start.location(), "nonce").is_empty());
    let back = browser
        .get(&format!(
            "/auth/oauth2_basic/callback?code=good-code&state={}",
            query_param(&start.location(), "state")
        ))
        .await;
    assert_eq!(back.status, StatusCode::FOUND, "{}", back.body);
    {
        let p = provider.lock().unwrap();
        let (headers, form) = &p.token_requests[0];
        // Header and body both, the plugin's default.
        assert_eq!(
            headers.get(header::AUTHORIZATION).unwrap(),
            "Basic YmFzaWMtY2xpZW50OmJhc2ljLXNlY3JldA=="
        );
        let get = |k: &str| form.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get("client_id").as_deref(), Some("basic-client"));
        assert_eq!(get("client_secret").as_deref(), Some("basic-secret"));
        assert_eq!(p.userinfo_auths, ["Bearer access-1"]);
    }
    let data = browser.authentication_data();
    assert_eq!(data["authenticated"], true, "{data}");
    let (uid, user_id): (String, Option<i32>) = sqlx::query_as(
        "SELECT provider_uid, user_id FROM user_associated_accounts WHERE provider_name = 'oauth2_basic'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(uid, "4242");
    assert_eq!(user_id, Some(4));
}

#[tokio::test]
async fn interaction_required_shows_the_confirmation_page() {
    let db = TestDb::new().await;
    let (state, _provider) = oidc_site(&db, "user1@example.com").await;
    set(&state, "auth_require_interaction", "t").await;
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let page = browser.get("/auth/oidc").await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(
        page.body.contains("Log in with OpenID Connect"),
        "{}",
        page.body
    );
    assert!(page.body.contains("name=\"authenticity_token\""));
    // A POST without the token is a CSRF failure.
    let post = browser.send(Method::POST, "/auth/oidc", "x=1").await;
    assert_eq!(
        post.location(),
        "/auth/failure?message=csrf_detected&strategy=oidc"
    );
    let unknown = browser.get("/auth/github").await;
    assert_eq!(unknown.status, StatusCode::FORBIDDEN);
}
