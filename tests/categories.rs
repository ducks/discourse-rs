//! Category behaviour the parity cases cannot record: states the reference
//! never has (a site without the Horizon theme), and what goes out on the
//! bus (MessageBus lives in Redis on Rails, outside the recordings).

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::config::RailsEnv;
use discourse_rs::guardian::Guardian;
use discourse_rs::session::current::SessionUser;
use discourse_rs::site_settings::SiteSettings;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

async fn admin_guardian(conn: &mut sqlx::PgConnection) -> Guardian {
    let user = SessionUser::load(conn, 1).await.unwrap().unwrap();
    Guardian::for_user(conn, &user).await.unwrap()
}

/// The Support type's category custom field keys, as the registry lists
/// them for an admin.
async fn support_custom_fields(db: &TestDb) -> Vec<String> {
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut conn = db.pool.acquire().await.unwrap();
    let s = SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap();
    let guardian = admin_guardian(&mut conn).await;
    let types: Value = discourse_rs::category_serializer::with_registry(
        &discourse_rs::category_serializer::Cx::of(&st),
        &s,
        async |r| r.list(&mut conn, &guardian).await,
    )
    .await
    .unwrap();
    let support = types
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "support")
        .unwrap();
    support["configuration_schema"]["category_custom_fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["key"].as_str().unwrap().to_string())
        .collect()
}

// A restored backup from before Horizon has no theme -2, so Horizon is
// not the default theme and Support offers the empty box toggle (Rails
// syncs the core themes on migration, so it always has the row).
#[tokio::test]
async fn category_types_without_horizon() {
    let db = TestDb::new().await;
    assert!(
        support_custom_fields(&db)
            .await
            .contains(&"empty_box_on_unsolved".to_string())
    );
    sqlx::query("DELETE FROM theme_fields WHERE theme_id = -2")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM child_themes WHERE parent_theme_id = -2 OR child_theme_id = -2")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM themes WHERE id = -2")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        support_custom_fields(&db)
            .await
            .contains(&"empty_box_on_unsolved".to_string())
    );
}

/// A session over the app, as the browser keeps one.
struct Client {
    state: discourse_rs::AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn send(
        &mut self,
        method: Method,
        path: &str,
        body: Option<(&str, &str)>,
        xhr: bool,
    ) -> (StatusCode, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        if xhr {
            request = request.header("x-requested-with", "XMLHttpRequest");
            request = request.header(header::ACCEPT, "application/json");
            if let Some(token) = &self.csrf {
                request = request.header("x-csrf-token", token.as_str());
            }
        }
        if !self.cookies.is_empty() {
            let cookie: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookie.join("; "));
        }
        let body = match body {
            Some((content_type, body)) => {
                request = request.header(header::CONTENT_TYPE, content_type);
                body.to_string()
            }
            None => String::new(),
        };
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body)).unwrap())
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

    async fn logged_in(state: discourse_rs::AppState, username: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client
            .send(Method::GET, "/session/csrf.json", None, true)
            .await;
        let csrf: Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
            .send(
                Method::POST,
                "/session",
                Some((
                    "application/x-www-form-urlencoded",
                    &format!("login={username}&password=password"),
                )),
                true,
            )
            .await;
        client
    }
}

// The editor: a new category for an admin, an existing one with its
// tabs, the voting toggle reflecting the category's voting setting, and
// none for a member who can't edit.
#[tokio::test]
async fn the_category_editor() {
    let db = TestDb::new().await;
    common::set_setting(&db.pool, "topic_voting_enabled", 5, "t").await;
    sqlx::query(
        "INSERT INTO topic_voting_category_settings (category_id, created_at, updated_at) VALUES (4, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, html) = admin.send(Method::GET, "/new-category", None, false).await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("class=\"edit-category-page\""), "{html}");
    assert!(html.contains("<h1 class=\"d-page-header__title\">New category</h1>"));
    assert!(html.contains("/assets/category-edit.js"));

    let (status, html) = admin
        .send(Method::GET, "/c/general/edit/security", None, false)
        .await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("Edit: General"), "{html}");
    assert!(html.contains("href=\"/c/general/edit/general\""));
    assert!(html.contains("<div class=\"edit-category-tab edit-category-tab-security\">"));
    assert!(html.contains("<div class=\"edit-category-tab edit-category-tab-general\" hidden>"));
    // Ideas (topic voting) is checked for a voting category.
    let ideas = html
        .split("data-category-type=\"ideas\"")
        .nth(1)
        .unwrap()
        .split("</div>")
        .next()
        .unwrap();
    assert!(ideas.contains(" checked"), "{ideas}");
    let (status, html) = admin
        .send(Method::GET, "/c/site-feedback/edit/general", None, false)
        .await;
    assert_eq!(status, StatusCode::OK);
    let ideas = html
        .split("data-category-type=\"ideas\"")
        .nth(1)
        .unwrap()
        .split("</div>")
        .next()
        .unwrap();
    assert!(!ideas.contains(" checked"), "{ideas}");

    let mut member = Client::logged_in(st.clone(), "user1").await;
    let (status, _) = member
        .send(Method::GET, "/c/general/edit/general", None, false)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = member.send(Method::GET, "/new-category", None, false).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// What the editor's Save sends turns voting on, and off again, through
// PUT /categories/:id's category_types; the category goes out on
// /categories each time.
#[tokio::test]
async fn the_voting_toggle_saves() {
    let db = TestDb::new().await;
    common::set_setting(&db.pool, "topic_voting_enabled", 5, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let from = st.bus.now().await.unwrap();
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let voting = |pool: sqlx::PgPool| async move {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM topic_voting_category_settings WHERE category_id = 2)",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let (status, body) = admin
        .send(
            Method::PUT,
            "/categories/2.json",
            Some((
                "application/json",
                r#"{"name": "Site Feedback", "category_types": ["discussion", "ideas"]}"#,
            )),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(voting(db.pool.clone()).await);
    let (status, _) = admin
        .send(
            Method::PUT,
            "/categories/2.json",
            Some(("application/json", r#"{"category_types": ["discussion"]}"#)),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!voting(db.pool.clone()).await);
    let messages = common::bus_messages(&st, from, &["/categories"]).await;
    let published: Vec<i64> = messages
        .iter()
        .filter_map(|m| m.data["categories"][0]["id"].as_i64())
        .collect();
    assert_eq!(published, vec![2, 2], "{messages:?}");
}
