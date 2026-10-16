//! The /admin/config areas' HTML (90g6): each area in the admin shell
//! with its header and content as Ember draws them.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use tower::ServiceExt;

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
    state(db.pool.clone(), config(RailsEnv::Test, &[])).await
}

#[tokio::test]
async fn the_config_areas_that_are_their_settings() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, html) = admin.page("/admin/config/localization").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Localization - Admin - "), "{html}");
    assert!(html.contains("<div class=\"admin-contents admin-config localization settings\">"));
    assert!(html.contains(
        "<li aria-current=\"page\" class=\"d-breadcrumbs__item\"><a class=\"d-breadcrumbs__link\" href=\"/admin/config/localization\">Localization</a></li>"
    ));
    assert!(html.contains("<h1 class=\"d-page-header__title\">Localization</h1>"));
    assert!(html.contains("<div class=\"admin-config-page__main-area\"><div class=\"content-body admin-config-area__settings"), "{html}");
    assert!(html.contains("data-setting=\"default_locale\""));
    // A locale list draws as a compact list.
    assert!(!html.contains("Not ported yet: the locale list setting editor"));
    assert!(html.contains("/site-settings.js"));
    // The categories an area takes, and its learn more link.
    let (_, html) = admin.page("/admin/config/spam").await;
    assert!(html.contains("<div class=\"admin-contents admin-config spam settings\">"));
    assert!(html.contains("https://meta.discourse.org/t/tips-for-preventing-spam/264020"));
    let (_, html) = admin.page("/admin/config/analytics-and-seo").await;
    assert!(html.contains("<div class=\"admin-contents admin-config admin-analytics settings\">"));
    // The JSON is the site settings controller's.
    let (status, body) = admin
        .send(
            Method::GET,
            "/admin/config/localization.json?filter_area=localization",
            "",
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("{\"site_settings\":["), "{body}");
    // Only admins.
    let (status, _) = Client::logged_in(st, "user1")
        .await
        .page("/admin/config/localization")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_flags_pages() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, html) = admin.page("/admin/config/flags").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Moderation - Admin - "), "{html}");
    assert!(html.contains("<div class=\"admin-contents admin-config flags\">"));
    assert!(html.contains(
        "admin-flags__header-add-flag\" title=\"Add flag\" data-href=\"/admin/config/flags/new\""
    ));
    assert!(html.contains("<li class=\"active admin-flags-tabs__flags\">"));
    assert!(
        html.contains(
            "<tr class=\"d-table__row admin-flag-item off_topic saved\" data-flag-id=\"3\""
        )
    );
    // A system flag can't be edited; notify_user can't move.
    assert!(html.contains("admin-flag-item__edit\" disabled=\"\""));
    assert_eq!(html.matches("flag-menu-trigger").count(), 5, "{html}");
    assert!(html.contains("/admin-config.js"));
    // The settings tab.
    let (_, html) = admin.page("/admin/config/flags/settings").await;
    assert!(html.contains("<div class=\"admin-contents admin-config flags settings\">"));
    assert!(html.contains("<title>Flags Settings - Admin - "));
    assert!(html.contains("content-body admin-config-area__settings"));
    // The forms, without the header.
    let (_, html) = admin.page("/admin/config/flags/new").await;
    assert!(html.contains("<div class=\"admin-contents admin-config flags new\"><div class=\"admin-container admin-config-page__main-area\"><a class=\"active btn btn-transparent back-button\" href=\"/admin/config/flags\">"), "{html}");
    assert!(html.contains("<h3 class=\"admin-config-area-card__title\">Add flag</h3>"));
    assert!(html.contains("name=\"enabled\" type=\"checkbox\" checked=\"\""));
    let (_, html) = admin.page("/admin/config/flags/3").await;
    assert!(html.contains("<title>Edit Flag - Admin - "));
    assert!(html.contains("<h3 class=\"admin-config-area-card__title\">Edit flag</h3>"));
    assert!(html.contains("value=\"Off-Topic\""));
    // The JSON stays the empty action.
    let (status, _) = admin
        .send(Method::GET, "/admin/config/flags.json", "", true)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}
