//! The styleguide plugin, ported from its specs: who may see /styleguide
//! (spec/requests/styleguide/styleguide_controller_spec.rb and
//! spec/integration/access_spec.rb), `can_see_styleguide` on /site.json,
//! the plugin's assets on its own pages only (spec/integration/assets_spec.rb),
//! and the section pages.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

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

    async fn request(&mut self, method: Method, path: &str, body: &str) -> (StatusCode, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        if path.ends_with(".json") || !body.is_empty() {
            request = request.header("x-requested-with", "XMLHttpRequest");
        }
        if !self.cookies.is_empty() {
            let cookie: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookie.join("; "));
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
            let pair = v.to_str().unwrap().split(';').next().unwrap();
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
        let (_, csrf) = client.request(Method::GET, "/session/csrf.json", "").await;
        let csrf: Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
            .request(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
            )
            .await;
        client
    }

    async fn status(&mut self, path: &str) -> StatusCode {
        self.request(Method::GET, path, "").await.0
    }
}

async fn exec(db: &TestDb, sql: &str) {
    sqlx::query(sql)
        .execute(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

/// `SiteSetting.styleguide_allowed_groups =`
async fn allow(db: &TestDb, groups: &str) {
    set_setting(&db.pool, "styleguide_allowed_groups", 20, groups).await;
}

/// `Fabricate(:moderator)`: user2 made one, in moderators and staff.
async fn moderator(db: &TestDb) {
    exec(
        db,
        "UPDATE users SET moderator = TRUE WHERE username = 'user2'",
    )
    .await;
    exec(
        db,
        "INSERT INTO group_users (group_id, user_id, created_at, updated_at) \
         SELECT g, u.id, now(), now() FROM users u, unnest(ARRAY[2, 3]) g \
         WHERE u.username = 'user2' ON CONFLICT DO NOTHING",
    )
    .await;
}

async fn fresh(db: &TestDb) -> AppState {
    state(db.pool.clone(), config(RailsEnv::Test, &[])).await
}

#[tokio::test]
async fn anonymous_and_logged_in_groups_let_everyone_in() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "styleguide_enabled", 5, "t").await;
    allow(&db, "4|5").await;
    let st = fresh(&db).await;
    assert_eq!(
        Client::anonymous(st.clone()).status("/styleguide").await,
        StatusCode::OK
    );
    assert_eq!(
        Client::logged_in(st, "user1")
            .await
            .status("/styleguide")
            .await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn staff_only_lets_moderators_and_admins_in() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "styleguide_enabled", 5, "t").await;
    allow(&db, "3").await;
    moderator(&db).await;
    let st = fresh(&db).await;
    assert_eq!(
        Client::anonymous(st.clone()).status("/styleguide").await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        Client::logged_in(st.clone(), "user1")
            .await
            .status("/styleguide")
            .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        Client::logged_in(st.clone(), "user2")
            .await
            .status("/styleguide")
            .await,
        StatusCode::OK
    );
    assert_eq!(
        Client::logged_in(st, "admin")
            .await
            .status("/styleguide")
            .await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_custom_group_lets_its_members_in() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "styleguide_enabled", 5, "t").await;
    exec(
        &db,
        "INSERT INTO groups (id, name, created_at, updated_at) VALUES (9101, 'stylists', now(), now())",
    )
    .await;
    exec(
        &db,
        "INSERT INTO group_users (group_id, user_id, created_at, updated_at) \
         SELECT 9101, id, now(), now() FROM users WHERE username = 'user3'",
    )
    .await;
    allow(&db, "9101").await;
    let st = fresh(&db).await;
    assert_eq!(
        Client::anonymous(st.clone()).status("/styleguide").await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        Client::logged_in(st.clone(), "user1")
            .await
            .status("/styleguide")
            .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        Client::logged_in(st, "user3")
            .await
            .status("/styleguide")
            .await,
        StatusCode::OK
    );
}

/// access_spec.rb: admins only by default; everyone (0) is not anonymous
/// visitors unless the granular setting is off.
#[tokio::test]
async fn admins_only_by_default_and_everyone_is_not_anonymous() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "styleguide_enabled", 5, "t").await;
    let st = fresh(&db).await;
    assert_eq!(
        Client::logged_in(st.clone(), "admin")
            .await
            .status("/styleguide")
            .await,
        StatusCode::OK
    );
    assert_eq!(
        Client::logged_in(st, "user1")
            .await
            .status("/styleguide")
            .await,
        StatusCode::NOT_FOUND
    );

    allow(&db, "0").await;
    let st = fresh(&db).await;
    assert_eq!(
        Client::anonymous(st).status("/styleguide").await,
        StatusCode::NOT_FOUND
    );
    allow(&db, "4").await;
    let st = fresh(&db).await;
    assert_eq!(
        Client::anonymous(st).status("/styleguide").await,
        StatusCode::OK
    );
    allow(&db, "0").await;
    set_setting(
        &db.pool,
        "granular_anonymous_and_logged_in_groups_permissions",
        5,
        "f",
    )
    .await;
    let st = fresh(&db).await;
    assert_eq!(
        Client::anonymous(st).status("/styleguide").await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn disabled_is_not_found() {
    let db = TestDb::new().await;
    let st = fresh(&db).await;
    let mut admin = Client::logged_in(st, "admin").await;
    assert_eq!(admin.status("/styleguide").await, StatusCode::NOT_FOUND);
    assert_eq!(
        admin.status("/styleguide/atoms/buttons").await,
        StatusCode::NOT_FOUND
    );
}

/// "Styleguide site capability"
#[tokio::test]
async fn site_json_says_who_can_see_it() {
    let db = TestDb::new().await;
    let st = fresh(&db).await;
    let (_, body) = Client::anonymous(st)
        .request(Method::GET, "/site.json", "")
        .await;
    let site: Value = serde_json::from_str(&body).unwrap();
    assert!(
        site.get("can_see_styleguide").is_none(),
        "omitted while off"
    );

    set_setting(&db.pool, "styleguide_enabled", 5, "t").await;
    allow(&db, "1").await;
    let st = fresh(&db).await;
    let (_, body) = Client::logged_in(st.clone(), "admin")
        .await
        .request(Method::GET, "/site.json", "")
        .await;
    let site: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(site["can_see_styleguide"], true);
    let (_, body) = Client::logged_in(st, "user1")
        .await
        .request(Method::GET, "/site.json", "")
        .await;
    let site: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(site["can_see_styleguide"], false);
}

/// assets_spec.rb, and the pages themselves.
#[tokio::test]
async fn the_pages_carry_the_sections() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "styleguide_enabled", 5, "t").await;
    let st = fresh(&db).await;
    let mut admin = Client::logged_in(st, "admin").await;

    let (_, home) = admin.request(Method::GET, "/", "").await;
    assert!(!home.contains("data-target=\"styleguide\""));

    let (status, index) = admin.request(Method::GET, "/styleguide", "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(index.contains("data-target=\"styleguide\""));
    assert!(index.contains("To get started, choose a section from the navigation menu."));
    for link in [
        "/styleguide/syntax/bem",
        "/styleguide/atoms/typography",
        "/styleguide/atoms/buttons",
        "/styleguide/molecules/bread-crumbs",
        "/styleguide/organisms/more-topics",
    ] {
        assert!(index.contains(link), "{link}");
    }

    let (status, buttons) = admin
        .request(Method::GET, "/styleguide/atoms/buttons", "")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(buttons.contains("styleguide-section buttons-examples"));
    assert!(buttons.contains("DButton - icon and text - btn-primary - states"));
    assert!(buttons.contains("btn btn-icon-text btn-primary btn-hover"));
    assert!(buttons.contains("d-toggle-switch"));

    let (_, colors) = admin
        .request(Method::GET, "/styleguide/atoms/colors", "")
        .await;
    assert!(colors.contains("<div class=\"color-name\">var(--tertiary-25)</div>"));

    let (_, icons) = admin
        .request(Method::GET, "/styleguide/atoms/icons", "")
        .await;
    assert!(
        icons.contains("<span>discourse-expand</span>"),
        "the sprite's symbols"
    );

    let (_, typography) = admin
        .request(Method::GET, "/styleguide/atoms/typography", "")
        .await;
    assert!(typography.contains("<h1>Welcome to Discourse</h1>"));

    let (status, forms) = admin
        .request(Method::GET, "/styleguide/atoms/forms", "")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(forms.contains("form-kit__control-input"));

    // sectionById: the section, whatever category the path names.
    assert_eq!(
        admin.status("/styleguide/organisms/buttons").await,
        StatusCode::OK
    );
    // An unknown section is the 404 page.
    assert_eq!(
        admin.status("/styleguide/atoms/nope").await,
        StatusCode::NOT_FOUND
    );
    let (status, post) = admin
        .request(Method::GET, "/styleguide/organisms/post", "")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(post.contains("styleguide-note"));
}
