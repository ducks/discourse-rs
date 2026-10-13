//! Middleware::RequestTracker's limits, which no recording can show (Rails
//! development does not load the tracker, and loopback is exempt): per
//! address or per signed-in user in 10 and 60 second windows, the asset
//! budget, the exemptions and the crawler slow-down.

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use tower::ServiceExt;

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: String,
}

async fn send(
    state: &AppState,
    method: Method,
    path: &str,
    from: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Reply {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "test.localhost");
    for (k, v) in headers {
        request = request.header(*k, *v);
    }
    if !body.is_empty() {
        request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    }
    let mut request = request.body(Body::from(body.to_string())).unwrap();
    let peer: std::net::SocketAddr = format!("{from}:443").parse().unwrap();
    request.extensions_mut().insert(ConnectInfo(peer));
    let response = discourse_rs::app(state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

async fn get(state: &AppState, path: &str, from: &str) -> Reply {
    send(state, Method::GET, path, from, &[], "").await
}

async fn tracked(globals: &[(&str, &str)]) -> (TestDb, AppState) {
    let db = TestDb::new().await;
    let mut cfg = config(RailsEnv::Test, globals);
    cfg.rate_limits = true;
    let app_state = state(db.pool.clone(), cfg).await;
    (db, app_state)
}

#[tokio::test]
async fn an_address_gets_ten_seconds_worth_then_a_plain_429() {
    let (_db, st) = tracked(&[("max_reqs_per_ip_per_10_seconds", "3")]).await;
    for _ in 0..3 {
        assert_eq!(
            get(&st, "/session/csrf.json", "203.0.113.9").await.status,
            StatusCode::OK
        );
    }
    let reply = get(&st, "/session/csrf.json", "203.0.113.9").await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(reply.headers["content-type"], "text/plain");
    assert_eq!(
        reply.headers["discourse-rate-limit-error-code"],
        "ip_10_secs_limit"
    );
    let wait = reply.headers["retry-after"]
        .to_str()
        .unwrap()
        .parse::<i64>()
        .unwrap();
    assert!((9..=10).contains(&wait), "{wait}");
    assert_eq!(
        reply.body,
        format!(
            "Slow down, you're making too many requests.\nPlease retry again in {wait} seconds.\nError code: ip_10_secs_limit.\n"
        )
    );
    // Another address has its own budget; loopback and private ones are
    // not limited.
    assert_eq!(
        get(&st, "/session/csrf.json", "198.51.100.7").await.status,
        StatusCode::OK
    );
    for _ in 0..5 {
        assert_eq!(
            get(&st, "/session/csrf.json", "10.0.0.5").await.status,
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn private_addresses_count_when_asked_and_exceptions_never() {
    let (_db, st) = tracked(&[
        ("max_reqs_per_ip_per_10_seconds", "1"),
        ("max_reqs_rate_limit_on_private", "true"),
        ("max_reqs_per_ip_exceptions", "203.0.113.0/24 2001:db8::1"),
    ])
    .await;
    assert_eq!(
        get(&st, "/session/csrf.json", "10.0.0.5").await.status,
        StatusCode::OK
    );
    assert_eq!(
        get(&st, "/session/csrf.json", "10.0.0.5").await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    for _ in 0..3 {
        assert_eq!(
            get(&st, "/session/csrf.json", "203.0.113.50").await.status,
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn assets_count_against_their_own_budget() {
    let (_db, st) = tracked(&[
        ("max_reqs_per_ip_per_10_seconds", "2"),
        ("max_asset_reqs_per_ip_per_10_seconds", "3"),
    ])
    .await;
    // Past the request limit, asset requests still go: they are given
    // back and counted as assets.
    for _ in 0..3 {
        let reply = get(
            &st,
            "/uploads/default/original/1X/missing.png",
            "203.0.113.9",
        )
        .await;
        assert_ne!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    }
    let reply = get(
        &st,
        "/uploads/default/original/1X/missing.png",
        "203.0.113.9",
    )
    .await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        reply.headers["discourse-rate-limit-error-code"],
        "ip_assets_10_secs_limit"
    );
    // The asset budget spent blocks the address's other requests too.
    assert_eq!(
        get(&st, "/session/csrf.json", "203.0.113.9").await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn signed_in_users_are_counted_by_user_across_addresses() {
    let (_db, st) = tracked(&[("max_reqs_per_ip_per_10_seconds", "3")]).await;
    // Log in from loopback (not counted), then use the cookie elsewhere.
    let csrf = get(&st, "/session/csrf.json", "127.0.0.1").await;
    let token: serde_json::Value = serde_json::from_str(&csrf.body).unwrap();
    let session_cookie = csrf
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string())
        .collect::<Vec<_>>()
        .join("; ");
    let login = send(
        &st,
        Method::POST,
        "/session",
        "127.0.0.1",
        &[
            ("x-csrf-token", token["csrf"].as_str().unwrap()),
            ("x-requested-with", "XMLHttpRequest"),
            ("cookie", &session_cookie),
        ],
        "login=user1&password=password",
    )
    .await;
    assert_eq!(login.status, StatusCode::OK, "{}", login.body);
    let auth = login
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap())
        .find(|c| c.starts_with("_t="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let with_cookie = [("cookie", auth.as_str())];
    for from in ["203.0.113.1", "203.0.113.2", "203.0.113.3"] {
        let reply = send(
            &st,
            Method::GET,
            "/session/current.json",
            from,
            &with_cookie,
            "",
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    }
    let reply = send(
        &st,
        Method::GET,
        "/session/current.json",
        "203.0.113.4",
        &with_cookie,
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        reply.headers["discourse-rate-limit-error-code"],
        "user_10_secs_limit"
    );
    assert_eq!(reply.headers["x-discourse-username"], "user1");
}

#[tokio::test]
async fn named_crawlers_are_slowed_down() {
    let (_db, st) = tracked(&[]).await;
    let ua = [("user-agent", "GPTBot/1.1")];
    let first = send(&st, Method::GET, "/session/csrf.json", "127.0.0.1", &ua, "").await;
    assert_eq!(first.status, StatusCode::OK);
    let second = send(&st, Method::GET, "/session/csrf.json", "127.0.0.1", &ua, "").await;
    assert_eq!(second.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        second.body,
        "Too many crawling requests. Error code: gptbot_crawler_rate_limit."
    );
    assert_eq!(
        second.headers["discourse-rate-limit-error-code"],
        "gptbot_crawler_rate_limit"
    );
    // A browser is not slowed down.
    let browser = [(
        "user-agent",
        "Mozilla/5.0 (X11; Linux x86_64) Firefox/140.0",
    )];
    for _ in 0..2 {
        let reply = send(
            &st,
            Method::GET,
            "/session/csrf.json",
            "127.0.0.1",
            &browser,
            "",
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK);
    }
}

#[tokio::test]
async fn admin_api_requests_give_back_the_address_count() {
    use sha2::{Digest, Sha256};
    let (db, st) = tracked(&[("max_reqs_per_ip_per_10_seconds", "2")]).await;
    sqlx::query(
        "INSERT INTO api_keys (id, user_id, created_by_id, created_at, updated_at, key_hash, truncated_key, scope_mode) \
         VALUES (9601, 1, 1, now(), now(), $1, 'abcd', 0)",
    )
    .bind(format!("{:x}", Sha256::digest(b"tracker-key")))
    .execute(&db.pool)
    .await
    .unwrap();
    let key = [("api-key", "tracker-key"), ("api-username", "admin")];
    for _ in 0..4 {
        let reply = send(&st, Method::GET, "/latest.json", "203.0.113.9", &key, "").await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    }
}

#[tokio::test]
async fn the_tracker_keeps_its_lists_in_the_process() {
    let (db, st) = tracked(&[("max_reqs_per_ip_per_10_seconds", "3")]).await;
    for _ in 0..2 {
        get(&st, "/session/csrf.json", "203.0.113.9").await;
    }
    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM discourse_rs.rate_limits WHERE key LIKE '%global_limit%'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(rows, 0);
}
