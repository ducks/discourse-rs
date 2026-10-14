//! The about page's HTML (/about): Ember's about route from the about#index
//! document, its navigation, stats, staff lists, contact and activity,
//! and the sidebar's About link. The document is covered by the parity
//! golden files.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use tower::ServiceExt;

const BOOL: i32 = 5;
const EMAIL: i32 = 14;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    fn anonymous(state: AppState) -> Client {
        Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        }
    }

    /// A request;  for the client's own, with its CSRF token.
    async fn send(
        &mut self,
        method: Method,
        path: &str,
        body: &str,
        xhr: bool,
    ) -> (StatusCode, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        if xhr {
            request = request.header("x-requested-with", "XMLHttpRequest");
            if let Some(token) = &self.csrf {
                request = request.header("x-csrf-token", token.as_str());
            }
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
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap();
            let pair = v.split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let status = response.status();
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8_lossy(&bytes).into_owned();
        (status, location.unwrap_or(body))
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client::anonymous(state);
        let (_, csrf) = client
            .send(Method::GET, "/session/csrf.json", "", true)
            .await;
        let csrf: serde_json::Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
            .send(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
                true,
            )
            .await;
        client
    }

    /// The page as a browser asks for it.
    async fn page(&mut self, path: &str) -> (StatusCode, String) {
        self.send(Method::GET, path, "", false).await
    }
}

/// The staff seen lately, as the about route keeps them.
async fn seen_lately(db: &TestDb) {
    sqlx::query("UPDATE users SET last_seen_at = now() WHERE admin OR moderator")
        .execute(&db.pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn the_about_page_shows_the_site_and_its_staff() {
    let db = TestDb::new().await;
    seen_lately(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (status, html) = Client::anonymous(st).page("/about").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<title>About - "), "{html}");
    assert!(html.contains("has-sidebar-page about-page\""));
    // The navigation: About, and the FAQ under its guidelines name.
    assert!(html.contains(
        "<ul class=\"nav-pills\"><li class=\"nav-item-about\"><a class=\"active\" href=\"/about\">About</a></li><li class=\"nav-item-guidelines\"><a href=\"/guidelines\">Guidelines</a></li></ul>"
    ));
    // Only admins may edit it.
    assert!(!html.contains("edit-about-page"));
    assert!(html.contains("<span class=\"about__stats-item members\">"));
    assert!(html.contains("<span>1 Admin</span>"));
    assert!(html.contains("<span class=\"about__stats-item site-creation-date\">"));
    assert!(html.contains("<section class=\"about__admins\"><h2>Our Admins</h2><div class=\"about-page-users-list\"><div class=\"user-info small\" data-username=\"admin\">"));
    assert!(html.contains("title=\"admin\">"));
    assert!(html.contains("<h2>Contact us</h2><p>If you come across any inappropriate content"));
    assert!(html.contains("<div class=\"about__activities-item likes\">"));
    assert!(html.contains("<div class=\"about__activities-item-period\">all time</div>"));
    // No contact details set: no contact line.
    assert!(!html.contains("about__contact-info"));
    // The sidebar's About link, out of the More menu and current.
    assert!(html.contains("<li class=\"sidebar-section-link-wrapper\" data-list-item-name=\"about\"><a class=\"active sidebar-section-link sidebar-row\""));
    assert!(!html.contains("dropdown-menu__item\" data-list-item-name=\"about\""));
    // crawlable_meta_data with the about title.
    assert!(html.contains("<meta property=\"og:title\" content=\"About - "));
}

#[tokio::test]
async fn admins_get_the_edit_link() {
    let db = TestDb::new().await;
    seen_lately(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (_, html) = admin.page("/about").await;
    assert!(
        html.contains("<a class=\"edit-about-page\" href=\"/admin/config/about\">"),
        "{html}"
    );
}

#[tokio::test]
async fn staff_not_seen_in_a_year_are_left_out() {
    let db = TestDb::new().await;
    seen_lately(&db).await;
    sqlx::query(
        "UPDATE users SET last_seen_at = now() - interval '13 months' WHERE username = 'admin'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (_, html) = Client::anonymous(st).page("/about").await;
    assert!(!html.contains("about__admins"), "{html}");
    assert!(!html.contains("about__stats-item admins"));
}

#[tokio::test]
async fn the_contact_email_shows_as_a_link() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "contact_email", EMAIL, "help@example.com").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (_, html) = Client::anonymous(st).page("/about").await;
    assert!(
        html.contains("<p class=\"about__contact-info\">In the event of a critical issue or urgent matter affecting this site, please contact <a href=\"mailto:help@example.com\">help@example.com</a>.</p>"),
        "{html}"
    );
}

#[tokio::test]
async fn more_than_six_admins_fold_behind_view_more() {
    let db = TestDb::new().await;
    for i in 0..7 {
        sqlx::query(
            "INSERT INTO users (username, username_lower, name, admin, active, approved, trust_level, created_at, updated_at, last_seen_at) \
             VALUES ($1, $1, NULL, true, true, true, 1, now(), now(), now())",
        )
        .bind(format!("boss{i}"))
        .execute(&db.pool)
        .await
        .unwrap();
    }
    seen_lately(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (_, html) = Client::anonymous(st).page("/about").await;
    assert!(
        html.contains("about-page-users-list__expand-button"),
        "{html}"
    );
    assert!(html.contains("<div hidden class=\"user-info small\""));
    assert!(html.contains("<span class=\"d-button-label\">View more</span>"));
}

#[tokio::test]
async fn login_required_sends_visitors_to_log_in() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "login_required", BOOL, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (status, location) = Client::anonymous(st).page("/about").await;
    assert_eq!(status, StatusCode::FOUND);
    assert!(location.ends_with("/login"), "{location}");
}
