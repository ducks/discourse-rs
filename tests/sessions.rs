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
    // The seed may already hold a visit for today (the reference was
    // browsed the day it was snapshotted); the test wants a first visit.
    sqlx::query("DELETE FROM user_visits WHERE user_id = 3 AND visited_at = now()::date")
        .execute(&db.pool)
        .await
        .unwrap();
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

#[tokio::test]
async fn pages_carry_the_viewer_and_the_logout_form_works() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[]));

    // Anonymous: the anon class, a login link, no CSRF meta, cacheable.
    let mut anon = Client::new(app_state.clone());
    let reply = anon.get("/latest").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(
        reply.body.contains(r#"<html lang="en" class="anon">"#),
        "{}",
        reply.body
    );
    assert!(reply.body.contains(r#"class="site-login""#));
    assert!(!reply.body.contains("csrf-token"));
    assert!(
        !reply
            .headers
            .iter()
            .any(|(k, _)| k == "x-discourse-username")
    );

    // Logged in: the viewer block with a masked CSRF token and no caching.
    let mut client = Client::new(app_state.clone());
    client.login("user1", "password").await;
    let reply = client.get("/t/parity-fixture-replies-and-posters/35").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.body.contains(r#"<html lang="en">"#), "{}", reply.body);
    assert!(reply.body.contains(r#"class="site-user" href="/u/user1""#));
    let token = reply
        .body
        .split(r#"<meta name="csrf-token" content=""#)
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("csrf meta")
        .to_string();
    assert_eq!(token.len(), 86, "{token}");
    assert!(
        reply
            .headers
            .contains(&("x-discourse-username".into(), "user1".into()))
    );
    assert!(
        reply
            .headers
            .contains(&("cache-control".into(), "no-cache, no-store".into()))
    );

    // The logout form posts _method=delete with the page's token.
    let reply = client
        .send(
            Method::POST,
            "/session/user1",
            &[("content-type", "application/x-www-form-urlencoded")],
            &format!("_method=delete&authenticity_token={token}"),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FOUND, "{}", reply.body);
    let tokens: i64 = sqlx::query_scalar("SELECT count(*) FROM user_auth_tokens WHERE user_id = 3")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(tokens, 0);
}

#[tokio::test]
async fn recent_notifications_bump_the_seen_id_unless_silent() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[]));
    let mut client = Client::new(app_state.clone());
    client.login("user1", "password").await;

    let reply = client
        .get("/notifications.json?recent=true&silent=true")
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let json = reply.json();
    assert_eq!(json["seen_notification_id"], 0);
    let ids: Vec<i64> = json["notifications"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_i64().unwrap())
        .collect();
    // Unread high priority first (the bookmark reminder is the newest),
    // then unread non-likes, the like, the read one.
    assert_eq!(ids, vec![52, 50, 48, 44, 51, 47, 41, 7, 45, 2]);

    // Without `silent` the newest visible id is recorded on the user.
    let reply = client.get("/notifications.json?recent=true&limit=3").await;
    let json = reply.json();
    assert_eq!(json["seen_notification_id"], 52);
    assert_eq!(json["notifications"].as_array().unwrap().len(), 3);
    let seen: i64 = sqlx::query_scalar("SELECT seen_notification_id FROM users WHERE id = 3")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(seen, 52);
    // And the counters current.json derives from it follow.
    let reply = client
        .send(
            Method::GET,
            "/session/current.json",
            &[("x-requested-with", "XMLHttpRequest")],
            "",
        )
        .await;
    let current = reply.json();
    assert_eq!(current["current_user"]["unread_notifications"], 0);
    assert_eq!(
        current["current_user"]["unread_high_priority_notifications"],
        4
    );
    assert_eq!(current["current_user"]["seen_notification_id"], 52);

    // Anonymous: 403 not_logged_in, like Rails' requires_login.
    let mut anon = Client::new(app_state);
    let reply = anon.get("/notifications.json").await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.json()["error_type"], "not_logged_in");
}

#[tokio::test]
async fn private_messages_open_for_participants_only() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[]));

    let mut user1 = Client::new(app_state.clone());
    user1.login("user1", "password").await;
    let reply = user1
        .get("/t/parity-fixture-pm-admin-to-user1-and-user2/43.json")
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let json = reply.json();
    // details.allowed_users: the direct members, order left to Postgres.
    let mut allowed: Vec<i64> = json["details"]["allowed_users"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["id"].as_i64().unwrap())
        .collect();
    allowed.sort_unstable();
    assert_eq!(allowed, vec![1, 3, 4]);
    assert_eq!(json["details"]["allowed_groups"], serde_json::json!([]));
    assert_eq!(json["details"]["can_remove_self_id"], 3);
    assert_eq!(json["message_archived"], false);
    assert_eq!(json["suggested_group_name"], Value::Null);
    // The archived one is flagged as such for its archiver only.
    let reply = user1
        .get("/t/parity-fixture-pm-user2-to-user1-archived/44.json")
        .await;
    assert_eq!(reply.json()["message_archived"], true);

    // user0 is not in 43: a 404 with the not-found extras, nothing deleted.
    let mut user0 = Client::new(app_state.clone());
    user0.login("user0", "password").await;
    let reply = user0.get("/t/43.json").await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.json()["error_type"], "not_found");
    let notifications: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id = 2")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        notifications, 6,
        "Rails' InvalidAccess rescue deletes notifications; the port must not"
    );

    // Anonymous: the same 404.
    let mut anon = Client::new(app_state);
    let reply = anon
        .get("/t/parity-fixture-pm-user1-to-user0/42.json")
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn own_profile_lists_auth_tokens_with_the_current_one_active() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[]));
    let mut client = Client::new(app_state.clone());
    client.login("user1", "password").await;
    let reply = client
        .send(
            Method::GET,
            "/u/user1.json",
            &[(
                "user-agent",
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0 Safari/537.36",
            )],
            "",
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let user = &reply.json()["user"];
    let tokens = user["user_auth_tokens"].as_array().unwrap();
    assert_eq!(tokens.len(), 1);
    let t = &tokens[0];
    assert_eq!(
        t.as_object().unwrap().keys().collect::<Vec<_>>(),
        [
            "id",
            "client_ip",
            "location",
            "browser",
            "device",
            "os",
            "icon",
            "created_at",
            "seen_at",
            "is_active"
        ]
    );
    assert_eq!(t["is_active"], true);
    assert_eq!(t["location"], "unknown");
    // The token records the login's user agent, which the test client
    // doesn't send; Rails' strings for that case.
    assert_eq!(t["browser"], "Unknown browser");
    assert_eq!(t["device"], "unknown device");
    assert_eq!(t["os"], "unknown operating system");
    assert_eq!(t["icon"], "question");
    assert_eq!(user["email"], "user1@example.com");
    assert_eq!(user["can_edit"], true);
    assert_eq!(user["group_users"][0]["owner"], false);

    // Another user's view carries none of the private block.
    let mut other = Client::new(app_state);
    other.login("user0", "password").await;
    let user = &other.get("/u/user1.json").await.json()["user"];
    assert!(user.get("email").is_none());
    assert!(user.get("user_auth_tokens").is_none());
    assert!(user.get("user_option").is_none());
}

/// The session layer has nothing to do for a request without the `_t`
/// cookie; a pool that can't connect proves it doesn't try.
#[tokio::test]
async fn requests_without_a_token_cookie_do_not_touch_the_pool() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(200))
        .connect_lazy("postgresql://127.0.0.1:1/nowhere")
        .unwrap();
    let app_state = state(pool, config(RailsEnv::Test, &[]));

    let mut client = Client::new(app_state.clone());
    let reply = client.get("/srv/status").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, "ok");
    assert!(reply.set_cookies().is_empty());

    // With the cookie the layer has a token to look up, and needs the pool.
    let mut client = Client::new(app_state);
    client.cookies.push(("_t".into(), "0".repeat(32)));
    let reply = client.get("/srv/status").await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
}

/// The ids of a bookmark list document, in order (empty for the bare
/// `{"bookmarks": []}` an empty list renders).
fn bookmark_ids(json: &Value) -> Vec<i64> {
    let list = if json["user_bookmark_list"].is_object() {
        &json["user_bookmark_list"]["bookmarks"]
    } else {
        &json["bookmarks"]
    };
    list.as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_i64().unwrap())
        .collect()
}

/// lib/bookmark_query_spec.rb: what drops out of a list as the viewer
/// loses sight of the bookmarked thing. user1's seeded bookmarks: 4
/// (pinned, a post in the PM 42), 3 (reminder, post 36), 5 (post 52),
/// 2 (topic 35) and 1 (post 40 in topic 37).
#[tokio::test]
async fn bookmark_lists_follow_what_the_viewer_can_see() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[]));
    let mut client = Client::new(app_state.clone());
    client.login("user1", "password").await;
    let mut admin = Client::new(app_state.clone());
    admin.login("admin", "password").await;
    let path = "/u/user1/bookmarks.json";

    // Pinned first, then by reminder, then most recently updated.
    let reply = client.get(path).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(bookmark_ids(&reply.json()), vec![4, 3, 5, 2, 1]);

    // Search matches the bookmark's name, the post's text or its topic's
    // title ("pinned" is in bookmark 4's name and topic 37's title).
    let reply = client.get(&format!("{path}?q=pinned")).await;
    assert_eq!(bookmark_ids(&reply.json()), vec![4, 1]);
    let reply = client.get(&format!("{path}?q=repliers")).await;
    assert_eq!(bookmark_ids(&reply.json()), vec![2]);

    // A hidden post of someone else stays listed, without its excerpt.
    sqlx::query("UPDATE posts SET hidden = TRUE WHERE id = 52")
        .execute(&db.pool)
        .await
        .unwrap();
    let json = client.get(path).await.json();
    let hidden = json["user_bookmark_list"]["bookmarks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == 5)
        .unwrap()
        .clone();
    assert_eq!(hidden["hidden"], true);
    assert!(hidden.get("excerpt").is_none());

    // A whisper is not for a regular user.
    sqlx::query("UPDATE posts SET post_type = 4 WHERE id = 52")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        bookmark_ids(&client.get(path).await.json()),
        vec![4, 3, 2, 1]
    );

    // A deleted post takes its bookmark out of the list.
    sqlx::query("UPDATE posts SET deleted_at = now() WHERE id = 36")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(bookmark_ids(&client.get(path).await.json()), vec![4, 2, 1]);

    // So does a topic moved into a category the viewer can't read; an
    // admin reading user1's list still sees it.
    sqlx::query("UPDATE topics SET category_id = 3 WHERE id = 37")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(bookmark_ids(&client.get(path).await.json()), vec![4, 2]);
    assert_eq!(bookmark_ids(&admin.get(path).await.json()), vec![4, 2, 1]);

    // And a private message the owner is no longer in, for the admin too:
    // the messages are the owner's, not the viewer's.
    sqlx::query("DELETE FROM topic_allowed_users WHERE topic_id = 42 AND user_id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(bookmark_ids(&client.get(path).await.json()), vec![2]);
    assert_eq!(bookmark_ids(&admin.get(path).await.json()), vec![2, 1]);

    // A topic bookmark goes with its topic's first post.
    sqlx::query("UPDATE posts SET deleted_at = now() WHERE topic_id = 35 AND post_number = 1")
        .execute(&db.pool)
        .await
        .unwrap();
    let reply = client.get(path).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), serde_json::json!({"bookmarks": []}));

    // A bookmark type only a plugin serializes is refused, not skipped.
    sqlx::query(
        "INSERT INTO bookmarks (user_id, bookmarkable_id, bookmarkable_type, created_at, updated_at) \
         VALUES (3, 1, 'Chat::Message', now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        client.get(path).await.status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

/// users_controller_spec.rb #user_menu_bookmarks: unread reminders first,
/// then the bookmarks they are not about.
#[tokio::test]
async fn the_user_menu_pairs_unread_reminders_with_the_other_bookmarks() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[]));
    let mut client = Client::new(app_state.clone());
    client.login("user1", "password").await;
    let path = "/u/user1/user-menu-bookmarks.json";
    let ids = |json: &Value, key: &str| -> Vec<i64> {
        json[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_i64().unwrap())
            .collect()
    };

    // Notification 52 is the fired reminder of bookmark 5.
    let reply = client.get(path).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let json = reply.json();
    assert_eq!(ids(&json, "notifications"), vec![52]);
    assert_eq!(ids(&json, "bookmarks"), vec![4, 3, 2, 1]);
    // The topic bookmark links to the first unread post (2 read, so 3).
    let topic = json["bookmarks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == 2)
        .unwrap();
    assert!(
        topic["bookmarkable_url"]
            .as_str()
            .unwrap()
            .ends_with("/t/parity-fixture-replies-and-posters/35/3")
    );

    // Only one's own menu.
    assert_eq!(
        client.get("/u/user0/user-menu-bookmarks.json").await.status,
        StatusCode::FORBIDDEN
    );

    // The reminder outlives its bookmark while the post is visible.
    sqlx::query("DELETE FROM bookmarks WHERE id = 5")
        .execute(&db.pool)
        .await
        .unwrap();
    let json = client.get(path).await.json();
    assert_eq!(ids(&json, "notifications"), vec![52]);
    assert_eq!(ids(&json, "bookmarks"), vec![4, 3, 2, 1]);

    // And goes once the viewer can't see the post any more.
    sqlx::query("UPDATE posts SET post_type = 4 WHERE id = 52")
        .execute(&db.pool)
        .await
        .unwrap();
    let json = client.get(path).await.json();
    assert_eq!(ids(&json, "notifications"), Vec::<i64>::new());
    assert_eq!(ids(&json, "bookmarks"), vec![4, 3, 2, 1]);

    // A read reminder is no longer listed, and frees its bookmark.
    sqlx::query("UPDATE posts SET post_type = 1 WHERE id = 52")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO bookmarks (id, user_id, name, bookmarkable_id, bookmarkable_type, created_at, updated_at) \
         VALUES (5, 3, 'back', 52, 'Post', now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let json = client.get(path).await.json();
    assert_eq!(ids(&json, "notifications"), vec![52]);
    assert_eq!(ids(&json, "bookmarks"), vec![4, 3, 2, 1]);
    sqlx::query("UPDATE notifications SET read = TRUE WHERE id = 52")
        .execute(&db.pool)
        .await
        .unwrap();
    let json = client.get(path).await.json();
    assert_eq!(ids(&json, "notifications"), Vec::<i64>::new());
    assert_eq!(ids(&json, "bookmarks"), vec![4, 3, 5, 2, 1]);
}
