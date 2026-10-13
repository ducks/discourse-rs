//! RateLimiter's Redis semantics on discourse_rs.rate_limits, and how a
//! refusal is worded and rendered: what no recording can show (Rails'
//! recorder disables the limiter).

mod common;

use chrono::{TimeZone, Utc};
use common::{TestDb, recorded_config, state};
use discourse_rs::clock;
use discourse_rs::posting::limits::Performed;
use discourse_rs::rate_limiter::{LimitExceeded, LimitUser, RateLimiter, time_left};

const T0: i64 = 1_800_000_000;

fn at(secs: i64) {
    clock::pin(Some(Utc.timestamp_opt(T0 + secs, 0).unwrap()));
}

fn member() -> Option<LimitUser> {
    Some(LimitUser {
        id: 3,
        staff: false,
    })
}

#[tokio::test]
async fn a_window_allows_max_then_slides() {
    let db = TestDb::new().await;
    let _app = state(db.pool.clone(), recorded_config()).await;
    let limiter = RateLimiter::new(member(), "create_post", 2, 60);
    assert_eq!(limiter.key(), "l-rate-limit3::3:create_post");
    at(0);
    assert!(limiter.performed(&db.pool, true).await.unwrap().is_ok());
    at(10);
    assert!(limiter.performed(&db.pool, true).await.unwrap().is_ok());
    at(20);
    let refused = limiter
        .performed(&db.pool, true)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.available_in, 40, "60 less the oldest's 20 seconds");
    assert!(!limiter.can_perform(&db.pool, true).await.unwrap());
    assert_eq!(limiter.remaining(&db.pool).await.unwrap(), 0);
    // The oldest stamp a window old: allowed again.
    at(60);
    assert!(limiter.performed(&db.pool, true).await.unwrap().is_ok());
    clock::pin(None);
}

#[tokio::test]
async fn an_aggressive_limiter_counts_refusals() {
    let db = TestDb::new().await;
    let _app = state(db.pool.clone(), recorded_config()).await;
    let limiter = RateLimiter::new(None, "ip-10-secs", 1, 10).aggressive();
    at(0);
    assert!(limiter.performed(&db.pool, true).await.unwrap().is_ok());
    at(5);
    // Pushed even when refused, the wait read after the push.
    assert_eq!(
        limiter
            .performed(&db.pool, true)
            .await
            .unwrap()
            .unwrap_err()
            .available_in,
        10
    );
    at(12);
    assert_eq!(
        limiter
            .performed(&db.pool, true)
            .await
            .unwrap()
            .unwrap_err()
            .available_in,
        10,
        "the refused attempt at 5 keeps it refused at 12"
    );
    clock::pin(None);
}

#[tokio::test]
async fn rollback_pops_the_newest_stamp() {
    let db = TestDb::new().await;
    let _app = state(db.pool.clone(), recorded_config()).await;
    let limiter = RateLimiter::new(member(), "create_topic", 1, 60);
    at(0);
    assert!(limiter.performed(&db.pool, true).await.unwrap().is_ok());
    limiter.rollback(&db.pool, true).await.unwrap();
    at(1);
    assert!(limiter.performed(&db.pool, true).await.unwrap().is_ok());
    clock::pin(None);
}

#[tokio::test]
async fn max_zero_refuses_with_a_negative_wait() {
    let db = TestDb::new().await;
    let _app = state(db.pool.clone(), recorded_config()).await;
    let limiter = RateLimiter::new(member(), "create_like", 0, 86_400);
    at(0);
    let refused = limiter
        .performed(&db.pool, true)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(
        refused.available_in,
        86_400 - (T0),
        "secs less now, the empty list's oldest 0"
    );
    clock::pin(None);
}

#[tokio::test]
async fn staff_and_a_disabled_limiter_are_unlimited() {
    let db = TestDb::new().await;
    let _app = state(db.pool.clone(), recorded_config()).await;
    let staff = Some(LimitUser { id: 1, staff: true });
    at(0);
    let limiter = RateLimiter::new(staff, "create_post", 1, 60);
    for _ in 0..3 {
        assert!(limiter.performed(&db.pool, true).await.unwrap().is_ok());
    }
    let limited = RateLimiter::new(staff, "create_post", 1, 60).apply_limit_to_staff();
    assert!(limited.performed(&db.pool, true).await.unwrap().is_ok());
    assert!(limited.performed(&db.pool, true).await.unwrap().is_err());
    let member_limiter = RateLimiter::new(member(), "create_post", 1, 60);
    for _ in 0..3 {
        assert!(
            member_limiter
                .performed(&db.pool, false)
                .await
                .unwrap()
                .is_ok()
        );
    }
    clock::pin(None);
}

#[tokio::test]
async fn a_list_expires_after_twice_the_window() {
    let db = TestDb::new().await;
    let _app = state(db.pool.clone(), recorded_config()).await;
    let limiter = RateLimiter::new(member(), "edit_post", 1, 60);
    at(0);
    assert!(limiter.performed(&db.pool, true).await.unwrap().is_ok());
    at(120);
    assert_eq!(limiter.remaining(&db.pool).await.unwrap(), 1);
    discourse_rs::rate_limiter::cleanup(&db.pool).await.unwrap();
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM discourse_rs.rate_limits")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
    clock::pin(None);
}

#[tokio::test]
async fn refusals_are_worded_and_rendered_as_rails_does() {
    let db = TestDb::new().await;
    let app = state(db.pool.clone(), recorded_config()).await;
    let i18n = &app.i18n;
    assert_eq!(time_left(i18n, 3), "a few seconds");
    assert_eq!(time_left(i18n, 4), "4 seconds");
    assert_eq!(time_left(i18n, 60), "1 minute");
    assert_eq!(time_left(i18n, 599), "9 minutes");
    assert_eq!(time_left(i18n, 86_399), "23 hours");
    assert_eq!(time_left(i18n, 86_400), "24 hours");
    let typed = LimitExceeded {
        available_in: 5,
        type_: "create_post".into(),
        error_code: None,
    };
    assert_eq!(
        typed.description(i18n),
        "You\u{2019}re replying a bit too quickly. Please wait 5 seconds before trying again."
    );
    // A type without words of its own (dots in an IP included).
    let ip = LimitExceeded {
        available_in: 30,
        type_: "login-min-10.0.0.1".into(),
        error_code: Some("ip_10_secs_limit".into()),
    };
    assert_eq!(
        ip.description(i18n),
        "You\u{2019}ve performed this action too many times. Please wait 30 seconds before trying again."
    );
    let response = ip.into_response(i18n);
    assert_eq!(response.status(), 429);
    assert_eq!(response.headers()["Retry-After"], "30");
    assert_eq!(
        response.headers()["Discourse-Rate-Limit-Error-Code"],
        "ip_10_secs_limit"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["error_type"], "rate_limit");
    assert_eq!(
        body["extras"],
        serde_json::json!({"wait_seconds": 30, "time_left": "30 seconds"})
    );
}

#[tokio::test]
async fn a_refused_create_rolls_back_what_it_performed() {
    let db = TestDb::new().await;
    let _app = state(db.pool.clone(), recorded_config()).await;
    let create_topic = RateLimiter::new(member(), "create_topic", 1, 60);
    let create_post = RateLimiter::new(member(), "create_post", 1, 60);
    at(0);
    // A reply a moment ago, then a new topic: the topic's limit performs,
    // the first post's refuses, and the topic's is popped again.
    assert!(create_post.performed(&db.pool, true).await.unwrap().is_ok());
    at(3);
    let mut performed = Performed::new(&db.pool, true);
    assert!(
        performed
            .perform(vec![create_topic.clone()])
            .await
            .unwrap()
            .is_ok()
    );
    let refused = performed
        .perform(vec![create_post.clone()])
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(
        (refused.type_.as_str(), refused.available_in),
        ("create_post", 57)
    );
    performed.undo().await.unwrap();
    assert!(create_topic.can_perform(&db.pool, true).await.unwrap());
    // An error path drops it instead: popped on a spawned task.
    let mut performed = Performed::new(&db.pool, true);
    assert!(
        performed
            .perform(vec![create_topic.clone()])
            .await
            .unwrap()
            .is_ok()
    );
    assert!(!create_topic.can_perform(&db.pool, true).await.unwrap());
    drop(performed);
    let mut popped = false;
    for _ in 0..50 {
        if create_topic.can_perform(&db.pool, true).await.unwrap() {
            popped = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(popped);
    // A committed create keeps its stamps; a disabled one performs nothing.
    let mut performed = Performed::new(&db.pool, true);
    at(100);
    assert!(
        performed
            .perform(vec![create_post.clone()])
            .await
            .unwrap()
            .is_ok()
    );
    performed.commit();
    assert!(!create_post.can_perform(&db.pool, true).await.unwrap());
    let mut performed = Performed::new(&db.pool, false);
    assert!(
        performed
            .perform(vec![create_post.clone()])
            .await
            .unwrap()
            .is_ok()
    );
    clock::pin(None);
}

/// A request holding the pool's last connection can still be limited:
/// the limiter's queries run on a pool of their own, so a full main pool
/// (every request waiting on its limiter) does not starve it.
#[tokio::test]
async fn limiting_while_holding_the_last_connection() {
    let db = TestDb::new().await;
    let _app = state(db.pool.clone(), recorded_config()).await;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(2))
        .connect_with((*db.pool.connect_options()).clone())
        .await
        .unwrap();
    let mut held = pool.acquire().await.unwrap();
    sqlx::query("SELECT 1").execute(&mut *held).await.unwrap();
    let limiter = RateLimiter::new(member(), "search", 1, 60);
    assert!(limiter.performed(&pool, true).await.unwrap().is_ok());
    assert!(limiter.performed(&pool, true).await.unwrap().is_err());
    drop(held);
}
