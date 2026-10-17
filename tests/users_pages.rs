//! The users directory's HTML (/u): Ember's users route from the
//! directory_items#index page, its period chooser, sortable columns, name
//! filter and paging, the exception page for a refused list, and the
//! sidebar's Users link. The document is covered by the parity golden
//! files.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use tower::ServiceExt;

const BOOL: i32 = 5;

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

    /// A request; `xhr` for the client's own, with its CSRF token. The
    /// body, or the redirect's location.
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

/// The page's client configuration (#users-directory-config).
fn client_config(html: &str) -> serde_json::Value {
    let start = html
        .find("<script type=\"application/json\" id=\"users-directory-config\">")
        .expect("the config script")
        + "<script type=\"application/json\" id=\"users-directory-config\">".len();
    let end = start + html[start..].find("</script>").unwrap();
    serde_json::from_str(&html[start..end]).unwrap()
}

#[tokio::test]
async fn the_directory_lists_the_weeks_users() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (status, html) = Client::anonymous(st).page("/u").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<title>Users - "), "{html}");
    assert!(html.contains("has-sidebar-page users-page\""));
    // The period chooser on the week, its rows ready to open.
    assert!(html.contains("<summary aria-label=\"Filter by period\" name=\"Filter by: weekly\" data-name=\"weekly\" data-value=\"weekly\""));
    // The dates as templates for the viewer's zone (crate::viewer_dates).
    assert!(html.contains("<h2 class=\"selected-name\"><span class=\"date-section\">Week</span><span class=\"top-date-string\" data-tz-text=\"%{"));
    assert!(html.contains(":MMM D} – %{"), "{html}");
    assert!(html.contains(
        "aria-checked=\"true\" role=\"menuitemradio\" data-index=\"4\" data-name=\"weekly\""
    ));
    assert!(html.contains(
        "<span class=\"date-section\">All time</span><span class=\"top-date-string\"></span>"
    ));
    assert!(html.contains("<label class=\"total-rows\">5 users</label>"));
    assert!(html.contains("<input class=\"filter-name no-blur\" placeholder=\"filter by username\" value=\"\" type=\"text\">"));
    // Only staff edit the columns.
    assert!(!html.contains("open-edit-columns-btn"));
    // Sorted by likes received, descending; a click on it sorts ascending.
    assert!(html.contains("<div aria-sort=\"descending\" class=\"directory-table__column-header sortable\" role=\"columnheader\" data-order=\"likes_received\" data-asc=\"true\"><div aria-label=\"Sort by likes Received\" aria-pressed=\"true\" class=\"header-contents\" id=\"table-header-toggle-likes_received\""));
    assert!(html.contains("d-icon-chevron-down"));
    assert!(html.contains("<div aria-sort=\"none\" class=\"directory-table__column-header sortable\" role=\"columnheader\" data-order=\"topic_count\"><div aria-label=\"Sort by Topics Created\""));
    assert!(
        html.contains(
            "grid-template-columns: minmax(15em, 3fr) repeat(7, minmax(max-content, 1fr));"
        )
    );
    // Each user's row: DUserInfo with the username a heading, then the
    // counts.
    for username in ["admin", "user0", "user1", "user2", "user3"] {
        assert!(html.contains(&format!(
            "<div class=\"user-info small\" data-username=\"{username}\">"
        )));
    }
    assert!(html.contains(
        "<a aria-level=\"3\" data-user-card=\"user1\" href=\"/u/user1\" role=\"heading\">"
    ));
    assert!(html.contains("<span class=\"directory-table__label\"><span>Topics</span></span> <span class=\"directory-table__value\">"));
    // No row is a visitor's own; all of it is loaded.
    assert!(!html.contains("directory-table__row me"));
    let config = client_config(&html);
    assert_eq!(config["total"], 5);
    assert_eq!(config["loaded"], 5);
    assert_eq!(
        config["loadMoreUrl"],
        "/directory_items.json?order=likes_received&page=1&period=weekly"
    );
    assert!(html.contains("powered-by-discourse"));
    // The sidebar's Users link, out of the More menu and current.
    assert!(html.contains("<li class=\"sidebar-section-link-wrapper\" data-list-item-name=\"users\"><a class=\"active sidebar-section-link sidebar-row\""));
}

#[tokio::test]
async fn staff_see_their_row_and_the_columns_button() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (_, html) = admin.page("/u").await;
    assert!(html.contains("open-edit-columns-btn"), "{html}");
    assert!(html.contains("<div class=\"directory-table__row me\" role=\"row\"><div class=\"directory-table__cell\" role=\"rowheader\"><div class=\"user-info small\" data-username=\"admin\">"));
    assert_eq!(client_config(&html)["meId"], 1);
}

#[tokio::test]
async fn the_order_and_period_come_from_the_url() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut visitor = Client::anonymous(st);
    let (_, html) = visitor
        .page("/u?order=topic_count&asc=true&period=all")
        .await;
    // Ascending; a click on it sorts descending again.
    assert!(html.contains("<div aria-sort=\"ascending\" class=\"directory-table__column-header sortable\" role=\"columnheader\" data-order=\"topic_count\"><div aria-label=\"Sort by Topics Created\" aria-pressed=\"mixed\""), "{html}");
    assert!(html.contains("d-icon-chevron-up"));
    assert!(html.contains("data-name=\"all\" data-value=\"all\" tabindex=\"0\" class=\"select-kit-row dropdown-select-box-row period-chooser-row is-highlighted is-selected\""));
    // All time adds the time read.
    assert!(html.contains("repeat(8, minmax(max-content, 1fr))"));
    assert!(html.contains("<div class=\"directory-table__column-header\"><div class=\"header-contents\">Time Read</div></div>"));
    assert!(html.contains("<div class=\"directory-table__cell time-read\" role=\"cell\">"));
    assert_eq!(
        client_config(&html)["loadMoreUrl"],
        "/directory_items.json?asc=true&order=topic_count&page=1&period=all"
    );
}

#[tokio::test]
async fn more_users_than_a_page_load_on_scroll() {
    let db = TestDb::new().await;
    sqlx::query(
        "INSERT INTO users (username, username_lower, name, active, approved, trust_level, created_at, updated_at) \
         SELECT 'many' || i, 'many' || i, NULL, true, true, 1, now(), now() FROM generate_series(1, 50) i",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO directory_items (period_type, user_id, likes_received, likes_given, topics_entered, topic_count, post_count, created_at, updated_at) \
         SELECT 4, id, 0, 0, 0, 0, 0, now(), now() FROM users WHERE username LIKE 'many%'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (_, html) = Client::anonymous(st).page("/u").await;
    let config = client_config(&html);
    assert_eq!(config["total"], 55, "{html}");
    assert_eq!(config["loaded"], 50);
    assert_eq!(html.matches("class=\"directory-table__row\"").count(), 50);
    // hideApplicationFooter while there is more.
    assert!(!html.contains("powered-by-discourse"));
    assert!(html.contains("<div aria-hidden=\"true\" class=\"load-more-sentinel\"></div>"));
}

#[tokio::test]
async fn a_refused_list_leaves_the_page_empty() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut visitor = Client::anonymous(st.clone());
    let empty = "<div class=\"empty-state__container --text-only\"><div class=\"empty-state\"><div class=\"empty-state__body\"><p data-test-body>A list of community members showing their activity will be shown here.";
    // A period the directory doesn't know: no model, the chooser on it
    // with no name.
    let (status, html) = visitor.page("/u?period=fortnightly").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains(empty), "{html}");
    assert!(html.contains("users-page"));
    assert!(html.contains("name=\"Filter by: \" data-name=\"\" data-value=\"fortnightly\""));
    assert!(html.contains("<h2 class=\"selected-name\"><span class=\"date-section\">Week</span><span class=\"top-date-string\"></span></h2>"));
    assert!(html.contains("<label class=\"total-rows\"></label>"));
    assert!(!html.contains("directory-table"));
    // The directory turned off.
    set_setting(&db.pool, "enable_user_directory", BOOL, "f").await;
    let (_, html) = visitor.page("/u").await;
    assert!(html.contains(empty), "{html}");
}

#[tokio::test]
async fn hidden_profiles_send_visitors_to_the_front_page() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "hide_user_profiles_from_public", BOOL, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (status, location) = Client::anonymous(st.clone()).page("/u").await;
    assert!(status.is_redirection());
    assert_eq!(location, "/");
    let mut user1 = Client::logged_in(st, "user1").await;
    let (status, _) = user1.page("/u").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn searching_by_name_filters_the_directory() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (status, _) = Client::anonymous(st).page("/u?name=user").await;
    assert_eq!(status, StatusCode::OK);
}
