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

#[tokio::test]
async fn the_permalinks_pages() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    // None yet: the empty list.
    let (status, html) = admin.page("/admin/config/permalinks").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Permalinks - Admin - "), "{html}");
    assert!(html.contains("<div class=\"admin-contents admin-config admin-permalinks\"><div class=\"admin-permalinks admin-config-page\"><div class=\"d-page-header\">"));
    assert!(
        html.contains("admin-config-area-empty-list__cta-button admin-permalinks__add-permalink")
    );
    sqlx::query("INSERT INTO permalinks (url, external_url, created_at, updated_at) VALUES ('old/path', 'https://example.com/x', now(), now())")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, html) = admin.page("/admin/config/permalinks").await;
    assert!(
        html.contains("<tr class=\"d-table__row admin-permalink-item old_path\""),
        "{html}"
    );
    assert!(html.contains("d-icon-up-right-from-square"));
    assert!(html.contains("<a href=\"https://example.com/x\">https://example.com/x</a>"));
    assert!(html.contains("d-filter-controls__input"));
    // A filter's results, for the script.
    let (_, html) = admin.page("/admin/config/permalinks?filter=zzz").await;
    assert!(
        html.starts_with("<div class=\"d-filter-controls__no-results\">"),
        "{html}"
    );
    // The forms go without the header; the edit shows the type's field.
    let id: i32 = sqlx::query_scalar("SELECT id FROM permalinks WHERE url = 'old/path'")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let (_, html) = admin.page(&format!("/admin/config/permalinks/{id}")).await;
    assert!(html.contains("<div class=\"admin-contents admin-config admin-permalinks edit\"><div class=\"admin-permalinks admin-config-page\"><div class=\"admin-container admin-config-page__main-area\"><a class=\"active btn btn-transparent back-button\""), "{html}");
    assert!(html.contains("selected=\"\" value=\"external_url\""));
    assert!(html.contains("name=\"externalUrl\" value=\"https://example.com/x\""));
    let (_, html) = admin.page("/admin/config/permalinks/new").await;
    assert!(html.contains("<h3 class=\"admin-config-area-card__title\">Add permalink</h3>"));
    assert!(html.contains("name=\"topicId\""));
    let (_, html) = admin.page("/admin/config/permalinks/settings").await;
    assert!(html.contains("<li class=\"active admin-permalinks-tabs__settings\">"));
    // The JSON stays the controller's.
    let (status, body) = admin
        .send(Method::GET, "/admin/config/permalinks.json", "", true)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("[{"), "{body}");
}

#[tokio::test]
async fn the_user_fields_pages() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, html) = admin.page("/admin/config/user-fields").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>User fields - Admin - "), "{html}");
    assert!(html.contains("admin-user_fields__add-emoji"));
    sqlx::query("INSERT INTO user_fields (name, description, field_type_enum, editable, requirement, show_on_profile, show_on_user_card, searchable, show_on_signup, position, created_at, updated_at) VALUES ('Company', 'Where you work', 0, false, 2, false, false, true, true, 1, now(), now())")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, html) = admin.page("/admin/config/user-fields").await;
    assert!(
        html.contains(
            "<div class=\"d-table__overview-name admin-user_field-item__name\">Company</div>"
        ),
        "{html}"
    );
    assert!(html.contains("<div class=\"d-table__overview-flags\">"));
    let id: i32 = sqlx::query_scalar("SELECT id FROM user_fields WHERE name = 'Company'")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let (_, html) = admin
        .page(&format!("/admin/config/user-fields/{id}/edit"))
        .await;
    assert!(
        html.contains("<title>Edit User Field - User fields - Admin - "),
        "{html}"
    );
    assert!(!html.contains("d-page-header__title"));
    assert!(html.contains("value=\"Company\""));
    assert!(html.contains("data-editable-false=\"\""));
    let (_, html) = admin.page("/admin/config/user-fields/new").await;
    assert!(html.contains("<title>Add User Field - User fields - Admin - "));
    assert!(html.contains("value=\"optional\" checked=\"\""));
    // The JSON stays the controller's.
    let (status, body) = admin
        .send(Method::GET, "/admin/config/user-fields.json", "", true)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("{\"user_fields\":["), "{body}");
}

#[tokio::test]
async fn the_content_and_login_areas() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    // Content: each tab titled by its own label.
    let (status, html) = admin.page("/admin/config/content").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(
        html.contains("<title>Categories &#38; tags - Admin - "),
        "{html}"
    );
    assert!(
        html.contains("<div class=\"admin-contents admin-config content categories-and-tags\">")
    );
    assert!(
        html.contains("<li class=\"active\"><a class=\"active\" href=\"/admin/config/content\">")
    );
    assert!(html.contains("data-setting=\"tag_style\""));
    // A setting's preview, with its value.
    assert!(
        html.contains("<div class=\"preview\" data-template="),
        "{html}"
    );
    let (_, html) = admin.page("/admin/config/content/sharing").await;
    assert!(html.contains("<div class=\"admin-contents admin-config content sharing\">"));
    // The settings route goes to the first tab.
    let (status, location) = admin.page("/admin/config/content/settings").await;
    assert!(status.is_redirection(), "{status}");
    assert_eq!(location, "/admin/config/content");
    // Login: titled by the area; its settings tab adds a crumb.
    let (_, html) = admin.page("/admin/config/login-and-authentication").await;
    assert!(
        html.contains("<title>Login &#38; authentication - Admin - "),
        "{html}"
    );
    assert!(html.contains("<div class=\"admin-contents admin-config login settings\">"));
    assert_eq!(
        html.matches("class=\"d-breadcrumbs__item\"").count(),
        3,
        "{html}"
    );
    let (_, html) = admin
        .page("/admin/config/login-and-authentication/authenticators")
        .await;
    assert!(html.contains("<div class=\"admin-contents admin-config login authenticators\">"));
    // Discourse ID's controller is not ported.
    let (status, _) = admin
        .page("/admin/config/login-and-authentication/discourse-id")
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    // The JSON is the site settings controller's.
    let (status, body) = admin
        .send(
            Method::GET,
            "/admin/config/content.json?filter_area=sharing",
            "",
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("{\"site_settings\":["), "{body}");
}

#[tokio::test]
async fn the_emoji_pages() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, html) = admin.page("/admin/config/emoji").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Emoji - Admin - "), "{html}");
    assert!(
        html.contains("<div class=\"admin-emoji admin-config-page\"><div class=\"d-page-header\">")
    );
    assert!(html.contains("<li class=\"active admin-emoji-tabs__emoji\">"));
    // No custom emoji: the empty list.
    assert!(html.contains("admin-config-area-empty-list__cta-button admin-emoji__add-emoji"));
    // The new and import routes hide the header.
    let (_, html) = admin.page("/admin/config/emoji/new").await;
    assert!(
        html.contains("<div class=\"admin-contents admin-config admin-emojis new\""),
        "{html}"
    );
    assert!(!html.contains("d-page-header__title"));
    assert!(html.contains("id=\"emoji-group-selector\""));
    assert!(html.contains("<h3 class=\"admin-config-area-card__title\">Add emoji</h3>"));
    let (_, html) = admin.page("/admin/config/emoji/import").await;
    assert!(html.contains("<title>Import - Emoji - Admin - "), "{html}");
    assert!(html.contains("class=\"admin-emoji-import__file-input\""));
    let (_, html) = admin.page("/admin/config/emoji/settings").await;
    assert!(
        html.contains("<title>Settings - Emoji - Admin - "),
        "{html}"
    );
    assert!(html.contains("data-setting=\"enable_emoji\""));
    // discourse-reactions' like reaction leads its enabled reactions.
    assert!(
        html.contains("data-fixed=\"heart\"><ul class=\"values emoji-value-list\"><li class=\"value\" data-index=\"0\" data-added=\"\"><button class=\"btn no-text btn-icon btn-default remove-value-btn btn-small\" disabled=\"\""),
        "{html}"
    );
    // The JSON stays the controller's, on the routes' paths too.
    let (status, body) = admin
        .send(Method::GET, "/admin/config/emoji.json", "", true)
        .await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "[]"));
    let (status, body) = admin
        .send(Method::GET, "/admin/config/emoji/new", "", true)
        .await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "[]"));
    // AdminConstraint: not found for anyone else.
    let mut user = Client::logged_in(st, "user1").await;
    let (status, _) = user.page("/admin/config/emoji/settings").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_logo_and_fonts_pages() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, html) = admin.page("/admin/config/logo").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Logo - Admin - "), "{html}");
    assert!(html.contains("<form class=\"form-kit admin-logo-form\""));
    // The seeded logo with its measured size, the square icon's
    // placeholder, the collapsed sections' fields waiting.
    assert!(
        html.contains("data-target-height=\"512\" data-target-width=\"1858\""),
        "{html}"
    );
    assert!(html.contains("<div class=\"placeholder-overlay\" style=\"background-image: url(http://test.localhost/images/discourse-logo-sketch-small.png)\">"), "{html}");
    assert!(html.contains("<template class=\"admin-config-area-card-section__template\"><div class=\"admin-config-area-card-section__content\"><div class=\"form-kit__container form-kit__field form-kit__field-image\" data-control-type=\"image\" data-name=\"mobile_logo\""));
    assert!(
        html.contains("<template class=\"admin-logo-form__dark\" data-for=\"logo_dark_required\">")
    );
    let (_, html) = admin.page("/admin/config/fonts").await;
    assert!(html.contains("<title>Fonts - Admin - "), "{html}");
    assert!(html.contains("admin-fonts-form__button-option font btn-flat body-font-inter active\" type=\"button\" data-value=\"inter\">Inter</button>"), "{html}");
    assert!(html.contains("text-size btn-flat normal active"));
    // The controllers' index answers JSON with no content.
    let (status, body) = admin
        .send(Method::GET, "/admin/config/fonts.json", "", true)
        .await;
    assert_eq!((status, body.as_str()), (StatusCode::NO_CONTENT, ""));
    let (status, _) = Client::logged_in(st, "user1")
        .await
        .page("/admin/config/logo")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
