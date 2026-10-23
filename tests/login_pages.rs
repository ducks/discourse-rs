//! The login flow's pages as the Ember app draws them: /login
//! (templates/login.gjs), /signup (templates/signup.gjs), the
//! login-required front page (discovery/login-required.gjs) and
//! /login-preferences, in the desktop and mobile views; and
//! users#check_email, which the signup page asks as its email field is
//! left.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

const BOOL: i32 = 5;
const STRING: i32 = 1;

struct Reply {
    status: StatusCode,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn cookies(&self) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(k, _)| k == "set-cookie")
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// The page's #login-data.
    fn data(&self) -> Value {
        let marker = "<script type=\"application/json\" id=\"login-data\">";
        let at = self.body.find(marker).expect("the page carries its data") + marker.len();
        let end = self.body[at..].find("</script>").unwrap();
        serde_json::from_str(&self.body[at..at + end]).unwrap()
    }
}

async fn request(st: &AppState, method: Method, path: &str, cookies: &[&str], body: &str) -> Reply {
    let app = discourse_rs::app(st.clone());
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "test.localhost")
        .header("x-requested-with", "XMLHttpRequest");
    if !cookies.is_empty() {
        builder = builder.header(header::COOKIE, cookies.join("; "));
    }
    let response = app
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

/// A page as a browser asks for it (no XHR header).
async fn page(st: &AppState, path: &str, cookies: &[&str]) -> Reply {
    let app = discourse_rs::app(st.clone());
    let mut builder = Request::get(path).header(header::HOST, "test.localhost");
    if !cookies.is_empty() {
        builder = builder.header(header::COOKIE, cookies.join("; "));
    }
    let response = app
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

/// The `_t` cookie of user1, logged in through POST /session.
async fn logged_in(st: &AppState) -> String {
    let csrf_reply = request(st, Method::GET, "/session/csrf.json", &[], "").await;
    let session = csrf_reply
        .cookies()
        .into_iter()
        .find(|c| c.starts_with("_forum_session="))
        .map(|c| c.split(';').next().unwrap().to_string())
        .unwrap();
    let csrf: Value = serde_json::from_str(&csrf_reply.body).unwrap();
    let app = discourse_rs::app(st.clone());
    let response = app
        .oneshot(
            Request::post("/session")
                .header(header::HOST, "test.localhost")
                .header("x-requested-with", "XMLHttpRequest")
                .header("x-csrf-token", csrf["csrf"].as_str().unwrap())
                .header(header::COOKIE, &session)
                .header(
                    header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded; charset=UTF-8",
                )
                .body(Body::from("login=user1&password=password"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with("_t="))
        .map(|c| c.split(';').next().unwrap().to_string())
        .expect("a login sets _t")
}

async fn code_logins(pool: &PgPool, on: bool) {
    set_setting(
        pool,
        "enable_local_logins_via_code",
        BOOL,
        if on { "t" } else { "f" },
    )
    .await;
}

#[tokio::test]
async fn the_login_page_draws_embers_full_page_login() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    code_logins(&db.pool, false).await;
    let login = page(&st, "/login", &[]).await;
    assert_eq!(login.status, StatusCode::OK);
    let html = &login.body;
    // hideApplicationSidebar, hideApplicationHeaderButtons and the body
    // class.
    assert!(!html.contains("id=\"d-sidebar\""), "{html}");
    assert!(!html.contains("has-sidebar-page"), "{html}");
    assert!(html.contains("<body hx-boost=\"true\" class=\""), "{html}");
    assert!(html.contains(" login-page\""), "{html}");
    let squeezed: String = html.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        squeezed.contains(
            "<span class=\"header-buttons\"><span class=\"auth-buttons\"> </span> </span>"
        ),
        "{html}"
    );
    assert!(!html.contains("id=\"search-button\""), "{html}");
    for needle in [
        "<div class=\"login-fullpage\" hx-boost=\"false\"><div class=\"login-body login-body has-alt-auth\"><div class=\"login-left-side\"><div class=\"login-welcome-header\"><h1 class=\"login-title\">Welcome back</h1></div>",
        "<form id=\"login-form\" method=\"post\" novalidate=\"\"><div class=\"\" id=\"credentials\">",
        "<input id=\"login-account-name\" class=\"ember-text-field ember-view\" autocapitalize=\"off\" autocomplete=\"username webauthn\" autocorrect=\"off\" autofocus=\"autofocus\" inputmode=\"email\" tabindex=\"1\" value=\"\" type=\"text\"><label class=\"alt-placeholder\" for=\"login-account-name\">Email / Username</label>",
        "<a class=\"no-login-filled\" href=\"\" id=\"email-login-link\" tabindex=\"3\">",
        "id=\"login-account-password\" class=\"ember-text-field ember-view\" type=\"password\">",
        "<div class=\"login__password-links\"><a href=\"\" id=\"forgot-password-link\" tabindex=\"2\">I forgot my password</a></div>",
        "<div class=\"caps-lock-warning hidden\">",
        "<div class=\"login-page-cta\"><div class=\"login-page-cta__buttons\"><button class=\"btn btn-large btn-primary login-page-cta__login\" form=\"login-form\" id=\"login-button\" tabindex=\"2\" type=\"submit\"><span class=\"d-button-label\">Log In</span></button><span class=\"login-page-cta__no-account-yet\">",
        "id=\"new-account-link\" tabindex=\"3\" type=\"button\"><span class=\"d-button-label\">Sign Up</span></button>",
        "<div class=\"login-right-side\"><div id=\"login-buttons\"><button class=\"btn btn-icon-text btn-social passkey-login-button\" type=\"button\">",
        // The static form the script submits once logged in.
        "<form id=\"hidden-login-form\" method=\"post\" action=\"/login\" style=\"display: none;\">",
    ] {
        assert!(html.contains(needle), "{needle}\n{html}");
    }
    // No code login, so no forms for its link to swap in.
    assert!(!html.contains("id=\"login-code-form\""), "{html}");
    let data = login.data();
    assert_eq!(data["page"], "login");
    assert!(data["csrf"].as_str().is_some_and(|c| !c.is_empty()));
    assert!(!html.contains("[en."), "every string is translated");
    assert!(
        login
            .cookies()
            .iter()
            .any(|c| c.starts_with("_forum_session="))
    );
}

#[tokio::test]
async fn the_mobile_login_page_puts_the_header_and_buttons_first() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    code_logins(&db.pool, false).await;
    let html = page(&st, "/login", &["mobile_view=1"]).await.body;
    assert!(html.contains("<div class=\"login-fullpage\" hx-boost=\"false\"><div class=\"login-body login-body has-alt-auth\"><div class=\"login-welcome-header\"><h1 class=\"login-title\">Welcome back</h1></div><div id=\"login-buttons\">"), "{html}");
    assert!(!html.contains("login-left-side"), "{html}");
    // The form in a classless wrapper, the call to action after it.
    assert!(
        html.contains("</div><div><form id=\"login-form\""),
        "{html}"
    );
    let form = html.find("id=\"login-form\"").unwrap();
    let cta = html.find("class=\"login-page-cta\"").unwrap();
    assert!(form < cta, "{html}");
}

#[tokio::test]
async fn code_logins_offer_the_one_time_code_form() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    code_logins(&db.pool, true).await;
    let html = page(&st, "/login", &[]).await.body;
    assert!(html.contains("<a href=\"\" id=\"one-time-code-link\" tabindex=\"3\">Email me a one-time login code</a>"), "{html}");
    for template in ["login-code-form", "login-password-form", "login-cta"] {
        assert!(
            html.contains(&format!("<template id=\"{template}\">")),
            "{template}"
        );
    }
    // ?mode=code starts on the code form, without the call to action.
    let html = page(&st, "/login?mode=code", &[]).await.body;
    assert!(
        html.contains("<div class=\"login-body login-body has-alt-auth code-login\">"),
        "{html}"
    );
    let shown = &html[..html.find("<template").unwrap()];
    assert!(shown.contains("<div class=\"code-login-form\"><form class=\"form-kit code-login-form__email-step\" novalidate=\"\">"), "{shown}");
    assert!(
        shown.contains("code-login-form__password-toggle"),
        "{shown}"
    );
    assert!(!shown.contains("login-page-cta"), "{shown}");
    assert!(!shown.contains("id=\"login-form\""), "{shown}");
}

#[tokio::test]
async fn no_local_logins_and_no_buttons_is_no_login_methods() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    set_setting(&db.pool, "enable_local_logins", BOOL, "f").await;
    let html = page(&st, "/login", &[]).await.body;
    assert!(html.contains("<div class=\"login-body login-body no-local-login\"><div class=\"login-left-side\"><div class=\"login-welcome-header no-login-methods-configured\">"), "{html}");
    assert!(html.contains("href='/u/admin-login'"), "{html}");
    assert!(!html.contains("login-page-cta"), "{html}");
}

#[tokio::test]
async fn login_redirect_param_sets_the_destination_or_sends_a_member_there() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let anonymous = page(&st, "/login?redirect=%2Flatest%3Forder%3Dviews", &[]).await;
    assert_eq!(anonymous.status, StatusCode::OK);
    assert!(
        anonymous
            .cookies()
            .contains(&"destination_url=%2Flatest%3Forder%3Dviews; path=/"),
        "{:?}",
        anonymous.cookies()
    );
    // Another host, or the login page itself, is no destination.
    for redirect in [
        "https%3A%2F%2Fevil.example%2Fx",
        "%2Flogin%3Fx%3D1",
        "%2Ffoo.bar",
    ] {
        let reply = page(&st, &format!("/login?redirect={redirect}"), &[]).await;
        assert!(
            !reply
                .cookies()
                .iter()
                .any(|c| c.starts_with("destination_url=")),
            "{redirect}"
        );
    }
    let t = logged_in(&st).await;
    let member = page(&st, "/login?redirect=%2Flatest", &[&t]).await;
    assert_eq!(member.status, StatusCode::FOUND);
    assert_eq!(member.header("location"), Some("/latest"));
    let member = page(&st, "/login", &[&t]).await;
    assert_eq!(member.header("location"), Some("/"));
    // ?email= is kept for the forms.
    let reply = page(&st, "/login?email=a%40example.com", &[]).await;
    assert!(
        reply
            .cookies()
            .iter()
            .any(|c| c.starts_with("email=a%40example.com; path=/; expires=")),
        "{:?}",
        reply.cookies()
    );
}

#[tokio::test]
async fn the_signup_page_draws_the_code_signup_or_the_form() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    code_logins(&db.pool, true).await;
    let signup = page(&st, "/signup", &[]).await;
    assert_eq!(signup.status, StatusCode::OK);
    let html = &signup.body;
    assert!(html.contains(" signup-page\""), "{html}");
    assert!(html.contains("<div class=\"signup-fullpage\" hx-boost=\"false\"><div class=\"signup-body has-user-fields passwordless-signup\"><div class=\"login-left-side\"><div class=\"code-login-form\"><div class=\"login-welcome-header\" id=\"create-account-title\"><h1 class=\"login-title\">Create your account</h1></div><p class=\"code-login-form__instructions\">"), "{html}");
    // The create-account form an external login's new account needs.
    assert!(
        html.contains("<template id=\"signup-auth-form\"><div class=\"signup-progress-bar\">"),
        "{html}"
    );
    assert_eq!(signup.data()["page"], "signup");
    assert!(!html.contains("[en."), "every string is translated");

    code_logins(&db.pool, false).await;
    let html = page(&st, "/signup", &[]).await.body;
    for needle in [
        "<div class=\"signup-body has-user-fields\"><div class=\"login-left-side\"><div class=\"signup-progress-bar\"><div class=\"signup-progress-bar__segment --active\">",
        "<div class=\"login-welcome-header\" id=\"create-account-title\"><h1 class=\"login-title\">Welcome!</h1></div><form id=\"login-form\">",
        "<div class=\"input-group create-account-email\"><input id=\"new-account-email\" class=\"ember-text-field ember-view\" aria-describedby=\"account-email-validation account-email-validation-more-info\" aria-invalid=\"\" autofocus=\"autofocus\" name=\"email\" type=\"email\">",
        "<span class=\"more-info\" id=\"account-email-validation-more-info\">Never shown to the public</span>",
        "<div class=\"input-group create-account__username\"><input aria-describedby=\"username-validation username-validation-more-info\" aria-invalid=\"\" autocomplete=\"off\" class=\"\" id=\"new-account-username\"",
        "<div class=\"tip bad\" id=\"username-validation\"></div>",
        "<div class=\"input-group create-account__password\"><input placeholder=\"\" aria-describedby=\"password-validation password-validation-more-info\" aria-invalid=\"\" autocomplete=\"current-password\" id=\"new-account-password\"",
        "class=\"btn no-text btn-icon btn-transparent toggle-password-mask\" title=\"Show password\" type=\"button\">",
        "<div class=\"password-confirmation\"><label for=\"new-account-password-confirmation\">Password Again</label><input placeholder=\"\" autocomplete=\"new-password\" id=\"new-account-confirmation\"",
        "<div class=\"signup-page-cta\"><div class=\"signup-page-cta__buttons\"><button class=\"btn btn-large btn-primary signup-page-cta__signup\" type=\"button\"><span class=\"d-button-label\">Sign Up</span></button><span class=\"signup-page-cta__existing-account\">Already have an account?</span>",
    ] {
        assert!(html.contains(needle), "{needle}\n{html}");
    }
    assert!(!html.contains("signup-auth-form"), "{html}");

    // A member goes home.
    let t = logged_in(&st).await;
    let member = page(&st, "/signup", &[&t]).await;
    assert_eq!(member.status, StatusCode::FOUND);
    assert_eq!(member.header("location"), Some("http://test.localhost/"));
}

#[tokio::test]
async fn the_signup_page_with_a_full_name_and_an_invite_code() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    code_logins(&db.pool, false).await;
    set_setting(&db.pool, "full_name_requirement", 7, "required_at_signup").await;
    set_setting(&db.pool, "invite_code", STRING, "secret").await;
    let html = page(&st, "/signup", &[]).await.body;
    // The required name follows the username, the invite code the password.
    let username = html.find("create-account__username").unwrap();
    let name = html.find("create-account__fullname required").unwrap();
    let password = html.find("create-account__password").unwrap();
    let invite = html.find("create-account__invite-code").unwrap();
    assert!(
        username < name && name < password && password < invite,
        "{html}"
    );
    assert!(
        html.contains("<label class=\"alt-placeholder\" for=\"new-account-name\">Name</label>"),
        "{html}"
    );
}

#[tokio::test]
async fn closed_signups_go_home() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    set_setting(&db.pool, "invite_only", BOOL, "t").await;
    let reply = page(&st, "/signup", &[]).await;
    assert_eq!(reply.status, StatusCode::FOUND);
    assert_eq!(reply.header("location"), Some("http://test.localhost/"));
}

#[tokio::test]
async fn login_required_front_page_is_the_static_login_page() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    set_setting(&db.pool, "login_required", BOOL, "t").await;
    let front = page(&st, "/", &[]).await;
    assert_eq!(front.status, StatusCode::OK);
    let html = &front.body;
    assert!(html.contains(" login-page static-login\""), "{html}");
    assert!(html.contains("<section class=\"container\"><div class=\"contents clearfix body-page\"><div class=\"login-welcome\"><div class=\"login-content\"><picture class=\"logo-container\"><img src=\""), "{html}");
    assert!(
        html.contains("class=\"site-logo\"></picture><div class=\"login-welcome__title\"><h1>"),
        "{html}"
    );
    assert!(
        html.contains("Welcome to Discourse</h1><p class=\"login-welcome__description\">"),
        "{html}"
    );
    assert!(html.contains("<div class=\"body-page-button-container\"><button class=\"btn btn-primary sign-up-button\" type=\"button\"><span class=\"d-button-label\">Sign Up</span></button><button class=\"btn btn-icon-text btn-primary login-button\" type=\"button\">"), "{html}");
    assert_eq!(front.data()["page"], "login-required");
}

/// What signing up needs passes the login gate, as UsersController skips
/// redirect_to_login_if_required for it; the rest of /u does not.
#[tokio::test]
async fn login_required_lets_signing_up_through() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    set_setting(&db.pool, "login_required", BOOL, "t").await;
    for path in [
        "/u/check_username.json?username=newbie",
        "/u/check_email.json?email=x%40example.com",
    ] {
        let reply = request(&st, Method::GET, path, &[], "").await;
        assert_eq!(reply.status, StatusCode::OK, "{path}: {}", reply.body);
    }
    let reply = request(&st, Method::GET, "/u/user1.json", &[], "").await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    // users#create is past the gate (its own checks answer).
    let reply = request(&st, Method::POST, "/u.json", &[], "").await;
    assert_ne!(reply.status, StatusCode::FORBIDDEN, "{}", reply.body);
    // The directory is not.
    let reply = page(&st, "/u", &[]).await;
    assert_eq!(reply.status, StatusCode::FOUND);
}

#[tokio::test]
async fn check_email_answers_as_users_check_email() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    set_setting(&db.pool, "hide_email_address_taken", BOOL, "f").await;
    let check = |email: &'static str| {
        let st = st.clone();
        async move {
            let reply = request(
                &st,
                Method::GET,
                &format!("/u/check_email.json?email={email}"),
                &[],
                "",
            )
            .await;
            assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
            serde_json::from_str::<Value>(&reply.body).unwrap()
        }
    };
    assert_eq!(check("new%40example.com").await["success"], "OK");
    assert_eq!(check("").await["success"], "OK");
    let invalid = check("not-an-email").await;
    assert_eq!(invalid["failed"], "FAILED");
    assert_eq!(invalid["errors"][0], "Email is invalid.");
    let taken = check("USER1%40example.com").await;
    assert_eq!(
        taken["errors"][0], "Email has already been taken",
        "{taken}"
    );
    set_setting(&db.pool, "blocked_email_domains", STRING, "blocked.example").await;
    let blocked = check("a%40blocked.example").await;
    assert_eq!(blocked["failed"], "FAILED", "{blocked}");
    // A hidden taken address is never told.
    set_setting(&db.pool, "hide_email_address_taken", BOOL, "t").await;
    assert_eq!(check("user1%40example.com").await["success"], "OK");
}

#[tokio::test]
async fn login_preferences_asks_for_a_login() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let reply = page(&st, "/login-preferences", &[]).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(
        reply.body.contains("<div class=\"container\"><h2>"),
        "{}",
        reply.body
    );
    assert!(
        reply.body.contains("<p style=\"margin-top: 1em\">"),
        "{}",
        reply.body
    );
    assert_eq!(reply.data()["page"], "login-preferences");
}

/// A password reset token for `username`, as forgot_password emails it.
async fn reset_token(pool: &PgPool, username: &str) -> String {
    let token = format!("{:032x}", rand_token(username));
    let hash = {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(token.as_bytes()))
    };
    sqlx::query(
        "INSERT INTO email_tokens (user_id, email, token_hash, scope, expired, confirmed, created_at, updated_at) \
         SELECT u.id, e.email, $2, 2, false, false, now(), now() FROM users u \
         JOIN user_emails e ON e.user_id = u.id AND e.\"primary\" WHERE u.username = $1",
    )
    .bind(username)
    .bind(hash)
    .execute(pool)
    .await
    .unwrap();
    token
}

fn rand_token(seed: &str) -> u128 {
    seed.bytes().fold(0x1234_5678_9abc_def0_u128, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(b as u128)
    })
}

#[tokio::test]
async fn the_password_reset_page_asks_for_the_new_password() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let token = reset_token(&db.pool, "user1").await;
    let reply = page(&st, &format!("/u/password-reset/{token}"), &[]).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.header("cache-control"), Some("no-cache, no-store"));
    let html = &reply.body;
    assert!(
        html.contains("<title>Reset Password - Discourse</title>"),
        "{html}"
    );
    assert!(
        html.contains("<meta name=\"referrer\" content=\"never\">"),
        "{html}"
    );
    assert!(html.contains(" password-reset-page\""), "{html}");
    assert!(html.contains("<div class=\"container password-reset clearfix\" hx-boost=\"false\"><form class=\"change-password-form login-left-side\"><h2>Choose a new password</h2><div class=\"input\"><input placeholder=\"\" autocomplete=\"new-password\" autofocus=\"autofocus\" id=\"new-account-password\""), "{html}");
    assert!(
        html.contains("<span class=\"d-button-label\">Set Password</span></button></form></div>"),
        "{html}"
    );
    let data = reply.data();
    assert_eq!(data["page"], "password-reset");
    assert_eq!(data["token"], token.as_str());
    assert_eq!(data["password_reset"]["second_factor_required"], false);
    assert_eq!(data["password_reset"]["admin"], false);
    assert!(!html.contains("[en."), "every string is translated");

    // The JSON twin, and the token still confirmable: nothing is used up.
    let json = request(
        &st,
        Method::GET,
        &format!("/u/password-reset/{token}.json"),
        &[],
        "",
    )
    .await;
    let json: Value = serde_json::from_str(&json.body).unwrap();
    assert_eq!(json["security_key_required"], false, "{json}");
    assert_eq!(json["multiple_second_factor_methods"], false, "{json}");

    // users#confirm_email_token: a reset token is no signup's, so it
    // stays as it was.
    let confirm = request(
        &st,
        Method::GET,
        &format!("/u/confirm-email-token/{token}.json"),
        &[],
        "",
    )
    .await;
    assert_eq!(confirm.status, StatusCode::OK);
    assert_eq!(confirm.body, "{\"success\":\"OK\"}");
    let confirmed: bool = sqlx::query_scalar(
        "SELECT confirmed FROM email_tokens WHERE scope = 2 ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(!confirmed);
}

#[tokio::test]
async fn a_reset_for_a_second_factor_user_asks_for_it_first() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    sqlx::query(
        "INSERT INTO user_second_factors (user_id, method, data, enabled, created_at, updated_at, name) \
         SELECT id, m, 'JBSWY3DPEHPK3PXP', true, now(), now(), 'app' FROM users, (VALUES (1), (2)) v(m) WHERE username = 'user2'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let token = reset_token(&db.pool, "user2").await;
    let html = page(&st, &format!("/u/password-reset/{token}"), &[])
        .await
        .body;
    assert!(html.contains("<form class=\"change-password-form login-left-side\"><h2>Password Reset</h2><p>To continue, please verify your identity.</p>"), "{html}");
    assert!(html.contains("<div id=\"second-factor\"><h3>Two-Factor Authentication</h3><p class=\"second-factor__description\">"), "{html}");
    assert!(html.contains("class=\"d-otp-input second-factor-token-input\" data-slot=\"input-otp\" inputmode=\"numeric\" maxlength=\"6\" spellcheck=\"false\" id=\"second-factor\">"), "{html}");
    assert!(
        html.contains("<p><a class=\"toggle-second-factor-method\" href=\"\">"),
        "{html}"
    );
    assert!(!html.contains("id=\"new-account-password\""), "{html}");
}

#[tokio::test]
async fn a_used_reset_link_is_the_no_ember_error_page() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let reply = page(
        &st,
        "/u/password-reset/0123456789abcdef0123456789abcdef",
        &[],
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let html = &reply.body;
    assert!(
        html.contains("<title>Reset Password - Discourse</title>"),
        "{html}"
    );
    assert!(html.contains("<div class='invite-error'>"), "{html}");
    assert!(
        html.contains("Oops! The link you used no longer works."),
        "{html}"
    );
    let json = request(
        &st,
        Method::GET,
        "/u/password-reset/0123456789abcdef0123456789abcdef.json",
        &[],
        "",
    )
    .await;
    let json: Value = serde_json::from_str(&json.body).unwrap();
    assert!(
        json["message"].as_str().unwrap().starts_with("Oops!"),
        "{json}"
    );
}

#[tokio::test]
async fn forgot_password_opens_over_the_login_page_when_login_is_required() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    set_setting(&db.pool, "login_required", BOOL, "t").await;
    let reply = page(&st, "/password-reset?email=a%40b.com", &[]).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.body.contains("id=\"login-form\""));
    assert_eq!(reply.data()["forgot_password"], true);
    assert!(
        reply
            .cookies()
            .iter()
            .any(|c| c.starts_with("email=a%40b.com;")),
        "{:?}",
        reply.cookies()
    );
}

#[tokio::test]
async fn forgot_password_opens_over_the_homepage() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let reply = page(
        &st,
        "/password-reset?email=a%40b.com&order=views&page=2",
        &[],
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let html = &reply.body;
    // discovery.latest under the modal, as /latest draws it.
    assert!(html.contains("list-container --topic-list"), "{html}");
    assert!(!html.contains("id=\"login-form\""), "{html}");
    let data = reply.data();
    assert_eq!(data["page"], "forgot-password");
    assert_eq!(data["forgot_password"], true);
    // Only the discovery route's params go along to the URL.
    assert_eq!(data["url"], "/latest?order=views");
    assert!(!data["csrf"].as_str().unwrap_or("").is_empty(), "{data}");
    let cookies = reply.cookies();
    assert!(
        cookies.iter().any(|c| c.starts_with("email=a%40b.com;")),
        "{cookies:?}"
    );
    assert_eq!(
        cookies
            .iter()
            .filter(|c| c.starts_with("_forum_session="))
            .count(),
        1,
        "{cookies:?}"
    );
    assert_eq!(
        reply.header("cache-control"),
        Some("no-store, must-revalidate, private, max-age=0")
    );

    // A member gets it over their homepage even with login required.
    set_setting(&db.pool, "login_required", BOOL, "t").await;
    set_setting(&db.pool, "top_menu", 8, "categories|latest|new|top").await;
    let t = logged_in(&st).await;
    let member = page(&st, "/password-reset", &[&t]).await;
    assert_eq!(member.status, StatusCode::OK, "{}", member.body);
    assert!(!member.body.contains("id=\"login-form\""));
    assert_eq!(member.data()["url"], "/categories");

    // The route is a page; asked for over XHR it is not drawn here.
    let xhr = request(&st, Method::GET, "/password-reset", &[&t], "").await;
    assert_eq!(xhr.status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn account_created_shows_the_signups_message() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    // No signup in this session: activation.missing_session.
    let reply = page(&st, "/u/account-created", &[]).await;
    assert_eq!(reply.status, StatusCode::OK);
    let html = &reply.body;
    assert!(
        html.contains("<title>Activate your account - Discourse</title>"),
        "{html}"
    );
    assert!(html.contains(" account-created-page\""), "{html}");
    assert!(html.contains("<div class=\"account-created\"><div class=\"signup-progress-bar\"><div class=\"signup-progress-bar__segment --completed\">"), "{html}");
    assert!(
        html.contains(
            "<h1 class=\"login-title\">Welcome to Discourse!</h1></div><div class=\"success-info\">"
        ),
        "{html}"
    );
    assert!(!html.contains("activation-controls"), "{html}");
    // A member goes home.
    let t = logged_in(&st).await;
    let member = page(&st, "/u/account-created", &[&t]).await;
    assert_eq!(member.status, StatusCode::FOUND);
    assert_eq!(member.header("location"), Some("/"));
}
