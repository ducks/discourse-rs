//! Service-level tests for `services::user_stats`. The harness creates a
//! stats row for every fixture user (mirrors the route behavior), so most
//! tests verify counter mutations rather than initial creation.

mod common;

use diesel::prelude::*;
use discourse_rs::schema::user_stats;
use discourse_rs::services::user_stats as svc;

#[test]
fn ensure_for_is_idempotent() {
    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());

    // create_user already called ensure_for. Calling it again should
    // return the same row, not duplicate or error.
    let first = svc::ensure_for(&mut ctx.conn, user.id).expect("ensure_for failed");
    let second = svc::ensure_for(&mut ctx.conn, user.id).expect("second ensure_for failed");

    assert_eq!(first.user_id, second.user_id);
    assert_eq!(first.post_count, second.post_count);

    let row_count: i64 = user_stats::table
        .filter(user_stats::user_id.eq(user.id))
        .count()
        .get_result(&mut ctx.conn)
        .unwrap();
    assert_eq!(row_count, 1, "expected exactly one stats row");
}

#[test]
fn fresh_user_has_zero_counters() {
    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());

    let stats = svc::get(&mut ctx.conn, user.id).expect("get failed");
    assert_eq!(stats.post_count, 0);
    assert_eq!(stats.topic_count, 0);
    assert_eq!(stats.time_read, 0);
    assert_eq!(stats.posts_read_count, 0);
    assert_eq!(stats.topics_entered, 0);
    assert_eq!(stats.days_visited, 0);
}

#[test]
fn incr_post_count_bumps_by_one() {
    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());

    svc::incr_post_count(&mut ctx.conn, user.id).expect("incr failed");
    svc::incr_post_count(&mut ctx.conn, user.id).expect("incr failed");

    let stats = svc::get(&mut ctx.conn, user.id).unwrap();
    assert_eq!(stats.post_count, 2);
    assert_eq!(stats.topic_count, 0, "topic_count should not move");
}

#[test]
fn incr_topic_count_bumps_by_one() {
    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());

    svc::incr_topic_count(&mut ctx.conn, user.id).expect("incr failed");

    let stats = svc::get(&mut ctx.conn, user.id).unwrap();
    assert_eq!(stats.topic_count, 1);
    assert_eq!(stats.post_count, 0);
}

#[test]
fn decr_post_count_floors_at_zero() {
    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());

    svc::incr_post_count(&mut ctx.conn, user.id).unwrap();
    svc::decr_post_count(&mut ctx.conn, user.id).unwrap();
    svc::decr_post_count(&mut ctx.conn, user.id).unwrap(); // second decr at zero

    let stats = svc::get(&mut ctx.conn, user.id).unwrap();
    assert_eq!(stats.post_count, 0, "counter must not go negative");
}

#[test]
fn updated_at_advances_when_counters_change() {
    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());

    let before = svc::get(&mut ctx.conn, user.id).unwrap().updated_at;
    // sleep just enough to guarantee a different timestamp; postgres
    // timestamps have microsecond resolution, but the Rust side reads
    // them at nanosecond width through diesel — a 1ms gap is safe.
    std::thread::sleep(std::time::Duration::from_millis(2));
    svc::incr_post_count(&mut ctx.conn, user.id).unwrap();
    let after = svc::get(&mut ctx.conn, user.id).unwrap().updated_at;

    assert!(after > before, "updated_at should advance on counter change");
}

// ─────────────────────────────────────────────────────────────────────────────
// Route-level: confirm POST /topics and POST /posts wire into the service.

#[actix_web::test]
async fn post_topic_route_bumps_topic_count() {
    use actix_web::test;

    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());

    let app = test::init_service(common::test_app_factory()).await;
    let (hk, hv) = common::auth_header_for(&user);
    let body = serde_json::json!({
        "title": "Hello",
        "slug": "hello-world",
        "user_id": user.id,
        "category_id": null,
    });
    let req = test::TestRequest::post()
        .uri("/api/topics")
        .insert_header((hk, hv))
        .set_json(&body)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 201);

    let stats = svc::get(&mut ctx.conn, user.id).unwrap();
    assert_eq!(stats.topic_count, 1);
    drop(ctx);
}

#[actix_web::test]
async fn post_post_route_bumps_post_count() {
    use actix_web::test;

    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(user.id));

    let app = test::init_service(common::test_app_factory()).await;
    let (hk, hv) = common::auth_header_for(&user);
    let body = serde_json::json!({
        "topic_id": topic.id,
        "user_id": user.id,
        "post_number": 1,
        "raw": "Hello world",
        "reply_to_post_number": null,
    });
    let req = test::TestRequest::post()
        .uri("/api/posts")
        .insert_header((hk, hv))
        .set_json(&body)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 201);

    let stats = svc::get(&mut ctx.conn, user.id).unwrap();
    assert_eq!(stats.post_count, 1);
    drop(ctx);
}
