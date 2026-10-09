//! Middleware::AnonymousCache: anonymous GETs of the actions that opt in
//! are stored from the threshold'th ask and then served from memory.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn send(
    st: &AppState,
    method: Method,
    path: &str,
    cookie: Option<&str>,
) -> (StatusCode, Option<String>, String) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "test.localhost");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    let response = discourse_rs::app(st.clone())
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let cached = response
        .headers()
        .get("x-discourse-cached")
        .map(|v| v.to_str().unwrap().to_string());
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, cached, String::from_utf8_lossy(&body).into_owned())
}

async fn get(st: &AppState, path: &str) -> (StatusCode, Option<String>, String) {
    send(st, Method::GET, path, None).await
}

#[tokio::test]
async fn stored_from_the_second_ask_and_served_from_the_third() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    for path in [
        "/latest.json",
        "/latest",
        "/t/parity-fixture-replies-and-posters/35.json",
    ] {
        let (status, first, _) = get(&st, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(first.as_deref(), Some("skip"), "{path}");
        let (_, second, stored) = get(&st, path).await;
        assert_eq!(second.as_deref(), Some("store"), "{path}");
        // The topic changes, but the cached copy is what an anonymous
        // visitor gets until it expires.
        sqlx::query("UPDATE topics SET title = 'Renamed while cached' WHERE id = 35")
            .execute(&db.pool)
            .await
            .unwrap();
        let (_, third, served) = get(&st, path).await;
        assert_eq!(third.as_deref(), Some("true"), "{path}");
        assert_eq!(served, stored, "{path}");
        sqlx::query(
            "UPDATE topics SET title = 'Parity fixture: replies and posters' WHERE id = 35",
        )
        .execute(&db.pool)
        .await
        .unwrap();
    }
    // Another format of the same path is another key.
    let (_, cached, _) = get(&st, "/latest.json?page=1").await;
    assert_eq!(cached.as_deref(), Some("skip"));
}

#[tokio::test]
async fn only_anonymous_gets_of_the_actions_that_opt_in() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    for _ in 0..3 {
        // site#site doesn't call discourse_expires_in.
        assert_eq!(get(&st, "/site.json").await.1, None);
        // A session cookie, even a stale one, or the bypass cookie.
        let (_, cached, _) = send(&st, Method::GET, "/latest.json", Some("_t=stale")).await;
        assert_eq!(cached, None);
        let (_, cached, _) =
            send(&st, Method::GET, "/latest.json", Some("_bypass_cache=true")).await;
        assert_eq!(cached, None);
        // An API key.
        assert_eq!(get(&st, "/latest.json?api_key=x").await.1, None);
        // Not a 200.
        assert_eq!(get(&st, "/t/999999.json").await.1, None);
    }
}

#[tokio::test]
async fn the_threshold_comes_from_anon_cache_store_threshold() {
    let db = TestDb::new().await;
    let st = state(
        db.pool.clone(),
        config(RailsEnv::Test, &[("anon_cache_store_threshold", "1")]),
    )
    .await;
    assert_eq!(get(&st, "/latest.json").await.1.as_deref(), Some("store"));
    assert_eq!(get(&st, "/latest.json").await.1.as_deref(), Some("true"));

    let st = state(
        db.pool.clone(),
        config(RailsEnv::Test, &[("anon_cache_store_threshold", "0")]),
    )
    .await;
    for _ in 0..3 {
        assert_eq!(get(&st, "/latest.json").await.1, None);
    }
}

#[tokio::test]
async fn off_in_development() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Development, &[])).await;
    for _ in 0..3 {
        assert_eq!(get(&st, "/latest.json").await.1, None);
    }
}

#[tokio::test]
async fn the_compressed_response_is_kept_per_encoding() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let get_br = |path: &'static str| {
        let st = st.clone();
        async move {
            let response = discourse_rs::app(st)
                .oneshot(
                    Request::get(path)
                        .header(header::HOST, "test.localhost")
                        .header(header::ACCEPT_ENCODING, "br, gzip")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let encoding = response
                .headers()
                .get(header::CONTENT_ENCODING)
                .map(|v| v.to_str().unwrap().to_string());
            let cached = response
                .headers()
                .get("x-discourse-cached")
                .map(|v| v.to_str().unwrap().to_string());
            let body = response.into_body().collect().await.unwrap().to_bytes();
            (encoding, cached, body)
        }
    };
    let (_, first, _) = get_br("/latest").await;
    assert_eq!(first.as_deref(), Some("skip"));
    let (encoding, second, stored) = get_br("/latest").await;
    assert_eq!(
        (encoding.as_deref(), second.as_deref()),
        (Some("br"), Some("store"))
    );
    let (encoding, third, served) = get_br("/latest").await;
    assert_eq!(
        (encoding.as_deref(), third.as_deref()),
        (Some("br"), Some("true"))
    );
    assert_eq!(served, stored);
    // Without Accept-Encoding the key differs: counted afresh, uncompressed.
    let (_, cached, body) = get(&st, "/latest").await;
    assert_eq!(cached.as_deref(), Some("skip"));
    assert!(body.contains("<html"));
}
