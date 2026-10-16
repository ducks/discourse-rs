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
async fn oidc_signup_creates_an_active_account_without_a_password() {
    let db = TestDb::new().await;
    let (state, provider) = oidc_site(&db, "Signup.Person@Example.com").await;
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let start = browser.get("/auth/oidc").await;
    let authorize = start.location();
    provider.lock().unwrap().nonce = query_param(&authorize, "nonce");
    let back = browser
        .get(&format!(
            "/auth/oidc/callback?code=good-code&state={}",
            query_param(&authorize, "state")
        ))
        .await;
    assert_eq!(back.status, StatusCode::FOUND, "{}", back.body);
    assert_eq!(browser.authentication_data()["email_valid"], true);

    let page = browser.get("/signup").await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(
        page.body.contains("class=\"create-account\""),
        "{}",
        page.body
    );
    assert!(!page.body.contains("[en."), "{}", page.body);
    let csrf = page
        .body
        .split_once("data-csrf=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(token, _)| token.to_string())
        .unwrap();

    let hp = browser.get("/session/hp.json").await;
    let hp: Value = serde_json::from_str(&hp.body).unwrap();
    let challenge: String = hp["challenge"].as_str().unwrap().chars().rev().collect();
    let body = form_urlencoded::Serializer::new(String::new())
        .append_pair("authenticity_token", &csrf)
        .append_pair("email", "signup.person@example.com")
        .append_pair("username", "signupperson")
        .append_pair("name", "Signup Person")
        .append_pair("password_confirmation", hp["value"].as_str().unwrap())
        .append_pair("challenge", &challenge)
        .finish();
    let created = browser.send(Method::POST, "/u.json", &body).await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.body);
    let created: Value = serde_json::from_str(&created.body).unwrap();
    assert_eq!(created["success"], true, "{created}");
    assert_eq!(created["active"], true, "{created}");
    assert!(browser.cookie("_t").is_some());

    let (user_id, active, password): (i32, bool, Option<String>) = sqlx::query_as(
        "SELECT u.id, u.active, \
                (SELECT password_hash FROM user_passwords WHERE user_id = u.id) \
         FROM users u WHERE username = 'signupperson'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(active);
    assert_eq!(password, None);
    let linked: Option<i32> = sqlx::query_scalar(
        "SELECT user_id FROM user_associated_accounts WHERE provider_name = 'oidc' AND provider_uid = 'sub-1'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(linked, Some(user_id));

    // Logged in now, the signup page sends the reader home.
    let again = browser.get("/signup").await;
    assert_eq!(again.status, StatusCode::FOUND);
    assert_eq!(again.location(), "http://test.localhost/");
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

/// What the fake GitHub and Google saw.
#[derive(Default)]
struct Hosted {
    base: String,
    /// Each token request's headers and form.
    token_requests: Vec<(HeaderMap, Vec<(String, String)>)>,
    /// The Accept header of the emails request.
    emails_accept: Option<String>,
    /// The service account's JWT, checked against this key.
    service_key: Option<rsa::RsaPublicKey>,
    group_pages: Vec<String>,
}

type SharedHosted = Arc<Mutex<Hosted>>;

async fn hosted_token(
    State(p): State<SharedHosted>,
    headers: HeaderMap,
    Form(form): Form<Vec<(String, String)>>,
) -> (StatusCode, Json<Value>) {
    let mut p = p.lock().unwrap();
    p.token_requests.push((headers, form.clone()));
    let get = |k: &str| form.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    if get("grant_type").as_deref() == Some("urn:ietf:params:oauth:grant-type:jwt-bearer") {
        use base64::Engine;
        use sha2::Digest;
        let jwt = get("assertion").unwrap_or_default();
        let (input, signature) = jwt.rsplit_once('.').unwrap();
        let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(signature)
            .unwrap();
        let mut digest_info = vec![
            0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
            0x01, 0x05, 0x00, 0x04, 0x20,
        ];
        digest_info.extend_from_slice(&sha2::Sha256::digest(input.as_bytes()));
        let key = p.service_key.clone().unwrap();
        if key
            .verify(
                rsa::Pkcs1v15Sign::new_unprefixed(),
                &digest_info,
                &signature,
            )
            .is_err()
        {
            return (StatusCode::UNAUTHORIZED, Json(json!({"error": "bad jwt"})));
        }
        let claims: Value = serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(input.split_once('.').unwrap().1)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(claims["sub"], "admin@example.com");
        assert_eq!(
            claims["scope"],
            "https://www.googleapis.com/auth/admin.directory.group.readonly"
        );
        return (
            StatusCode::OK,
            Json(json!({"access_token": "service-token", "token_type": "Bearer"})),
        );
    }
    if get("code").as_deref() != Some("good-code") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "bad_verification_code"})),
        );
    }
    (
        StatusCode::OK,
        Json(
            json!({"access_token": "hosted-token", "token_type": "bearer", "scope": "user:email"}),
        ),
    )
}

async fn hosted_github_user() -> Json<Value> {
    Json(json!({"id": 77, "login": "octo", "name": null, "avatar_url": null}))
}

async fn hosted_github_emails(State(p): State<SharedHosted>, headers: HeaderMap) -> Json<Value> {
    p.lock().unwrap().emails_accept = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    Json(json!([
        {"email": "nobody-here@example.com", "primary": true, "verified": true},
        {"email": "user2@example.com", "primary": false, "verified": true},
    ]))
}

async fn hosted_google_userinfo() -> Json<Value> {
    Json(json!({
        "sub": "g-77",
        "email": "user3@example.com",
        "email_verified": true,
        "hd": "example.com",
        "picture": "https://lh3.example/p=s96-c",
    }))
}

async fn hosted_google_tokeninfo() -> Json<Value> {
    Json(json!({"aud": "gclient", "scope": "email profile"}))
}

async fn hosted_google_groups(
    State(p): State<SharedHosted>,
    axum::extract::Query(q): axum::extract::Query<Vec<(String, String)>>,
) -> Json<Value> {
    let page = q
        .iter()
        .find(|(k, _)| k == "pageToken")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    p.lock().unwrap().group_pages.push(page.clone());
    if page.is_empty() {
        Json(json!({
            "groups": [{"id": "g1", "name": "Engineering", "directMembersCount": "3"}],
            "nextPageToken": "page-2",
        }))
    } else {
        Json(json!({"groups": [{"id": "g2", "name": "Support"}]}))
    }
}

/// A site whose GitHub and Google are the test's.
async fn hosted_site(db: &TestDb) -> (AppState, SharedHosted) {
    let shared: SharedHosted = Arc::new(Mutex::new(Hosted::default()));
    let app = Router::new()
        .route("/gh/login/oauth/access_token", post(hosted_token))
        .route("/gh-api/user", get(hosted_github_user))
        .route("/gh-api/user/emails", get(hosted_github_emails))
        .route("/g-oauth/token", post(hosted_token))
        .route("/g-apis/oauth2/v3/userinfo", get(hosted_google_userinfo))
        .route("/g-apis/oauth2/v3/tokeninfo", post(hosted_google_tokeninfo))
        .route(
            "/g-admin/admin/directory/v1/groups",
            get(hosted_google_groups),
        )
        .with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    shared.lock().unwrap().base = base.clone();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut config = config(RailsEnv::Test, &[]);
    config.auth_provider_hosts = vec![
        ("https://github.com".into(), format!("{base}/gh")),
        ("https://api.github.com".into(), format!("{base}/gh-api")),
        (
            "https://oauth2.googleapis.com".into(),
            format!("{base}/g-oauth"),
        ),
        (
            "https://www.googleapis.com".into(),
            format!("{base}/g-apis"),
        ),
        (
            "https://admin.googleapis.com".into(),
            format!("{base}/g-admin"),
        ),
    ];
    let state = state(db.pool.clone(), config).await;
    set(&state, "allowed_internal_hosts", "127.0.0.1").await;
    set(&state, "auth_require_interaction", "f").await;
    (state, shared)
}

#[tokio::test]
async fn github_logs_on_by_a_verified_secondary_email() {
    let db = TestDb::new().await;
    let (state, hosted) = hosted_site(&db).await;
    set(&state, "github_client_id", "ghclient").await;
    set(&state, "github_client_secret", "abc123").await;
    set(&state, "enable_github_logins", "t").await;
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let start = browser.get("/auth/github").await;
    let authorize = start.location();
    assert!(
        authorize.starts_with("https://github.com/login/oauth/authorize?"),
        "{authorize}"
    );
    assert_eq!(query_param(&authorize, "scope"), "user:email");
    let back = browser
        .get(&format!(
            "/auth/github/callback?code=good-code&state={}",
            query_param(&authorize, "state")
        ))
        .await;
    assert_eq!(back.status, StatusCode::FOUND, "{}", back.body);
    assert_eq!(back.location(), "http://test.localhost/");
    {
        let h = hosted.lock().unwrap();
        // request_body: the client's id and secret in the form, no header.
        let (headers, form) = &h.token_requests[0];
        assert!(headers.get(header::AUTHORIZATION).is_none());
        let get = |k: &str| form.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get("client_id").as_deref(), Some("ghclient"));
        assert_eq!(get("client_secret").as_deref(), Some("abc123"));
        assert_eq!(
            h.emails_accept.as_deref(),
            Some("application/vnd.github.v3")
        );
    }
    assert!(browser.cookie("_t").is_some());
    let (user_id, info): (Option<i32>, Value) = sqlx::query_as(
        "SELECT user_id, info FROM user_associated_accounts WHERE provider_name = 'github'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(user_id, Some(4));
    assert_eq!(info["email"], "nobody-here@example.com");
    assert_eq!(info["nickname"], "octo");
}

#[tokio::test]
async fn google_reads_groups_through_the_service_account() {
    use rsa::pkcs8::EncodePrivateKey;
    let db = TestDb::new().await;
    let (state, hosted) = hosted_site(&db).await;
    let key = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 1024).unwrap();
    hosted.lock().unwrap().service_key = Some(key.to_public_key());
    let pem = key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
    let account = json!({"client_email": "svc@example.iam", "private_key": pem.as_str()});
    for (k, v) in [
        ("google_oauth2_client_id", "gclient"),
        ("google_oauth2_client_secret", "gsecret"),
        ("google_oauth2_hd", "example.com"),
        ("google_oauth2_hd_groups", "t"),
        (
            "google_oauth2_hd_groups_service_account_admin_email",
            "admin@example.com",
        ),
        (
            "google_oauth2_hd_groups_service_account_json",
            &account.to_string(),
        ),
        ("enable_google_oauth2_logins", "t"),
    ] {
        set(&state, k, v).await;
    }
    let mut browser = Browser {
        state: state.clone(),
        cookies: Vec::new(),
    };
    let start = browser.get("/auth/google_oauth2").await;
    let authorize = start.location();
    assert_eq!(query_param(&authorize, "hd"), "example.com");
    assert_eq!(query_param(&authorize, "scope"), "email profile");
    assert_eq!(query_param(&authorize, "access_type"), "offline");
    let back = browser
        .get(&format!(
            "/auth/google_oauth2/callback?code=good-code&state={}",
            query_param(&authorize, "state")
        ))
        .await;
    assert_eq!(back.status, StatusCode::FOUND, "{}", back.body);
    assert!(browser.cookie("_t").is_some());
    {
        let h = hosted.lock().unwrap();
        // The code exchanged with Basic auth, then the service account's.
        assert_eq!(
            h.token_requests[0].0.get(header::AUTHORIZATION).unwrap(),
            "Basic Z2NsaWVudDpnc2VjcmV0"
        );
        assert_eq!(h.token_requests.len(), 2);
        assert_eq!(h.group_pages, ["", "page-2"]);
    }
    let (extra, credentials): (Value, Value) = sqlx::query_as(
        "SELECT extra, credentials FROM user_associated_accounts WHERE provider_name = 'google_oauth2'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(extra["raw_groups"][0]["direct_members_count"], "3");
    assert_eq!(credentials["scope"], "email profile");
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT g.name FROM user_associated_groups ug \
         JOIN associated_groups g ON g.id = ug.associated_group_id \
         WHERE ug.user_id = 5 ORDER BY g.name",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(names, ["Engineering", "Support"]);
}
