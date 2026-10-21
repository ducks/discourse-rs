//! The preferences pages (`/u/:username/preferences/...`): the shell and
//! the account, security, profile, emails, notifications, tracking,
//! users, interface, apps, navigation menu, calendar and chat tabs, what
//! each tab's Save Changes sends, the email change page and its
//! confirmation links, the second factor page behind a confirmed
//! session, where the routes Ember redirects go, and what other items
//! port, refused.

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
    /// The last send's status.
    status: StatusCode,
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
        self.status = response.status();
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
            status: StatusCode::OK,
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
    // The second factor page wants a trusted session.
    let second_factor = user1.page("/u/user1/preferences/second-factor").await;
    assert_eq!(second_factor.status, StatusCode::FOUND);
    assert!(
        second_factor
            .location
            .unwrap()
            .ends_with("/u/user1/preferences/security")
    );
}

#[tokio::test]
async fn the_security_apps_calendar_and_chat_tabs_draw() {
    let (db, mut user1) = client("user1").await;
    sqlx::query(
        "INSERT INTO user_api_key_clients (id, client_id, application_name, created_at, updated_at) \
         VALUES (9801, 'some-app', 'Some App', now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO user_api_keys (id, user_id, key_hash, user_api_key_client_id, created_at, updated_at) \
         VALUES (9802, 3, 'hash-9802', 9801, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let html = user1.page("/u/user1/preferences/security").await.html;
    assert!(html.contains("<title>Security - Preferences - user1 - "));
    assert!(html.contains("id=\"change-password-button\""));
    assert!(html.contains("id=\"remove-password-link\""));
    assert!(html.contains("class=\"control-group pref-passkeys\""));
    assert!(html.contains("btn-second-factor"));
    // The current device, active, without its menu.
    assert!(html.contains("class=\"row auth-token\""));
    assert!(html.contains("<span class=\"active\">"));
    assert!(html.contains("<div class=\"user-api-key__name\">Some App</div>"));
    assert!(html.contains("id=\"prefs-security-strings\""));

    let html = user1.page("/u/user1/preferences/apps").await.html;
    assert!(html.contains("<title>Preferences - user1 - "));
    assert!(html.contains("data-key-id=\"9802\""));

    let html = user1
        .page("/u/user1/preferences/calendar-subscriptions")
        .await
        .html;
    assert!(html.contains("calendar-subscriptions__description"));
    assert!(html.contains("calendar-generate"));

    let html = user1.page("/u/user1/preferences/chat").await.html;
    assert!(html.contains("class=\"form-kit chat-preferences\""));
    assert!(html.contains("name=\"chat_enabled\""));
    assert!(html.contains("name=\"chat_separate_sidebar_mode\""));
}

#[tokio::test]
async fn a_confirmed_session_opens_the_second_factor_page() {
    let (_db, mut user1) = client("user1").await;
    let trusted = user1.send(Method::GET, "/u/trusted-session.json", "").await;
    assert_eq!(trusted, "{\"failed\":\"FAILED\"}");
    let confirmed = user1
        .send(Method::POST, "/u/confirm-session.json", "password=password")
        .await;
    assert_eq!(confirmed, "{\"success\":\"OK\"}");
    let page = user1.page("/u/user1/preferences/second-factor").await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.html.contains("solo-preference second-factor"));
    assert!(page.html.contains("new-totp"));
    assert!(page.html.contains("new-security-key"));
    // What the modals draw from.
    let needle = "id=\"second-factor-data\">";
    let start = page.html.find(needle).unwrap() + needle.len();
    let end = start + page.html[start..].find("</script>").unwrap();
    let data: serde_json::Value = serde_json::from_str(&page.html[start..end]).unwrap();
    assert_eq!(data["username_lower"], "user1");
    assert_eq!(data["totps"], serde_json::json!([]));
    assert_eq!(data["enforced"], false);
    assert_eq!(data["strings"]["totp_add"], "Add Authenticator");
    assert!(
        data["strings"]["delete_confirm_instruction"]
            .as_str()
            .unwrap()
            .contains("%{confirm}")
    );
    // The authenticator's secret, staged with its QR code.
    let created = user1
        .send(Method::POST, "/u/create_second_factor_totp.json", "")
        .await;
    assert_eq!(user1.status, StatusCode::OK, "{created}");
    let created: serde_json::Value = serde_json::from_str(&created).unwrap();
    assert_eq!(created["key"].as_str().unwrap().len(), 39);
    assert!(
        created["qr"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
}

async fn enforce(db: &TestDb, value: &str) {
    sqlx::query(
        "INSERT INTO site_settings (name, data_type, value, created_at, updated_at) \
         VALUES ('enforce_second_factor', 7, $1, now(), now())",
    )
    .bind(value)
    .execute(&db.pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn enforced_second_factors_send_members_without_one_to_add_one() {
    let (db, mut user1) = client("user1").await;
    enforce(&db, "all").await;
    let page = user1.page("/latest").await;
    assert_eq!(page.status, StatusCode::FOUND);
    assert_eq!(
        page.location.as_deref(),
        Some("http://test.localhost/u/user1/preferences/second-factor")
    );
    // Not the page itself, and not JSON.
    let page = user1.page("/u/user1/preferences/second-factor").await;
    // (Its own redirect, for an unconfirmed session, is the security tab.)
    assert_eq!(
        page.location.as_deref(),
        Some("http://test.localhost/u/user1/preferences/security")
    );
    user1.send(Method::GET, "/latest.json", "").await;
    assert_eq!(user1.status, StatusCode::OK);
    // With an authenticator, nothing.
    sqlx::query(
        "INSERT INTO user_second_factors (user_id, method, data, enabled, name, created_at, updated_at) \
         SELECT id, 1, 'JBSWY3DPEHPK3PXP', true, 'phone', now(), now() FROM users WHERE username = 'user1'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    assert_eq!(user1.page("/latest").await.status, StatusCode::OK);
}

#[tokio::test]
async fn enforcing_for_staff_leaves_members_be() {
    let (db, mut user1) = client("user1").await;
    enforce(&db, "staff").await;
    assert_eq!(user1.page("/latest").await.status, StatusCode::OK);
}

#[tokio::test]
async fn the_second_factor_page_draws_the_challenge() {
    let (db, mut user1) = client("user1").await;
    sqlx::query(
        "INSERT INTO user_second_factors (user_id, method, data, enabled, name, created_at, updated_at) \
         SELECT id, 1, 'JBSWY3DPEHPK3PXP', true, 'phone', now(), now() FROM users WHERE username = 'user1'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let page = user1.page("/session/2fa?nonce=nope").await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.html.contains("id=\"second-factor-auth-data\""));
    assert!(page.html.contains("\"load_error\":true"), "{}", page.html);
    // A challenge staged by an email change's confirmation.
    let token = "known-token-193d";
    sqlx::query(
        "INSERT INTO email_tokens (id, user_id, email, token_hash, scope, expired, confirmed, created_at, updated_at) \
         SELECT 9601, id, 'user1-new@example.com', \
         'c351f96f39d5ae3328d96a8e325d0cb426475009121f75c932d9aef4104096b9', 4, false, false, now(), now() \
         FROM users WHERE username = 'user1'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO email_change_requests (user_id, old_email, new_email, new_email_token_id, change_state, created_at, updated_at) \
         SELECT id, 'user1@example.com', 'user1-new@example.com', 9601, 2, now(), now() FROM users WHERE username = 'user1'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let challenge = user1
        .send(
            Method::PUT,
            &format!("/u/confirm-new-email/{token}.json"),
            "",
        )
        .await;
    assert_eq!(user1.status, StatusCode::FORBIDDEN, "{challenge}");
    let challenge: serde_json::Value = serde_json::from_str(&challenge).unwrap();
    let nonce = challenge["second_factor_challenge_nonce"].as_str().unwrap();
    let page = user1.page(&format!("/session/2fa?nonce={nonce}")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.html.contains("\"available\":[1]"), "{}", page.html);
    assert!(page.html.contains("\"shown\":1"));
    assert!(page.html.contains("\"totp_enabled\":true"));
    // hideApplicationSidebar
    assert!(!page.html.contains("sidebar.js"));
}

#[tokio::test]
async fn anonymous_preferences_are_not_found() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut anon = Client {
        state: st,
        cookies: Vec::new(),
        csrf: None,
        status: StatusCode::OK,
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
async fn the_notifications_page_lists_ones_notifications() {
    let (_db, mut user1) = client("user1").await;
    let page = user1.page("/u/user1/notifications").await;
    assert_eq!(page.status, StatusCode::OK);
    let html = page.html;
    assert!(
        html.contains("<title>All - Notifications - user1 - "),
        "{html}"
    );
    assert!(html.contains("user-notifications-page"));
    assert!(html.contains(
        "<li aria-current=\"page\" class=\"user-nav__notifications\"><a class=\"active\" href=\"/u/user1/notifications\">"
    ));
    assert!(html.contains(
        "<li aria-current=\"location\" class=\"user-nav__notifications-all\"><a class=\"active\" href=\"/u/user1/notifications\">"
    ));
    assert!(html.contains("dismiss-notifications"));
    assert!(html.contains("notifications-filter"));
    assert!(html.contains("<li class=\"notification unread granted-badge\""));
    assert!(html.contains("<span class=\"relative-date\""));
    // Read ones only.
    let read = user1.page("/u/user1/notifications?filter=read").await.html;
    assert!(!read.contains("class=\"notification unread"));
    assert!(read.contains("class=\"notification read"));
}

#[tokio::test]
async fn the_notifications_stream_tabs_draw_the_user_actions() {
    let (_db, mut user1) = client("user1").await;
    let html = user1.page("/u/user1/notifications/responses").await.html;
    assert!(html.contains("<title>Responses - Notifications - user1 - "));
    assert!(html.contains("class=\"post-list user-stream filter-6\""));
    let likes = user1
        .page("/u/user1/notifications/likes-received")
        .await
        .html;
    assert!(likes.contains("filter-2"));
    // A tab the app has no route for.
    let unknown = user1.page("/u/user1/notifications/bogus").await;
    assert!(unknown.html.contains("page-not-found"));
}

#[tokio::test]
async fn another_members_notifications_go_to_the_404_route() {
    let (_db, mut user2) = client("user2").await;
    let page = user2.page("/u/user1/notifications").await;
    assert_eq!(page.status, StatusCode::FOUND);
    assert!(page.location.unwrap().ends_with("/404"));
    let (_db, mut admin) = client("admin").await;
    assert_eq!(
        admin.page("/u/user1/notifications").await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn the_interface_tab_draws_the_palettes_members_may_pick() {
    let (db, mut user1) = client("user1").await;
    sqlx::query("UPDATE color_schemes SET user_selectable = TRUE WHERE id IN (1, 2)")
        .execute(&db.pool)
        .await
        .unwrap();
    let html = user1.page("/u/user1/preferences/interface").await.html;
    // Light: the theme default, then the light palettes before the dark
    // ones; dark: the theme default and the dark ones.
    let light = &html[html.find("light-color-scheme").unwrap()..];
    let light = &light[..light.find("dark-color-scheme").unwrap()];
    let names: Vec<&str> = light
        .split("class=\"color-palette-picker-row__name\">")
        .skip(1)
        .map(|s| &s[..s.find('<').unwrap()])
        .collect();
    assert_eq!(names, ["Theme default", "Horizon", "Horizon Dark"]);
    // The session's palette (none: the theme default) is the active one.
    assert!(light.contains(
        "data-value=\"-1\" title=\"Theme default\" tabindex=\"0\" class=\"select-kit-row color-palette-picker-row is-highlighted is-selected\"><span class=\"color-palette-picker-row__badge --active\">Active</span>"
    ));
    assert!(light.contains("--primary--preview: #1a1a1a"));
    let dark = &html[html.find("dark-color-scheme").unwrap()..];
    assert!(dark.contains("data-name=\"Horizon Dark\""));
    assert!(!dark[..dark.find("</fieldset>").unwrap()].contains("data-name=\"Horizon\""));
    // Foundation has no light palette of its own, so no mode selector.
    assert!(!html.contains("interface-color-mode"));
    let attrs = save_attrs(&html);
    assert_eq!(attrs["color_scheme_id"], -1);
    assert_eq!(attrs["dark_scheme_id"], -1);

    // A dark palette of one's own shows the mode selector too.
    sqlx::query("UPDATE user_options SET dark_scheme_id = 2 WHERE user_id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let html = user1.page("/u/user1/preferences/interface").await.html;
    assert!(html.contains("control-subgroup interface-color-mode"));
    assert_eq!(save_attrs(&html)["dark_scheme_id"], 2);
}

#[tokio::test]
async fn a_palette_preview_names_its_stylesheet() {
    let (_db, mut user1) = client("user1").await;
    let json = |body: &str| serde_json::from_str::<serde_json::Value>(body).unwrap();
    let scheme = user1
        .send(Method::GET, "/color-scheme-stylesheet/2.json", "")
        .await;
    assert_eq!(
        json(&scheme),
        serde_json::json!({
            "color_scheme_id": 2,
            "new_href": "/assets/color_definitions_scheme/2.css",
        })
    );
    // The default theme's dark palette, and with the theme named.
    let dark = user1
        .send(Method::GET, "/color-scheme-stylesheet/13/-1.json", "")
        .await;
    assert_eq!(
        json(&dark)["new_href"],
        "/assets/color_definitions_dark.css"
    );
    // No such palette: the theme's, or the base one (-1).
    for missing in [
        "/color-scheme-stylesheet/9999.json",
        "/color-scheme-stylesheet/abc",
    ] {
        let base = user1.send(Method::GET, missing, "").await;
        assert_eq!(
            json(&base),
            serde_json::json!({
                "color_scheme_id": -1,
                "new_href": "/assets/color_definitions_light.css",
            }),
            "{missing}"
        );
    }
    user1
        .send(Method::GET, "/color-scheme-stylesheet/2/x.json", "")
        .await;
    assert_eq!(user1.status, StatusCode::NOT_FOUND);
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

#[tokio::test]
async fn the_emails_tracking_users_and_navigation_tabs_draw_their_options() {
    let (_db, mut user1) = client("user1").await;
    let html = user1.page("/u/user1/preferences/emails").await.html;
    assert!(html.contains("<title>Emails - Preferences - user1 - "));
    assert!(html.contains("data-pref=\"email_messages_level\""));
    assert!(html.contains("data-pref=\"digest_after_minutes\""));
    assert!(html.contains("data-disabled-by=\"mailing_list_mode\""));
    let attrs = save_attrs(&html);
    assert!(attrs.get("email_level").is_some());
    assert!(attrs.get("chat_email_frequency").is_some());

    let html = user1.page("/u/user1/preferences/tracking").await.html;
    assert!(html.contains("user-preferences-tracking-page"));
    assert!(html.contains("data-chooser=\"category\" data-pref=\"watched_category_ids\""));
    assert!(html.contains("data-chooser=\"tag\" data-pref=\"watched_tags\""));
    assert!(html.contains("id=\"prefs-choosers-strings\""));
    let attrs = save_attrs(&html);
    // Category id lists, [-1] standing in for an empty one.
    for key in [
        "watched_category_ids",
        "tracked_category_ids",
        "muted_category_ids",
        "watched_first_post_category_ids",
    ] {
        assert!(!attrs[key].as_array().unwrap().is_empty(), "{key}");
    }

    let html = user1.page("/u/user1/preferences/tags").await.html;
    assert!(html.contains("<title>Preferences - user1 - "));
    assert!(html.contains("data-pref=\"tracked_tags\""));

    let html = user1.page("/u/user1/preferences/users").await.html;
    assert!(html.contains("<title>Users - Preferences - user1 - "));
    assert!(html.contains("add-ignored-user"));
    assert!(html.contains("data-chooser=\"user\" data-pref=\"muted_usernames\""));
    assert!(html.contains("data-disabled-unless=\"allow_private_messages\""));

    let html = user1
        .page("/u/user1/preferences/navigation-menu")
        .await
        .html;
    assert!(html.contains("data-pref=\"sidebar_link_to_filtered_list\""));
    let attrs = save_attrs(&html);
    assert_eq!(attrs["sidebar_show_count_of_new_items"], false);
}

#[tokio::test]
async fn the_email_page_changes_or_adds_an_address() {
    let (db, mut user1) = client("user1").await;
    let email: String =
        sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = 3 AND \"primary\"")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let html = user1.page("/u/user1/preferences/email").await.html;
    assert!(html.contains("id=\"change-email\""));
    assert!(html.contains(&format!("data-email-old=\"{email}\"")));
    assert!(html.contains("data-email-new=\"false\""));
    let html = user1.page("/u/user1/preferences/email?new=1").await.html;
    assert!(html.contains("data-email-new=\"true\""));
    assert!(html.contains("value=\"\" id=\"change-email\""));
}

#[tokio::test]
async fn an_email_change_is_confirmed_from_the_new_address() {
    let (db, mut user1) = client("user1").await;
    let body = user1
        .send(
            Method::PUT,
            "/u/user1/preferences/email.json",
            "email=changed%40example.com",
        )
        .await;
    // UsersEmailController#update renders an empty body.
    assert_eq!(body, "");
    let args: serde_json::Value = sqlx::query_scalar(
        "SELECT args FROM discourse_rs.jobs WHERE name = 'critical_user_email' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(args["type"], "confirm_new_email");
    assert_eq!(args["to_address"], "changed@example.com");
    let token = args["email_token"].as_str().unwrap().to_string();

    let page = user1.page(&format!("/u/confirm-new-email/{token}")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.html.contains("class=\"confirm-new-email\""));
    assert!(page.html.contains("changed@example.com"));
    let json = user1
        .send(
            Method::GET,
            &format!("/u/confirm-new-email/{token}.json"),
            "",
        )
        .await;
    let json: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(json["new_email"], "changed@example.com");

    let body = user1
        .send(
            Method::PUT,
            &format!("/u/confirm-new-email/{token}.json"),
            "",
        )
        .await;
    assert_eq!(body, "{\"success\":\"OK\"}");
    let primary: String =
        sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = 3 AND \"primary\"")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(primary, "changed@example.com");
    // The token is spent.
    let again = user1
        .send(
            Method::PUT,
            &format!("/u/confirm-new-email/{token}.json"),
            "",
        )
        .await;
    assert!(!again.contains("success"));
}
