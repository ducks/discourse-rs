//! users#send_activation_email and users#update_activation_email: the
//! activation email of an account that is not active yet, sent again or
//! to another address. No recording covers either endpoint; every answer
//! here is read from users_controller.rb, UserEmail, EmailValidator and
//! EmailToken, and agrees with spec/requests/users_controller_spec.rb.

use crate::common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const BOOL: i32 = 5;
const LIST: i32 = 8;
const PASSWORD: &str = "a-long-enough-pass-phrase";
const SEND: &str = "/u/action/send_activation_email";
const UPDATE: &str = "/u/update-activation-email.json";

struct Client {
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
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

impl Client {
    fn new(state: &AppState) -> Client {
        Client {
            state: state.clone(),
            cookies: Vec::new(),
            csrf: None,
        }
    }

    /// A client with the session's CSRF token.
    async fn with_csrf(state: &AppState) -> Client {
        let mut client = Client::new(state);
        let reply = client.send(Method::GET, "/session/csrf.json", &[]).await;
        client.csrf = reply.json()["csrf"].as_str().map(str::to_string);
        client
    }

    async fn send(&mut self, method: Method, path: &str, form: &[(&str, &str)]) -> Reply {
        self.request(method, path, form, true).await
    }

    /// A page load: no XHR header.
    async fn page(&mut self, path: &str) -> Reply {
        self.request(Method::GET, path, &[], false).await
    }

    async fn request(
        &mut self,
        method: Method,
        path: &str,
        form: &[(&str, &str)],
        xhr: bool,
    ) -> Reply {
        let body = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        if xhr {
            request = request.header("x-requested-with", "XMLHttpRequest");
        } else {
            request = request.header(header::ACCEPT, "text/html");
        }
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
        if !body.is_empty() {
            request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap_or("");
            let (pair, attrs) = v.split_once(';').unwrap_or((v, ""));
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
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

    async fn post(&mut self, path: &str, form: &[(&str, &str)]) -> Reply {
        self.send(Method::POST, path, form).await
    }

    async fn put(&mut self, path: &str, form: &[(&str, &str)]) -> Reply {
        self.send(Method::PUT, path, form).await
    }

    /// The honeypot answer and reversed challenge signup and activation send.
    async fn honeypot(&mut self) -> (String, String) {
        let hp = self.send(Method::GET, "/session/hp.json", &[]).await.json();
        (
            hp["value"].as_str().unwrap().to_string(),
            hp["challenge"].as_str().unwrap().chars().rev().collect(),
        )
    }

    /// Signs `username` up: the session then names the new, inactive
    /// account. Answers its id.
    async fn sign_up(&mut self, username: &str) -> i32 {
        let (value, challenge) = self.honeypot().await;
        let email = format!("{username}@example.com");
        let created = self
            .post(
                "/u.json",
                &[
                    ("name", "New Member"),
                    ("email", &email),
                    ("password", PASSWORD),
                    ("username", username),
                    ("password_confirmation", &value),
                    ("challenge", &challenge),
                ],
            )
            .await;
        assert_eq!(created.status, StatusCode::OK, "{}", created.body);
        assert_eq!(created.json()["success"], true, "{}", created.body);
        sqlx::query_scalar("SELECT id FROM users WHERE username = $1 AND NOT active")
            .bind(username)
            .fetch_one(&self.state.pool)
            .await
            .unwrap()
    }

    async fn login(&mut self, login: &str, password: &str) -> Reply {
        self.post("/session", &[("login", login), ("password", password)])
            .await
    }
}

async fn app() -> (TestDb, AppState) {
    let db = TestDb::new().await;
    let state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    (db, state)
}

/// A client that just signed `username` up, with the account's id.
async fn signed_up(state: &AppState, username: &str) -> (Client, i32) {
    let mut client = Client::with_csrf(state).await;
    let id = client.sign_up(username).await;
    (client, id)
}

/// The signup emails enqueued for `user_id`, oldest first.
async fn signup_jobs(db: &TestDb, user_id: i32) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT args FROM discourse_rs.jobs WHERE name = 'critical_user_email' \
         AND args->>'type' = 'signup' AND (args->>'user_id')::int = $1 ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

/// The user's email tokens, oldest first: (email, hash, expired, scope).
async fn tokens(db: &TestDb, user_id: i32) -> Vec<(String, String, bool, Option<i32>)> {
    sqlx::query_as(
        "SELECT email, token_hash, expired, scope FROM email_tokens WHERE user_id = $1 ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

fn hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

async fn primary_email(db: &TestDb, user_id: i32) -> (String, Option<String>) {
    sqlx::query_as(
        "SELECT email, normalized_email FROM user_emails WHERE user_id = $1 AND \"primary\"",
    )
    .bind(user_id)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

fn assert_invalid_access(reply: &Reply) {
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.body);
    assert_eq!(
        reply.json(),
        json!({
            "errors": ["You are not permitted to view the requested resource."],
            "error_type": "invalid_access",
        })
    );
}

fn assert_errors(reply: &Reply, errors: &[&str]) {
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    assert_eq!(reply.json(), json!({ "errors": errors }));
}

async fn activate(client: &mut Client, token: &str) -> Reply {
    let (value, challenge) = client.honeypot().await;
    client
        .put(
            &format!("/u/activate-account/{token}.json"),
            &[("password_confirmation", &value), ("challenge", &challenge)],
        )
        .await
}

// ---- account-created -------------------------------------------------

#[tokio::test]
async fn account_created_draws_its_routes_for_the_sessions_account() {
    let (_db, state) = app().await;
    let (mut client, _) = signed_up(&state, "pagesone").await;

    // The index: the signup's message and ActivationControls.
    let index = client.page("/u/account-created").await;
    assert_eq!(index.status, StatusCode::OK);
    assert!(
        index.body.contains(
            "<div class=\"activation-controls\"><button class=\"btn btn-icon-text btn-primary resend\" type=\"button\">"
        ),
        "{}",
        index.body
    );
    assert!(
        index.body.contains("<span class=\"d-button-label\">Resend Activation Email</span></button><button class=\"btn btn-icon-text btn-default edit-email\" type=\"button\">"),
        "{}",
        index.body
    );
    assert!(index.body.contains(
        "<span class=\"d-button-label\">Change Email Address</span></button></div></div>"
    ));
    // The accountCreated preload the script draws the routes from.
    assert!(index.body.contains("\"show_controls\":true"));
    assert!(index.body.contains("\"username\":\"pagesone\""));
    assert!(index.body.contains("\"email\":\"pagesone@example.com\""));

    // edit-email: ActivationEmailForm with the account's address, the
    // submit disabled until it changes.
    let edit = client.page("/u/account-created/edit-email").await;
    assert_eq!(edit.status, StatusCode::OK);
    assert!(
        edit.body.contains(
            "</div></div><div class=\"ac-message\"><p>Provide a new address and we&#x27;ll resend your confirmation email.</p>\
             <input class=\"activate-new-email\" type=\"text\" value=\"pagesone@example.com\"></div>\
             <div class=\"activation-controls\">\
             <button class=\"btn btn-primary\" disabled=\"\" type=\"button\"><span class=\"d-button-label\">Update Email Address</span></button>\
             <button class=\"btn edit-cancel\" type=\"button\"><span class=\"d-button-label\">Cancel</span></button></div></div>"
        ),
        "{}",
        edit.body
    );
    assert!(!edit.body.contains("login-welcome-header"));
    assert!(edit.body.contains(" account-created-page\""));

    // resent, under the old root too: the address the email went to.
    let resent = client.page("/users/account-created/resent").await;
    assert_eq!(resent.status, StatusCode::OK);
    assert!(
        resent.body.contains(
            "<div class=\"ac-message\">We sent another activation email to you at <b>pagesone@example.com</b>. \
             It might take a few minutes for it to arrive; be sure to check your spam folder.</div></div>"
        ),
        "{}",
        resent.body
    );
    assert!(!resent.body.contains("activation-controls"));

    // No account in the session: the generic notice, an empty form.
    let mut stranger = Client::new(&state);
    let resent = stranger.page("/u/account-created/resent").await;
    assert!(
        resent.body.contains(
            "<div class=\"ac-message\">We sent another activation email. It might take a few minutes \
             for it to arrive; be sure to check your spam folder.</div></div>"
        ),
        "{}",
        resent.body
    );
    let edit = stranger.page("/u/account-created/edit-email").await;
    assert!(
        edit.body
            .contains("<input class=\"activate-new-email\" type=\"text\" value=\"\"></div>")
    );
    assert!(edit.body.contains("\"show_controls\":false"));
}

// ---- send_activation_email ------------------------------------------

#[tokio::test]
async fn resend_needs_the_csrf_token() {
    let (_db, state) = app().await;
    let (mut client, _) = signed_up(&state, "resendcsrf").await;
    client.csrf = None;
    let reply = client.post(SEND, &[("username", "resendcsrf")]).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.body, "[\"BAD CSRF\"]");
}

#[tokio::test]
async fn resend_sends_the_signup_email_again_once_per_session() {
    let (db, state) = app().await;
    let (mut client, id) = signed_up(&state, "resendone").await;
    let first = signup_jobs(&db, id).await;
    assert_eq!(first.len(), 1);

    let reply = client.post(SEND, &[("username", "resendone")]).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    // `render body: nil`
    assert_eq!(reply.body, "");

    // A new signup token for the address, the signup's own expired; the
    // same job as the signup's, addressed.
    let jobs = signup_jobs(&db, id).await;
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[1]["to_address"], "resendone@example.com");
    let new_token = jobs[1]["email_token"].as_str().unwrap();
    assert_ne!(jobs[0]["email_token"], jobs[1]["email_token"]);
    // (User's after_create made a token of its own before the signup's.)
    let rows = tokens(&db, id).await;
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[1],
        (
            "resendone@example.com".into(),
            hash(first[0]["email_token"].as_str().unwrap()),
            true,
            Some(1)
        )
    );
    assert_eq!(
        rows[2],
        (
            "resendone@example.com".into(),
            hash(new_token),
            false,
            Some(1)
        )
    );

    // The session named the account once: a second request is refused.
    let again = client.post(SEND, &[("username", "resendone")]).await;
    assert_invalid_access(&again);
    assert_eq!(signup_jobs(&db, id).await.len(), 2);

    // The emailed token activates the account, the first no longer does.
    let old = activate(&mut client, first[0]["email_token"].as_str().unwrap()).await;
    assert_eq!(old.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", old.body);
    let new = activate(&mut client, new_token).await;
    assert_eq!(new.status, StatusCode::OK, "{}", new.body);

    // Both emails go out the way the signup's does.
    discourse_rs::jobs::drain(&state).await.unwrap();
    let sent: Vec<String> = sqlx::query_scalar(
        "SELECT to_address FROM email_logs WHERE user_id = $1 AND email_type = 'signup' ORDER BY id",
    )
    .bind(id)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(sent, vec!["resendone@example.com", "resendone@example.com"]);
}

#[tokio::test]
async fn resend_finds_the_account_by_email_and_under_users() {
    let (db, state) = app().await;
    let (mut client, id) = signed_up(&state, "resendmail").await;
    let reply = client
        .post(
            "/users/action/send_activation_email.json",
            &[("username", "ResendMail@example.com")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(signup_jobs(&db, id).await.len(), 2);
}

#[tokio::test]
async fn resend_without_an_account_is_not_found() {
    let (db, state) = app().await;
    let (mut client, id) = signed_up(&state, "resendnone").await;
    for form in [
        &[][..],
        &[("username", "")],
        &[("username", "nobodyhere")],
        &[("username", "nobody@example.com")],
    ] {
        let reply = client.post(SEND, form).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{}", reply.body);
        assert_eq!(reply.json()["error_type"], "not_found");
    }
    // The session still names the account.
    let reply = client.post(SEND, &[("username", "resendnone")]).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(signup_jobs(&db, id).await.len(), 2);
}

#[tokio::test]
async fn resend_is_for_the_sessions_account_only() {
    let (db, state) = app().await;
    let (_, other) = signed_up(&state, "resendother").await;
    // No account in the session.
    let mut stranger = Client::with_csrf(&state).await;
    let reply = stranger.post(SEND, &[("username", "resendother")]).await;
    assert_invalid_access(&reply);
    // Another account in the session; the refusal keeps the session's.
    let (mut client, id) = signed_up(&state, "resendmine").await;
    let reply = client.post(SEND, &[("username", "resendother")]).await;
    assert_invalid_access(&reply);
    assert_eq!(signup_jobs(&db, other).await.len(), 1);
    let reply = client.post(SEND, &[("username", "resendmine")]).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(signup_jobs(&db, id).await.len(), 2);
}

#[tokio::test]
async fn resend_is_refused_where_accounts_wait_for_approval() {
    let (db, state) = app().await;
    let (mut client, id) = signed_up(&state, "resendapprove").await;
    set_setting(&db.pool, "must_approve_users", BOOL, "t").await;
    // Before the account is looked for.
    for username in ["resendapprove", "nobodyhere"] {
        let reply = client.post(SEND, &[("username", username)]).await;
        assert_invalid_access(&reply);
    }
    assert_eq!(signup_jobs(&db, id).await.len(), 1);
}

#[tokio::test]
async fn resend_for_an_activated_account_is_a_conflict() {
    let (db, state) = app().await;
    let (mut client, id) = signed_up(&state, "resendactive").await;
    let token = signup_jobs(&db, id).await[0]["email_token"]
        .as_str()
        .unwrap()
        .to_string();
    // Activated from another browser: this session still names it.
    let mut elsewhere = Client::with_csrf(&state).await;
    let activated = activate(&mut elsewhere, &token).await;
    assert_eq!(activated.status, StatusCode::OK, "{}", activated.body);

    let reply = client.post(SEND, &[("username", "resendactive")]).await;
    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.body);
    assert_eq!(
        reply.json(),
        json!({ "errors": ["Sorry, this account has already been activated."] })
    );
    assert_eq!(signup_jobs(&db, id).await.len(), 1);
    // The session forgot the account all the same.
    let again = client.post(SEND, &[("username", "resendactive")]).await;
    assert_invalid_access(&again);
}

#[tokio::test]
async fn staff_resend_for_any_account() {
    let (db, state) = app().await;
    let (_, id) = signed_up(&state, "resendstaff").await;
    let mut admin = Client::with_csrf(&state).await;
    let login = admin.login("admin", "password").await;
    assert!(login.json()["error"].is_null(), "{}", login.body);
    // The login gave the session another token.
    let csrf = admin
        .send(Method::GET, "/session/csrf.json", &[])
        .await
        .json();
    admin.csrf = csrf["csrf"].as_str().map(str::to_string);

    for _ in 0..2 {
        let reply = admin.post(SEND, &[("username", "resendstaff")]).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    }
    assert_eq!(signup_jobs(&db, id).await.len(), 3);
    // An active, confirmed account.
    let reply = admin.post(SEND, &[("username", "user1")]).await;
    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.body);
    // Not where accounts wait for approval.
    set_setting(&db.pool, "must_approve_users", BOOL, "t").await;
    let reply = admin.post(SEND, &[("username", "resendstaff")]).await;
    assert_invalid_access(&reply);
}

#[tokio::test]
async fn a_refused_login_names_the_account_for_a_resend() {
    let (db, state) = app().await;
    let (_, id) = signed_up(&state, "resendlogin").await;
    let mut client = Client::with_csrf(&state).await;
    let login = client.login("resendlogin", PASSWORD).await;
    assert_eq!(login.status, StatusCode::OK, "{}", login.body);
    let body = login.json();
    assert_eq!(body["reason"], "not_activated", "{}", login.body);
    assert_eq!(body["sent_to_email"], "resendlogin@example.com");
    assert_eq!(body["current_email"], "resendlogin@example.com");
    // NotActivated resends for the address it was given.
    let reply = client
        .post(SEND, &[("username", "resendlogin@example.com")])
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(signup_jobs(&db, id).await.len(), 2);
}

#[tokio::test]
async fn resends_are_limited_per_address() {
    let db = TestDb::new().await;
    let mut cfg = config(RailsEnv::Test, &[]);
    cfg.rate_limits = true;
    let state = state(db.pool.clone(), cfg).await;
    let mut client = Client::with_csrf(&state).await;
    // Six a minute, whatever they answer.
    for _ in 0..6 {
        let reply = client.post(SEND, &[("username", "nobodyhere")]).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{}", reply.body);
    }
    let reply = client.post(SEND, &[("username", "nobodyhere")]).await;
    assert_eq!(
        reply.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        reply.body
    );
    assert_eq!(reply.json()["error_type"], "rate_limit");
}

// ---- update_activation_email ----------------------------------------

#[tokio::test]
async fn change_needs_the_csrf_token() {
    let (db, state) = app().await;
    let (mut client, id) = signed_up(&state, "changecsrf").await;
    client.csrf = None;
    let reply = client.put(UPDATE, &[("email", "moved@example.com")]).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.body, "[\"BAD CSRF\"]");
    assert_eq!(primary_email(&db, id).await.0, "changecsrf@example.com");
}

#[tokio::test]
async fn change_moves_the_sessions_account_and_its_activation() {
    let (db, state) = app().await;
    let (mut client, id) = signed_up(&state, "changeone").await;
    let first = signup_jobs(&db, id).await[0]["email_token"]
        .as_str()
        .unwrap()
        .to_string();

    let reply = client
        .put(UPDATE, &[("email", " Moved.Address+tag@Example.com ")])
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json(), json!({ "success": "OK" }));

    // Stripped and downcased, the normalized form with it.
    assert_eq!(
        primary_email(&db, id).await,
        (
            "moved.address+tag@example.com".into(),
            Some("movedaddress@example.com".into())
        )
    );
    // The old address's tokens are gone; the new one has a signup token,
    // mailed there by the signup's job.
    let jobs = signup_jobs(&db, id).await;
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[1]["to_address"], "moved.address+tag@example.com");
    let new_token = jobs[1]["email_token"].as_str().unwrap().to_string();
    assert_eq!(
        tokens(&db, id).await,
        vec![(
            "moved.address+tag@example.com".into(),
            hash(&new_token),
            false,
            Some(1)
        )]
    );

    // The session keeps the account: the address can change again, and
    // the email be sent again.
    let reply = client.put(UPDATE, &[("email", "second@example.com")]).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let jobs = signup_jobs(&db, id).await;
    let last_token = jobs[2]["email_token"].as_str().unwrap().to_string();
    // Each email goes out the way the signup's does, to its address; the
    // signup's own names none, so sent this late it finds the last one.
    discourse_rs::jobs::drain(&state).await.unwrap();
    let sent: Vec<String> = sqlx::query_scalar(
        "SELECT to_address FROM email_logs WHERE user_id = $1 AND email_type = 'signup' ORDER BY id",
    )
    .bind(id)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        sent,
        vec![
            "second@example.com",
            "moved.address+tag@example.com",
            "second@example.com"
        ]
    );

    // Neither earlier token activates the account; the last one does, for
    // the new address.
    for token in [&first, &new_token] {
        let reply = activate(&mut client, token).await;
        assert_errors(
            &reply,
            &[
                "Sorry, this account confirmation link is no longer valid. Perhaps your account is already active?",
            ],
        );
    }
    let reply = activate(&mut client, &last_token).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let active: bool = sqlx::query_scalar("SELECT active FROM users WHERE id = $1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert!(active);
    assert_eq!(primary_email(&db, id).await.0, "second@example.com");
}

#[tokio::test]
async fn change_to_the_same_address_sends_it_again() {
    let (db, state) = app().await;
    let (mut client, id) = signed_up(&state, "changesame").await;
    let reply = client
        .put(UPDATE, &[("email", "ChangeSame@example.com")])
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    // Nothing changed, so the signup's tokens stay, expired by the new one.
    let rows = tokens(&db, id).await;
    assert_eq!(
        rows.iter().map(|r| r.2).collect::<Vec<_>>(),
        vec![true, true, false]
    );
    let jobs = signup_jobs(&db, id).await;
    assert_eq!(jobs[1]["to_address"], "changesame@example.com");
}

#[tokio::test]
async fn change_without_an_account_is_refused() {
    let (db, state) = app().await;
    // Neither a session's account nor a username.
    let mut stranger = Client::with_csrf(&state).await;
    let reply = stranger
        .put(UPDATE, &[("email", "moved@example.com")])
        .await;
    assert_invalid_access(&reply);

    // An unknown username, a wrong password.
    let (_, id) = signed_up(&state, "changenamed").await;
    for (username, password) in [
        ("nobodyhere", PASSWORD),
        ("nobody@example.com", PASSWORD),
        ("changenamed", "not-the-pass-phrase"),
    ] {
        let reply = stranger
            .put(
                UPDATE,
                &[
                    ("username", username),
                    ("password", password),
                    ("email", "moved@example.com"),
                ],
            )
            .await;
        assert_invalid_access(&reply);
    }
    assert_eq!(primary_email(&db, id).await.0, "changenamed@example.com");
    assert_eq!(signup_jobs(&db, id).await.len(), 1);
}

#[tokio::test]
async fn change_by_username_and_password() {
    let (db, state) = app().await;
    let (_, id) = signed_up(&state, "changenamed2").await;
    let mut stranger = Client::with_csrf(&state).await;
    let reply = stranger
        .put(
            "/users/update-activation-email",
            &[
                ("username", "changenamed2@example.com"),
                ("password", PASSWORD),
                ("email", "moved2@example.com"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json(), json!({ "success": "OK" }));
    assert_eq!(primary_email(&db, id).await.0, "moved2@example.com");
    assert_eq!(
        signup_jobs(&db, id).await[1]["to_address"],
        "moved2@example.com"
    );
    // A username wins over the session's account.
    let (mut client, mine) = signed_up(&state, "changenamed3").await;
    let reply = client
        .put(
            UPDATE,
            &[
                ("username", "changenamed2"),
                ("password", PASSWORD),
                ("email", "moved3@example.com"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(primary_email(&db, id).await.0, "moved3@example.com");
    assert_eq!(primary_email(&db, mine).await.0, "changenamed3@example.com");
}

#[tokio::test]
async fn change_is_for_accounts_not_active_yet_and_nobody_logged_in() {
    let (db, state) = app().await;
    // An active account, by its password.
    let mut stranger = Client::with_csrf(&state).await;
    let reply = stranger
        .put(
            UPDATE,
            &[
                ("username", "user1"),
                ("password", "password"),
                ("email", "moved@example.com"),
            ],
        )
        .await;
    assert_invalid_access(&reply);

    // The session's account, activated since.
    let (mut client, id) = signed_up(&state, "changeactive").await;
    sqlx::query("UPDATE users SET active = TRUE WHERE id = $1")
        .bind(id)
        .execute(&db.pool)
        .await
        .unwrap();
    let reply = client.put(UPDATE, &[("email", "moved@example.com")]).await;
    assert_invalid_access(&reply);

    // A staged user's signup.
    let (mut client, id) = signed_up(&state, "changestaged").await;
    sqlx::query(
        "INSERT INTO user_custom_fields (user_id, name, value, created_at, updated_at) \
         VALUES ($1, 'from_staged', 't', now(), now())",
    )
    .bind(id)
    .execute(&db.pool)
    .await
    .unwrap();
    let reply = client.put(UPDATE, &[("email", "moved@example.com")]).await;
    assert_invalid_access(&reply);
    assert_eq!(primary_email(&db, id).await.0, "changestaged@example.com");

    // Somebody logged in, whoever the request names.
    let (_, id) = signed_up(&state, "changemember").await;
    let mut member = Client::with_csrf(&state).await;
    let login = member.login("user1", "password").await;
    assert!(login.json()["error"].is_null(), "{}", login.body);
    let csrf = member
        .send(Method::GET, "/session/csrf.json", &[])
        .await
        .json();
    member.csrf = csrf["csrf"].as_str().map(str::to_string);
    let reply = member
        .put(
            UPDATE,
            &[
                ("username", "changemember"),
                ("password", PASSWORD),
                ("email", "moved@example.com"),
            ],
        )
        .await;
    assert_invalid_access(&reply);
    assert_eq!(primary_email(&db, id).await.0, "changemember@example.com");
}

#[tokio::test]
async fn change_answers_the_addresss_errors() {
    let (db, state) = app().await;
    let (mut client, id) = signed_up(&state, "changeerrors").await;
    sqlx::query(
        "INSERT INTO screened_emails (email, action_type, match_count, created_at, updated_at) \
         VALUES ('spammer@elsewhere.example', 1, 0, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();

    let reply = client.put(UPDATE, &[("email", "not-an-email")]).await;
    assert_errors(&reply, &["Email is invalid."]);
    // Presence, then EmailValidator's own blank.
    for form in [&[("email", "  ")][..], &[]] {
        let reply = client.put(UPDATE, form).await;
        assert_errors(&reply, &["Email can't be blank", "Email can't be blank."]);
    }
    // Another account's address, in any case.
    let reply = client.put(UPDATE, &[("email", "User1@Example.com")]).await;
    assert_errors(&reply, &["Email has already been taken"]);
    // A screened address, its match counted.
    let reply = client
        .put(UPDATE, &[("email", "spammer@elsewhere.example")])
        .await;
    assert_errors(&reply, &["Email is not allowed."]);
    let matches: i32 = sqlx::query_scalar(
        "SELECT match_count FROM screened_emails WHERE email = 'spammer@elsewhere.example'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(matches, 1);
    // A blocked domain.
    set_setting(&db.pool, "blocked_email_domains", LIST, "blocked.example").await;
    let reply = client
        .put(UPDATE, &[("email", "someone@blocked.example")])
        .await;
    assert_errors(
        &reply,
        &["Email is not allowed from that email provider. Please use another email address."],
    );
    // An alias of another account's address, unless emails are not
    // normalized.
    let reply = client
        .put(UPDATE, &[("email", "u.ser1+other@example.com")])
        .await;
    assert_errors(&reply, &["Email has already been taken"]);
    set_setting(&db.pool, "normalize_emails", BOOL, "f").await;
    let reply = client
        .put(UPDATE, &[("email", "user.1+alias@example.com")])
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);

    // Nothing but the one accepted change went through.
    assert_eq!(primary_email(&db, id).await.0, "user.1+alias@example.com");
    assert_eq!(signup_jobs(&db, id).await.len(), 2);
}

#[tokio::test]
async fn change_revokes_an_approval_given_to_the_confirmed_address() {
    let (db, state) = app().await;
    let (_, id) = signed_up(&state, "changeapproved").await;
    // Approved with the address confirmed, then deactivated.
    sqlx::query("UPDATE email_tokens SET confirmed = TRUE WHERE user_id = $1")
        .bind(id)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE users SET approved = TRUE, approved_by_id = 1, approved_at = now() WHERE id = $1",
    )
    .bind(id)
    .execute(&db.pool)
    .await
    .unwrap();
    let approval = |db: &TestDb| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_as::<_, (bool, Option<i32>, bool)>(
                "SELECT approved, approved_by_id, approved_at IS NOT NULL FROM users WHERE id = $1",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let change = |email: &'static str| {
        let state = state.clone();
        async move {
            let mut client = Client::with_csrf(&state).await;
            client
                .put(
                    UPDATE,
                    &[
                        ("username", "changeapproved"),
                        ("password", PASSWORD),
                        ("email", email),
                    ],
                )
                .await
        }
    };
    // Kept where accounts need no approval.
    let reply = change("approved1@example.com").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(approval(&db).await, (true, Some(1), true));

    // The new address is not confirmed: nothing to revoke.
    set_setting(&db.pool, "must_approve_users", BOOL, "t").await;
    let reply = change("approved2@example.com").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(approval(&db).await, (true, Some(1), true));

    // Confirmed again: the change takes the approval back.
    sqlx::query("UPDATE email_tokens SET confirmed = TRUE WHERE user_id = $1")
        .bind(id)
        .execute(&db.pool)
        .await
        .unwrap();
    let reply = change("approved3@example.com").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(approval(&db).await, (false, None, false));
    assert_eq!(primary_email(&db, id).await.0, "approved3@example.com");
}

#[tokio::test]
async fn changes_are_limited_per_address() {
    let db = TestDb::new().await;
    let mut cfg = config(RailsEnv::Test, &[]);
    cfg.rate_limits = true;
    let state = state(db.pool.clone(), cfg).await;
    let mut client = Client::with_csrf(&state).await;
    // Five an hour from an address, whatever they answer.
    for _ in 0..5 {
        let reply = client.put(UPDATE, &[("email", "moved@example.com")]).await;
        assert_invalid_access(&reply);
    }
    let reply = client.put(UPDATE, &[("email", "moved@example.com")]).await;
    assert_eq!(
        reply.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        reply.body
    );
    assert_eq!(reply.json()["error_type"], "rate_limit");
    let waits = reply.json()["extras"]["wait_seconds"].as_i64().unwrap();
    assert!((1..=3600).contains(&waits), "{}", reply.body);
}
