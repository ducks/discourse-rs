//! The not-found page for browsers: a 404 the handlers answer as JSON
//! becomes the page for a request that does not show JSON errors. Its
//! fragment (topics#show's extras.html) is covered by the write cases.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;

async fn get(pool: &PgPool, path: &str, xhr: bool) -> (StatusCode, String, String) {
    let app = discourse_rs::app(state(pool.clone(), config(RailsEnv::Test, &[])).await);
    let mut request = Request::get(path).header(header::HOST, "test.localhost");
    if xhr {
        request = request.header("x-requested-with", "XMLHttpRequest");
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        content_type,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

#[tokio::test]
async fn browsers_get_the_not_found_page() {
    let db = TestDb::new().await;
    let (status, content_type, body) = get(&db.pool, "/t/nope-nope", false).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(content_type.starts_with("text/html"), "{content_type}");
    assert!(body.contains("<title>Page Not Found - "), "{body}");
    assert!(body.contains(r#"<div id="main-outlet" class="wrap not-found-container">"#));
    assert!(body.contains("Oops! That page doesn’t exist or is private."));
    assert!(body.contains(r#"name="q" value="nope nope""#));
    assert!(body.contains(r#"<h2 class="recent-topics-title">Recent topics</h2>"#));

    // A missing topic of a slug and id searches for the slug.
    let (_, _, body) = get(&db.pool, "/t/made-up-topic/999999", false).await;
    assert!(body.contains(r#"value="made up topic""#));

    // Other 404s get the page too, searching for nothing.
    let (status, content_type, body) = get(&db.pool, "/u/nobody-at-all", false).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(content_type.starts_with("text/html"), "{content_type}");
    assert!(body.contains(r#"name="q" value="""#));
}

#[tokio::test]
async fn json_and_xhr_requests_keep_the_json_error() {
    let db = TestDb::new().await;
    for (path, xhr) in [("/t/nope-nope.json", false), ("/t/nope-nope", true)] {
        let (status, content_type, body) = get(&db.pool, path, xhr).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert!(content_type.starts_with("application/json"), "{path}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["error_type"], "not_found");
        assert!(
            json["extras"]["html"]
                .as_str()
                .unwrap()
                .contains(r#"value="nope nope""#)
        );
    }
}

#[tokio::test]
async fn login_only_pages_are_not_found_for_anonymous_browsers() {
    // Rails rescues NotLoggedIn on a page request as not found; JSON and
    // XHR requests keep the 403.
    let db = TestDb::new().await;
    for path in ["/read", "/new", "/drafts", "/notifications", "/review"] {
        let (status, content_type, body) = get(&db.pool, path, false).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert!(
            content_type.starts_with("text/html"),
            "{path}: {content_type}"
        );
        assert!(body.contains("<title>Page Not Found - "), "{path}");

        let (status, _, body) = get(&db.pool, path, true).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["error_type"], "not_logged_in", "{path}");
    }
}
