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
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8_lossy(&bytes).into_owned();
        // A redirect's location, else the body.
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

#[tokio::test]
async fn the_users_index_goes_to_the_active_list() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st, "admin").await;
    for path in ["/admin/users", "/admin/users/list"] {
        let (status, location) = admin.page(path).await;
        assert_eq!(status, StatusCode::SEE_OTHER, "{path}");
        assert_eq!(location, "/admin/users/list/active");
    }
    // The JSON stays the index's.
    let (status, body) = admin
        .send(Method::GET, "/admin/users/list/active", "", true)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("[{\"id\":"), "{body}");
}

#[tokio::test]
async fn the_users_list_page() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, html) = admin.page("/admin/users/list/active").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Users - Admin - "));
    assert!(html.contains("<div class=\"admin-contents admin-users admin-users-list\">"));
    assert!(html.contains("data-link-name=\"admin_users\" href=\"/admin/users\">"));
    assert!(html.contains("<a class=\"active sidebar-section-link sidebar-row admin-sidebar-nav-link\" data-link-name=\"admin_users\""));
    // The header: its actions, description and tabs.
    assert!(html.contains("<button class=\"btn d-page-action-button btn-small btn-primary admin-users__header-export-users\" title=\"Export full user list in CSV format.\" type=\"button\"><span class=\"d-button-label\">Export</span></button>"));
    assert!(html.contains("<span class=\"d-page-header__learn-more\"><a href='https://meta.discourse.org/t/accessing-a-user-s-admin-page/311859' target='_blank'>Learn more…</a></span>"));
    assert!(html.contains("<li class=\"active admin-users-tabs__active\"><a class=\"active\" href=\"/admin/users/list/active\">Active</a></li>"));
    assert!(html.contains(
        "<li class=\"admin-users-tabs__staff\"><a href=\"/admin/users/list/staff\">Staff</a></li>"
    ));
    // The list.
    assert!(html.contains("<h2 class=\"d-page-subheader__title\">Active users</h2>"));
    assert!(html.contains("admin-users__subheader-show-emails"));
    assert!(html.contains("class=\"directory-table users-list active\" role=\"table\" style=\"grid-template-columns: minmax(min-content, 2fr) repeat(7, minmax(min-content, 1fr))\""));
    assert!(html.contains("<a href=\"/admin/users/1/admin\">admin</a>"));
    assert!(html.contains("<span class=\"svg-icon-title\" title=\"Discourse Admin\">"));
    assert!(html.contains("/assets/admin-users.js?v="));
    // The filter and order come from the URL.
    let (_, html) = admin
        .page("/admin/users/list/active?filter=user1&order=username&asc=true")
        .await;
    assert!(html.contains("value=\"user1\""), "{html}");
    assert!(html.contains("<a href=\"/admin/users/3/user1\">user1</a>"));
    assert!(!html.contains("<a href=\"/admin/users/1/admin\">admin</a>"));
    assert!(html.contains("aria-sort=\"ascending\" role=\"columnheader\" data-order=\"username\""));
    // Showing emails adds their column.
    let (_, html) = admin
        .page("/admin/users/list/active?show_emails=true")
        .await;
    assert!(
        html.contains("repeat(8, minmax(min-content, 1fr))"),
        "{html}"
    );
    assert!(html.contains("admin-users__subheader-hide-emails"));
}

#[tokio::test]
async fn the_users_lists_filters_and_empty_states() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (_, html) = admin.page("/admin/users/list/staff").await;
    assert!(html.contains("<select aria-label=\"Human accounts\" class=\"d-filter-controls__dropdown d-native-select\">"), "{html}");
    let (_, html) = admin.page("/admin/users/list/staff?account_type=all").await;
    assert!(html.contains("<select aria-label=\"All accounts\" class=\"d-filter-controls__dropdown --active d-native-select\">"), "{html}");
    assert!(html.contains("<a href=\"/admin/users/-1/system\">system</a>"));
    let (_, html) = admin.page("/admin/users/list/suspended").await;
    assert!(
        html.contains("<p class=\"admin-users-list__no-results\">No results found.</p>"),
        "{html}"
    );
    let (_, html) = admin
        .page("/admin/users/list/active?filter=nobody-at-all")
        .await;
    assert!(
        html.contains("<div class=\"d-filter-controls__no-results\"><p>No results found.</p>"),
        "{html}"
    );
}

#[tokio::test]
async fn a_users_admin_page() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, html) = admin.page("/admin/users/3/user1").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>user1 - Users - Admin - "));
    assert!(html.contains("<div class=\"admin-contents admin-users admin-user\" data-user-id=\"3\" data-username=\"user1\">"));
    assert!(html.contains("<a class=\"btn btn-default\" href=\"/u/user1\">"));
    assert!(html.contains("<div class=\"display-row username\" data-editable=\"username\"><div class=\"field\">Username</div><div class=\"value\"><a class=\"inline-editable-field\" href=\"\"><span>user1</span></a></div>"));
    // Another member's email is revealed on request.
    assert!(html.contains("<span class=\"d-button-label\">Show</span>"));
    assert!(html.contains("<h1>Permissions</h1>"));
    assert!(html.contains("data-name=\"2: member\" data-value=\"2\""));
    assert!(html.contains("Grant Moderation"));
    assert!(
        html.contains(
            "<button class=\"btn btn-icon-text btn-danger suspend-user\" type=\"button\">"
        )
    );
    assert!(html.contains("<h1>Groups</h1>"));
    assert!(html.contains("<a href=\"/g/trust_level_0\">trust_level_0</a>"));
    assert!(html.contains("<h1>Activity</h1>"));
    assert!(html.contains("<h1>User exports</h1>"));
    assert!(html.contains("btn-impersonate"));
    assert!(html.contains(
        "<details class=\"select-kit single-select dropdown-select-box btn-user-delete\">"
    ));
    assert!(html.contains("<script type=\"application/json\" id=\"admin-user-strings\">"));
    // An admin's own page shows its email; admins can't be deleted.
    let (_, html) = admin.page("/admin/users/1/admin").await;
    assert!(
        html.contains("<a href=\"mailto:admin@example.com\">admin@example.com</a>"),
        "{html}"
    );
    assert!(html.contains("<span class=\"delete-explanation\">"));
    // Nobody by that id.
    let (status, _) = admin.page("/admin/users/999999/nobody").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Its tabs are not ported.
    let (status, _) = admin.page("/admin/users/3/user1/badges").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn the_site_settings_index_goes_to_required() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, location) = admin.page("/admin/site_settings").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location, "/admin/site_settings/category/required");
    // The route's query rides along.
    let (_, location) = admin.page("/admin/site_settings?filter=title").await;
    assert_eq!(
        location,
        "/admin/site_settings/category/required?filter=title"
    );
    // The JSON stays the index's.
    let (status, body) = admin
        .send(Method::GET, "/admin/site_settings", "", true)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("{\"site_settings\":["), "{body}");
    // AdminConstraint: only admins.
    let (status, _) = Client::logged_in(st, "user1")
        .await
        .page("/admin/site_settings/category/required")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_site_settings_category_page() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, html) = admin.page("/admin/site_settings/category/required").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(
        html.contains("<title>All site settings - Admin - "),
        "{html}"
    );
    // The filter controls and the category nav, Required active.
    assert!(html.contains("id=\"setting-filter\""));
    assert!(html.contains("id=\"setting-filter-toggle-overridden\""));
    assert!(html.contains(
        "<a class=\"active required\" title=\"Required\" href=\"/admin/site_settings/category/required\">Required</a>"
    ));
    // Each setting's row: its form-kit control and description.
    assert!(html.contains("data-setting=\"title\""));
    assert!(html.contains(
        "<input class=\"form-kit__control-input\" id=\"setting-title\" name=\"title\" value=\"Discourse\" type=\"text\">"
    ));
    assert!(html.contains("<select class=\"form-kit__control-select d-native-select\""));
    assert!(html.contains("id=\"site-settings-strings\""));
    assert!(html.contains("/site-settings.js"));
    // The legacy editors: a simple list's values and a group list's names.
    let (_, html) = admin.page("/admin/site_settings/category/basic").await;
    assert!(
        html.contains("<div class=\"simple-list value-list\">"),
        "{html}"
    );
    assert!(html.contains("title=\"latest\" value=\"latest\""));
    assert!(html.contains("class=\"select-kit single-select combobox combo-box add-value-input\""));
    let (_, html) = admin.page("/admin/site_settings/category/posting").await;
    assert!(html.contains("<span class=\"formatted-selection\">admins, moderators</span>"));
    // Not ported yet, said in place.
    let (_, html) = admin
        .page("/admin/site_settings/category/all_results?filter=moderator_guide_topic")
        .await;
    assert!(
        html.contains("Not ported yet: the topic setting editor."),
        "{html}"
    );
}

#[tokio::test]
async fn the_site_settings_filters() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "short_site_description", 1, "A test forum").await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, html) = admin
        .page("/admin/site_settings/category/all_results?filter=contact")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("data-setting=\"contact_email\""));
    assert!(!html.contains("data-setting=\"title\""));
    assert!(html.contains("value=\"contact\""));
    // A category without a match goes to All.
    let (status, location) = admin
        .page("/admin/site_settings/category/posting?filter=contact_email")
        .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        location,
        "/admin/site_settings/category/all_results?filter=contact_email"
    );
    // Only the overridden ones, with their undo button.
    let (_, html) = admin
        .page("/admin/site_settings/category/all_results?onlyOverridden=true")
        .await;
    assert!(
        html.contains("data-setting=\"short_site_description\""),
        "{html}"
    );
    assert!(!html.contains("data-setting=\"title\""));
    assert!(html.contains("setting-controls__undo"));
}

#[tokio::test]
async fn the_admin_groups_pages() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, html) = admin.page("/admin/groups").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Groups - Admin - "), "{html}");
    assert!(html.contains("<div class=\"admin-contents admin-groups\">"));
    assert!(html.contains(
        "<li class=\"active admin-groups-tabs__index\"><a class=\"active\" href=\"/admin/groups\">Groups</a></li>"
    ));
    assert!(html.contains("<section class=\"container groups-index\">"));
    assert!(html.contains("data-group-name=\"admins\""));
    assert!(html.contains("/groups.js"));
    // The index's JSON is empty.
    let (status, body) = admin
        .send(Method::GET, "/admin/groups.json", "", true)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(body.is_empty());
    // The settings tab: the groups category's settings.
    let (status, html) = admin.page("/admin/groups/settings").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Settings - Admin - "), "{html}");
    assert!(html.contains("<div class=\"admin-contents admin-groups settings\">"));
    assert!(html.contains(
        "<div class=\"content-body admin-config-area__settings admin-detail pull-left\">"
    ));
    assert!(html.contains("data-setting=\"enable_group_directory\""));
    assert!(!html.contains("data-setting=\"title\""));
    assert!(html.contains("<a class=\"d-breadcrumbs__link\" href=\"/admin/groups/settings\">"));
    // Not for members.
    let (status, _) = Client::logged_in(st, "user1")
        .await
        .page("/admin/groups")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_users_settings_tab() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, html) = admin.page("/admin/users/settings").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<div class=\"admin-contents admin-users settings\">"));
    assert!(html.contains(
        "<li class=\"active admin-users-tabs__settings\"><a class=\"active\" href=\"/admin/users/settings\">Settings</a></li>"
    ));
    // The users area's settings, whatever their category.
    assert!(html.contains("data-setting=\"min_username_length\""));
    assert!(html.contains("data-setting=\"enable_user_tips\""));
    let (_, html) = admin.page("/admin/users/settings?filter=username").await;
    assert!(html.contains("data-setting=\"min_username_length\""));
    assert!(!html.contains("data-setting=\"enable_user_tips\""));
}

#[tokio::test]
async fn the_config_areas_settings_json() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, body) = admin
        .send(
            Method::GET,
            "/admin/config/site_settings.json?filter_area=flags",
            "",
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let names: Vec<&str> = json["site_settings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["setting"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"flag_post_allowed_groups"), "{names:?}");
    assert!(!names.contains(&"title"));
    let (status, _) = admin
        .send(Method::GET, "/admin/config/site_settings.json", "", true)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
