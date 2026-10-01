//! Local login and the session cookie: CSRF, POST /session, the `_t`
//! cookie resolving to a user, rotation, logout, and what the login gate
//! does for a logged-in user. The seeded users' password is "password".

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use discourse_rs::session::token::hash_token;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

const BOOL: i32 = 5;

/// A browser-ish client: keeps cookies between requests.
struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

struct Reply {
    status: StatusCode,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }

    fn set_cookies(&self) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(k, _)| k == "set-cookie")
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

impl Client {
    fn new(state: AppState) -> Client {
        Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        }
    }

    async fn send(
        &mut self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Reply {
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
            request = request.header("x-csrf-token", token.as_str());
        }
        for (k, v) in headers {
            request = request.header(*k, *v);
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
        self.send(Method::GET, path, &[], "").await
    }

    async fn fetch_csrf(&mut self) -> Reply {
        let reply = self.get("/session/csrf.json").await;
        self.csrf = reply.json()["csrf"].as_str().map(str::to_string);
        reply
    }

    async fn login(&mut self, login: &str, password: &str) -> Reply {
        if self.csrf.is_none() {
            self.fetch_csrf().await;
        }
        self.send(
            Method::POST,
            "/session",
            &[("x-requested-with", "XMLHttpRequest")],
            &format!("login={login}&password={password}"),
        )
        .await
    }

    fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

#[tokio::test]
async fn csrf_and_current_for_an_anonymous_visitor() {
    let db = TestDb::new().await;
    let mut client = Client::new(state(db.pool.clone(), config(RailsEnv::Test, &[])));
    let reply = client.get("/session/current.json").await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.body, "");
    assert!(
        reply
            .headers
            .iter()
            .any(|(k, v)| k == "content-type" && v.starts_with("text/plain"))
    );

    let reply = client.fetch_csrf().await;
    assert_eq!(reply.status, StatusCode::OK);
    let token = reply.json()["csrf"].as_str().unwrap().to_string();
    assert_eq!(token.len(), 86);
    assert!(
        reply
            .set_cookies()
            .iter()
            .any(|c| c.starts_with("_forum_session=")
                && c.contains("HttpOnly")
                && c.contains("SameSite=Lax"))
    );
    // A second call issues a new mask on the same session, no new cookie.
    let again = client.fetch_csrf().await;
    assert_ne!(again.json()["csrf"], token);
    assert!(again.set_cookies().is_empty());

    // Without a token the login is refused the way Rails does it.
    client.csrf = None;
    let reply = client
        .send(
            Method::POST,
            "/session",
            &[("x-requested-with", "XMLHttpRequest")],
            "login=user1&password=password",
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.body, "[\"BAD CSRF\"]");
}

#[tokio::test]
async fn logs_in_and_sets_a_rails_compatible_auth_cookie() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[]));
    let mut client = Client::new(app_state.clone());
    let reply = client.login("user1", "password").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let json = reply.json();
    assert!(json.get("error").is_none(), "{}", reply.body);
    assert_eq!(json["user"]["username"], "user1");
    assert!(
        reply
            .headers
            .iter()
            .any(|(k, v)| k == "x-discourse-username" && v == "user1")
    );
    let auth = reply
        .set_cookies()
        .into_iter()
        .find(|c| c.starts_with("_t="))
        .expect("_t cookie")
        .to_string();
    assert!(
        auth.contains("; path=/")
            && auth.contains("HttpOnly")
            && auth.contains("SameSite=Lax")
            && auth.contains("expires="),
        "{auth}"
    );

    // The cookie decrypts with our key and names the token row.
    let raw = client.cookie("_t").unwrap();
    let map = app_state.keys.codec.decrypt("_t", raw).unwrap();
    let token = map["token"].as_str().unwrap();
    assert_eq!(token.len(), 32);
    assert_eq!(map["user_id"].as_i64(), Some(3));
    let stored: (String, String, bool) =
        sqlx::query_as("SELECT auth_token, prev_auth_token, auth_token_seen FROM user_auth_tokens WHERE user_id = 3")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(stored.0, hash_token(token, &app_state.keys.secret_key_base));
    assert_eq!(stored.0, stored.1);
    assert!(!stored.2);
    let logged: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_auth_token_logs WHERE action = 'generate' AND user_id = 3",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(logged, 1);

    // The session now resolves, and the first use marks the token seen.
    let reply = client.get("/session/current.json").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["current_user"]["username"], "user1");
    assert_eq!(reply.json()["current_user"]["login_method"], "local");
    let seen: bool =
        sqlx::query_scalar("SELECT auth_token_seen FROM user_auth_tokens WHERE user_id = 3")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(seen);

    // Logout deletes the row and clears the cookie.
    let reply = client
        .send(
            Method::DELETE,
            "/session/user1",
            &[("x-requested-with", "XMLHttpRequest")],
            "",
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["redirect_url"], "/");
    assert!(
        reply
            .set_cookies()
            .iter()
            .any(|c| c.starts_with("_t=;") && c.contains("max-age=0"))
    );
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM user_auth_tokens WHERE user_id = 3")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    assert!(client.cookie("_t").is_none());
    let reply = client.get("/session/current.json").await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn login_errors_match_rails() {
    let db = TestDb::new().await;
    let mut client = Client::new(state(db.pool.clone(), config(RailsEnv::Test, &[])));
    for (login, password) in [
        ("user1", "wrong"),
        ("nobody", "password"),
        ("user1", &"x".repeat(201)),
    ] {
        let reply = client.login(login, password).await;
        assert_eq!(reply.status, StatusCode::OK, "{login}");
        assert_eq!(
            reply.json()["error"],
            "Incorrect username, email or password",
            "{login}"
        );
        assert!(
            reply.set_cookies().iter().all(|c| !c.starts_with("_t=")),
            "{login}"
        );
    }
    let reply = client.login("user1", "").await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.json()["errors"][0],
        "param is missing or the value is empty or invalid: password"
    );

    // Leading @, surrounding spaces, and the email all log in.
    let email: String =
        sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = 3 AND \"primary\"")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    for login in ["%40user1", "+user1+", &email.replace('@', "%40"), "USER1"] {
        let reply = client.login(login, "password").await;
        assert_eq!(
            reply.json()["user"]["username"],
            "user1",
            "{login}: {}",
            reply.body
        );
    }

    // Deactivated users are told to activate; unapproved ones to wait.
    sqlx::query("UPDATE users SET active = false WHERE id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let reply = client.login("user1", "password").await;
    assert_eq!(reply.json()["reason"], "not_activated");
    assert_eq!(reply.json()["current_email"], email);
    sqlx::query("UPDATE users SET active = true WHERE id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    set_setting(&db.pool, "must_approve_users", BOOL, "t").await;
    let reply = client.login("user1", "password").await;
    assert!(
        reply.json()["error"]
            .as_str()
            .unwrap()
            .contains("hasn't been approved"),
        "{}",
        reply.body
    );
    let reply = client.login("admin", "password").await;
    assert_eq!(
        reply.json()["user"]["username"],
        "admin",
        "admins skip approval"
    );
    set_setting(&db.pool, "must_approve_users", BOOL, "f").await;

    // An expired password matches but is reported expired.
    sqlx::query("UPDATE user_passwords SET password_expired_at = now() WHERE user_id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let reply = client.login("user1", "password").await;
    assert_eq!(
        reply.json(),
        serde_json::json!({"error": "expired", "reason": "expired"})
    );

    set_setting(&db.pool, "enable_local_logins", BOOL, "f").await;
    let reply = client.login("admin", "password").await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.json()["error_type"], "invalid_access");
}

#[tokio::test]
async fn sessions_rotate_expire_and_drop_suspended_users() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[]));
    let mut client = Client::new(app_state.clone());
    client.login("user1", "password").await;
    let first = client.cookie("_t").unwrap().to_string();
    client.get("/session/current.json").await; // marks seen

    // Eleven minutes after a seen rotation, the next request rotates.
    sqlx::query(
        "UPDATE user_auth_tokens SET rotated_at = now() - interval '11 minutes' WHERE user_id = 3",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let reply = client.get("/latest.json").await;
    assert_eq!(reply.status, StatusCode::OK);
    let rotated = reply
        .set_cookies()
        .into_iter()
        .find(|c| c.starts_with("_t="))
        .expect("rotated cookie")
        .to_string();
    assert_ne!(rotated.split(';').next().unwrap(), format!("_t={first}"));
    let (auth, prev, seen): (String, String, bool) =
        sqlx::query_as("SELECT auth_token, prev_auth_token, auth_token_seen FROM user_auth_tokens WHERE user_id = 3")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_ne!(auth, prev);
    assert!(!seen);
    let old_token = app_state.keys.codec.decrypt("_t", &first).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        prev,
        hash_token(&old_token, &app_state.keys.secret_key_base)
    );

    // The previous cookie still authenticates until the new one is seen.
    let mut stale = Client::new(app_state.clone());
    stale.cookies.push(("_t".into(), first.clone()));
    let reply = stale.get("/session/current.json").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);

    // A cookie older than maximum_session_age is dead, and gets cleared.
    sqlx::query(
        "UPDATE user_auth_tokens SET rotated_at = now() - interval '61 days' WHERE user_id = 3",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let reply = client.get("/session/current.json").await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(
        reply
            .set_cookies()
            .iter()
            .any(|c| c.starts_with("_t=;") && c.contains("max-age=0")),
        "{:?}",
        reply.set_cookies()
    );

    // A suspended user with a valid token is anonymous.
    let mut client = Client::new(app_state.clone());
    client.login("user1", "password").await;
    sqlx::query("UPDATE users SET suspended_till = now() + interval '2 days' WHERE id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let reply = client.get("/session/current.json").await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    // A tampered cookie is anonymous too.
    let mut bad = Client::new(app_state.clone());
    let mut chars: Vec<char> = first.chars().collect();
    chars.swap(5, 6);
    bad.cookies.push(("_t".into(), chars.into_iter().collect()));
    assert_eq!(
        bad.get("/session/current.json").await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn logged_in_users_pass_the_login_gate_and_update_last_seen() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[]));
    let mut client = Client::new(app_state.clone());
    client.login("user1", "password").await;

    // Logging in records nothing; a plain navigation records the visit;
    // XHR without Discourse-Present does not.
    let before: (Option<chrono::NaiveDateTime>, i32) =
        sqlx::query_as("SELECT u.last_seen_at, s.days_visited FROM users u JOIN user_stats s ON s.user_id = u.id WHERE u.id = 3")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    client
        .send(
            Method::GET,
            "/latest.json",
            &[("x-requested-with", "XMLHttpRequest")],
            "",
        )
        .await;
    let mid: (Option<chrono::NaiveDateTime>, i32) =
        sqlx::query_as("SELECT u.last_seen_at, s.days_visited FROM users u JOIN user_stats s ON s.user_id = u.id WHERE u.id = 3")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        before, mid,
        "xhr without Discourse-Present leaves last seen alone"
    );
    client.get("/latest").await;
    let after: (Option<chrono::NaiveDateTime>, i32) =
        sqlx::query_as("SELECT u.last_seen_at, s.days_visited FROM users u JOIN user_stats s ON s.user_id = u.id WHERE u.id = 3")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(after.0 > before.0, "{before:?} -> {after:?}");
    assert_eq!(after.1, before.1 + 1);
    let visits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_visits WHERE user_id = 3 AND visited_at = now()::date",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(visits, 1);

    // POST /login is the hidden form the client submits afterwards.
    // The gate lets a session through and still lets others log in.
    set_setting(&db.pool, "login_required", BOOL, "t").await;
    assert_eq!(client.get("/latest.json").await.status, StatusCode::OK);
    let mut anon = Client::new(app_state.clone());
    assert_eq!(anon.get("/latest.json").await.status, StatusCode::FORBIDDEN);
    let reply = anon.login("user0", "password").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);

    let reply = client
        .send(
            Method::POST,
            "/login",
            &[],
            "username=user1&password=x&redirect=%2Ft%2Ffoo%2F5",
        )
        .await;
    assert_eq!(reply.status, StatusCode::FOUND);
    assert!(
        reply
            .headers
            .iter()
            .any(|(k, v)| k == "location" && v == "http://test.localhost/t/foo/5")
    );
    let reply = client
        .send(
            Method::POST,
            "/login",
            &[],
            "redirect=https%3A%2F%2Fevil.test%2F",
        )
        .await;
    assert!(
        reply
            .headers
            .iter()
            .any(|(k, v)| k == "location" && v == "http://test.localhost/")
    );
}
