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
    // An earlier operation's line, then the start of this one.
    st.bus
        .publish(
            &mut conn,
            "/admin/backups/logs",
            &serde_json::json!({"timestamp": "2026-10-01 00:00:00", "operation": "backup", "message": "old line"}),
            Some(&["user:1".to_string()]),
        )
        .await
        .unwrap();
    let position = st.bus.now().await.unwrap();
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
