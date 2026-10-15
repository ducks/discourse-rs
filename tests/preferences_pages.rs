//! The preferences pages (`/u/:username/preferences/...`): the shell and
//! the account, profile, notifications and interface tabs, what each
//! tab's Save Changes sends, where the routes Ember redirects go, and
//! the tabs other branches port, refused.

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
}

struct Page {
    status: StatusCode,
    location: Option<String>,
    html: String,
}

impl Client {
    fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    async fn send(&mut self, method: Method, path: &str, body: &str) -> String {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest");
        if !self.cookies.is_empty() {
            request = request.header(header::COOKIE, self.cookie_header());
        }
        if let Some(token) = &self.csrf {
            request = request.header("x-csrf-token", token.as_str());
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
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let csrf = client.send(Method::GET, "/session/csrf.json", "").await;
        let csrf: serde_json::Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
            .send(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
            )
            .await;
        client
    }

    /// The page as a browser asks for it.
    async fn page(&mut self, path: &str) -> Page {
        let mut request = Request::get(path).header(header::HOST, "test.localhost");
        if !self.cookies.is_empty() {
            request = request.header(header::COOKIE, self.cookie_header());
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|l| l.to_str().unwrap().to_string());
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Page {
            status,
            location,
            html: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }
}

/// The form's data-save-attrs, decoded.
fn save_attrs(html: &str) -> serde_json::Value {
    let start = html.find("data-save-attrs=\"").unwrap() + "data-save-attrs=\"".len();
    let end = start + html[start..].find('"').unwrap();
    let decoded = html[start..end]
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#x3D;", "=")
        .replace("&#x60;", "`")
        .replace("&amp;", "&");
    serde_json::from_str(&decoded).unwrap()
}

async fn client(username: &str) -> (TestDb, Client) {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let client = Client::logged_in(st, username).await;
    (db, client)
}

#[tokio::test]
async fn the_account_tab_draws_ones_account() {
    let (_db, mut user1) = client("user1").await;
    let page = user1.page("/u/user1/preferences/account").await;
    assert_eq!(page.status, StatusCode::OK);
    let html = page.html;
    assert!(
        html.contains("<title>Account - Preferences - user1 - "),
        "{html}"
    );
    assert!(html.contains("user-preferences-page"));
    // The Preferences tab, then the preferences' own navigation.
    assert!(html.contains(
        "<li aria-current=\"page\" class=\"user-nav__preferences\"><a class=\"active\" href=\"/u/user1/preferences\">"
    ));
    assert!(html.contains(
        "<li aria-current=\"location\" class=\"user-nav__preferences-account\"><a class=\"active\" href=\"/u/user1/preferences/account\">"
    ));
    assert!(html.contains("class=\"user-nav__preferences-tracking\""));
    assert!(
        html.contains("<span class=\"static username-preference__current-username\">user1</span>")
    );
    assert!(html.contains("People can mention you as @user1"));
    assert!(html.contains("<div class=\"email-first\">user1@example.com</div>"));
    assert!(html.contains("href=\"/u/user1/preferences/email?new=1\""));
    assert!(html.contains("data-pref=\"name\""));
    assert!(html.contains("btn-request-archive"));
    assert!(html.contains("<span class=\"d-button-label\">Save Changes</span>"));
    // What Save Changes sends: the account's attributes the model has.
    let attrs = save_attrs(&html);
    assert_eq!(attrs["name"], "User1");
    assert!(attrs.get("title").is_some());
    assert!(attrs.get("status").is_none());
    assert!(html.contains("data-save-url=\"/u/user1.json\""));
}

#[tokio::test]
async fn the_profile_tab_draws_the_profile_fields() {
    let (_db, mut user1) = client("user1").await;
    let html = user1.page("/u/user1/preferences/profile").await.html;
    assert!(html.contains("<title>Profile - Preferences - user1 - "));
    assert!(html.contains("data-pref=\"hide_profile\""));
    assert!(html.contains("class=\"controls bio-composer input-xxlarge\""));
    assert!(html.contains("data-pref=\"bio_raw\""));
    assert!(html.contains("timezone-input"));
    assert!(html.contains("id=\"edit-location\""));
    assert!(html.contains("id=\"edit-website\""));
    assert!(html.contains("id=\"profile-background-uploader\""));
    assert!(html.contains("id=\"profile-card-background-uploader\""));
    assert!(html.contains("feature-topic-on-profile-btn"));
    let attrs = save_attrs(&html);
    assert_eq!(attrs["hide_profile"], false);
    assert_eq!(attrs["default_calendar"], "none_selected");
}

#[tokio::test]
async fn the_notifications_tab_draws_the_options_and_plugins() {
    let (_db, mut user1) = client("user1").await;
    let html = user1.page("/u/user1/preferences/notifications").await.html;
    assert!(html.contains("<title>Notifications - Preferences - user1 - "));
    assert!(html.contains("data-pref=\"like_notification_frequency\""));
    assert!(html.contains("data-name=\"First time a post is liked and daily\" data-value=\"1\""));
    assert!(html.contains("data-pref=\"notify_on_linked_posts\""));
    assert!(html.contains("push-notification-select"));
    assert!(html.contains("data-pref=\"user_notification_schedule.enabled\""));
    assert!(html.contains("<td class=\"day-label\">Monday</td>"));
    assert!(html.contains("class=\"control-group chat-notifications\""));
    let attrs = save_attrs(&html);
    assert_eq!(attrs["user_notification_schedule"]["day_0_start_time"], 480);
    assert_eq!(attrs["chat_header_indicator_preference"], "all_new");
    assert!(attrs.get("chat_sound").is_none());
}

#[tokio::test]
async fn the_interface_tab_draws_the_options() {
    let (_db, mut user1) = client("user1").await;
    let html = user1.page("/u/user1/preferences/interface").await.html;
    assert!(html.contains("<title>Interface - Preferences - user1 - "));
    assert!(html.contains("id=\"text-size-selector\""));
    assert!(html.contains("id=\"locale-selector\""));
    assert!(html.contains("id=\"home-selector\""));
    assert!(html.contains("data-pref=\"external_links_in_new_tab\""));
    assert!(html.contains("id=\"bookmark-after-notification-mode\""));
    let attrs = save_attrs(&html);
    assert_eq!(attrs["theme_ids"], serde_json::json!([-1]));
    assert_eq!(attrs["color_scheme_id"], serde_json::Value::Null);
    assert_eq!(attrs["text_size"], "normal");
}

#[tokio::test]
async fn the_saved_text_size_sizes_every_page() {
    let (db, mut user1) = client("user1").await;
    sqlx::query("UPDATE user_options SET text_size_key = 1 WHERE user_id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let html = user1.page("/latest").await.html;
    assert!(html.contains("class=\"text-size-larger"), "{html}");
    // This device's own size, as new as the option's.
    user1
        .cookies
        .push(("text_size".into(), "smallest%7C0".into()));
    let html = user1.page("/latest").await.html;
    assert!(html.contains("class=\"text-size-smallest"));
}

#[tokio::test]
async fn the_routes_ember_redirects_go_where_it_does() {
    let (_db, mut user1) = client("user1").await;
    let index = user1.page("/u/user1/preferences").await;
    assert_eq!(index.status, StatusCode::FOUND);
    assert!(
        index
            .location
            .unwrap()
            .ends_with("/u/user1/preferences/account")
    );
    // Another member's preferences: their activity (RestrictedUserRoute).
    let other = user1.page("/u/user2/preferences/account").await;
    assert_eq!(other.status, StatusCode::FOUND);
    assert!(other.location.unwrap().ends_with("/u/user2/activity"));
    let my = user1.page("/my/preferences/interface").await;
    assert!(
        my.location
            .unwrap()
            .ends_with("/u/user1/preferences/interface")
    );
    // A route Rails lacks, and one only the Ember app lacks.
    assert_eq!(
        user1.page("/u/user1/preferences/bogus").await.status,
        StatusCode::NOT_FOUND
    );
    let unknown = user1.page("/u/user1/preferences/username").await;
    assert_eq!(unknown.status, StatusCode::OK);
    assert!(unknown.html.contains("page-not-found"));
    // The tabs another branch ports.
    assert_eq!(
        user1.page("/u/user1/preferences/security").await.status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[tokio::test]
async fn anonymous_preferences_are_not_found() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut anon = Client {
        state: st,
        cookies: Vec::new(),
        csrf: None,
    };
    let page = anon.page("/u/user1/preferences/account").await;
    assert_eq!(page.status, StatusCode::NOT_FOUND);
    let my = anon.page("/my/preferences").await;
    assert!(my.location.unwrap().ends_with("/login-preferences"));
}

#[tokio::test]
async fn staff_edit_another_members_preferences() {
    let (_db, mut admin) = client("admin").await;
    let page = admin.page("/u/user1/preferences/account").await;
    assert_eq!(page.status, StatusCode::OK);
    // No email until looked up, and no data export for another member.
    assert!(page.html.contains("check-email"));
    assert!(!page.html.contains("btn-request-archive"));
}

#[tokio::test]
async fn a_second_theme_is_refused() {
    let (db, mut user1) = client("user1").await;
    sqlx::query(
        "INSERT INTO themes (id, name, user_id, created_at, updated_at, compiler_version, user_selectable, component) \
         VALUES (900, 'Another', -1, now(), now(), 0, true, false)",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let page = user1.page("/u/user1/preferences/interface").await;
    assert_eq!(page.status, StatusCode::INTERNAL_SERVER_ERROR);
}
