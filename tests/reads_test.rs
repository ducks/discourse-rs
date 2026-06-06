//! Tests for `services::reads` and the `POST /api/read/topic` route.

mod common;

use diesel::prelude::*;
use discourse_rs::schema::topic_views;
use discourse_rs::services::reads::{MAX_SECONDS_PER_CALL, ReadError, ReadOutcome, record_topic_view};
use discourse_rs::services::user_stats;

// ─────────────────────────────────────────────────────────────────────────────
// Service-level

#[test]
fn first_view_records_row_and_bumps_topics_entered() {
    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(user.id));

    let outcome = record_topic_view(&mut ctx.conn, user.id, topic.id, 10).unwrap();
    assert_eq!(outcome, ReadOutcome::NewView);

    let row_count: i64 = topic_views::table
        .filter(topic_views::user_id.eq(user.id))
        .count()
        .get_result(&mut ctx.conn)
        .unwrap();
    assert_eq!(row_count, 1);

    let stats = user_stats::get(&mut ctx.conn, user.id).unwrap();
    assert_eq!(stats.topics_entered, 1);
    assert_eq!(stats.time_read, 10);
}

#[test]
fn revisit_updates_timestamp_but_does_not_double_count() {
    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(user.id));

    let first = record_topic_view(&mut ctx.conn, user.id, topic.id, 10).unwrap();
    let second = record_topic_view(&mut ctx.conn, user.id, topic.id, 5).unwrap();
    assert_eq!(first, ReadOutcome::NewView);
    assert_eq!(second, ReadOutcome::Revisit);

    let row_count: i64 = topic_views::table
        .filter(topic_views::user_id.eq(user.id))
        .count()
        .get_result(&mut ctx.conn)
        .unwrap();
    assert_eq!(row_count, 1, "expected exactly one row, not a second insert");

    let stats = user_stats::get(&mut ctx.conn, user.id).unwrap();
    assert_eq!(
        stats.topics_entered, 1,
        "topics_entered must not double-count revisits"
    );
    assert_eq!(stats.time_read, 15, "time_read accumulates across calls");
}

#[test]
fn seconds_are_capped_per_call() {
    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(user.id));

    record_topic_view(&mut ctx.conn, user.id, topic.id, 9999).unwrap();

    let stats = user_stats::get(&mut ctx.conn, user.id).unwrap();
    assert_eq!(stats.time_read, MAX_SECONDS_PER_CALL);
}

#[test]
fn negative_seconds_are_clamped_to_zero() {
    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(user.id));

    record_topic_view(&mut ctx.conn, user.id, topic.id, -100).unwrap();

    let stats = user_stats::get(&mut ctx.conn, user.id).unwrap();
    assert_eq!(stats.time_read, 0);
}

#[test]
fn missing_topic_returns_topic_not_found() {
    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());

    let result = record_topic_view(&mut ctx.conn, user.id, 999_999, 10);
    assert!(matches!(result, Err(ReadError::TopicNotFound)));

    // No side effects.
    let stats = user_stats::get(&mut ctx.conn, user.id).unwrap();
    assert_eq!(stats.topics_entered, 0);
    assert_eq!(stats.time_read, 0);
}

#[test]
fn separate_users_have_independent_views() {
    let mut ctx = common::setup();
    let alice = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let bob = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(alice.id));

    record_topic_view(&mut ctx.conn, alice.id, topic.id, 5).unwrap();
    let bob_outcome = record_topic_view(&mut ctx.conn, bob.id, topic.id, 5).unwrap();

    assert_eq!(bob_outcome, ReadOutcome::NewView);
    assert_eq!(
        user_stats::get(&mut ctx.conn, alice.id).unwrap().topics_entered,
        1
    );
    assert_eq!(
        user_stats::get(&mut ctx.conn, bob.id).unwrap().topics_entered,
        1
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Route-level

#[actix_web::test]
async fn post_read_topic_returns_204() {
    use actix_web::test;

    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(user.id));

    let app = test::init_service(common::test_app_factory()).await;
    let (hk, hv) = common::auth_header_for(&user);
    let body = serde_json::json!({ "topic_id": topic.id, "seconds": 30 });
    let req = test::TestRequest::post()
        .uri("/api/read/topic")
        .insert_header((hk, hv))
        .set_json(&body)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 204);

    let stats = user_stats::get(&mut ctx.conn, user.id).unwrap();
    assert_eq!(stats.topics_entered, 1);
    assert_eq!(stats.time_read, 30);
    drop(ctx);
}

#[actix_web::test]
async fn post_read_topic_without_auth_is_401() {
    use actix_web::test;

    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(user.id));

    let app = test::init_service(common::test_app_factory()).await;
    let body = serde_json::json!({ "topic_id": topic.id, "seconds": 5 });
    let req = test::TestRequest::post()
        .uri("/api/read/topic")
        .set_json(&body)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 401);
    drop(ctx);
}

#[actix_web::test]
async fn post_read_topic_missing_topic_is_404() {
    use actix_web::test;

    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());

    let app = test::init_service(common::test_app_factory()).await;
    let (hk, hv) = common::auth_header_for(&user);
    let body = serde_json::json!({ "topic_id": 999999, "seconds": 5 });
    let req = test::TestRequest::post()
        .uri("/api/read/topic")
        .insert_header((hk, hv))
        .set_json(&body)
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 404);
    drop(ctx);
}
