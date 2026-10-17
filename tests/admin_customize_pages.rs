//! The customize admin pages' HTML (90g7): each page in the admin shell
//! as Ember draws it.

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
async fn the_watched_words_pages() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    // The index goes on to the first action.
    let (status, location) = admin.page("/admin/customize/watched_words").await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{location}");
    assert_eq!(location, "/admin/customize/watched_words/action/block");
    sqlx::query(
        "INSERT INTO watched_words (word, action, replacement, case_sensitive, created_at, updated_at) \
         VALUES ('bad', 1, NULL, false, now(), now()), ('cat', 5, 'dog', true, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let (status, html) = admin
        .page("/admin/customize/watched_words/action/block")
        .await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Watched words - Admin - "), "{html}");
    assert!(html.contains("<div class=\"admin-contents admin-customize admin-watched-words action\" data-action=\"block\">"));
    assert!(html.contains("<li class=\"block\"><a class=\"active\" href=\"/admin/customize/watched_words/action/block\">Block <span class=\"count\">(1)</span></a></li>"), "{html}");
    assert!(html.contains("<li class=\"censor\"><a href=\"/admin/customize/watched_words/action/censor\">Censor</a></li>"));
    assert!(html.contains("href=\"/admin/customize/watched_words/action/block/download\""));
    assert!(html.contains("<p class=\"about\">An error message will be displayed when attempting to create a post containing these words.</p>"));
    assert!(html.contains("show 1 word"));
    // The list waits for the checkbox; a filter shows it.
    assert!(!html.contains("watched-words-list"));
    let (_, html) = admin
        .page("/admin/customize/watched_words/action/replace?filter=ca")
        .await;
    assert!(
        html.contains("<div class=\"watched-words-list watched-words-replace\">"),
        "{html}"
    );
    assert!(html.contains("<span class=\"replacement\">dog</span>"));
    assert!(html.contains("(case-sensitive)"));
    assert!(html.contains("id=\"watched-replacement\""));
    // The filter leaves block's word out.
    assert!(html.contains(
        "<li class=\"block\"><a href=\"/admin/customize/watched_words/action/block\">Block</a></li>"
    ));
    // The JSON is unchanged.
    let (status, body) = admin
        .send(Method::GET, "/admin/customize/watched_words.json", "", true)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("{\"actions\":"), "{body}");
    // Moderators see it too; anyone else gets not found.
    let mut user = Client::logged_in(st.clone(), "user1").await;
    let (status, _) = user
        .page("/admin/customize/watched_words/action/block")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_robots_page() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, html) = admin.page("/admin/customize/robots").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<div class=\"admin-contents admin-customize admin-customize-robots-txt\"><div class=\"admin-config-page\"><div class=\"robots-txt-edit\"><h3>Override your site&#x27;s robots.txt file:</h3>") || html.contains("<h3>Override your site's robots.txt file:</h3>"), "{html}");
    assert!(html.contains(
        "<textarea class=\"ember-text-area robots-txt-input\"># See https://datatracker.ietf.org"
    ));
    assert!(!html.contains("class=\"overridden\""));
    let (status, _) = admin
        .send(
            Method::PUT,
            "/admin/customize/robots.json",
            "robots_txt=User-agent%3A+*",
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, html) = admin.page("/admin/customize/robots").await;
    assert!(
        html.contains(
            "<div class=\"overridden\">Your site’s default robots.txt file is overridden.</div>"
        ) || html.contains("class=\"overridden\""),
        "{html}"
    );
    assert!(html.contains(">User-agent: *</textarea>"));
    let mut user = Client::logged_in(st.clone(), "user1").await;
    let (status, _) = user.page("/admin/customize/robots").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_embedding_pages() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, html) = admin.page("/admin/customize/embedding").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<div class=\"admin-contents admin-customize admin-embedding\">"));
    assert!(html.contains("<div class=\"admin-config-area-empty-list\"><span class=\"admin-config-area-empty-list__title\">If you&#x27;d like to embed") || html.contains("If you'd like to embed Discourse"), "{html}");
    assert!(html.contains("<li class=\"active admin-embedding-tabs__hosts\"><a class=\"active\" href=\"/admin/customize/embedding\">Hosts</a></li>"));
    sqlx::query(
        "INSERT INTO embeddable_hosts (host, allowed_paths, category_id, created_at, updated_at) \
         VALUES ('blog.example.com', '/blog/.*', 1, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let (_, html) = admin.page("/admin/customize/embedding").await;
    assert!(
        html.contains("<td class=\"d-table__cell --overview\">blog.example.com</td>"),
        "{html}"
    );
    assert!(html.contains("admin-embedding-index__full-app-toggle"));
    assert!(html.contains("admin-embedding-index__code"));
    assert!(html.contains("data-identifier=\"embedding-host-menu\""));
    let id: i32 = sqlx::query_scalar("SELECT id FROM embeddable_hosts")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let (_, html) = admin
        .page(&format!("/admin/customize/embedding/{id}"))
        .await;
    assert!(html.contains("<title>Edit host - Admin - "));
    assert!(html.contains("<h3 class=\"admin-config-area-card__title\">Edit host</h3>"));
    assert!(html.contains("value=\"blog.example.com\""));
    assert!(html.contains("category-chooser has-selection"));
    let (_, html) = admin.page("/admin/customize/embedding/new").await;
    assert!(html.contains("<title>Add host - Admin - "));
    assert!(!html.contains("d-page-header__title"));
    let (_, html) = admin
        .page("/admin/customize/embedding/posts_and_topics")
        .await;
    assert!(
        html.contains(
            "<h2 class=\"d-page-subheader__title\">Posts &amp; topics configuration</h2>"
        ),
        "{html}"
    );
    assert!(html.contains("name=\"embed_post_limit\""));
    let (_, html) = admin.page("/admin/customize/embedding/crawlers").await;
    assert!(html.contains("placeholder=\"article, #story, .post\""));
    let (_, html) = admin.page("/admin/customize/embedding/settings").await;
    assert!(
        html.contains("<div class=\"admin-contents admin-customize admin-embedding settings\">")
    );
    // The JSON is unchanged.
    let (status, body) = admin
        .send(Method::GET, "/admin/customize/embedding.json", "", true)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"embeddable_hosts\""));
}

#[tokio::test]
async fn the_site_texts_pages() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, html) = admin.page("/admin/customize/site_texts").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Site texts - Admin - "));
    assert!(html.contains("<div class=\"admin-contents admin-customize admin-site-text\">"));
    assert!(html.contains("<p class=\"site-texts__recommended\">Suggested texts to customize</p>"));
    assert!(html.contains("data-site-text-id=\"education.new-reply\""));
    assert!(html.contains("<span class=\"d-button-label\">Filters</span>"));
    // A search, and a filter with its drawer open.
    let (_, html) = admin
        .page("/admin/customize/site_texts?q=create%20topic&overridden=true")
        .await;
    assert!(html.contains("Filters (1)"), "{html}");
    assert!(html.contains("<input checked=\"\" id=\"toggle-overridden\" type=\"checkbox\">"));
    assert!(!html.contains("site-texts__recommended"));
    let (status, html) = admin
        .page("/admin/customize/site_texts/education.new-reply?locale=en")
        .await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<h1 class=\"d-page-header__title\">Edit site text</h1>"));
    assert!(html.contains("<code class=\"edit-site-text__key\">education.new-reply</code>"));
    assert!(html.contains("Your text · English"));
    assert!(html.contains("<button class=\"interpolation-keys__pill is-used\""));
    assert!(html.contains("<a class=\"go-back\" href=\"/admin/customize/site_texts?locale=en\">"));
    // An unknown key is the JSON's 404.
    let (status, _) = admin
        .page("/admin/customize/site_texts/no.such.key?locale=en")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_email_template_editor() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, html) = admin
        .page("/admin/email/templates/user_notifications.signup")
        .await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Email settings - Admin - "));
    assert!(html.contains("<div class=\"admin-contents admin-email admin-email-templates edit\">"));
    assert!(html.contains("<li class=\"active\"><a class=\"active\" href=\"/admin/email/templates\">Templates</a></li>"), "{html}");
    assert!(html.contains("<input class=\"ember-text-field email-template__subject\" type=\"text\" value=\"[%{email_prefix}] Confirm your new account\">"));
    assert!(html.contains("<div class=\"d-editor email-template__body\">"));
    assert!(html.contains("toolbar-menu__heading-trigger"));
    assert!(html.contains("<button class=\"interpolation-keys__pill is-used\" title=\"Already used\" type=\"button\">email_token</button>"));
    assert!(!html.contains("revert-email-template"));
    // A pluralized subject links to the site texts.
    let (_, html) = admin
        .page("/admin/email/templates/system_messages.pending_users_reminder")
        .await;
    assert!(html.contains("class=\"email-template__has-multiple-subjects\" href=\"/admin/customize/site_texts?q=system_messages.pending_users_reminder\""));
    // A saved template can be reverted.
    let (status, body) = admin
        .send(
            Method::PUT,
            "/admin/email/templates/test_mailer",
            "email_template%5Bsubject%5D=Hello&email_template%5Bbody%5D=World",
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, html) = admin.page("/admin/email/templates/test_mailer").await;
    assert!(html.contains("value=\"Hello\""), "{html}");
    assert!(html.contains("revert-email-template"));
}

#[tokio::test]
async fn the_color_palettes_pages() {
    let db = TestDb::new().await;
    let st = admin_state(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, html) = admin.page("/admin/config/colors").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<div class=\"admin-contents admin-config color-palettes\">"));
    assert!(html.contains("<h2 class=\"d-page-subheader__title\">Color palettes</h2>"));
    assert!(html.contains("create-new-palette"));
    // The built-in default first, with its preview and disabled edit.
    assert!(
        html.contains("<li class=\"admin-config-area-card color-palette\" data-palette-id=\"-1\"")
    );
    assert!(html.contains("--primary-low--preview: #e9e9e9; --tertiary-low--preview: #d1f0ff;"));
    assert!(html.contains("<svg fill=\"none\" viewBox=\"0 0 558 171\""));
    let id: i32 = sqlx::query_scalar(
        "SELECT id FROM color_schemes WHERE theme_id IS NULL ORDER BY id LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(html.contains(&format!("<a href=\"/admin/config/colors/{id}\">")));
    // The filter's query params.
    let (_, html) = admin
        .page("/admin/config/colors?filter=nothing-matches")
        .await;
    assert!(html.contains("No color palettes found"));
    let (status, html) = admin.page(&format!("/admin/config/colors/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains(&format!(
        "<form class=\"form-kit\" data-palette-id=\"{id}\" novalidate=\"\">"
    )));
    assert!(html.contains("admin-config-color-palettes__edit-name"));
    assert!(html.contains("data-color-name=\"primary\""));
    assert!(html.contains("Main text and icons"));
    assert!(html.contains("data-name=\"user_selectable\""));
    // A palette that isn't there goes back to the list; the old paths go on.
    let (status, location) = admin.page("/admin/config/colors/99999").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location, "/admin/config/colors");
    let (_, location) = admin.page("/admin/customize/colors/5").await;
    assert_eq!(location, "/admin/config/colors/5");
    // The JSON is unchanged.
    let (status, body) = admin
        .send(Method::GET, "/admin/config/colors.json", "", true)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("{\"palettes\":"));
}
