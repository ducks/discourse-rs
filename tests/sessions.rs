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
use serde_json::{Value, json};
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
    let mut client = Client::new(state(db.pool.clone(), config(RailsEnv::Test, &[])).await);
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
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
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
    let mut client = Client::new(state(db.pool.clone(), config(RailsEnv::Test, &[])).await);
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
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
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
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
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
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;

    // Anonymous: the anon class, a login link, no CSRF meta, cacheable.
    let mut anon = Client::new(app_state.clone());
    let reply = anon.get("/latest").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(
        reply.body.contains(
            r#"<html lang="en" class="text-size-normal anon no-touch discourse-no-touch desktop-view not-mobile-device">"#
        ),
        "{}",
        reply.body
    );
    assert!(
        reply
            .body
            .contains(r#"class="btn btn-icon-text btn-primary btn-small login-button""#)
    );
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
    assert!(
        reply.body.contains(
            r#"<html lang="en" class="text-size-normal no-touch discourse-no-touch desktop-view not-mobile-device">"#
        ),
        "{}",
        reply.body
    );
    // The avatar button, with the avatar Rails shows user1 (the reference's
    // header).
    assert!(reply.body.contains(r#"id="toggle-current-user""#));
    assert!(
        reply
            .body
            .contains(r#"src="/letter_avatar_proxy/v4/letter/u/5daacb/48.png" class="avatar""#),
        "{}",
        reply.body
    );
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
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
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
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;

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
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
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
    // The bus needs a database at start; the app's pool is the one tested.
    let db = TestDb::new().await;
    let app_state =
        common::state_with_bus_on(db.pool.clone(), pool, config(RailsEnv::Test, &[])).await;

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
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
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
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
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

/// Marking notifications read publishes the user's notification state
/// (User#publish_notifications_state) to them alone, and their poll gets it.
#[tokio::test(flavor = "multi_thread")]
async fn notification_state_goes_live_to_its_user_only() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut client = Client::new(app_state.clone());
    let reply = client.login("user1", "password").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let user_id: i32 = sqlx::query_scalar(
        "UPDATE users SET last_seen_at = now() WHERE username = 'user1' RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let notification_id: i64 = sqlx::query_scalar(
        "INSERT INTO notifications (notification_type, user_id, data, read, high_priority, \
                                    created_at, updated_at) \
         VALUES (1, $1, '{}', FALSE, FALSE, now(), now()) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let from = app_state.bus.now().await.unwrap();

    let reply = client
        .send(Method::PUT, "/notifications/mark-read.json", &[], "")
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);

    let channel = format!("/notification/{user_id}");
    let reply = client
        .get(&format!("/bus/poll?channels={channel}&position={from}"))
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let poll = reply.json();
    let messages = poll["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1, "{poll}");
    assert_eq!(messages[0]["channel"], channel.as_str());
    let data = &messages[0]["data"];
    assert_eq!(data["unread_notifications"], 0, "{data}");
    assert_eq!(data["all_unread_notifications_count"], 0, "{data}");
    // Newest first, and the seed's notifications were marked read with it.
    let recent = data["recent"].as_array().unwrap();
    assert_eq!(recent[0], serde_json::json!([notification_id, true]));
    assert!(recent.iter().all(|r| r[1] == true), "{data}");
    assert_eq!(
        data["last_notification"]["notification"]["id"],
        notification_id
    );
    assert_eq!(poll["position"], messages[0]["id"]);

    // Nobody else holds the user's tag: not an anonymous viewer, not
    // another user.
    let mut conn = db.pool.acquire().await.unwrap();
    let other: i32 = sqlx::query_scalar("SELECT id FROM users WHERE username = 'user2'")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    for viewer in [None, Some(other)] {
        let tags = discourse_rs::bus::tags(&mut conn, viewer).await.unwrap();
        let heard = app_state
            .bus
            .backlog(
                from,
                &pg_bus::Filter {
                    channels: vec![channel.clone()],
                    tags,
                },
                10,
            )
            .await
            .unwrap();
        assert!(heard.is_empty(), "{viewer:?} heard {heard:?}");
    }

    // A malformed position is refused rather than read as "now".
    let reply = client
        .get(&format!("/bus/poll?channels={channel}&position=nope"))
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
}

/// A member's topic page carries the composer and their CSRF token, and
/// posting as composer.js does (form fields, the token and XHR headers)
/// makes the reply, which the live stream then delivers.
#[tokio::test(flavor = "multi_thread")]
async fn members_reply_from_the_topic_page() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut client = Client::new(app_state.clone());
    let reply = client.login("user1", "password").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);

    let page = client.get("/t/parity-fixture-replies-and-posters/35").await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains(r#"id="reply-control""#), "{}", page.body);
    assert!(page.body.contains(r#"data-topic-id="35" id="topic""#));
    // hx-headers is single-quoted JSON; only the token itself is escaped.
    let marker = r#""X-CSRF-Token": ""#;
    let at = page.body.find(marker).expect("the body carries the token");
    let rest = &page.body[at + marker.len()..];
    let token = html_escape::decode_html_entities(&rest[..rest.find('"').unwrap()]).into_owned();

    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE topic_id = 35")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    client.csrf = Some(token);
    let reply = client
        .send(
            Method::POST,
            "/posts",
            &[
                ("x-requested-with", "XMLHttpRequest"),
                ("hx-request", "true"),
            ],
            "topic_id=35&raw=A+reply+from+the+topic+page%2C+long+enough+to+pass.",
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE topic_id = 35")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(after, before + 1);
}

/// The token the login page's form sends, from its hx-headers.
fn form_token(html: &str) -> String {
    let marker = r#""X-CSRF-Token": ""#;
    let at = html.find(marker).expect("the form carries a token");
    let rest = &html[at + marker.len()..];
    html_escape::decode_html_entities(&rest[..rest.find('"').unwrap()]).into_owned()
}

/// Logging in from the page as its script does: the form's token on
/// POST /session, then static#enter to where the reader was going.
#[tokio::test(flavor = "multi_thread")]
async fn the_login_page_logs_in() {
    for login_required in [false, true] {
        let db = TestDb::new().await;
        if login_required {
            set_setting(&db.pool, "login_required", BOOL, "t").await;
        }
        let mut client = Client::new(state(db.pool.clone(), config(RailsEnv::Test, &[])).await);

        let page = client.get("/login").await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.body.contains(r#"hx-post="/session""#), "{}", page.body);
        assert!(
            client.cookie("_forum_session").is_some(),
            "the token lives in the session the page started"
        );
        // htmx is served even while reading needs an account.
        assert_eq!(
            client.get("/assets/htmx.min.js").await.status,
            StatusCode::OK
        );

        client.csrf = Some(form_token(&page.body));
        let refused = client
            .send(
                Method::POST,
                "/session",
                &[("x-requested-with", "XMLHttpRequest")],
                "login=user1&password=wrong",
            )
            .await;
        assert_eq!(refused.status, StatusCode::OK, "Rails answers 200");
        assert!(refused.json()["error"].is_string(), "{}", refused.body);

        let reply = client
            .send(
                Method::POST,
                "/session",
                &[("x-requested-with", "XMLHttpRequest")],
                "login=user1&password=password",
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_eq!(reply.json()["user"]["username"], "user1", "{}", reply.body);

        let enter = client
            .send(Method::POST, "/login", &[], "redirect=%2Flatest")
            .await;
        assert_eq!(
            enter.status,
            StatusCode::FOUND,
            "login_required={login_required}"
        );
        assert!(
            enter
                .headers
                .iter()
                .any(|(k, v)| k == "location" && v == "http://test.localhost/latest"),
            "{:?}",
            enter.headers
        );
    }
}

#[tokio::test]
async fn no_login_form_without_local_logins() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "enable_local_logins", BOOL, "f").await;
    let mut client = Client::new(state(db.pool.clone(), config(RailsEnv::Test, &[])).await);
    let page = client.get("/login").await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(!page.body.contains("hx-post="), "{}", page.body);
}

impl Client {
    /// A GET whose body is streamed (an event stream), with the client's
    /// cookies.
    async fn open(&self, path: &str) -> Body {
        let cookie: Vec<String> = self
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let response = discourse_rs::app(self.state.clone())
            .oneshot(
                Request::get(path)
                    .header(header::HOST, "test.localhost")
                    .header(header::COOKIE, cookie.join("; "))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        response.into_body()
    }
}

/// A member's list pages show their new and unread counts from their
/// tracking state, as the reference does: in the New pill (with unified
/// new, which folds unread into it and drops the Unread pill; the count is
/// the /new list's) and as dots on the sidebar's Topics, category and tag
/// links. The page's stream sends them again, for the pill and sidebar it
/// names.
#[tokio::test(flavor = "multi_thread")]
async fn list_pages_show_the_members_unread_and_new_counts() {
    let db = TestDb::new().await;
    let mut client = Client::new(state(db.pool.clone(), config(RailsEnv::Test, &[])).await);
    let reply = client.login("user1", "password").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);

    let new_list = client.get("/new.json").await.json();
    assert_eq!(
        new_list["topic_list"]["topics"].as_array().unwrap().len(),
        2
    );
    let page = client.get("/latest").await.body;
    assert!(page.contains(
        r#"<li class="new nav-item_new" title="topics created or replied to in the last few days">"#
    ));
    assert!(page.contains(r#"href="/new">New (2)</a>"#));
    assert!(!page.contains("nav-item_unread"));
    assert!(
        page.contains("&#38;sidebar=discovery&#38;nav=latest\""),
        "{page}"
    );
    let dot = r#"<span class="sidebar-section-link-suffix icon unread">"#;
    let link = |page: &str, marker: &str| -> String {
        let start = page.find(marker).unwrap_or_else(|| panic!("{marker}"));
        page[start..start + page[start..].find("</li>").unwrap()].to_string()
    };
    // Topic 34 (Sub General's definition) is new, topic 35 in General,
    // tagged guide, unread.
    assert!(link(&page, r#"data-list-item-name="everything""#).contains(dot));
    assert!(link(&page, r#"data-category-id="4""#).contains(dot));
    assert!(!link(&page, r#"data-category-id="2""#).contains(dot));

    let mut body = client.open("/live?nav=latest&sidebar=discovery").await;
    let mut buffer = String::new();
    let html = common::next_sse_event(&mut body, &mut buffer, "list").await;
    assert!(html.starts_with(
        r#"<a hx-swap-oob="innerHTML:#navigation-bar > li.nav-item_new > a">New (2)</a>"#
    ));
    let everything = link(
        &html,
        r#"hx-swap-oob="outerHTML:#d-sidebar li[data-list-item-name='everything']""#,
    );
    assert!(everything.contains(r#"class="active sidebar-section-link sidebar-row""#));
    assert!(everything.contains(dot));
    assert!(link(&html, "li[data-category-id='4']").contains(dot));
}

/// A member's header: every page connects, the unread notification count
/// arrives first and follows their notification state, and alerts show
/// with their user content escaped.
#[tokio::test(flavor = "multi_thread")]
async fn members_hear_their_notifications_on_every_page() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut client = Client::new(app_state.clone());
    let reply = client.login("user1", "password").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let user_id: i32 = sqlx::query_scalar(
        "UPDATE users SET last_seen_at = now() WHERE username = 'user1' RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    // One unread notification newer than the last seen.
    sqlx::query(
        "INSERT INTO notifications (notification_type, user_id, data, read, high_priority, \
                                    created_at, updated_at) \
         VALUES (1, $1, '{}', FALSE, FALSE, now(), now())",
    )
    .bind(user_id)
    .execute(&db.pool)
    .await
    .unwrap();

    let page = client.get("/categories").await;
    assert!(page.body.contains(
        r#"<span id="notification-count" class="badge-notification unread-notifications"></span>"#
    ));
    assert!(
        page.body.contains(r#"sse-connect="/live?position="#),
        "{}",
        page.body
    );

    let mut conn = db.pool.acquire().await.unwrap();
    let unread = discourse_rs::bus::all_unread_notifications_count(&mut conn, user_id)
        .await
        .unwrap();
    drop(conn);
    assert!(unread > 0);
    let mut body = client.open("/live").await;
    let mut buffer = String::new();
    assert_eq!(
        common::next_sse_event(&mut body, &mut buffer, "header").await,
        format!(
            r#"<span id="notification-count" class="badge-notification unread-notifications" hx-swap-oob="true">{unread}</span>"#
        )
    );

    // An alert, its user content escaped.
    let mut tx = db.pool.begin().await.unwrap();
    discourse_rs::bus::publish_notification_alert(
        &app_state.bus,
        &mut tx,
        user_id,
        &serde_json::json!({
            "notification_type": 2,
            "username": "user2",
            "topic_title": "A <b>bold</b> title",
            "excerpt": "<script>alert(1)</script>",
            "post_url": "/t/a-topic/35/2",
        }),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let alert = common::next_sse_event(&mut body, &mut buffer, "header").await;
    assert!(
        alert.starts_with(r#"<div id="notification-alert""#),
        "{alert}"
    );
    assert!(alert.contains(r#"href="/t/a-topic/35/2""#), "{alert}");
    assert!(
        alert.contains("<strong>user2</strong> replied in"),
        "{alert}"
    );
    assert!(alert.contains("&lt;script&gt;"), "{alert}");
    assert!(!alert.contains("<script>"), "{alert}");
    assert!(!alert.contains("<b>"), "{alert}");

    // Reading them all clears the count.
    let reply = client
        .send(Method::PUT, "/notifications/mark-read.json", &[], "")
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        common::next_sse_event(&mut body, &mut buffer, "header").await,
        r#"<span id="notification-count" class="badge-notification unread-notifications" hx-swap-oob="true"></span>"#
    );
}

/// A member likes a post from the topic page as its button does, and the
/// post comes back on their stream as liked, with the button to undo it.
#[tokio::test(flavor = "multi_thread")]
async fn members_like_from_the_topic_page() {
    let db = TestDb::new().await;
    let mut client = Client::new(state(db.pool.clone(), config(RailsEnv::Test, &[])).await);
    let reply = client.login("user1", "password").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let (post_id, post_number): (i32, i32) = sqlx::query_as(
        "SELECT p.id, p.post_number FROM posts p JOIN users u ON u.id = p.user_id \
         WHERE p.topic_id = 35 AND p.post_type = 1 AND p.deleted_at IS NULL \
           AND u.username <> 'user1' \
           AND NOT EXISTS (SELECT 1 FROM post_actions a WHERE a.post_id = p.id \
                             AND a.post_action_type_id = 2 AND a.user_id = \
                               (SELECT id FROM users WHERE username = 'user1')) \
         ORDER BY p.post_number LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();

    let page = client.get("/t/parity-fixture-replies-and-posters/35").await;
    assert!(
        page.body.contains(&format!(
            r#"hx-vals='{{"id": {post_id}, "post_action_type_id": 2}}'"#
        )),
        "{}",
        page.body
    );
    let marker = r#""X-CSRF-Token": ""#;
    let at = page.body.find(marker).expect("the body carries the token");
    let rest = &page.body[at + marker.len()..];
    client.csrf =
        Some(html_escape::decode_html_entities(&rest[..rest.find('"').unwrap()]).into_owned());

    let from = {
        let marker = r#"<meta name="bus-position" content=""#;
        let at = page.body.find(marker).unwrap() + marker.len();
        page.body[at..at + page.body[at..].find('"').unwrap()].to_string()
    };
    let mut body = client
        .open(&format!("/live?topic=35&tail=1&position={from}"))
        .await;
    let mut buffer = String::new();

    let reply = client
        .send(
            Method::POST,
            "/post_actions",
            &[
                ("x-requested-with", "XMLHttpRequest"),
                ("hx-request", "true"),
            ],
            &format!("id={post_id}&post_action_type_id=2"),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let html = common::next_sse_event(&mut body, &mut buffer, "post").await;
    assert!(html.contains(&post_swap(post_number)), "{html}");
    assert!(
        html.contains(&format!(r#"hx-delete="/post_actions/{post_id}""#)),
        "the member's own render shows their like: {html}"
    );
    assert!(
        html.contains("post-action-menu__like toggle-like btn-icon has-like btn-flat"),
        "{html}"
    );
}

/// A member bookmarks a post from the topic page as its button does: the
/// bookmark, then the post fetched again as they now see it.
#[tokio::test(flavor = "multi_thread")]
async fn members_bookmark_from_the_topic_page() {
    let db = TestDb::new().await;
    let mut client = Client::new(state(db.pool.clone(), config(RailsEnv::Test, &[])).await);
    let reply = client.login("user1", "password").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let (post_id, post_number): (i32, i32) = sqlx::query_as(
        "SELECT p.id, p.post_number FROM posts p WHERE p.topic_id = 35 AND p.post_type = 1 \
           AND p.deleted_at IS NULL \
           AND NOT EXISTS (SELECT 1 FROM bookmarks b WHERE b.bookmarkable_type = 'Post' \
                             AND b.bookmarkable_id = p.id \
                             AND b.user_id = (SELECT id FROM users WHERE username = 'user1')) \
         ORDER BY p.post_number LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();

    let page = client.get("/t/parity-fixture-replies-and-posters/35").await;
    assert!(
        page.body.contains(&format!(
            r#"hx-vals='{{"bookmarkable_id": {post_id}, "bookmarkable_type": "Post"}}'"#
        )),
        "{}",
        page.body
    );
    let marker = r#""X-CSRF-Token": ""#;
    let at = page.body.find(marker).expect("the body carries the token");
    let rest = &page.body[at + marker.len()..];
    client.csrf =
        Some(html_escape::decode_html_entities(&rest[..rest.find('"').unwrap()]).into_owned());

    let headers = [
        ("x-requested-with", "XMLHttpRequest"),
        ("hx-request", "true"),
    ];
    let made = client
        .send(
            Method::POST,
            "/bookmarks",
            &headers,
            &format!("bookmarkable_id={post_id}&bookmarkable_type=Post"),
        )
        .await;
    assert_eq!(made.status, StatusCode::OK, "{}", made.body);
    let bookmark_id = made.json()["id"].as_i64().expect("the new bookmark's id");

    let html = client.get(&format!("/live/post/{post_id}")).await;
    assert_eq!(html.status, StatusCode::OK);
    assert!(html.body.contains(&post_swap(post_number)), "{}", html.body);
    assert!(
        html.body
            .contains(&format!(r#"hx-delete="/bookmarks/{bookmark_id}""#)),
        "{}",
        html.body
    );
    assert!(
        html.body
            .contains("bookmark-menu__trigger btn-icon no-text bookmarked")
    );

    let removed = client
        .send(
            Method::DELETE,
            &format!("/bookmarks/{bookmark_id}"),
            &headers,
            "",
        )
        .await;
    assert_eq!(removed.status, StatusCode::OK, "{}", removed.body);
    let html = client.get(&format!("/live/post/{post_id}")).await;
    assert!(
        html.body.contains(r#"hx-post="/bookmarks""#),
        "{}",
        html.body
    );
}

/// The post endpoint shows what the viewer may see: no bookmark button for
/// an anonymous reader, and nothing at all of a post they cannot see.
#[tokio::test]
async fn post_fragments_follow_what_the_viewer_may_see() {
    let db = TestDb::new().await;
    let mut client = Client::new(state(db.pool.clone(), config(RailsEnv::Test, &[])).await);
    let public: i32 =
        sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = 35 AND post_number = 1")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let html = client.get(&format!("/live/post/{public}")).await;
    assert_eq!(html.status, StatusCode::OK);
    assert!(!html.body.contains("bookmark"), "{}", html.body);

    let private: i32 = sqlx::query_scalar(
        "SELECT p.id FROM posts p JOIN topics t ON t.id = p.topic_id \
         WHERE t.archetype = 'private_message' ORDER BY p.id LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        client.get(&format!("/live/post/{private}")).await.status,
        StatusCode::NOT_FOUND
    );
}

/// The header's user menu: a member's recent notifications, each linking
/// to its post; opening it marks them seen, which clears the header's
/// count over the live stream.
#[tokio::test(flavor = "multi_thread")]
async fn the_user_menu_lists_notifications_and_clears_the_count() {
    let db = TestDb::new().await;
    let mut client = Client::new(state(db.pool.clone(), config(RailsEnv::Test, &[])).await);
    assert_ne!(
        client.get("/user-menu").await.status,
        StatusCode::OK,
        "members only"
    );
    let reply = client.login("user1", "password").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let user_id: i32 = sqlx::query_scalar(
        "UPDATE users SET last_seen_at = now() WHERE username = 'user1' RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO notifications (notification_type, user_id, topic_id, post_number, data, read, \
                                    high_priority, created_at, updated_at) \
         VALUES (1, $1, 35, 2, $2, FALSE, FALSE, now(), now())",
    )
    .bind(user_id)
    .bind(
        serde_json::json!({
            "topic_title": "Parity fixture: replies and posters",
            "display_username": "user2",
            "original_post_id": 1,
        })
        .to_string(),
    )
    .execute(&db.pool)
    .await
    .unwrap();

    let page = client.get("/latest").await;
    assert!(
        page.body.contains(r#"hx-get="/user-menu""#),
        "{}",
        page.body
    );
    let mut body = client.open("/live").await;
    let mut buffer = String::new();
    let count = common::next_sse_event(&mut body, &mut buffer, "header").await;
    assert_ne!(
        count,
        r#"<span id="notification-count" class="badge-notification unread-notifications" hx-swap-oob="true"></span>"#,
        "something unread before the menu opens"
    );

    let menu = client.get("/user-menu").await;
    assert_eq!(menu.status, StatusCode::OK);
    assert!(
        menu.body.contains(
            r#"<li class="notification unread"><a href="/t/parity-fixture-replies-and-posters/35/2"><strong>user2</strong> mentioned you in"#
        ),
        "{}",
        menu.body
    );
    assert!(menu.body.contains("Mark all read"));
    assert_eq!(
        common::next_sse_event(&mut body, &mut buffer, "header").await,
        r#"<span id="notification-count" class="badge-notification unread-notifications" hx-swap-oob="true"></span>"#,
        "seen now"
    );
}

/// A member's sidebar: their community links, their own categories, a dot
/// on My posts while they have drafts; an admin's review and admin links.
#[tokio::test]
async fn members_get_their_own_sidebar() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;

    let mut client = Client::new(app_state.clone());
    client.login("user1", "password").await;
    let page = client.get("/latest").await.body;
    assert!(
        page.contains(r#"<div class="sidebar-sections"><div class="sidebar-custom-sections">"#)
    );
    assert!(page.contains(
        r#"<a class="sidebar-section-link sidebar-row" title="My recent topic activity" data-link-name="my-posts" href="/u/user1/activity">"#
    ));
    assert!(page.contains(r#"data-link-name="my-messages" href="/u/user1/messages""#));
    assert!(page.contains(r#"data-link-name="invite""#));
    assert!(!page.contains(r#"data-link-name="review""#));
    assert!(!page.contains(r#"data-link-name="admin""#));
    assert!(!page.contains("configure-default-navigation-menu"));

    // The section follows their own category links (General and Site
    // Feedback in the fixture).
    sqlx::query(
        "DELETE FROM sidebar_section_links WHERE linkable_type = 'Category' AND linkable_id = 4 \
         AND user_id = (SELECT id FROM users WHERE username = 'user1')",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    // Drafts move My posts to them, with a dot; with unified new (on in
    // the fixture) it reads My drafts.
    sqlx::query(
        "UPDATE user_stats SET draft_count = 2 WHERE user_id = (SELECT id FROM users WHERE username = 'user1')",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let page = client.get("/latest").await.body;
    assert!(page.contains(r#"<li class="sidebar-section-link-wrapper" data-category-id="2">"#));
    assert!(!page.contains(r#"<li class="sidebar-section-link-wrapper" data-category-id="4">"#));
    assert!(page.contains(
        r#"title="My unposted drafts" data-link-name="my-posts" href="/u/user1/activity/drafts">"#
    ));
    assert!(page.contains(
        r#"My drafts</span><span class="sidebar-section-link-suffix icon unread"><svg class="fa d-icon d-icon-circle "#
    ));

    let mut admin = Client::new(app_state.clone());
    admin.login("admin", "password").await;
    let page = admin.get("/latest").await.body;
    assert!(page.contains(r#"data-link-name="review" href="/review""#));
    assert!(page.contains(r#"title="Admin" data-link-name="admin" href="/admin""#));
    assert!(
        page.contains(r#"<li class="sidebar-section-link-wrapper" data-category-id="3">"#),
        "Staff"
    );
    assert!(page.contains(
        r#"data-link-name="configure-default-navigation-menu-tags" href="/admin/site_settings/category/sidebar?filter=default_navigation_menu_tags""#
    ));
}

/// A member's topic: the post menu collapses its hidden items behind show
/// more, the footer has flag, defer and the notification level with its
/// reason, and the browse-more line counts what is left to read.
#[tokio::test]
async fn members_get_the_topic_controls() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut client = Client::new(app_state.clone());
    client.login("user0", "password").await;
    let page = client
        .get("/t/parity-fixture-replies-and-posters/35")
        .await
        .body;

    // post_menu_hidden_items: flag, bookmark and (their own reply's)
    // delete hide behind show more; their own first post offers the
    // delete-topic-disallowed button.
    assert!(page.contains(r#"<nav class="post-controls collapsed" role="none">"#));
    assert!(page.contains(
        r#"class="btn no-text btn-icon post-action-menu__show-more show-more-actions btn-flat""#
    ));
    assert!(page.contains(r#"<button hidden aria-label="privately flag this post for attention or send a personal message about it" class="btn no-text btn-icon post-action-menu__flag create-flag btn-flat""#));
    assert!(page.contains(
        r#"<button hidden aria-label="you don&#x27;t have permission to delete this topic""#
    ));
    assert!(page.contains(r#"<button hidden aria-label="delete this post" class="btn no-text btn-icon post-action-menu__delete delete btn-flat" hx-delete="/posts/52""#));

    // The footer.
    assert!(page.contains(r#"id="topic-footer-button-flag""#));
    // Mark unread unreads the last post, then goes home.
    assert!(page.contains(
        r#"id="topic-footer-button-defer" title="Mark topic as unread" data-defer-url="/t/35/timings.json?last=1" data-defer-to="/""#
    ));
    assert!(page.contains(r#"data-level-id="3" data-level-name="watching""#));
    assert!(page.contains(
        r#"<span class="text">You will receive notifications because you created this topic.</span>"#
    ));
    // The level menu: each option posts its level, the current selected,
    // and carries the reason it shows once chosen (the reason cleared).
    assert!(page.contains(
        r#"<button class="btn no-text notifications-tracking-btn -selected" data-level-id="3" data-level-name="watching""#
    ));
    assert!(page.contains(
        r#"data-reason="You will see a count of new replies because you &lt;a href&#x3D;&quot;/u/user0/preferences/notifications&quot;&gt;read this topic&lt;/a&gt;." hx-post="/t/35/notifications" hx-vals='{"notification_level": 2}'"#
    ));
    // This topic counts as read; the others are new.
    assert!(page.contains(r#"There are <a href="/new?subset=topics">3 new</a> topics remaining,"#));

    // Anonymous readers keep the plain line and no controls.
    let mut anon = Client::new(app_state.clone());
    let page = anon
        .get("/t/parity-fixture-replies-and-posters/35")
        .await
        .body;
    assert!(!page.contains("post-action-menu__show-more"));
    assert!(!page.contains("notifications-tracking-trigger"));
    assert!(page.contains("Want to read more?"));
}

/// One's own profile opens the activity stream (user/index); its filters
/// load their user actions, and others' profiles keep the summary.
#[tokio::test]
async fn own_profile_opens_the_activity_stream() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut client = Client::new(app_state.clone());
    client.login("user0", "password").await;

    let page = client.get("/u/user0").await.body;
    assert!(page.contains(" user-activity-page\""));
    assert!(page.contains(
        r#"<li aria-current="page" class="user-nav__activity"><a class="active" href="/u/user0/activity">"#
    ));
    assert!(page.contains(
        r#"<li class="user-nav__personal-messages"><a class="" href="/u/user0/messages">"#
    ));
    assert!(page.contains(
        r#"<li aria-current="location" class="user-nav__activity-all"><a class="active" href="/u/user0/activity">"#
    ));
    assert!(page.contains(r#"class="user-nav__activity-drafts""#));
    // My posts is the current sidebar link.
    assert!(page.contains(
        r#"<a class="active sidebar-section-link sidebar-row" title="My recent topic activity" data-link-name="my-posts""#
    ));
    // Topics and replies, newest first, linked to the post.
    assert!(page.contains(r#"<div class="post-list user-stream">"#));
    assert!(page.contains(
        r#"<a aria-label="Parity fixture: replies and posters - post #5" href="/t/parity-fixture-replies-and-posters/35/5">"#
    ));
    assert!(page.contains(
        r#"<div class="excerpt" data-post-id="52" data-topic-id="35" data-user-id="2">"#
    ));
    assert!(page.contains(r#"<span class="topic-status --archived""#));
    // The collapsed summary (no about panel) and no summary stats.
    assert!(page.contains(r#"<section class="collapsed-info about no-background">"#));
    assert!(!page.contains("stats-section"));

    // Replies are the user's own posts (TYPES.posts), not responses.
    let page = client.get("/u/user0/activity/replies").await.body;
    assert!(page.contains(r#"<div class="post-list user-stream filter-5">"#));
    assert!(page.contains("Reply from user0 mentioning"));
    assert!(!page.contains("Reply one from user1"));

    // Likes collapse onto the liked post, with the liker under a heart.
    let page = client.get("/u/user0/activity/likes-given").await.body;
    assert!(page.contains(r#"<div class="post-list user-stream filter-1">"#));
    assert!(
        page.contains(
            r#"<div class="user-stream-item-actions"><svg class="fa d-icon d-icon-heart "#
        )
    );
    assert!(page.contains(r#"<a class="avatar-link" data-user-card="user0" href="/u/user0"><div class="avatar-wrapper"><img alt="" width="24" height="24""#));

    // Another member's profile: the summary, no Messages or Drafts.
    let page = client.get("/u/user1").await.body;
    assert!(page.contains(" user-summary-page\""));
    assert!(!page.contains("user-nav__personal-messages"));
    let page = client.get("/u/user1/activity").await.body;
    assert!(page.contains(r#"class="post-list user-stream""#));
    assert!(!page.contains("user-nav__activity-drafts"));
    assert!(!page.contains(
        r#"class="active sidebar-section-link sidebar-row" title="My recent topic activity""#
    ));
}

/// How a live update replaces a post on the page: the post's wrapper, or
/// the first post's main row, which keeps its topic map.
fn post_swap(post_number: i32) -> String {
    if post_number == 1 {
        r#"hx-swap-oob="outerHTML:#post_1 > .post__row:has(> .post__body)""#.to_string()
    } else {
        format!(r#"hx-swap-oob="outerHTML:#posts > [data-post-number='{post_number}']""#)
    }
}

/// A member's pages carry the composer (and the lists a New Topic button);
/// /raw serves a post's markdown to edit, only to who can see it.
#[tokio::test]
async fn members_get_the_composer_and_raw_posts() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;

    let mut anon = Client::new(app_state.clone());
    let page = anon.get("/latest").await.body;
    assert!(!page.contains(r#"id="reply-control""#));
    assert!(!page.contains(r#"id="create-topic""#));

    let mut client = Client::new(app_state.clone());
    client.login("user1", "password").await;
    let page = client.get("/latest").await.body;
    assert!(page.contains(r#"<div id="reply-control" class="closed hide-preview""#));
    assert!(page.contains(&format!(
        r#"<script src="{}" defer></script>"#,
        discourse_rs::assets::url("", "composer.js")
    )));
    assert!(page.contains(r#"id="create-topic" type="button">"#));
    // The chooser: the categories user1 may create topics in, the
    // default_composer_category selected.
    assert!(
        page.contains(r#"class="select-kit single-select combobox combo-box category-chooser"#)
    );
    assert!(page.contains(r#"class="category-row select-kit-row" data-name="Site Feedback""#));

    // Their own post can be edited from its menu.
    let page = client
        .get("/t/parity-fixture-replies-and-posters/35")
        .await
        .body;
    assert!(page.contains(r#"class="btn no-text btn-icon post-action-menu__edit edit btn-flat" data-post-id="36" data-post-number="2""#));
    assert!(page.contains(r#"<section class="topic-area" data-topic-id="35" id="topic">"#));
    // A member's reading is timed (static/js/screen-track.js).
    assert!(page.contains(&format!(
        r#"<script src="{}" defer></script>"#,
        discourse_rs::assets::url("", "screen-track.js")
    )));
    let anon_topic = anon
        .get("/t/parity-fixture-replies-and-posters/35")
        .await
        .body;
    assert!(!anon_topic.contains("screen-track.js"));
    assert_eq!(
        client.get("/assets/screen-track.js").await.status,
        StatusCode::OK
    );
    assert!(
        !page.contains(r#"<form class="reply""#),
        "the composer replaces the form"
    );

    let raw = client.get("/raw/35/2").await;
    assert_eq!(raw.status, StatusCode::OK);
    assert!(
        raw.headers
            .iter()
            .any(|(k, v)| k == "content-type" && v.starts_with("text/plain"))
    );
    assert!(raw.body.starts_with("Reply one from user1"), "{}", raw.body);
    assert_eq!(client.get("/raw/35/99").await.status, StatusCode::NOT_FOUND);
}

/// The composer's preview: its toggle and where it loads the renderer and
/// the settings from, which cook as the server does.
#[tokio::test]
async fn the_composer_preview_loads_the_renderer_and_settings() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut client = Client::new(app_state.clone());
    client.login("user1", "password").await;
    let page = client.get("/latest").await.body;
    assert!(page.contains(r#"class="btn no-text btn-icon btn-transparent btn-mini-toggle toggle-preview" title="hide preview""#));
    let wasm_url = page
        .split(r#"data-preview-wasm=""#)
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("the composer names the renderer")
        .replace("&#x3D;", "=");
    assert!(
        wasm_url.starts_with("/assets/markdown.wasm?v="),
        "{wasm_url}"
    );
    assert!(page.contains(r#"data-preview-settings="/assets/markdown-settings.json" data-label-show-preview="show preview" data-label-hide-preview="hide preview""#));

    let wasm = client.get(&wasm_url).await;
    assert_eq!(wasm.status, StatusCode::OK);
    assert!(wasm.body.starts_with("\0asm"));
    assert!(
        wasm.headers
            .iter()
            .any(|(k, v)| k == "content-type" && v == "application/wasm")
    );

    let settings = client.get("/assets/markdown-settings.json").await;
    assert_eq!(settings.status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&settings.body).unwrap();
    assert_eq!(json["emoji_set"], "twitter");
    assert_eq!(json["breaks"], true);
    assert!(
        json.get("linkify").is_none(),
        "compiled where it is received"
    );
    // What the preview renders with them is what the post cooks to.
    let render =
        serde_json::from_str::<discourse_rs::pretty_text::render::RenderSettings>(&settings.body)
            .unwrap()
            .compiled()
            .unwrap();
    let (html, _) = discourse_rs::pretty_text::render::render(
        "**hi** :smile: https://example.com",
        &render,
        Default::default(),
    )
    .unwrap();
    assert_eq!(
        html,
        r#"<p><strong>hi</strong> <img src="/images/emoji/twitter/smile.png?v=15" title=":smile:" class="emoji" alt=":smile:" loading="lazy" width="20" height="20"> <a href="https://example.com">https://example.com</a></p>"#
    );
}

/// GET /drafts.json: the member's drafts as the reference serializes the
/// same four (Draft.stream through DraftSerializer, recorded with the
/// drafts made and rolled back): a reply's (shown under the topic's
/// author), a long new topic's (excerpt cut, truncated), an edit's (the
/// post's author) and one on a topic they cannot see (no topic fields).
#[tokio::test]
async fn the_drafts_list_matches_the_reference() {
    let db = TestDb::new().await;
    let mut client = Client::new(state(db.pool.clone(), config(RailsEnv::Test, &[])).await);
    client.login("user1", "password").await;
    let post_id: i32 =
        sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = 38 AND post_number = 1")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(post_id, 42, "the reference's post");
    let long = "x".repeat(350);
    let drafts = [
        (
            "topic_35",
            r#"{"reply":"Hello **world** :smile:","action":"reply","reply_to_post_number":null}"#
                .to_string(),
        ),
        (
            "new_topic_1",
            format!(r#"{{"reply":"{long}","title":"T","action":"createTopic","categoryId":4}}"#),
        ),
        (
            "topic_38",
            r#"{"reply":"edit","action":"edit","postId":42}"#.to_string(),
        ),
        (
            "topic_2",
            r#"{"reply":"staff","action":"reply"}"#.to_string(),
        ),
    ];
    for (key, data) in &drafts {
        let current = client.get(&format!("/drafts/{key}.json")).await.json()["draft_sequence"]
            .as_i64()
            .unwrap();
        let body = form_urlencoded::Serializer::new(String::new())
            .append_pair("draft_key", key)
            .append_pair("sequence", &current.to_string())
            .append_pair("data", data)
            .append_pair("owner", "test")
            .finish();
        let reply = client
            .send(
                Method::POST,
                "/drafts.json",
                &[("x-requested-with", "XMLHttpRequest")],
                &body,
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    }

    let reply = client.get("/drafts.json").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let list = reply.json()["drafts"].as_array().unwrap().clone();
    let by_key = |key: &str| -> Value {
        let mut d = list
            .iter()
            .find(|d| d["draft_key"] == key)
            .unwrap_or_else(|| panic!("{key}"))
            .clone();
        assert!(d["created_at"].is_string());
        d.as_object_mut().unwrap().remove("created_at");
        d
    };
    let avatar = "/letter_avatar_proxy/v4/letter/u/5daacb/{size}.png";
    let mut reply_draft = by_key("topic_35");
    // The reference's user1 has posted to the topic more often.
    assert!(reply_draft["sequence"].is_i64());
    reply_draft["sequence"] = json!(3);
    assert_eq!(
        reply_draft,
        json!({"excerpt":"Hello **world** :smile:","draft_key":"topic_35","sequence":3,"draft_username":"user1","avatar_template":avatar,"data":drafts[0].1,"topic_id":35,"username":"user0","username_lower":"user0","name":"User0","user_id":3,"title":"Parity fixture: replies and posters","slug":"parity-fixture-replies-and-posters","category_id":4,"archetype":"regular"})
    );
    assert_eq!(
        by_key("new_topic_1"),
        json!({"excerpt":format!("{}&hellip;", "x".repeat(300)),"truncated":true,"draft_key":"new_topic_1","sequence":0,"draft_username":"user1","avatar_template":avatar,"data":drafts[1].1,"topic_id":null,"username":"user1","username_lower":"user1","name":"User1","user_id":3,"title":null,"archetype":null})
    );
    assert_eq!(
        by_key("topic_38"),
        json!({"excerpt":"edit","draft_key":"topic_38","sequence":0,"draft_username":"user1","avatar_template":avatar,"data":drafts[2].1,"topic_id":38,"username":"user1","username_lower":"user1","name":"User1","user_id":3,"title":"Parity fixture: tagged in a subcategory","slug":"parity-fixture-tagged-in-a-subcategory","category_id":34,"archetype":"regular"})
    );
    assert_eq!(
        by_key("topic_2"),
        json!({"excerpt":"staff","draft_key":"topic_2","sequence":0,"draft_username":"user1","avatar_template":avatar,"data":drafts[3].1,"topic_id":2,"username":"user1","username_lower":"user1","name":"User1","user_id":3,"title":null,"archetype":null})
    );
    assert_eq!(
        client.get("/drafts.json?limit=51").await.status,
        StatusCode::BAD_REQUEST
    );
}

/// The composer's drafts: the New Topic button gains its drafts menu once
/// the member has a draft, and a new topic posted with its draft's key
/// (PostCreator's draft_key) takes the draft with it.
#[tokio::test]
async fn a_new_topic_takes_its_draft_and_the_drafts_menu_follows_the_count() {
    let db = TestDb::new().await;
    let mut client = Client::new(state(db.pool.clone(), config(RailsEnv::Test, &[])).await);
    client.login("user1", "password").await;
    let page = client.get("/latest").await.body;
    assert!(page.contains(r#"<div class="d-combo-button topic-create-button__combo""#));
    assert!(!page.contains("topic-drafts-menu-trigger"));
    assert!(page.contains(r#"<div id="draft-status" hidden><span class="draft-error" title="">"#));

    let key = "new_topic_1791297368548";
    let body = form_urlencoded::Serializer::new(String::new())
        .append_pair("draft_key", key)
        .append_pair("sequence", "0")
        .append_pair(
            "data",
            r#"{"reply":"A body long enough to be a post here.","action":"createTopic","title":"A drafted topic title","categoryId":4}"#,
        )
        .append_pair("owner", "test")
        .finish();
    let reply = client
        .send(
            Method::POST,
            "/drafts.json",
            &[("x-requested-with", "XMLHttpRequest")],
            &body,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let page = client.get("/latest").await.body;
    assert!(page.contains(r#"<div class="d-combo-button --has-menu topic-create-button__combo""#));
    assert!(page.contains(r#"<button aria-expanded="false" aria-label="Open the latest drafts menu" class="btn no-text btn-icon fk-d-menu__trigger topic-drafts-menu-trigger d-combo-button-menu btn-primary" title="Open the latest drafts menu" data-identifier="topic-drafts-menu" data-trigger="" type="button" data-draft-count="1""#));

    let body = form_urlencoded::Serializer::new(String::new())
        .append_pair("raw", "A body long enough to be a post here.")
        .append_pair("title", "A drafted topic title")
        .append_pair("category", "4")
        .append_pair("draft_key", key)
        .finish();
    let reply = client
        .send(
            Method::POST,
            "/posts.json",
            &[("x-requested-with", "XMLHttpRequest")],
            &body,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let left: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM drafts WHERE draft_key = $1 AND user_id = (SELECT id FROM users WHERE username = 'user1')",
    )
    .bind(key)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(left, 0);
    assert!(
        !client
            .get("/latest")
            .await
            .body
            .contains("topic-drafts-menu-trigger")
    );
}

/// `Upload.base62_sha1`
fn base62(sha1: &str) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut n: Vec<u8> = (0..sha1.len() / 2)
        .map(|i| u8::from_str_radix(&sha1[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    let mut out = Vec::new();
    while n.iter().any(|b| *b != 0) {
        let mut rem = 0u32;
        for b in n.iter_mut() {
            let acc = (rem << 8) | u32::from(*b);
            *b = (acc / 62) as u8;
            rem = acc % 62;
        }
        out.push(DIGITS[rem as usize]);
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// The composer's uploads: the toolbar button, the file picker with the
/// extensions a member may send, the progress line; none of them when
/// nothing may be uploaded. POST /uploads/lookup-urls answers the
/// preview's upload:// urls with the ones that exist, for members only.
#[tokio::test]
async fn the_composer_uploads_and_resolves_short_urls() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut client = Client::new(app_state.clone());
    client.login("user1", "password").await;
    let page = client.get("/latest").await.body;
    assert!(page.contains(r#"<button class="btn no-text btn-icon toolbar__button upload" data-action="upload" tabindex="-1" title="Upload" type="button">"#), "{page}");
    assert!(page.contains(r#"<div class="pick-files-button"><input accept=".jpg,.jpeg,.png,.gif,.heic,.heif,.webp,.avif,.svg,.jxl" id="file-uploader" multiple type="file"></div>"#));
    assert!(page.contains(r#"<div id="file-uploading" hidden><div class="spinner small"></div><span></span><a href id="cancel-file-upload">"#));
    assert!(page.contains(r#"data-label-uploading-filename="Uploading: %{filename}…""#));

    let sha1 = "e9d71f5ee7c92d6dc9e92ffdad17b8bd49418f98";
    sqlx::query(
        "INSERT INTO uploads (user_id, original_filename, filesize, sha1, url, extension, created_at, updated_at) \
         VALUES (3, 'a.png', 10, $1, $2, 'png', now(), now())",
    )
    .bind(sha1)
    .bind(format!("/uploads/default/original/1X/{sha1}.png"))
    .execute(&db.pool)
    .await
    .unwrap();
    let short = format!("upload://{}.png", base62(sha1));
    let unknown = format!(
        "upload://{}.png",
        base62("0000000000000000000000000000000000000001")
    );
    let body = form_urlencoded::Serializer::new(String::new())
        .append_pair("short_urls[]", &short)
        .append_pair("short_urls[]", &unknown)
        .finish();
    let reply = client
        .send(
            Method::POST,
            "/uploads/lookup-urls",
            &[("x-requested-with", "XMLHttpRequest")],
            &body,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        reply.json(),
        json!([{
            "short_url": short,
            "url": format!("/uploads/default/original/1X/{sha1}.png"),
            "short_path": format!("/uploads/short-url/{}.png", base62(sha1)),
        }])
    );

    let mut anon = Client::new(app_state.clone());
    anon.fetch_csrf().await;
    let reply = anon
        .send(
            Method::POST,
            "/uploads/lookup-urls",
            &[("x-requested-with", "XMLHttpRequest")],
            &body,
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.body);

    set_setting(&db.pool, "authorized_extensions", 8, "").await;
    let page = client.get("/latest").await.body;
    assert!(!page.contains("file-uploader"));
    assert!(!page.contains("toolbar__button upload"));
}
