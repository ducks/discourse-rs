//! Read-only mode where the write cases can't show it: the page's notice,
//! the CSRF check ahead of the refusal (the recorder turns forgery
//! protection off), the bus message, and the scheduled jobs skipped (the
//! recorder runs a job's `execute`, past Jobs::Scheduled#perform).

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, bus_messages, recorded_config, state};
use discourse_rs::readonly;
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn send(
    st: &discourse_rs::AppState,
    request: Request<Body>,
) -> (StatusCode, Option<String>, String) {
    let response = discourse_rs::app(st.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let header = response
        .headers()
        .get("discourse-readonly")
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, header, String::from_utf8_lossy(&bytes).into_owned())
}

async fn set(pool: &sqlx::PgPool, key: &str) {
    let mut conn = pool.acquire().await.unwrap();
    readonly::set_key(&mut conn, key).await.unwrap();
}

#[tokio::test]
async fn pages_show_the_notice_and_carry_the_header() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), recorded_config()).await;
    let page = || {
        Request::get("/latest")
            .header(header::HOST, "localhost")
            .body(Body::empty())
            .unwrap()
    };
    let (status, flag, body) = send(&st, page()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(flag, None);
    assert!(!body.contains("global-notice"));

    set(&db.pool, readonly::USER_READONLY_MODE_KEY).await;
    let (status, flag, body) = send(&st, page()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(flag.as_deref(), Some("true"));
    assert!(body.contains(
        "<div class=\"alert alert-info alert-read-only\" id=\"global-notice-alert-read-only\"><span class=\"text\">This site is in read only mode. Please continue to browse, but signing up and logging in are unavailable for now.</span></div>"
    ));

    // Staff-writes-only alone has its own notice; with a read-only key the
    // site is plainly read only.
    sqlx::query("DELETE FROM discourse_rs.expiring_keys")
        .execute(&db.pool)
        .await
        .unwrap();
    set(&db.pool, readonly::STAFF_WRITES_ONLY_MODE_KEY).await;
    let (_, flag, body) = send(&st, page()).await;
    assert_eq!(flag.as_deref(), Some("true"));
    assert!(body.contains("id=\"global-notice-alert-staff-writes-only\""));
    set(&db.pool, readonly::READONLY_MODE_KEY).await;
    let (_, _, body) = send(&st, page()).await;
    assert!(body.contains("id=\"global-notice-alert-read-only\""));
}

#[tokio::test]
async fn a_bad_csrf_token_is_refused_before_read_only() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), recorded_config()).await;
    set(&db.pool, readonly::USER_READONLY_MODE_KEY).await;
    let request = Request::post("/bookmarks.json")
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-requested-with", "XMLHttpRequest")
        .body(Body::from(
            r#"{"bookmarkable_id":37,"bookmarkable_type":"Post"}"#,
        ))
        .unwrap();
    let (status, _, body) = send(&st, request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body, "[\"BAD CSRF\"]");
}

#[tokio::test]
async fn toggling_tells_browsers() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), recorded_config()).await;
    let from = st.bus.now().await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    readonly::enable(&mut tx, &st.bus, readonly::USER_READONLY_MODE_KEY)
        .await
        .unwrap();
    readonly::disable(&mut tx, &st.bus, readonly::USER_READONLY_MODE_KEY)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let messages = bus_messages(&st, from, &[readonly::CHANNEL]).await;
    let data: Vec<_> = messages.iter().map(|m| m.data.clone()).collect();
    assert_eq!(
        data,
        vec![serde_json::json!(true), serde_json::json!(false)]
    );
    assert!(messages.iter().all(|m| m.audience.is_none()));
}

#[tokio::test]
async fn scheduled_jobs_wait_while_read_only() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), recorded_config()).await;
    // A topic timer due now: the enquer queues its job unless read only.
    let queued = || async {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM discourse_rs.jobs")
            .fetch_one(&db.pool)
            .await
            .unwrap()
    };
    sqlx::query(
        "INSERT INTO topic_timers (execute_at, status_type, user_id, timerable_id, based_on_last_post, \
           created_at, updated_at, type) \
         VALUES (now() - interval '1 minute', 1, 1, 35, FALSE, now(), now(), 'TopicTimer')",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    set(&db.pool, readonly::USER_READONLY_MODE_KEY).await;
    discourse_rs::jobs::run_scheduled(&st, discourse_rs::jobs::topic_timers::ENQUEUER)
        .await
        .unwrap();
    assert_eq!(queued().await, 0);
    // Staff-writes-only mode is not Discourse.readonly_mode?.
    sqlx::query("DELETE FROM discourse_rs.expiring_keys")
        .execute(&db.pool)
        .await
        .unwrap();
    set(&db.pool, readonly::STAFF_WRITES_ONLY_MODE_KEY).await;
    discourse_rs::jobs::run_scheduled(&st, discourse_rs::jobs::topic_timers::ENQUEUER)
        .await
        .unwrap();
    assert!(queued().await > 0);
}
