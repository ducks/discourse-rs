//! Service-level tests for `services::trust_levels`.
//!
//! Production calls `evaluate` from the `CheckTrustLevelPromotionJob`
//! background job. Tests call it directly for determinism — the job is
//! a thin wrapper, exercised separately.

mod common;

use diesel::prelude::*;
use discourse_rs::guardian::{TRUST_LEVEL_BASIC, TRUST_LEVEL_NEW_USER};
use discourse_rs::schema::users;
use discourse_rs::services::trust_levels::{TL1_MIN_POSTS, evaluate};
use discourse_rs::services::user_stats;

fn current_tl(conn: &mut PgConnection, user_id: i32) -> i32 {
    users::table
        .find(user_id)
        .select(users::trust_level)
        .first(conn)
        .unwrap()
}

#[test]
fn fresh_user_with_no_posts_stays_at_tl0() {
    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());

    let outcome = evaluate(&mut ctx.conn, user.id).unwrap();
    assert!(!outcome.changed());
    assert_eq!(outcome.previous, TRUST_LEVEL_NEW_USER);
    assert_eq!(outcome.current, TRUST_LEVEL_NEW_USER);
    assert_eq!(current_tl(&mut ctx.conn, user.id), TRUST_LEVEL_NEW_USER);
}

#[test]
fn user_below_threshold_stays_at_tl0() {
    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());

    for _ in 0..(TL1_MIN_POSTS - 1) {
        user_stats::incr_post_count(&mut ctx.conn, user.id).unwrap();
    }

    let outcome = evaluate(&mut ctx.conn, user.id).unwrap();
    assert!(!outcome.changed());
    assert_eq!(current_tl(&mut ctx.conn, user.id), TRUST_LEVEL_NEW_USER);
}

#[test]
fn user_at_threshold_is_promoted_to_tl1() {
    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());

    for _ in 0..TL1_MIN_POSTS {
        user_stats::incr_post_count(&mut ctx.conn, user.id).unwrap();
    }

    let outcome = evaluate(&mut ctx.conn, user.id).unwrap();
    assert!(outcome.changed());
    assert_eq!(outcome.previous, TRUST_LEVEL_NEW_USER);
    assert_eq!(outcome.current, TRUST_LEVEL_BASIC);
    assert_eq!(current_tl(&mut ctx.conn, user.id), TRUST_LEVEL_BASIC);
}

#[test]
fn evaluate_is_idempotent_after_promotion() {
    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());
    for _ in 0..TL1_MIN_POSTS {
        user_stats::incr_post_count(&mut ctx.conn, user.id).unwrap();
    }
    evaluate(&mut ctx.conn, user.id).unwrap();

    // Second call shouldn't change anything.
    let outcome = evaluate(&mut ctx.conn, user.id).unwrap();
    assert!(!outcome.changed());
    assert_eq!(outcome.previous, TRUST_LEVEL_BASIC);
    assert_eq!(outcome.current, TRUST_LEVEL_BASIC);
}

#[test]
fn already_tl2_user_is_not_demoted_by_low_post_count() {
    // Simulate a manually-promoted user with insufficient posts.
    let mut ctx = common::setup();
    let mut opts = common::UserOpts::default();
    opts.trust_level = 2;
    let user = common::create_user(&mut ctx.conn, opts);

    let outcome = evaluate(&mut ctx.conn, user.id).unwrap();
    assert!(!outcome.changed());
    assert_eq!(current_tl(&mut ctx.conn, user.id), 2);
}

#[test]
fn evaluate_persists_across_calls() {
    // Make sure the update is committed, not just held in memory.
    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());
    for _ in 0..TL1_MIN_POSTS {
        user_stats::incr_post_count(&mut ctx.conn, user.id).unwrap();
    }
    evaluate(&mut ctx.conn, user.id).unwrap();

    // Re-read via a fresh query (same connection, but the previous update
    // should be visible).
    let tl: i32 = users::table
        .find(user.id)
        .select(users::trust_level)
        .first(&mut ctx.conn)
        .unwrap();
    assert_eq!(tl, TRUST_LEVEL_BASIC);
}

// ─────────────────────────────────────────────────────────────────────────────
// Integration: route -> counter -> evaluate flow.

#[actix_web::test]
async fn creating_enough_posts_via_route_lets_evaluate_promote() {
    use actix_web::test;

    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(user.id));

    let app = test::init_service(common::test_app_factory()).await;
    let (hk, hv) = common::auth_header_for(&user);

    for n in 1..=TL1_MIN_POSTS {
        let body = serde_json::json!({
            "topic_id": topic.id,
            "user_id": user.id,
            "post_number": n,
            "raw": format!("post {n}"),
            "reply_to_post_number": null,
        });
        let req = test::TestRequest::post()
            .uri("/api/posts")
            .insert_header((hk, hv.clone()))
            .set_json(&body)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status().as_u16(), 201, "post {n} failed");
    }

    // The route enqueues a job but the test harness has no worker. We
    // run the evaluator directly to confirm the counters reached the
    // threshold via the production code path.
    let outcome = evaluate(&mut ctx.conn, user.id).unwrap();
    assert!(outcome.changed());
    assert_eq!(outcome.current, TRUST_LEVEL_BASIC);
    drop(ctx);
}
