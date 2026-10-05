//! Search for anonymous users. The documents are covered by the parity
//! golden files; these pin the search log, the HTML page, and rules the
//! fixtures can't show.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

const BOOL: i32 = 5;

async fn get_with(
    app_state: &AppState,
    path: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, Vec<(String, String)>, String) {
    let app = discourse_rs::app(app_state.clone());
    let mut request = Request::get(path).header(header::HOST, "test.localhost");
    for (k, v) in headers {
        request = request.header(*k, *v);
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
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

async fn get(pool: &PgPool, path: &str) -> (StatusCode, Vec<(String, String)>, String) {
    get_with(
        &state(pool.clone(), config(RailsEnv::Test, &[])).await,
        path,
        &[],
    )
    .await
}

fn post_ids(body: &str) -> Vec<i64> {
    let json: Value = serde_json::from_str(body).unwrap();
    json["grouped_search_result"]["post_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn logs_searches_and_folds_extensions_of_the_last_term() {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (status, headers, body) = get_with(
        &app_state,
        "/search/query.json?term=fix",
        &[
            ("User-Agent", "Mozilla/5.0 test"),
            ("Discourse-Pageview-Session-Id", "abc123"),
            ("X-Forwarded-For", "10.1.2.3, 127.0.0.1"),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers
            .iter()
            .any(|(k, v)| k == "x-robots-tag" && v == "noindex")
    );
    let json: Value = serde_json::from_str(&body).unwrap();
    let id = json["grouped_search_result"]["search_log_id"]
        .as_i64()
        .unwrap();
    let row: (String, i32, String, Option<String>, Option<String>, bool) = sqlx::query_as(
        "SELECT term, search_type, host(ip_address), user_agent, session_id, crawler FROM search_logs WHERE id = $1",
    )
    .bind(id as i32)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    // "Mozilla/5.0 test" names no browser engine, so it counts as a crawler.
    assert_eq!(
        row,
        (
            "fix".into(),
            1,
            "10.1.2.3".into(),
            Some("Mozilla/5.0 test".into()),
            Some("abc123".into()),
            true
        )
    );

    // Same IP within 5 s, term extended: the row is updated, not added.
    let (_, _, body) = get_with(
        &app_state,
        "/search.json?q=fixture",
        &[("X-Forwarded-For", "10.1.2.3")],
    )
    .await;
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["grouped_search_result"]["search_log_id"], id);
    let (term, search_type): (String, i32) =
        sqlx::query_as("SELECT term, search_type FROM search_logs WHERE id = $1")
            .bind(id as i32)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!((term.as_str(), search_type), ("fixture", 1));

    // A different term, or another IP, gets its own row.
    let (_, _, body) = get_with(
        &app_state,
        "/search.json?q=parity",
        &[("X-Forwarded-For", "10.1.2.3")],
    )
    .await;
    let json: Value = serde_json::from_str(&body).unwrap();
    let second = json["grouped_search_result"]["search_log_id"]
        .as_i64()
        .unwrap();
    assert_ne!(second, id);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM search_logs")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
    let search_type: i32 = sqlx::query_scalar("SELECT search_type FROM search_logs WHERE id = $1")
        .bind(second as i32)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(search_type, 2, "full page");

    // Too-short terms are logged before validation; exclude_topics never is.
    let (_, _, body) = get_with(
        &app_state,
        "/search.json?q=ab%20cd",
        &[("X-Forwarded-For", "10.9.9.9")],
    )
    .await;
    assert_eq!(body, r#"{"grouped_search_result":null}"#);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM search_logs")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(count, 3);
    let (_, _, body) = get_with(
        &app_state,
        "/search/query.json?term=fixture&type_filter=exclude_topics",
        &[("X-Forwarded-For", "10.9.9.8")],
    )
    .await;
    let json: Value = serde_json::from_str(&body).unwrap();
    assert!(json["grouped_search_result"].get("search_log_id").is_none());
}

#[tokio::test]
async fn does_not_log_when_disabled() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "log_search_queries", BOOL, "f").await;
    let (_, _, body) = get(&db.pool, "/search.json?q=fixture").await;
    let json: Value = serde_json::from_str(&body).unwrap();
    assert!(json["grouped_search_result"].get("search_log_id").is_none());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM search_logs")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn secures_and_ranks_results() {
    let db = TestDb::new().await;
    // Topic 38 lives in Sub General; moving it to Staff hides its post.
    // Post 34 is Sub General's own About topic and matches too.
    let (_, _, body) = get(&db.pool, "/search.json?q=subcategory").await;
    assert_eq!(post_ids(&body), vec![42, 34]);
    sqlx::query("UPDATE topics SET category_id = 3 WHERE id = 38")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, _, body) = get(&db.pool, "/search.json?q=subcategory").await;
    assert_eq!(post_ids(&body), vec![34]);

    // Closed and archived topics rank below open ones with the same match.
    let (_, _, body) = get(&db.pool, "/search.json?q=fixture").await;
    let ids = post_ids(&body);
    let pos = |id: i64| ids.iter().position(|x| *x == id).unwrap();
    assert!(pos(35) < pos(40), "open before closed: {ids:?}");
    assert!(pos(40) < pos(46), "closed before archived: {ids:?}");

    // Advanced syntax is refused rather than approximated.
    for q in [
        "fixture%20in:title",
        "%23general",
        "fixture%20order:latest",
        "%40user1",
    ] {
        let (status, _, _) = get(&db.pool, &format!("/search.json?q={q}")).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{q}");
    }
}

#[tokio::test]
async fn renders_a_results_page() {
    let db = TestDb::new().await;
    let (status, headers, html) = get(&db.pool, "/search?q=fixture").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers
            .iter()
            .any(|(k, v)| k == "x-robots-tag" && v == "noindex")
    );
    assert!(
        html.contains("<title>Search results for &#x27;fixture&#x27; - Discourse</title>")
            || html.contains("<title>Search results for &#39;fixture&#x27; - Discourse</title>")
            || html.contains("<title>Search results for 'fixture' - Discourse</title>"),
        "{html}"
    );
    assert!(html.contains(r#"href="/t/parity-fixture-replies-and-posters/35""#));
    assert!(html.contains("A topic in a subcategory carrying a tag"));
    assert!(html.contains(r#"value="fixture""#));

    let (status, _, html) = get(&db.pool, "/search").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<title>Search - Discourse</title>"));
    assert!(!html.contains("No results found"));

    let (status, _, html) = get(&db.pool, "/search?q=nothingmatchesatall").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("No results found"));

    let (status, _, body) = get(&db.pool, "/search?q=ba").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("invalid_parameters"));
}
