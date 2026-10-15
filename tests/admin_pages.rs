//! The admin pages' HTML: the admin shell (the admin sidebar panel, the
//! page header with its breadcrumbs, the admin classes) and the /admin
//! dashboard drawn from the document /admin/dashboard.json serves.

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
    mobile: bool,
}

impl Client {
    fn anonymous(state: AppState) -> Client {
        Client {
            state,
            cookies: Vec::new(),
            csrf: None,
            mobile: false,
        }
    }

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
        let mut cookies: Vec<String> = self
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        if self.mobile {
            cookies.push("mobile_view=1".into());
        }
        if !cookies.is_empty() {
            request = request.header(header::COOKIE, cookies.join("; "));
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
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
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

    async fn page(&mut self, path: &str) -> (StatusCode, String) {
        self.send(Method::GET, path, "", false).await
    }
}

async fn admin_state(db: &TestDb) -> AppState {
    // The redesigned dashboard, as the reference runs it.
    set_setting(&db.pool, "dashboard_improvements", BOOL, "t").await;
    state(db.pool.clone(), config(RailsEnv::Test, &[])).await
}

#[tokio::test]
async fn only_staff_get_the_admin() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let (status, html) = Client::anonymous(st.clone()).page("/admin").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!html.contains("admin-panel"));
    let (status, _) = Client::logged_in(st, "user1").await.page("/admin").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_dashboard_has_the_admin_shell() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let (status, html) = Client::logged_in(st, "admin").await.page("/admin").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Dashboard - Admin - "), "{html}");
    assert!(html.contains(" admin-area\""));
    assert!(html.contains(" admin-interface\""));
    // Rails serves staff its admin stylesheet.
    assert!(html.contains("/assets/admin.css?v="));
    // The admin panel in place of the forum's sections.
    assert!(html.contains("<div class=\"sidebar-sections admin-panel\">"));
    assert!(html.contains("<a class=\"sidebar-sections__back-to-forum\" href=\"/\">"));
    assert!(html.contains("<span>Back to Forum</span>"));
    assert!(!html.contains("data-section-name=\"community\""));
    assert!(html.contains("<li class=\"sidebar-section-link-wrapper\" data-list-item-name=\"admin_home\"><a class=\"active sidebar-section-link sidebar-row admin-sidebar-nav-link\" data-link-name=\"admin_home\" href=\"/admin\">"));
    assert!(html.contains("data-link-name=\"admin_all_site_settings\" href=\"/admin/site_settings/category/all_results\""));
    assert!(html.contains("data-link-name=\"admin_upcoming_changes\""));
    // The other sections start collapsed, their links hidden.
    assert!(html.contains("<div class=\"sidebar-section sidebar-section-wrapper sidebar-section--collapsed\" data-section-name=\"admin-reports\">"));
    assert!(html.contains(
        "<ul class=\"sidebar-section-content\" id=\"sidebar-section-content-admin-reports\" hidden>"
    ));
    // Admins search; the filter is the moderators'.
    assert!(html.contains("class=\"sidebar-search__input\""));
    assert!(!html.contains("sidebar-filter__input"));
    // The page header.
    assert!(html.contains("<ul class=\"d-breadcrumbs\"><li class=\"d-breadcrumbs__item\"><a class=\"d-breadcrumbs__link\" href=\"/admin\">Admin</a> <span class=\"separator\">"));
    assert!(html.contains("<h1 class=\"d-page-header__title\">Dashboard</h1>"));
    assert!(html.contains("<span class=\"d-button-label\">Last 30 days</span>"));
    assert!(html.contains("data-identifier=\"db-configure\""));
    assert!(html.contains("<div class=\"admin-contents dashboard general dashboard-next\">"));
    // No Powered by Discourse in the admin (hideApplicationFooter).
    assert!(!html.contains("class=\"powered-by-discourse\""));
}

#[tokio::test]
async fn the_dashboard_draws_its_sections() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let (_, html) = Client::logged_in(st, "admin").await.page("/admin").await;
    for id in [
        "highlights",
        "reports",
        "traffic",
        "engagement",
        "search",
        "system",
    ] {
        assert!(
            html.contains(&format!(
                "<section class=\"db-section --{id}\" data-section-id=\"{id}\">"
            )),
            "{id}: {html}"
        );
    }
    // The highlights' KPI tiles link to their reports for the range.
    assert!(
        html.contains(
            "<a class=\"db-kpi db-section__row-block\" aria-label=\"DAU / MAU stickiness, "
        )
    );
    assert!(html.contains("<div class=\"db-kpi__label\">New sign-ups <span aria-expanded=\"false\" class=\"fk-d-tooltip__trigger db-kpi__tooltip\""));
    // The pinned reports, loaded by admin-dashboard.js.
    assert!(html.contains("data-identifier=\"core_report:daily_engaged_users\""));
    assert!(html.contains("<a class=\"db-report__name\" href=\"/admin/reports/daily_engaged_users\">Daily Engaged Users</a>"));
    assert!(html.contains("/assets/admin-dashboard.js?v="));
    assert!(html.contains("/assets/chart.umd.min.js?v="));
    // The traffic chart's data.
    assert!(html.contains("class=\"db-section__traffic-chart-canvas admin-report-chart admin-report-stacked-chart\" data-chart="));
    // The trust level pipeline, from top to bottom.
    assert!(html.contains("<ol class=\"db-tl-pipeline__rows\"><li class=\"db-tl-pipeline__row\"><div class=\"db-tl-pipeline__label\"><span class=\"db-tl-pipeline__name\">Leader</span>"));
    // The system section's version.
    assert!(html.contains("<h3 class=\"db-section__header\">"));
    assert!(html.contains("View on GitHub"));
    // The strings admin-dashboard.js draws with.
    assert!(html.contains("<script type=\"application/json\" id=\"db-strings\">"));
    assert!(html.contains("\"admin.dashboard.reports_section.no_data\":\"No data to display\""));
}

#[tokio::test]
async fn the_range_comes_from_the_query() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (_, html) = admin.page("/admin?range=last_7_days").await;
    assert!(
        html.contains("<span class=\"d-button-label\">Last 7 days</span>"),
        "{html}"
    );
    assert!(html.contains("data-active=\"last_7_days\""));
    let (_, html) = admin
        .page("/admin?range=custom&start_date=2026-01-02&end_date=2026-02-03")
        .await;
    assert!(
        html.contains("<span class=\"d-button-label\">Jan 2, 2026 – Feb 3, 2026</span>"),
        "{html}"
    );
    assert!(html.contains("data-from=\"2026-01-02\" data-to=\"2026-02-03\""));
}

#[tokio::test]
async fn moderators_are_refused_with_their_current_user() {
    let db = TestDb::new().await;
    sqlx::query("UPDATE users SET moderator = true WHERE username = 'user1'")
        .execute(&db.pool)
        .await
        .unwrap();
    let st = admin_state(&db).await;
    // The page is refused before it is drawn: a moderator's current user
    // (its reviewable counts) is not ported. The panel's moderator links
    // and filter are covered by admin_view's unit tests.
    let (status, _) = Client::logged_in(st, "user1").await.page("/admin").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn the_mobile_view_puts_the_panel_in_the_header_menu() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st, "admin").await;
    admin.mobile = true;
    let (_, html) = admin.page("/admin").await;
    assert!(
        html.contains(
            "<template id=\"hamburger-dropdown\"><div class=\"hamburger-dropdown-wrapper\">"
        ),
        "{html}"
    );
    assert!(html.contains(
        "<div class=\"sidebar-hamburger-dropdown\"><div class=\"sidebar-sections admin-panel\">"
    ));
    assert!(!html.contains("id=\"d-sidebar\""));
}

#[tokio::test]
async fn the_legacy_dashboard_is_refused() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "dashboard_improvements", BOOL, "f").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (status, _) = Client::logged_in(st, "admin").await.page("/admin").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}
