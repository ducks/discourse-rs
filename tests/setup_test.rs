//! Sanity checks that the test framework itself works:
//! - the test DB is reachable
//! - fixture helpers create records
//! - truncation between tests actually isolates state
//!
//! Real feature tests live in sibling files (e.g. `tests/likes_test.rs`).

mod common;

use diesel::prelude::*;
use discourse_rs::schema::users;

#[test]
fn fixtures_create_a_user() {
    let mut ctx = common::setup();
    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());
    assert!(user.id > 0);
    assert!(!user.admin);
}

#[test]
fn fixtures_create_a_topic_with_user_and_category() {
    let mut ctx = common::setup();

    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let category = common::create_category(&mut ctx.conn, common::CategoryOpts::default());

    let mut topic_opts = common::TopicOpts::for_user(user.id);
    topic_opts.category_id = Some(category.id);
    let topic = common::create_topic(&mut ctx.conn, topic_opts);

    assert_eq!(topic.user_id, user.id);
    assert_eq!(topic.category_id, Some(category.id));
}

#[test]
fn fixtures_create_a_post_on_a_topic() {
    let mut ctx = common::setup();

    let user = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(user.id));
    let post = common::create_post(
        &mut ctx.conn,
        common::PostOpts::for_topic(topic.id, user.id),
    );

    assert_eq!(post.topic_id, topic.id);
    assert_eq!(post.user_id, user.id);
}

#[test]
fn truncate_isolates_tests() {
    // Two tests in a row: each should see an empty users table at start
    // even though the previous one wrote a row. This proves Drop ran (or
    // setup() truncated before this test started, which is equally fine).
    let mut ctx = common::setup();

    let count_before: i64 = users::table
        .count()
        .get_result(&mut ctx.conn)
        .expect("count failed");
    assert_eq!(
        count_before, 0,
        "users table should be empty at the start of a test"
    );

    common::create_user(&mut ctx.conn, common::UserOpts::default());

    let count_after: i64 = users::table
        .count()
        .get_result(&mut ctx.conn)
        .expect("count failed");
    assert_eq!(count_after, 1);
}
