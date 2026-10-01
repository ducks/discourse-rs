//! `login_required`: anonymous requests are turned away everywhere except
//! the status and basic-info endpoints.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

const BOOL: i32 = 5;

async fn get(pool: &PgPool, path: &str) -> (StatusCode, Vec<(String, String)>, String) {
    let app = discourse_rs::app(state(pool.clone(), config(RailsEnv::Test, &[])));
    let response = app
        .oneshot(
            Request::get(path)
                .header(header::HOST, "test.localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

#[tokio::test]
async fn json_requests_get_a_403() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "login_required", BOOL, "t").await;
    for path in [
        "/latest.json",
        "/site.json",
        "/categories.json",
        "/top.json",
        "/tags.json",
        "/tag/howto.json",
        "/c/general/4.json",
    ] {
        let (status, headers, body) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(
            header(&headers, "cache-control"),
            Some("no-cache, no-store")
        );
        let json: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["error_type"], "not_logged_in", "{path}");
        assert_eq!(json["errors"][0], "You need to be logged in to do that.");
        assert!(json.get("extras").is_none(), "{path}");
    }
    // topics#show adds the not-found extras, even for a missing topic: the
    // gate runs before the action.
    for path in ["/t/welcome-to-discourse/5.json", "/t/999999.json"] {
        let (status, _, body) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
        let json: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["extras"]["title"], "Page Not Found");
    }
}

#[tokio::test]
async fn html_requests_redirect_to_login_with_a_destination_cookie() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "login_required", BOOL, "t").await;
    let (status, headers, body) = get(&db.pool, "/t/welcome-to-discourse/5?page=2").await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(
        header(&headers, "location"),
        Some("http://test.localhost/login")
    );
    assert_eq!(
        header(&headers, "set-cookie"),
        Some(
            "destination_url=http%3A%2F%2Ftest.localhost%2Ft%2Fwelcome-to-discourse%2F5%3Fpage%3D2; path=/; SameSite=Lax"
        )
    );
    assert!(body.is_empty());
    for path in [
        "/latest",
        "/c/general/4",
        "/tag/howto/1",
        "/tags",
        "/categories",
    ] {
        let (status, _, _) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::FOUND, "{path}");
    }

    // The front page and /login render the login page, with nothing from
    // the forum on it.
    for path in ["/", "/login"] {
        let (status, headers, html) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(
            header(&headers, "cache-control"),
            Some("no-store, must-revalidate, private, max-age=0")
        );
        assert!(html.contains("<h1>Welcome to Discourse</h1>"), "{html}");
        assert!(!html.contains("Parity fixture"), "{path}");
        assert!(!html.contains("welcome-to-discourse"), "{path}");
    }
}

#[tokio::test]
async fn status_basic_info_and_static_files_stay_open() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "login_required", BOOL, "t").await;
    for path in [
        "/srv/status",
        "/site/basic-info.json",
        "/site/basic-info",
        "/assets/site.css",
    ] {
        let (status, _, _) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
    }
    // Static trees pass through to the file server (404 here, not 302).
    let (status, _, _) = get(&db.pool, "/uploads/default/original/1X/nope.png").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn nothing_changes_when_the_setting_is_off() {
    let db = TestDb::new().await;
    let (status, headers, _) = get(&db.pool, "/latest").await;
    assert_eq!(status, StatusCode::OK);
    assert!(header(&headers, "set-cookie").is_none());
    // /login is the login page whether or not login is required.
    let (status, _, html) = get(&db.pool, "/login").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Welcome to Discourse"));
}
