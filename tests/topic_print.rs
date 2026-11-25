//! A topic's print view (topics#show with print: true): the crawler
//! layout with js/print-page, and print-topic-per-hour. Its body is
//! compared with Rails' in parity/writes/topic_print.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

const INTEGER: i32 = 3;
const BROWSER: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0 Safari/537.36";

struct Reply {
    status: StatusCode,
    cookies: Vec<String>,
    body: String,
}

async fn send(st: &AppState, request: Request<Body>) -> Reply {
    let response = discourse_rs::app(st.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
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
        cookies,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

/// A browser's page load of `path`.
async fn page(st: &AppState, path: &str, cookie: Option<&str>) -> Reply {
    let mut builder = Request::get(path)
        .header(header::HOST, "test.localhost")
        .header(header::USER_AGENT, BROWSER)
        .header(header::ACCEPT, "text/html");
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

/// The print route draws every post in the crawler layout and loads the
/// script that opens the print dialog.
#[tokio::test]
async fn the_print_view_draws_the_crawler_layout() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let reply = page(&st, "/t/parity-fixture-replies-and-posters/35/print", None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert!(
        reply.body.contains("<body class=\"crawler \">"),
        "{}",
        reply.body
    );
    assert_eq!(
        reply
            .body
            .matches("class='topic-body crawler-post'")
            .count(),
        5
    );
    assert!(reply.body.contains("/print-page.js"));
    assert!(
        reply
            .body
            .contains("<title>Parity fixture: replies and posters - General - Discourse</title>")
    );
}

/// print-topic-per-hour: a member's second print in the hour is refused,
/// an admin's never.
#[tokio::test]
async fn prints_are_limited_per_hour_but_for_admins() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "max_prints_per_hour_per_user", INTEGER, "1").await;
    let mut cfg = config(RailsEnv::Test, &[]);
    cfg.rate_limits = true;
    let st = state(db.pool.clone(), cfg).await;
    let path = "/t/parity-fixture-replies-and-posters/35/print";

    let member = logged_in(&st, "user1").await;
    assert_eq!(page(&st, path, Some(&member)).await.status, StatusCode::OK);
    let reply = page(&st, path, Some(&member)).await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    assert!(reply.body.contains("too many times"), "{}", reply.body);

    let admin = logged_in(&st, "admin").await;
    for _ in 0..2 {
        assert_eq!(page(&st, path, Some(&admin)).await.status, StatusCode::OK);
    }
}
