//! Service-layer tests for `services::likes`. Talk to the real test DB
//! through the shared `TestCtx`; no actix involved.

mod common;

use diesel::prelude::*;
use discourse_rs::schema::{notifications, post_likes, posts, users};
use discourse_rs::services::likes::{LikeError, LikeOutcome, UnlikeOutcome, like_post, unlike_post};

// ─────────────────────────────────────────────────────────────────────────────
// like_post

#[test]
fn like_post_inserts_row_and_bumps_counters() {
    let mut ctx = common::setup();

    let author = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let liker = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(author.id));
    let post = common::create_post(
        &mut ctx.conn,
        common::PostOpts::for_topic(topic.id, author.id),
    );

    let outcome = like_post(&mut ctx.conn, liker.id, post.id).expect("like failed");
    assert!(matches!(outcome, LikeOutcome::Created(_)));

    // Counters bumped
    let post_like_count: i32 = posts::table
        .find(post.id)
        .select(posts::like_count)
        .first(&mut ctx.conn)
        .unwrap();
    assert_eq!(post_like_count, 1);

    let (likes_given, likes_received): (i32, i32) = users::table
        .find(liker.id)
        .select((users::likes_given, users::likes_received))
        .first(&mut ctx.conn)
        .unwrap();
    assert_eq!(likes_given, 1);
    assert_eq!(likes_received, 0);

    let author_received: i32 = users::table
        .find(author.id)
        .select(users::likes_received)
        .first(&mut ctx.conn)
        .unwrap();
    assert_eq!(author_received, 1);
}

#[test]
fn like_post_creates_notification_for_author() {
    let mut ctx = common::setup();

    let author = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let liker = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(author.id));
    let post = common::create_post(
        &mut ctx.conn,
        common::PostOpts::for_topic(topic.id, author.id),
    );

    like_post(&mut ctx.conn, liker.id, post.id).expect("like failed");

    let (notif_user, notif_type, acting): (i32, String, Option<i32>) = notifications::table
        .select((
            notifications::user_id,
            notifications::notification_type,
            notifications::acting_user_id,
        ))
        .first(&mut ctx.conn)
        .expect("expected exactly one notification");
    assert_eq!(notif_user, author.id);
    assert_eq!(notif_type, "post_liked");
    assert_eq!(acting, Some(liker.id));
}

#[test]
fn like_post_is_idempotent() {
    let mut ctx = common::setup();

    let author = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let liker = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(author.id));
    let post = common::create_post(
        &mut ctx.conn,
        common::PostOpts::for_topic(topic.id, author.id),
    );

    let first = like_post(&mut ctx.conn, liker.id, post.id).expect("first like failed");
    let second = like_post(&mut ctx.conn, liker.id, post.id).expect("second like failed");

    assert!(matches!(first, LikeOutcome::Created(_)));
    assert!(matches!(second, LikeOutcome::AlreadyLiked(_)));

    // Same row returned both times
    assert_eq!(first.into_like().id, second.into_like().id);

    // Counters unchanged by the second call
    let post_like_count: i32 = posts::table
        .find(post.id)
        .select(posts::like_count)
        .first(&mut ctx.conn)
        .unwrap();
    assert_eq!(post_like_count, 1);

    // Only one row in post_likes
    let row_count: i64 = post_likes::table.count().get_result(&mut ctx.conn).unwrap();
    assert_eq!(row_count, 1);

    // Only one notification (no spam on re-like)
    let notif_count: i64 = notifications::table
        .count()
        .get_result(&mut ctx.conn)
        .unwrap();
    assert_eq!(notif_count, 1);
}

#[test]
fn like_post_rejects_self_like() {
    let mut ctx = common::setup();

    let author = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(author.id));
    let post = common::create_post(
        &mut ctx.conn,
        common::PostOpts::for_topic(topic.id, author.id),
    );

    let result = like_post(&mut ctx.conn, author.id, post.id);
    assert!(matches!(result, Err(LikeError::SelfLike)));

    // No side effects
    let row_count: i64 = post_likes::table.count().get_result(&mut ctx.conn).unwrap();
    assert_eq!(row_count, 0);
    let post_like_count: i32 = posts::table
        .find(post.id)
        .select(posts::like_count)
        .first(&mut ctx.conn)
        .unwrap();
    assert_eq!(post_like_count, 0);
}

#[test]
fn like_post_rejects_missing_post() {
    let mut ctx = common::setup();
    let liker = common::create_user(&mut ctx.conn, common::UserOpts::default());

    let result = like_post(&mut ctx.conn, liker.id, 999_999);
    assert!(matches!(result, Err(LikeError::PostNotFound)));
}

#[test]
fn like_post_rejects_hidden_post() {
    let mut ctx = common::setup();

    let author = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let liker = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(author.id));
    let post = common::create_post(
        &mut ctx.conn,
        common::PostOpts::for_topic(topic.id, author.id),
    );

    diesel::update(posts::table.find(post.id))
        .set(posts::hidden.eq(true))
        .execute(&mut ctx.conn)
        .unwrap();

    let result = like_post(&mut ctx.conn, liker.id, post.id);
    assert!(matches!(result, Err(LikeError::PostNotFound)));
}

#[test]
fn like_post_rejects_soft_deleted_post() {
    let mut ctx = common::setup();

    let author = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let liker = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(author.id));
    let post = common::create_post(
        &mut ctx.conn,
        common::PostOpts::for_topic(topic.id, author.id),
    );

    diesel::update(posts::table.find(post.id))
        .set(posts::deleted_at.eq(Some(chrono::Utc::now().naive_utc())))
        .execute(&mut ctx.conn)
        .unwrap();

    let result = like_post(&mut ctx.conn, liker.id, post.id);
    assert!(matches!(result, Err(LikeError::PostNotFound)));
}

// ─────────────────────────────────────────────────────────────────────────────
// unlike_post

#[test]
fn unlike_post_removes_like_and_decrements_counters() {
    let mut ctx = common::setup();

    let author = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let liker = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(author.id));
    let post = common::create_post(
        &mut ctx.conn,
        common::PostOpts::for_topic(topic.id, author.id),
    );

    like_post(&mut ctx.conn, liker.id, post.id).unwrap();
    let outcome = unlike_post(&mut ctx.conn, liker.id, post.id).expect("unlike failed");
    assert!(matches!(outcome, UnlikeOutcome::Removed));

    let row_count: i64 = post_likes::table.count().get_result(&mut ctx.conn).unwrap();
    assert_eq!(row_count, 0);

    let post_like_count: i32 = posts::table
        .find(post.id)
        .select(posts::like_count)
        .first(&mut ctx.conn)
        .unwrap();
    assert_eq!(post_like_count, 0);

    let liker_given: i32 = users::table
        .find(liker.id)
        .select(users::likes_given)
        .first(&mut ctx.conn)
        .unwrap();
    assert_eq!(liker_given, 0);

    let author_received: i32 = users::table
        .find(author.id)
        .select(users::likes_received)
        .first(&mut ctx.conn)
        .unwrap();
    assert_eq!(author_received, 0);
}

#[test]
fn unlike_post_is_noop_when_never_liked() {
    let mut ctx = common::setup();

    let author = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let liker = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(author.id));
    let post = common::create_post(
        &mut ctx.conn,
        common::PostOpts::for_topic(topic.id, author.id),
    );

    let outcome = unlike_post(&mut ctx.conn, liker.id, post.id).expect("unlike failed");
    assert!(matches!(outcome, UnlikeOutcome::NothingToRemove));
}

#[test]
fn unlike_post_is_noop_when_post_missing() {
    let mut ctx = common::setup();
    let liker = common::create_user(&mut ctx.conn, common::UserOpts::default());

    let outcome = unlike_post(&mut ctx.conn, liker.id, 999_999).expect("unlike failed");
    assert!(matches!(outcome, UnlikeOutcome::NothingToRemove));
}

#[test]
fn unlike_floors_counters_at_zero() {
    // Simulate pre-existing drift: a like row exists but the counter is
    // already at zero. Unliking should not push the counter negative.
    let mut ctx = common::setup();

    let author = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let liker = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(author.id));
    let post = common::create_post(
        &mut ctx.conn,
        common::PostOpts::for_topic(topic.id, author.id),
    );

    // Insert the like row directly without bumping counters
    diesel::insert_into(post_likes::table)
        .values((
            post_likes::user_id.eq(liker.id),
            post_likes::post_id.eq(post.id),
        ))
        .execute(&mut ctx.conn)
        .unwrap();

    let outcome = unlike_post(&mut ctx.conn, liker.id, post.id).expect("unlike failed");
    assert!(matches!(outcome, UnlikeOutcome::Removed));

    let post_like_count: i32 = posts::table
        .find(post.id)
        .select(posts::like_count)
        .first(&mut ctx.conn)
        .unwrap();
    assert_eq!(post_like_count, 0, "counter must not go negative");
}
