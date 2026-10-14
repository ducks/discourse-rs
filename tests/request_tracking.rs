//! Middleware::RequestTracker's logging, which no recording can show (Rails
//! development does not load the tracker): application_requests and
//! web_crawler_requests counts, the beacons, topic views from browser page
//! views, and the events queued while Postgres is read only. Each case
//! follows spec/lib/middleware/request_tracker_spec.rb and the models'
//! specs.

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use discourse_rs::request_tracking;
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;

const BROWSER: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const PHONE: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";
const GOOGLEBOT: &str = "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)";

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: String,
}

async fn send(
    st: &AppState,
    method: Method,
    path: &str,
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
    let mut request = request.body(Body::from(body.to_string())).unwrap();
    let peer: std::net::SocketAddr = "203.0.113.7:443".parse().unwrap();
    request.extensions_mut().insert(ConnectInfo(peer));
    let response = discourse_rs::app(st.clone())
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

async fn tracking(env: RailsEnv) -> (TestDb, AppState) {
    let db = TestDb::new().await;
    let mut cfg = config(env, &[]);
    cfg.track_requests = env != RailsEnv::Development;
    let st = state(db.pool.clone(), cfg).await;
    (db, st)
}

/// The deferred work done and the counts written, as the minute's flush
/// would.
async fn settle(pool: &PgPool) {
    request_tracking::drain().await;
    request_tracking::flush(pool).await.unwrap();
}

async fn counts(pool: &PgPool) -> Vec<(String, i32)> {
    let rows: Vec<(i32, i32)> =
        sqlx::query_as("SELECT req_type, count FROM application_requests ORDER BY req_type")
            .fetch_all(pool)
            .await
            .unwrap();
    rows.into_iter()
        .map(|(t, c)| {
            let name = discourse_rs::reports::REQ_TYPES
                .iter()
                .find(|(_, id)| *id == t)
                .unwrap()
                .0;
            (name.to_string(), c)
        })
        .collect()
}

fn named(pairs: &[(&str, i32)]) -> Vec<(String, i32)> {
    pairs.iter().map(|(n, c)| (n.to_string(), *c)).collect()
}

async fn sign_in(st: &AppState) -> String {
    let csrf = send(st, Method::GET, "/session/csrf.json", &[], "").await;
    let token: serde_json::Value = serde_json::from_str(&csrf.body).unwrap();
    let session_cookie = csrf
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string())
        .collect::<Vec<_>>()
        .join("; ");
    let login = send(
        st,
        Method::POST,
        "/session",
        &[
            ("x-csrf-token", token["csrf"].as_str().unwrap()),
            ("x-requested-with", "XMLHttpRequest"),
            ("cookie", &session_cookie),
            ("content-type", "application/x-www-form-urlencoded"),
        ],
        "login=user1&password=password",
    )
    .await;
    assert_eq!(login.status, StatusCode::OK, "{}", login.body);
    login
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap())
        .find(|c| c.starts_with("_t="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

/// Clears what earlier requests (signing in) logged.
async fn reset(pool: &PgPool) {
    settle(pool).await;
    for table in [
        "application_requests",
        "web_crawler_requests",
        "topic_views",
        "topic_view_stats",
    ] {
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(pool)
            .await
            .unwrap();
    }
    sqlx::query("DELETE FROM discourse_rs.expiring_keys WHERE key LIKE 'view:%'")
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn development_logs_nothing() {
    let (db, st) = tracking(RailsEnv::Development).await;
    let reply = send(&st, Method::GET, "/", &[("user-agent", BROWSER)], "").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(!reply.headers.contains_key("x-discourse-trackview"));
    assert!(!reply.headers.contains_key("x-runtime"));
    settle(&db.pool).await;
    assert_eq!(counts(&db.pool).await, vec![]);
}

#[tokio::test]
async fn an_html_page_is_an_anonymous_page_view() {
    let (db, st) = tracking(RailsEnv::Test).await;
    let reply = send(&st, Method::GET, "/", &[("user-agent", BROWSER)], "").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.headers["x-discourse-trackview"], "1");
    assert!(!reply.headers.contains_key("x-discourse-browserpageview"));
    assert!(reply.headers.contains_key("x-runtime"));
    // A phone's adds the mobile count; an XHR or a JSON answer is not a
    // page view.
    send(&st, Method::GET, "/", &[("user-agent", PHONE)], "").await;
    send(
        &st,
        Method::GET,
        "/",
        &[
            ("user-agent", BROWSER),
            ("x-requested-with", "XMLHttpRequest"),
        ],
        "",
    )
    .await;
    send(
        &st,
        Method::GET,
        "/latest.json",
        &[("user-agent", BROWSER)],
        "",
    )
    .await;
    send(
        &st,
        Method::GET,
        "/no-such-page",
        &[("user-agent", BROWSER)],
        "",
    )
    .await;
    settle(&db.pool).await;
    assert_eq!(
        counts(&db.pool).await,
        named(&[
            ("http_total", 5),
            ("http_2xx", 4),
            ("http_4xx", 1),
            ("page_view_anon", 2),
            ("page_view_anon_mobile", 1),
        ])
    );
}

#[tokio::test]
async fn crawlers_are_counted_by_user_agent() {
    let (db, st) = tracking(RailsEnv::Test).await;
    send(&st, Method::GET, "/", &[("user-agent", GOOGLEBOT)], "").await;
    send(&st, Method::GET, "/", &[("user-agent", GOOGLEBOT)], "").await;
    // An app calling itself "Discourse" without "mobile" is one too.
    send(
        &st,
        Method::GET,
        "/",
        &[("user-agent", "Discourse Forum Onebox v3")],
        "",
    )
    .await;
    settle(&db.pool).await;
    assert_eq!(
        counts(&db.pool).await,
        named(&[("http_total", 3), ("http_2xx", 3), ("page_view_crawler", 3)])
    );
    let agents: Vec<(String, i32)> = sqlx::query_as(
        "SELECT user_agent, count FROM web_crawler_requests WHERE date = CURRENT_DATE ORDER BY user_agent",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        agents,
        vec![
            ("Discourse Forum Onebox v3".to_string(), 1),
            (GOOGLEBOT.to_string(), 2)
        ]
    );
}

#[tokio::test]
async fn background_requests_and_the_pageview_ping() {
    let (db, st) = tracking(RailsEnv::Test).await;
    let reply = send(
        &st,
        Method::POST,
        "/pageview",
        &[("user-agent", BROWSER)],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    send(
        &st,
        Method::POST,
        "/topics/timings",
        &[("user-agent", BROWSER)],
        "",
    )
    .await;
    settle(&db.pool).await;
    assert_eq!(
        counts(&db.pool).await,
        named(&[("http_total", 2), ("http_2xx", 1), ("http_background", 1)])
    );
}

#[tokio::test]
async fn an_explicit_browser_page_view_records_the_topic_view_once() {
    let (db, st) = tracking(RailsEnv::Test).await;
    let headers = [
        ("user-agent", BROWSER),
        ("discourse-track-view", "1"),
        ("discourse-track-view-topic-id", "35"),
    ];
    let reply = send(&st, Method::GET, "/t/35.json", &headers, "").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.headers["x-discourse-trackview"], "1");
    assert_eq!(reply.headers["x-discourse-browserpageview"], "1");
    settle(&db.pool).await;
    // Twice from the same address: one view.
    send(&st, Method::GET, "/t/35.json", &headers, "").await;
    settle(&db.pool).await;
    assert_eq!(
        counts(&db.pool).await,
        named(&[
            ("http_total", 2),
            ("http_2xx", 2),
            ("page_view_anon", 2),
            ("page_view_anon_browser", 2),
        ])
    );
    let views: Vec<(Option<String>, Option<i32>)> =
        sqlx::query_as("SELECT host(ip_address), user_id FROM topic_views WHERE topic_id = 35")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(views, vec![(Some("203.0.113.7".to_string()), None)]);
    let topic_views: i32 = sqlx::query_scalar("SELECT views FROM topics WHERE id = 35")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(topic_views, 1);
    let stats: (i32, i32) = sqlx::query_as(
        "SELECT anonymous_views, logged_in_views FROM topic_view_stats WHERE topic_id = 35",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(stats, (1, 0));

    // A topic the viewer cannot see is not viewed.
    let hidden = [
        ("user-agent", BROWSER),
        ("discourse-track-view", "1"),
        ("discourse-track-view-topic-id", "2"),
    ];
    send(&st, Method::GET, "/latest.json", &hidden, "").await;
    settle(&db.pool).await;
    let none: i64 = sqlx::query_scalar("SELECT count(*) FROM topic_views WHERE topic_id = 2")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(none, 0);
}

#[tokio::test]
async fn a_signed_in_view_counts_for_the_user() {
    let (db, st) = tracking(RailsEnv::Test).await;
    let auth = sign_in(&st).await;
    reset(&db.pool).await;
    let entered: i32 =
        sqlx::query_scalar("SELECT topics_entered FROM user_stats WHERE user_id = 3")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let headers = [
        ("user-agent", PHONE),
        ("cookie", auth.as_str()),
        ("discourse-track-view", "1"),
        ("discourse-track-view-topic-id", "35"),
    ];
    send(&st, Method::GET, "/t/35.json", &headers, "").await;
    settle(&db.pool).await;
    assert_eq!(
        counts(&db.pool).await,
        named(&[
            ("http_total", 1),
            ("http_2xx", 1),
            ("page_view_logged_in", 1),
            ("page_view_logged_in_mobile", 1),
            ("page_view_logged_in_browser", 1),
            ("page_view_logged_in_browser_mobile", 1),
        ])
    );
    let views: Vec<(Option<String>, Option<i32>)> =
        sqlx::query_as("SELECT host(ip_address), user_id FROM topic_views WHERE topic_id = 35")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(views, vec![(None, Some(3))]);
    let after: i32 = sqlx::query_scalar("SELECT topics_entered FROM user_stats WHERE user_id = 3")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(after, entered + 1);
    let stats: (i32, i32) = sqlx::query_as(
        "SELECT anonymous_views, logged_in_views FROM topic_view_stats WHERE topic_id = 35",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(stats, (0, 1));
}

#[tokio::test]
async fn login_required_leaves_anonymous_views_uncounted() {
    let (db, st) = tracking(RailsEnv::Test).await;
    common::set_setting(&db.pool, "login_required", 5, "t").await;
    send(
        &st,
        Method::GET,
        "/latest.json",
        &[("user-agent", BROWSER), ("discourse-track-view", "1")],
        "",
    )
    .await;
    settle(&db.pool).await;
    let names: Vec<String> = counts(&db.pool).await.into_iter().map(|(n, _)| n).collect();
    assert!(
        !names.iter().any(|n| n.starts_with("page_view")),
        "{names:?}"
    );
}

const BEACON: &str = r#"{"url":"http://test.localhost/t/a-topic/35?utm_source=x","referrer":"https://www.Google.com/search?q=1","session_id":"abc123","topic_id":35,"language":"en_us"}"#;

#[tokio::test]
async fn a_same_origin_beacon_records_the_event() {
    let (db, st) = tracking(RailsEnv::Test).await;
    let reply = send(
        &st,
        Method::POST,
        "/srv/pv",
        &[
            ("user-agent", BROWSER),
            ("origin", "http://test.localhost"),
            ("content-type", "application/json"),
        ],
        BEACON,
    )
    .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert!(!reply.headers.contains_key("content-type"));
    assert_eq!(reply.headers["x-discourse-browserpageview"], "1");
    settle(&db.pool).await;
    assert_eq!(
        counts(&db.pool).await,
        named(&[
            ("http_total", 1),
            ("http_2xx", 1),
            ("page_view_anon_browser_beacon", 1),
        ])
    );
    let event: serde_json::Value = sqlx::query_scalar(
        "SELECT row_to_json(e) FROM (SELECT url, normalized_url, normalized_url_version, \
                host(ip_address) AS ip, referrer, normalized_referrer, normalized_referrer_version, \
                user_agent, session_id, topic_id, user_id, language, normalized_language, browser, \
                country_code, asn FROM browser_pageview_events) e",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        event,
        serde_json::json!({
            "url": "http://test.localhost/t/a-topic/35?utm_source=x",
            "normalized_url": "/t/a-topic/35",
            "normalized_url_version": 1,
            "ip": "203.0.113.7",
            "referrer": "https://www.Google.com/search?q=1",
            "normalized_referrer": "google.com/search?q=1",
            "normalized_referrer_version": 1,
            "user_agent": BROWSER,
            "session_id": "abc123",
            "topic_id": 35,
            "user_id": null,
            "language": "en_us",
            "normalized_language": "en",
            "browser": 1,
            "country_code": null,
            "asn": null
        })
    );
    // The beacon's topic is viewed too.
    let views: i64 = sqlx::query_scalar("SELECT count(*) FROM topic_views WHERE topic_id = 35")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(views, 1);
}

#[tokio::test]
async fn a_cross_origin_beacon_is_refused_and_not_logged() {
    let (db, st) = tracking(RailsEnv::Test).await;
    for origin in [Some("http://evil.example"), None] {
        let mut headers = vec![
            ("user-agent", BROWSER),
            ("content-type", "application/json"),
        ];
        if let Some(o) = origin {
            headers.push(("origin", o));
        }
        let reply = send(&st, Method::POST, "/srv/pv", &headers, BEACON).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
    }
    settle(&db.pool).await;
    assert_eq!(counts(&db.pool).await, vec![]);
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM browser_pageview_events")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(events, 0);
}

#[tokio::test]
async fn a_beacon_without_a_session_is_counted_but_not_recorded() {
    let (db, st) = tracking(RailsEnv::Test).await;
    let reply = send(
        &st,
        Method::POST,
        "/srv/pv",
        &[
            ("user-agent", PHONE),
            ("referer", "http://test.localhost/latest"),
            ("content-type", "application/json"),
        ],
        r#"{"url":"http://test.localhost/latest"}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    settle(&db.pool).await;
    assert_eq!(
        counts(&db.pool).await,
        named(&[
            ("http_total", 1),
            ("http_2xx", 1),
            ("page_view_anon_browser_beacon", 1),
            ("page_view_anon_browser_mobile_beacon", 1),
        ])
    );
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM browser_pageview_events")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(events, 0);
}

#[tokio::test]
async fn an_embedded_beacon_counts_as_an_embed() {
    let (db, st) = tracking(RailsEnv::Test).await;
    send(
        &st,
        Method::POST,
        "/srv/pv",
        &[
            ("user-agent", BROWSER),
            ("origin", "http://test.localhost"),
            ("content-type", "application/json"),
        ],
        r#"{"url":"http://test.localhost/t/a-topic/35","session_id":"s1","topic_id":35,"embed":true}"#,
    )
    .await;
    settle(&db.pool).await;
    assert_eq!(
        counts(&db.pool).await,
        named(&[("http_total", 1), ("http_2xx", 1), ("page_view_embed", 1)])
    );
    // An embed's topic is not viewed; its event is still kept.
    let views: i64 = sqlx::query_scalar("SELECT count(*) FROM topic_views")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(views, 0);
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM browser_pageview_events")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(events, 1);
}

#[tokio::test]
async fn engagement_keeps_each_count_at_its_highest() {
    let (db, st) = tracking(RailsEnv::Test).await;
    let post = |body: &'static str, origin: &'static str| {
        let st = st.clone();
        async move {
            send(
                &st,
                Method::POST,
                "/srv/se",
                &[
                    ("user-agent", BROWSER),
                    ("origin", origin),
                    ("content-type", "application/json"),
                ],
                body,
            )
            .await
        }
    };
    let reply = post(
        r#"{"session_id":"s1","click_events":3,"scroll_events":"7","engaged_seconds":40}"#,
        "http://test.localhost",
    )
    .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    post(
        r#"{"session_id":"s1","click_events":1,"key_events":2,"engaged_seconds":30,"time_to_first_interaction_ms":900}"#,
        "http://test.localhost",
    )
    .await;
    let refused = post(
        r#"{"session_id":"s2","click_events":5}"#,
        "http://evil.example",
    )
    .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    settle(&db.pool).await;
    let rows: Vec<(String, i32, i32, i32, i32, Option<i32>)> = sqlx::query_as(
        "SELECT session_id, click_events, key_events, scroll_events, engaged_seconds, \
                time_to_first_interaction_ms FROM browser_pageview_session_engagements",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(rows, vec![("s1".to_string(), 3, 2, 7, 40, Some(900))]);
    // Engagement requests are not logged.
    assert_eq!(counts(&db.pool).await, vec![]);
}

#[tokio::test]
async fn a_client_signed_out_behind_its_back_is_told() {
    let (_db, st) = tracking(RailsEnv::Test).await;
    let reply = send(
        &st,
        Method::GET,
        "/latest.json",
        &[("user-agent", BROWSER), ("discourse-logged-in", "true")],
        "",
    )
    .await;
    assert_eq!(reply.headers["discourse-logged-out"], "1");
    let reply = send(
        &st,
        Method::GET,
        "/latest.json",
        &[("user-agent", BROWSER)],
        "",
    )
    .await;
    assert!(!reply.headers.contains_key("discourse-logged-out"));
}

#[tokio::test]
async fn read_only_postgres_queues_events_for_the_flush_job() {
    let (db, st) = tracking(RailsEnv::Test).await;
    {
        let mut conn = db.pool.acquire().await.unwrap();
        discourse_rs::readonly::set_key(&mut conn, discourse_rs::readonly::PG_READONLY_MODE_KEY)
            .await
            .unwrap();
    }
    let reply = send(
        &st,
        Method::POST,
        "/srv/pv",
        &[
            ("user-agent", BROWSER),
            ("origin", "http://test.localhost"),
            ("content-type", "application/json"),
        ],
        r#"{"url":"http://test.localhost/latest","session_id":"s1","language":"zh-hant-tw"}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    settle(&db.pool).await;
    // Nothing counted or written while read only; the event waits.
    assert_eq!(counts(&db.pool).await, vec![]);
    assert_eq!(request_tracking::queued_count(), 1);
    // The job does nothing until Postgres is writable again.
    discourse_rs::jobs::run_scheduled(
        &st,
        discourse_rs::jobs::rollups::FLUSH_BROWSER_PAGEVIEW_EVENTS,
    )
    .await
    .unwrap();
    assert_eq!(request_tracking::queued_count(), 1);
    sqlx::query("DELETE FROM discourse_rs.expiring_keys")
        .execute(&db.pool)
        .await
        .unwrap();
    discourse_rs::jobs::run_scheduled(
        &st,
        discourse_rs::jobs::rollups::FLUSH_BROWSER_PAGEVIEW_EVENTS,
    )
    .await
    .unwrap();
    assert_eq!(request_tracking::queued_count(), 0);
    let event: (String, String, Option<String>) = sqlx::query_as(
        "SELECT normalized_url, session_id, normalized_language FROM browser_pageview_events",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        event,
        (
            "/latest".to_string(),
            "s1".to_string(),
            Some("zh-Hant".to_string())
        )
    );
}

#[tokio::test]
async fn admin_api_requests_count_as_api() {
    let (db, st) = tracking(RailsEnv::Test).await;
    use sha2::{Digest, Sha256};
    sqlx::query(
        "INSERT INTO api_keys (id, user_id, created_by_id, created_at, updated_at, key_hash, truncated_key, scope_mode) \
         VALUES (9601, 1, 1, now(), now(), $1, 'abcd', 0)",
    )
    .bind(format!("{:x}", Sha256::digest(b"tracker-key")))
    .execute(&db.pool)
    .await
    .unwrap();
    let reply = send(
        &st,
        Method::GET,
        "/latest.json",
        &[
            ("user-agent", BROWSER),
            ("api-key", "tracker-key"),
            ("api-username", "admin"),
        ],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    settle(&db.pool).await;
    assert_eq!(
        counts(&db.pool).await,
        named(&[("http_total", 1), ("http_2xx", 1), ("api", 1)])
    );
}
