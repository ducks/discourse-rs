//! The guidelines, terms and privacy pages (static#show) as the Ember app
//! draws them: templates/faq.gjs around show.html.erb, the route's body
//! class, title and sidebar link, WatchRead for a member who has not read
//! the guidelines, and the login wall's exceptions. The XHR fragment and
//! the redirects are write cases (parity/writes/static).

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

const BOOL: i32 = 5;
const INTEGER: i32 = 3;

struct Reply {
    status: StatusCode,
    location: Option<String>,
    cookies: Vec<String>,
    body: String,
}

async fn send(st: &AppState, request: Request<Body>) -> Reply {
    let response = discourse_rs::app(st.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_string());
    let cookies = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|c| c.split(';').next().unwrap().to_string())
        .collect();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        location,
        cookies,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

/// A page as a browser asks for it (no XHR header).
async fn page(st: &AppState, path: &str, cookie: Option<&str>) -> Reply {
    let mut builder = Request::get(path).header(header::HOST, "test.localhost");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    send(st, builder.body(Body::empty()).unwrap()).await
}

/// The `_t` cookie of `username`, logged in through POST /session.
async fn logged_in(st: &AppState, username: &str) -> String {
    let csrf_reply = send(
        st,
        Request::get("/session/csrf.json")
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let session = csrf_reply
        .cookies
        .iter()
        .find(|c| c.starts_with("_forum_session="))
        .unwrap()
        .clone();
    let csrf: Value = serde_json::from_str(&csrf_reply.body).unwrap();
    let reply = send(
        st,
        Request::post("/session")
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest")
            .header("x-csrf-token", csrf["csrf"].as_str().unwrap())
            .header(header::COOKIE, &session)
            .header(
                header::CONTENT_TYPE,
                "application/x-www-form-urlencoded; charset=UTF-8",
            )
            .body(Body::from(format!("login={username}&password=password")))
            .unwrap(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    reply
        .cookies
        .into_iter()
        .find(|c| c.starts_with("_t="))
        .expect("a login sets _t")
}

#[tokio::test]
async fn the_guidelines_page_draws_embers_static_route() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let reply = page(&st, "/guidelines", None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let html = &reply.body;
    assert!(html.contains("<title>Guidelines - "), "{html}");
    assert!(html.contains(" static-guidelines"), "{html}");
    assert!(html.contains("has-sidebar-page"), "{html}");
    for needle in [
        // faq.gjs around the preloaded show.html.erb.
        "<section class=\"container\"><div class=\"contents clearfix body-page\"><div itemscope itemtype='http://schema.org/WebPage'>",
        "<li class='nav-item-guidelines'><a class='active' href='/guidelines'>Guidelines</a></li>",
        "<div itemprop='mainContentOfPage'>",
        "This is a Civilized Place for Public Discussion",
        // The sidebar's FAQ link, renamed, on its route.
        "<a class=\"active sidebar-section-link sidebar-row\" title=\"Guidelines for using this site\" data-link-name=\"guidelines\" href=\"/guidelines\">",
        // crawlable_meta_data(title: @title)
        "<meta property=\"og:title\" content=\"Guidelines - ",
    ] {
        assert!(html.contains(needle), "{needle}\n{html}");
    }
    // Anonymous: no edit link, nothing to mark read.
    assert!(!html.contains("Edit this page"), "{html}");
    assert!(!html.contains("/u/read-faq"), "{html}");
}

#[tokio::test]
async fn members_who_have_not_read_the_guidelines_are_watched() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    sqlx::query(
        "UPDATE user_stats SET read_faq = NULL WHERE user_id IN \
         (SELECT id FROM users WHERE username IN ('user1', 'admin'))",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let member = logged_in(&st, "user1").await;
    let html = page(&st, "/guidelines", Some(&member)).await.body;
    assert!(html.contains("fetch(\"/u/read-faq\""), "{html}");
    assert!(!html.contains("Edit this page"), "{html}");

    sqlx::query(
        "UPDATE user_stats SET read_faq = now() WHERE user_id = \
         (SELECT id FROM users WHERE username = 'user1')",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let html = page(&st, "/guidelines", Some(&member)).await.body;
    assert!(!html.contains("/u/read-faq"), "{html}");

    // Staff get the topic's edit link.
    let admin = logged_in(&st, "admin").await;
    let html = page(&st, "/guidelines", Some(&admin)).await.body;
    assert!(
        html.contains("<p><a href='http://test.localhost/t/guidelines/4'>"),
        "{html}"
    );
    assert!(html.contains("Edit this page"), "{html}");
}

#[tokio::test]
async fn the_faq_route_without_the_rename() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    set_setting(&db.pool, "rename_faq_to_guidelines", BOOL, "f").await;
    let reply = page(&st, "/faq", None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let html = &reply.body;
    assert!(html.contains("<title>FAQ - "), "{html}");
    assert!(html.contains(" static-faq"), "{html}");
    assert!(
        html.contains("<li class='nav-item-faq'><a class='active' href='/faq'>FAQ</a></li>"),
        "{html}"
    );
    assert!(
        html.contains("<a class=\"active sidebar-section-link sidebar-row\" title=\"Guidelines for using this site\" data-link-name=\"faq\" href=\"/faq\">"),
        "{html}"
    );
    // /rules is its own route, the FAQ topic under its own name.
    let html = page(&st, "/rules", None).await.body;
    assert!(html.contains("<title>Rules - "), "{html}");
    assert!(html.contains(" static-rules"), "{html}");
    assert!(
        html.contains("data-link-name=\"faq\" href=\"/faq\""),
        "{html}"
    );
    assert!(!html.contains("active sidebar-section-link sidebar-row\" title=\"Guidelines for using this site\" data-link-name=\"faq\""), "{html}");
}

#[tokio::test]
async fn terms_and_privacy_pages_from_their_topics() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    assert_eq!(page(&st, "/tos", None).await.status, StatusCode::NOT_FOUND);
    set_setting(&db.pool, "tos_topic_id", INTEGER, "4").await;
    let reply = page(&st, "/tos", None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let html = &reply.body;
    assert!(html.contains("<title>Terms of Service - "), "{html}");
    assert!(html.contains(" static-tos"), "{html}");
    assert!(
        html.contains(
            "<li class='nav-item-tos'><a href='/tos' class='active'>Terms of Service</a></li>"
        ),
        "{html}"
    );

    // A deleted topic is no page, and no link.
    sqlx::query("UPDATE topics SET deleted_at = now() WHERE id = 4")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(page(&st, "/tos", None).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn login_required_lets_legal_pages_through_only() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    set_setting(&db.pool, "login_required", BOOL, "t").await;
    set_setting(&db.pool, "tos_topic_id", INTEGER, "4").await;
    let reply = page(&st, "/guidelines", None).await;
    assert_eq!(reply.status, StatusCode::FOUND);
    assert!(reply.location.unwrap().ends_with("/login"));
    let reply = page(&st, "/tos", None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    // No navigation behind the wall but the legal links.
    assert!(!reply.body.contains("nav-item-about"), "{}", reply.body);
    assert!(reply.body.contains("nav-item-tos"), "{}", reply.body);
}

#[tokio::test]
async fn a_page_load_of_the_html_path_is_refused() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let reply = page(&st, "/guidelines.html", None).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
}
