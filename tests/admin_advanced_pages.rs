//! The advanced and customize admin pages' HTML (90g8, 90g7): each page in
//! the admin shell with its header, tabs and content as Ember draws them,
//! from the documents their JSON routes serve.

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
async fn the_backups_pages() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, html) = admin.page("/admin/backups").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Backups - Admin - "), "{html}");
    assert!(html.contains("<div class=\"admin-contents backups\" data-read-only=\"false\""));
    assert!(html.contains("<button class=\"btn btn-icon-text d-page-action-button btn-small btn-primary admin-backups__start\" title=\"Create a backup\" type=\"button\">"), "{html}");
    assert!(html.contains("admin-backups__toggle-read-only"));
    assert!(html.contains(
        "<li class=\"active admin-backups-tabs__files\"><a class=\"active\" href=\"/admin/backups\">Backup files</a></li>"
    ));
    assert!(html.contains("<h2 class=\"d-page-subheader__title\">Backup files</h2>"));
    assert!(html.contains("<label class=\"btn btn-small btn-primary admin-backups-upload\""));
    assert!(html.contains("/admin-backups.js"));
    // The test env doesn't allow restores: the notice, with its link.
    assert!(html.contains("<div class=\"backup-message alert alert-info\">"));
    assert!(html.contains("href='/admin/backups/settings?filter=allow_restore'"));
    // The logs tab.
    let (status, html) = admin.page("/admin/backups/logs").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Logs - Backups - Admin - "), "{html}");
    assert!(html.contains(
        "<div class=\"admin-backups-logs\"><p>No logs yet…</p><div class=\"loading-container inline-spinner\"></div></div>"
    ));
    // The settings tab: the backups category's settings.
    let (status, html) = admin.page("/admin/backups/settings").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(
        html.contains("<title>Settings - Backups - Admin - "),
        "{html}"
    );
    assert!(html.contains("data-setting=\"maximum_backups\""));
    assert!(!html.contains("data-setting=\"title\""));
    assert!(html.contains("/site-settings.js"));
    assert!(html.contains("/admin-backups.js"));
    // The JSON stays the files.
    let (status, body) = admin
        .send(Method::GET, "/admin/backups.json", "", true)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with('['), "{body}");
    // Not for members.
    let (status, _) = Client::logged_in(st, "user1")
        .await
        .page("/admin/backups")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_backups_logs_since_the_operation_started() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut conn = db.pool.acquire().await.unwrap();
    // An earlier operation's line, then the start of this one: just past
    // that line. Not bus.now(): its horizon is the cluster's oldest open
    // transaction, which another database's test can hold back.
    let position = st
        .bus
        .publish(
            &mut conn,
            "/admin/backups/logs",
            &serde_json::json!({"timestamp": "2026-10-01 00:00:00", "operation": "backup", "message": "old line"}),
            Some(&["user:1".to_string()]),
        )
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO discourse_rs.cached_values (key, value, expires_at) VALUES ('start_logs_message_id', $1, now() + interval '1 day')",
    )
    .bind(position.to_string())
    .execute(&mut *conn)
    .await
    .unwrap();
    for message in [
        "[STARTED]",
        "Dumping the public schema of the database...",
        "<b>",
    ] {
        st.bus
            .publish(
                &mut conn,
                "/admin/backups/logs",
                &serde_json::json!({"timestamp": "2026-10-01 00:00:01", "operation": "backup", "message": message}),
                Some(&["user:1".to_string()]),
            )
            .await
            .unwrap();
    }
    let mut admin = Client::logged_in(st, "admin").await;
    let (_, html) = admin.page("/admin/backups/logs").await;
    assert!(html.contains(
        "<pre>[2026-10-01 00:00:01] Dumping the public schema of the database...\n[2026-10-01 00:00:01] &lt;b&gt;\n</pre>"
    ), "{html}");
    assert!(!html.contains("old line"));
}

#[tokio::test]
async fn the_api_keys_pages() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    // No keys yet: the empty list.
    let (status, html) = admin.page("/admin/api/keys").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>API keys - Admin - "), "{html}");
    assert!(html.contains("<div class=\"admin-contents admin-api admin-api-keys\">"));
    assert!(html.contains(
        "<span class=\"admin-config-area-empty-list__title\">You don't have any API keys yet.</span>"
    ));
    let (status, _) = admin
        .send(
            Method::POST,
            "/admin/api/keys.json",
            "key[description]=Parity+key&key[scope_mode]=global",
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, html) = admin.page("/admin/api/keys").await;
    assert!(html.contains("<table class=\"d-table admin-api_keys__items\">"));
    assert!(html.contains("Parity key"));
    assert!(html.contains("data-href=\"/admin/api/keys/new\""));
    assert!(html.contains("<li class=\"active admin-api-keys-tabs__index\">"));
    // The key's page: no tabs, Admin's title.
    let (status, html) = admin.page("/admin/api/keys/1").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Admin - "), "{html}");
    assert!(html.contains("<div class=\"api-key api-key-show\" data-api-key-id=\"1\""));
    assert!(!html.contains("d-nav-submenu"));
    assert!(html.contains("api-key-show__revoke"));
    // The new key's form, without the header.
    let (status, html) = admin.page("/admin/api/keys/new").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<div class=\"admin-contents admin-api admin-api-keys new\">"));
    assert!(!html.contains("d-page-header"));
    assert!(html.contains(
        "<select class=\"d-native-select form-kit__control-select\" id=\"control-scope_mode-input\" name=\"scope_mode\">"
    ));
    assert!(html.contains("data-scope-id=\"topics:write\""));
    // The settings tab.
    let (status, html) = admin.page("/admin/api/keys/settings").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Settings - Admin - "), "{html}");
    assert!(html.contains("data-setting=\"user_api_key_allowed_groups\""));
    // /admin/api goes on to the keys; the JSON stays.
    let (status, location) = admin.page("/admin/api").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location, "/admin/api/keys");
    let (status, body) = admin
        .send(Method::GET, "/admin/api/keys/1.json", "", true)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("{\"key\":"), "{body}");
    // A missing key's page.
    let (status, _) = admin.page("/admin/api/keys/99").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_installed_plugins_pages() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    // The list: every recorded plugin, its switch and settings link.
    let (status, html) = admin.page("/admin/plugins").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Plugins - Admin - "), "{html}");
    assert!(html.contains("<div class=\"admin-contents admin-plugins\">"));
    assert!(html.contains(
        "<div class=\"admin-config-page -no-header\"><div class=\"admin-plugins-list-container\">"
    ));
    assert!(html.contains("<a class=\"admin-plugins-howto\" href=\"https://meta.discourse.org/t/install-a-plugin/19157\">"));
    assert_eq!(
        html.matches("class=\"d-table__row admin-plugins-list__row\"")
            .count(),
        46
    );
    assert!(html.contains("data-plugin-setting-button=\"poll\" href=\"/admin/plugins/poll\""));
    // Only an enabled setting: the name unlinked, Settings disabled.
    assert!(
        html.contains(
            "<div class=\"d-table__overview-name admin-plugins-list__name\">Checklist</div>"
        ),
        "{html}"
    );
    assert!(html.contains("class=\"btn btn-default btn-text btn-small disabled\" data-plugin-setting-button=\"checklist\" href=\"/admin/site_settings/category/plugins?filter&#x3D;plugin%3Achecklist\""), "{html}");
    assert!(html.contains("data-enabled-setting=\"poll_enabled\""));
    // A plugin's settings.
    let (status, html) = admin.page("/admin/plugins/poll").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(
        html.contains("<title>Settings - Poll - Plugins - Admin - "),
        "{html}"
    );
    assert!(html.contains("<div class=\"admin-contents admin-plugins settings\">"));
    assert!(html.contains("<section class=\"admin-plugin-config-area\">"));
    assert!(html.contains("data-setting=\"poll_enabled\""));
    assert!(!html.contains("d-nav-submenu"));
    let (status, _) = admin.page("/admin/plugins/poll/settings").await;
    assert_eq!(status, StatusCode::OK);
    // The JSON: the plugins, one by name (or without its prefix).
    let (status, body) = admin
        .send(Method::GET, "/admin/plugins.json", "", true)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.starts_with("{\"plugins\":[{\"id\":\"discourse-adplugin\""),
        "{body}"
    );
    let (status, body) = admin
        .send(Method::GET, "/admin/plugins/solved.json", "", true)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("{\"id\":\"discourse-solved\""), "{body}");
    let (status, _) = admin
        .send(Method::GET, "/admin/plugins/nope.json", "", true)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Staff only.
    let (status, _) = Client::logged_in(st, "user1")
        .await
        .page("/admin/plugins")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_badges_pages() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    // The index: the header's actions, the list and the intro.
    let (status, html) = admin.page("/admin/badges").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Badges - Admin - "), "{html}");
    assert!(html.contains("<div class=\"admin-contents admin-badges\">"));
    assert!(html.contains("data-href=\"/admin/badges/award/new\""));
    assert!(html.contains("edit-groupings-btn"));
    assert!(html.contains("<li class=\"active admin-badges-tabs__index\">"));
    assert!(html.contains("<ul class=\"admin-badge-list\">"));
    assert!(html.contains("<a href=\"/admin/badges/1\"><span class=\"user-badge badge-type-bronze\" data-badge-name=\"Basic\""));
    assert!(html.contains("<section class=\"current-badge content-body\"><h2>"));
    assert!(html.contains("/admin-badges.js"));
    // A system badge: its texts read-only with their site text links.
    let (status, html) = admin.page("/admin/badges/1").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<a class=\"active\" href=\"/admin/badges/1\">"));
    assert!(html.contains(
        "<form class=\"form-kit badge-form current-badge content-body\" novalidate=\"\""
    ));
    assert!(html.contains("<span class=\"readonly-field\">Basic</span><a href=\"/admin/customize/site_texts?q=badges.basic_user.name\">"));
    assert!(html.contains("title=\"Selected icon: user\""));
    assert!(html.contains("data-name=\"badge_grouping_id\""));
    assert!(!html.contains("badge-form__delete-badge-btn"));
    // A new one: its name to type, the first type and grouping, Delete.
    let (_, html) = admin.page("/admin/badges/new").await;
    assert!(html.contains("<span class=\"badge-display-name\">New Badge</span></h2>"));
    assert!(html.contains("name=\"name\" type=\"text\" value=\"New Badge\""));
    assert!(html.contains("<span class=\"d-icon-grid-picker__label\">Select icon</span>"));
    assert!(html.contains("badge-form__delete-badge-btn"));
    // Bulk award: the note with no badge, the CSV form with one.
    let (_, html) = admin.page("/admin/badges/award/new").await;
    assert!(html.contains("<div class=\"admin-contents admin-badges award\">"));
    assert!(html.contains("<span class=\"badge-required\">"));
    assert!(html.contains("<a href=\"/admin/badges/award/1\">"));
    let (_, html) = admin.page("/admin/badges/award/1").await;
    assert!(html.contains("<form class=\"form-horizontal\" data-badge-id=\"1\">"));
    assert!(html.contains("id=\"massAwardCSVUpload\""));
    // The settings tab.
    let (status, html) = admin.page("/admin/badges/settings").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(
        html.contains("<title>Settings - Badges - Admin - "),
        "{html}"
    );
    assert!(html.contains("data-setting=\"enable_badges\""));
    // The JSON stays.
    let (status, body) = admin
        .send(Method::GET, "/admin/badges.json", "", true)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("{\"badges\":"), "{body}");
    let (status, _) = admin.send(Method::GET, "/admin/badges/1", "", true).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = Client::logged_in(st, "user1")
        .await
        .page("/admin/badges")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_problem_checks_page() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    sqlx::query(
        "INSERT INTO problem_check_trackers (id, identifier, target, last_run_at, last_success_at, last_problem_at, ignored_at) VALUES \
         (1, 'email_sending_failures', '__NULL__', now(), now(), NULL, NULL), \
         (2, 'upcoming_change_stale', 'voice_enabled', now(), NULL, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, html) = admin.page("/admin/problem-checks").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(
        html.contains("<title>Problem checks - Problem checks - Admin - "),
        "{html}"
    );
    assert!(html.contains("<div class=\"admin-problem-checks admin-config-page\">"));
    assert!(html.contains("<table class=\"d-table admin-problem-checks__items\">"));
    // A passing check, its target blank, to ignore.
    assert!(html.contains("<tr class=\"d-table__row --passing\"><td class=\"d-table__cell --overview\"><div class=\"status-label --success\">"));
    assert!(html.contains("data-problem-check-id=\"1\" data-action=\"ignore\""));
    // A failing, ignored one with its target, to watch.
    assert!(html.contains("<div class=\"status-label --critical\">"));
    assert!(html.contains("</div>voice_enabled</td>"));
    assert!(html.contains("data-problem-check-id=\"2\" data-action=\"watch\""));
    assert!(html.contains("/admin-problem-checks.js"));
    // The JSON stays.
    let (status, body) = admin
        .send(Method::GET, "/admin/problem_checks.json", "", true)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("[{\"id\":1,"), "{body}");
    let (status, _) = Client::logged_in(st, "user1")
        .await
        .page("/admin/problem-checks")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_email_pages() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let tabs =
        |active: &str| format!("<li class=\"active\"><a class=\"active\" href=\"{active}\">");
    // The settings tab: the email area's settings, titled twice.
    let (status, html) = admin.page("/admin/email").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(
        html.contains("<title>Email settings - Email settings - Admin - "),
        "{html}"
    );
    assert!(html.contains("<div class=\"admin-contents admin-email settings\">"));
    assert!(html.contains(&tabs("/admin/email")));
    assert!(html.contains("<li><a href=\"/admin/email/server-settings\">Server settings</a></li>"));
    assert!(html.contains("data-setting=\"notification_email\""));
    assert!(html.contains("/site-settings.js"));
    assert!(html.contains("/admin-email.js"));
    // The server settings: the delivery method's empty cell and the test.
    let (status, html) = admin.page("/admin/email/server-settings").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Email settings - Admin - "), "{html}");
    assert!(html.contains(&tabs("/admin/email/server-settings")));
    assert!(
        html.contains("<table class=\"table\"><tbody><tr><th>Delivery method</th><td></td></tr>")
    );
    assert!(html.contains("<input placeholder=\"email address to test\" type=\"text\">"));
    // The summary preview: a week back, for the viewer, its html shown.
    let (status, html) = admin.page("/admin/email/preview-digest").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains(&tabs("/admin/email/preview-digest")));
    let week_ago = (discourse_rs::clock::now() - chrono::Duration::days(7))
        .format("%Y-%m-%d")
        .to_string();
    assert!(
        html.contains(&format!("data-value=\"{week_ago}\"")),
        "{html}"
    );
    assert!(html.contains("<span class=\"formatted-selection\">admin</span>"));
    assert!(
        html.contains("<iframe srcdoc=\"&lt;!DOCTYPE html&gt;"),
        "{html}"
    );
    assert!(html.contains("<span>html</span> | <a class=\"show-text-link\" href>text</a>"));
    // The advanced test.
    let (status, html) = admin.page("/admin/email/advanced-test").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<textarea class=\"email-body\" name=\"email\"></textarea>"));
    // The templates, by title, filtered as the URL says.
    let (status, html) = admin.page("/admin/email/templates").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains(&tabs("/admin/email/templates")));
    let first = html
        .find("data-template-id=\"user_notifications.account_exists\"")
        .unwrap();
    let second = html
        .find("data-template-id=\"user_notifications.account_created\"")
        .unwrap();
    assert!(
        first < second,
        "Account already exists, then Account Created"
    );
    let (_, html) = admin.page("/admin/email/templates?filter=zzzz").await;
    assert!(html.contains(
        "<div class=\"d-filter-controls__no-results\"><p>No templates match your filters.</p>"
    ));
    assert!(!html.contains("email-templates-list__row"));
    // The JSON stays.
    let (status, _) = admin.send(Method::GET, "/admin/email.json", "", true).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = admin
        .send(Method::GET, "/admin/email/server-settings.json", "", true)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"delivery_method\""), "{body}");
    let (status, body) = admin
        .send(Method::GET, "/admin/email/templates.json", "", true)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("{\"email_templates\":"), "{body}");
    let (status, body) = admin
        .send(
            Method::GET,
            &format!("/admin/email/preview-digest.json?last_seen_at={week_ago}&username=admin"),
            "",
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"html_content\""), "{body}");
    // Not for members.
    let (status, _) = Client::logged_in(st, "user1")
        .await
        .page("/admin/email/preview-digest")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_web_hooks_pages() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, body) = admin
        .send(
            Method::POST,
            "/admin/api/web_hooks.json",
            "web_hook[payload_url]=https://example.com/hook&web_hook[content_type]=1&web_hook[secret]=abcdefghijklmnop&web_hook[wildcard_web_hook]=false&web_hook[verify_certificate]=true&web_hook[active]=true&web_hook[web_hook_event_type_ids][]=101",
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The index: the hook's row and its menu.
    let (status, html) = admin.page("/admin/api/web_hooks").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Admin - "), "{html}");
    assert!(html.contains("<div class=\"admin-webhooks admin-config-page\">"));
    assert!(html.contains("<table class=\"d-table admin-web_hooks__items\">"));
    assert!(html.contains("<tr class=\"d-table__row\" data-web-hook-id=\"1\">"));
    assert!(html.contains("<a href=\"/admin/api/web_hooks/1/edit\">https://example.com/hook</a>"));
    assert!(html.contains("/admin-web-hooks.js"));
    // The new hook's form: the event chooser and no hook id.
    let (status, html) = admin.page("/admin/api/web_hooks/new").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("name=\"payload_url\""), "{html}");
    assert!(html.contains(
        "<fieldset class=\"form-kit__fieldset form-kit__control-radio-group\" name=\"wildcard\">"
    ));
    assert!(html.contains("data-name=\"topic_created\""));
    assert!(!html.contains("data-web-hook-id"));
    // The edit form carries the hook's id for the save, and its event.
    let (status, html) = admin.page("/admin/api/web_hooks/1/edit").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("data-web-hook-id=\"1\""), "{html}");
    assert!(html.contains("value=\"https://example.com/hook\""));
    assert!(html.contains(" checked=\"\" data-id=\"101\""), "{html}");
    // The events page: the summary and the empty listing.
    let (status, html) = admin.page("/admin/api/web_hooks/1").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<div class=\"admin-webhooks__summary\" data-web-hook-id=\"1\">"));
    assert!(html.contains("<div class=\"web-hook-events-listing\" data-web-hook-id=\"1\">"));
    assert!(html.contains("<a class=\"go-back active\" href=\"/admin/api/web_hooks\">"));
    // The JSON stays.
    let (status, body) = admin
        .send(Method::GET, "/admin/api/web_hooks/1.json", "", true)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with('{'), "{body}");
    // A missing hook; not for members.
    let (status, _) = admin.page("/admin/api/web_hooks/99/edit").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = Client::logged_in(st, "user1")
        .await
        .page("/admin/api/web_hooks")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
