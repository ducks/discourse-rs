//! ComposerMessagesFinder and ComposerMessagesController, after
//! spec/lib/composer_messages_finder_spec.rb,
//! spec/requests/composer_messages_controller_spec.rb and
//! discourse-solved's spec/components/composer_messages_finder_spec.rb.
//! Where the reference stubs a user's counts (`user.stubs(:post_count)`)
//! the user's stats hold them; where it mocks posted_too_much_in_topic?
//! the member has posted that much. Every test fabricates its own users
//! and topics.

use crate::common;

use axum::body::Body;
use axum::http::{Method, Request, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::composer::messages::{self, Details, Finder};
use discourse_rs::config::RailsEnv;
use discourse_rs::guardian::Guardian;
use discourse_rs::session::current::SessionUser;
use discourse_rs::site_settings::SiteSettings;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use sqlx::pool::PoolConnection;
use tower::ServiceExt;

const INTEGER: i32 = 3;
const BOOL: i32 = 5;
const GROUP_LIST: i32 = 20;

async fn setup() -> (TestDb, AppState) {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    (db, st)
}

async fn exec(pool: &PgPool, sql: &str) {
    sqlx::query(sql).execute(pool).await.unwrap();
}

/// `Fabricate(:user)`: a copy of user0 (trust level 1, password
/// "password") with options, an email and empty stats of its own.
async fn fabricate_user(pool: &PgPool, username: &str) -> i32 {
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO users SELECT (jsonb_populate_record(u, jsonb_build_object(\
           'id', nextval('users_id_seq'), 'username', $1::text, 'username_lower', lower($1::text), \
           'name', $1::text, 'created_at', now(), 'updated_at', now(), 'last_seen_at', now()))).* \
         FROM users u WHERE u.id = 2 RETURNING id",
    )
    .bind(username)
    .fetch_one(pool)
    .await
    .unwrap();
    exec(
        pool,
        &format!(
            "INSERT INTO user_options SELECT (jsonb_populate_record(o, jsonb_build_object('user_id', {id}))).* \
             FROM user_options o WHERE o.user_id = 2"
        ),
    )
    .await;
    exec(
        pool,
        &format!(
            "INSERT INTO user_passwords SELECT (jsonb_populate_record(p, jsonb_build_object(\
               'id', nextval('user_passwords_id_seq'), 'user_id', {id}))).* \
             FROM user_passwords p WHERE p.user_id = 2"
        ),
    )
    .await;
    exec(
        pool,
        &format!(
            "INSERT INTO user_emails (user_id, email, \"primary\", created_at, updated_at) \
             VALUES ({id}, '{username}@example.com', TRUE, now(), now())"
        ),
    )
    .await;
    exec(
        pool,
        &format!(
            "INSERT INTO user_stats SELECT (jsonb_populate_record(s, jsonb_build_object(\
               'user_id', {id}, 'topic_count', 0, 'post_count', 0))).* \
             FROM user_stats s WHERE s.user_id = 2"
        ),
    )
    .await;
    id
}

/// `refresh_auto_groups`: trust_level_0 and trust_level_1 for the
/// fabricated (trust level 1) user.
async fn refresh_auto_groups(pool: &PgPool, user_id: i32) {
    exec(
        pool,
        &format!(
            "INSERT INTO group_users (group_id, user_id, created_at, updated_at) \
             VALUES (10, {user_id}, now(), now()), (11, {user_id}, now(), now())"
        ),
    )
    .await;
}

/// The counts `user.stubs(:post_count)` and `:topic_count` answer.
async fn stats(pool: &PgPool, user_id: i32, topics: i32, posts: i32) {
    exec(
        pool,
        &format!(
            "UPDATE user_stats SET topic_count = {topics}, post_count = {posts} WHERE user_id = {user_id}"
        ),
    )
    .await;
}

/// `Fabricate(:topic, user:)` in a category (none for a message), created
/// `created_at` (SQL).
async fn topic(pool: &PgPool, user_id: i32, category: Option<i32>, created_at: &str) -> i32 {
    let archetype = if category.is_some() {
        "regular"
    } else {
        "private_message"
    };
    let id: i32 = sqlx::query_scalar(&format!(
        "INSERT INTO topics (title, fancy_title, slug, user_id, last_post_user_id, category_id, archetype, \
           created_at, updated_at, bumped_at, last_posted_at, posts_count, highest_post_number, visible) \
         VALUES ('A topic to compose a reply in ' || nextval('topics_id_seq'), 'A topic', 'a-topic', $1, $1, $2, $3, \
           {created_at}, {created_at}, {created_at}, {created_at}, 0, 0, TRUE) RETURNING id"
    ))
    .bind(user_id)
    .bind(category)
    .bind(archetype)
    .fetch_one(pool)
    .await
    .unwrap();
    if category.is_none() {
        exec(
            pool,
            &format!(
                "INSERT INTO topic_allowed_users (topic_id, user_id, created_at, updated_at) \
                 VALUES ({id}, {user_id}, now(), now())"
            ),
        )
        .await;
    }
    id
}

/// `Fabricate(:post, topic:, user:, reply_to_user_id:)`, counted on the
/// topic.
async fn post(pool: &PgPool, topic_id: i32, user_id: i32, reply_to_user_id: Option<i32>) -> i32 {
    sqlx::query_scalar(
        "WITH t AS (UPDATE topics SET posts_count = posts_count + 1, \
                      highest_post_number = highest_post_number + 1 \
                    WHERE id = $1 RETURNING highest_post_number) \
         INSERT INTO posts (topic_id, user_id, post_number, raw, cooked, reply_to_user_id, \
           created_at, updated_at, last_version_at, sort_order) \
         SELECT $1, $2, t.highest_post_number, 'Hello there, a post', '<p>Hello there, a post</p>', $3, \
           clock_timestamp(), clock_timestamp(), clock_timestamp(), t.highest_post_number FROM t \
         RETURNING id",
    )
    .bind(topic_id)
    .bind(user_id)
    .bind(reply_to_user_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// `Fabricate(:flag_post_action, post:, user:)`, a spam flag; `extra`
/// sets more columns (SQL assignments).
async fn flag(pool: &PgPool, post_id: i32, user_id: i32, extra: &str) {
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO post_actions (post_id, user_id, post_action_type_id, created_at, updated_at) \
         VALUES ($1, $2, 8, now(), now()) RETURNING id",
    )
    .bind(post_id)
    .bind(user_id)
    .fetch_one(pool)
    .await
    .unwrap();
    if !extra.is_empty() {
        exec(
            pool,
            &format!("UPDATE post_actions SET {extra} WHERE id = {id}"),
        )
        .await;
    }
}

/// `Fabricate(:private_category, group:)` with a topic in it.
async fn private_topic(pool: &PgPool, user_id: i32) -> i32 {
    let group: i32 = sqlx::query_scalar(
        "INSERT INTO groups (name, created_at, updated_at) \
         VALUES ('private_' || nextval('groups_id_seq'), now(), now()) RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let category: i32 = sqlx::query_scalar(
        "INSERT INTO categories (name, name_lower, slug, user_id, read_restricted, created_at, updated_at) \
         VALUES ('Private ' || nextval('categories_id_seq'), 'private ' || currval('categories_id_seq'), \
           'private-' || currval('categories_id_seq'), \
           1, TRUE, now(), now()) RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    exec(
        pool,
        &format!(
            "INSERT INTO category_groups (category_id, group_id, permission_type, created_at, updated_at) \
             VALUES ({category}, {group}, 1, now(), now())"
        ),
    )
    .await;
    topic(pool, user_id, Some(category), "now()").await
}

fn details(action: &str, topic_id: Option<i32>, post_id: Option<i32>) -> Details {
    Details {
        composer_action: Some(action.to_string()),
        topic_id: topic_id.map(|id| id.to_string()),
        post_id: post_id.map(|id| id.to_string()),
    }
}

/// What a finder for the user reads, loaded after the settings are set.
struct Ctx {
    conn: PoolConnection<sqlx::Postgres>,
    settings: SiteSettings,
    guardian: Guardian,
}

async fn ctx(st: &AppState, user_id: i32) -> Ctx {
    let mut conn = st.pool.acquire().await.unwrap();
    let settings = SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap();
    let user = SessionUser::load(&mut conn, user_id)
        .await
        .unwrap()
        .unwrap();
    let guardian = Guardian::for_user(&mut conn, &user).await.unwrap();
    Ctx {
        conn,
        settings,
        guardian,
    }
}

/// `UserHistory.exists_for_user?(user, action)`
async fn logged(pool: &PgPool, user_id: i32, action: &str) -> bool {
    let mut conn = pool.acquire().await.unwrap();
    messages::exists_for_user(&mut conn, user_id, action, None)
        .await
        .unwrap()
}

/// `UserHistory.create!(action:, target_user_id:, topic_id:)`
async fn history(pool: &PgPool, user_id: i32, action: &str, topic_id: i32) {
    sqlx::query(
        "INSERT INTO user_histories (action, target_user_id, topic_id, created_at, updated_at) \
         VALUES ($1, $2, $3, now(), now())",
    )
    .bind(discourse_rs::staff_action_logs::action_id(action))
    .bind(user_id)
    .bind(topic_id)
    .execute(pool)
    .await
    .unwrap();
}

// describe "delegates work"

#[tokio::test]
async fn calls_all_the_message_finders() {
    // The checks run in their definition order, then the plugins': with
    // nothing to say each gives way to the next.
    let (db, st) = setup().await;
    let user = fabricate_user(&db.pool, "delegated").await;
    set_setting(&db.pool, "educate_until_posts", INTEGER, "0").await;
    let mut c = ctx(&st, user).await;
    let finder = Finder::new(
        &mut c.conn,
        &st,
        &c.settings,
        &c.guardian,
        details("createTopic", None, None),
    )
    .await
    .unwrap();
    assert!(finder.find(&mut c.conn).await.unwrap().is_none());
}

// describe ".check_education_message"

#[tokio::test]
async fn education_when_creating_a_topic() {
    let (db, st) = setup().await;
    let user = fabricate_user(&db.pool, "educated").await;
    set_setting(&db.pool, "educate_until_posts", INTEGER, "10").await;

    // returns a message for a user who has not posted any topics
    stats(&db.pool, user, 1, 8).await;
    let mut c = ctx(&st, user).await;
    let finder = Finder::new(
        &mut c.conn,
        &st,
        &c.settings,
        &c.guardian,
        details("createTopic", None, None),
    )
    .await
    .unwrap();
    let message = finder.check_education_message(&mut c.conn).await.unwrap();
    let message = message.expect("a message");
    assert_eq!(message["id"], "education");
    assert_eq!(message["templateName"], "education");
    assert_eq!(message["wait_for_typing"], true);
    let body = message["body"].as_str().unwrap();
    assert!(body.contains("select a category"), "{body}");
    assert!(body.contains("href=\"/guidelines\""), "{body}");

    // returns no message when the user has posted enough topics
    stats(&db.pool, user, 2, 8).await;
    assert!(
        finder
            .check_education_message(&mut c.conn)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn no_education_in_a_private_message() {
    let (db, st) = setup().await;
    let user = fabricate_user(&db.pool, "messaging").await;
    let pm = topic(&db.pool, user, None, "now()").await;
    let mut c = ctx(&st, user).await;
    for action in ["createTopic", "reply"] {
        let finder = Finder::new(
            &mut c.conn,
            &st,
            &c.settings,
            &c.guardian,
            details(action, Some(pm), None),
        )
        .await
        .unwrap();
        assert!(
            finder
                .check_education_message(&mut c.conn)
                .await
                .unwrap()
                .is_none(),
            "{action}"
        );
    }
}

#[tokio::test]
async fn education_when_creating_a_reply() {
    let (db, st) = setup().await;
    let user = fabricate_user(&db.pool, "replying").await;
    set_setting(&db.pool, "educate_until_posts", INTEGER, "10").await;
    stats(&db.pool, user, 0, 9).await;
    let mut c = ctx(&st, user).await;
    let finder = Finder::new(
        &mut c.conn,
        &st,
        &c.settings,
        &c.guardian,
        details("reply", None, None),
    )
    .await
    .unwrap();
    let message = finder.check_education_message(&mut c.conn).await.unwrap();
    let body = message.expect("a message")["body"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(body.contains("all real people"), "{body}");

    stats(&db.pool, user, 0, 10).await;
    assert!(
        finder
            .check_education_message(&mut c.conn)
            .await
            .unwrap()
            .is_none()
    );
}

// describe ".check_new_user_many_replies"

#[tokio::test]
async fn new_user_many_replies() {
    let (db, st) = setup().await;
    let author = fabricate_user(&db.pool, "manyauthor").await;
    let user = fabricate_user(&db.pool, "manyreplies").await;
    exec(
        &db.pool,
        &format!("UPDATE users SET trust_level = 0 WHERE id = {user}"),
    )
    .await;
    set_setting(&db.pool, "newuser_max_replies_per_topic", INTEGER, "2").await;
    let t = topic(&db.pool, author, Some(4), "now()").await;
    post(&db.pool, t, author, None).await;
    post(&db.pool, t, user, None).await;

    // has no message when `posted_too_much_in_topic?` is false
    let mut c = ctx(&st, user).await;
    let finder = Finder::new(
        &mut c.conn,
        &st,
        &c.settings,
        &c.guardian,
        details("reply", Some(t), None),
    )
    .await
    .unwrap();
    assert!(
        finder
            .check_new_user_many_replies(&mut c.conn)
            .await
            .unwrap()
            .is_none()
    );

    // has a message when a user has posted too much
    post(&db.pool, t, user, None).await;
    let message = finder
        .check_new_user_many_replies(&mut c.conn)
        .await
        .unwrap()
        .expect("a message");
    assert_eq!(message["id"], "too_many_replies");
    assert_eq!(message["templateName"], "education");
    let body = message["body"].as_str().unwrap();
    assert!(body.contains("limited to 2 replies"), "{body}");
}

// describe ".check_dominating_topic"

/// The spec's before: educate_until_posts 10, the user's 11 posts,
/// summary_posts_required 1, and a topic with two posts of the user's and
/// one of another's.
async fn dominating(db: &TestDb, name: &str) -> (i32, i32) {
    let user = fabricate_user(&db.pool, name).await;
    let other = fabricate_user(&db.pool, &format!("{name}_other")).await;
    set_setting(&db.pool, "educate_until_posts", INTEGER, "10").await;
    stats(&db.pool, user, 0, 11).await;
    set_setting(&db.pool, "summary_posts_required", INTEGER, "1").await;
    let author = fabricate_user(&db.pool, &format!("{name}_author")).await;
    let t = topic(&db.pool, author, Some(4), "now()").await;
    post(&db.pool, t, user, None).await;
    post(&db.pool, t, user, None).await;
    post(&db.pool, t, other, None).await;
    (user, t)
}

async fn dominating_message(st: &AppState, user: i32, d: Details) -> Option<Value> {
    let mut c = ctx(st, user).await;
    let finder = Finder::new(&mut c.conn, st, &c.settings, &c.guardian, d)
        .await
        .unwrap();
    finder.check_dominating_topic(&mut c.conn).await.unwrap()
}

#[tokio::test]
async fn dominating_topic_not_for_new_topics_or_without_a_topic() {
    let (db, st) = setup().await;
    let (user, _) = dominating(&db, "dom_new").await;
    assert!(
        dominating_message(&st, user, details("createTopic", None, None))
            .await
            .is_none()
    );
    assert!(
        dominating_message(&st, user, details("reply", None, None))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn dominating_topic_on_a_reply() {
    let (db, st) = setup().await;
    let (user, t) = dominating(&db, "dom_reply").await;
    let reply = || details("reply", Some(t), None);

    // does not give a message to users who are still in the 'education' phase
    stats(&db.pool, user, 0, 9).await;
    assert!(dominating_message(&st, user, reply()).await.is_none());
    stats(&db.pool, user, 0, 11).await;

    // does not notify if the `summary_posts_required` has not been reached
    set_setting(&db.pool, "summary_posts_required", INTEGER, "100").await;
    assert!(dominating_message(&st, user, reply()).await.is_none());

    // doesn't notify a user if the topic has less than `summary_posts_required` posts
    set_setting(&db.pool, "summary_posts_required", INTEGER, "5").await;
    assert!(dominating_message(&st, user, reply()).await.is_none());
    set_setting(&db.pool, "summary_posts_required", INTEGER, "1").await;

    // doesn't notify a user if they've posted less than the percentage
    set_setting(&db.pool, "dominating_topic_minimum_percent", INTEGER, "100").await;
    assert!(dominating_message(&st, user, reply()).await.is_none());
    exec(
        &db.pool,
        "DELETE FROM site_settings WHERE name = 'dominating_topic_minimum_percent'",
    )
    .await;

    // doesn't notify you if it's your own topic
    exec(
        &db.pool,
        &format!("UPDATE topics SET user_id = {user} WHERE id = {t}"),
    )
    .await;
    assert!(dominating_message(&st, user, reply()).await.is_none());
    exec(
        &db.pool,
        &format!("UPDATE topics SET user_id = 1 WHERE id = {t}"),
    )
    .await;

    // nothing logged yet
    assert!(!logged(&db.pool, user, "notified_about_dominating_topic").await);

    // notifies a user if the topic is different
    history(&db.pool, user, "notified_about_dominating_topic", t + 1).await;
    let message = dominating_message(&st, user, reply())
        .await
        .expect("a message");
    assert_eq!(message["id"], "dominating_topic");
    assert_eq!(message["templateName"], "dominating-topic");
    assert_eq!(message["wait_for_typing"], true);
    assert_eq!(
        message["extraClass"],
        "education-message dominating-topic-message"
    );
    // creates a notified_about_dominating_topic log
    assert!(logged(&db.pool, user, "notified_about_dominating_topic").await);

    // doesn't notify a user it has already notified in this topic
    assert!(dominating_message(&st, user, reply()).await.is_none());
}

#[tokio::test]
async fn dominating_topic_not_in_a_private_message() {
    let (db, st) = setup().await;
    let (user, t) = dominating(&db, "dom_pm").await;
    exec(
        &db.pool,
        &format!(
            "UPDATE topics SET category_id = NULL, archetype = 'private_message' WHERE id = {t}"
        ),
    )
    .await;
    exec(
        &db.pool,
        &format!(
            "INSERT INTO topic_allowed_users (topic_id, user_id, created_at, updated_at) \
             VALUES ({t}, {user}, now(), now())"
        ),
    )
    .await;
    assert!(
        dominating_message(&st, user, details("reply", Some(t), None))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn dominating_topic_not_when_the_user_cannot_see_the_topic() {
    let (db, st) = setup().await;
    let (user, _) = dominating(&db, "dom_private").await;
    let other = fabricate_user(&db.pool, "dom_private_poster").await;
    let private = private_topic(&db.pool, 1).await;
    post(&db.pool, private, user, None).await;
    post(&db.pool, private, user, None).await;
    post(&db.pool, private, other, None).await;
    assert!(
        dominating_message(&st, user, details("reply", Some(private), None))
            .await
            .is_none()
    );
}

// describe "#dont_feed_the_trolls"

async fn trolls_message(st: &AppState, user: i32, d: Details) -> Option<Value> {
    let mut c = ctx(st, user).await;
    let finder = Finder::new(&mut c.conn, st, &c.settings, &c.guardian, d)
        .await
        .unwrap();
    finder
        .check_dont_feed_the_trolls(&mut c.conn)
        .await
        .unwrap()
}

#[tokio::test]
async fn dont_feed_the_trolls() {
    let (db, st) = setup().await;
    let user = fabricate_user(&db.pool, "troll_replier").await;
    let author = fabricate_user(&db.pool, "troll_author").await;
    let other_user = fabricate_user(&db.pool, "troll_other").await;
    let third_user = fabricate_user(&db.pool, "troll_third").await;
    let t = topic(&db.pool, author, Some(4), "now()").await;
    let original_post = post(&db.pool, t, author, None).await;
    let unflagged_post = post(&db.pool, t, author, None).await;
    let self_flagged_post = post(&db.pool, t, author, None).await;
    let under_flagged_post = post(&db.pool, t, author, None).await;
    let over_flagged_post = post(&db.pool, t, author, None).await;
    let resolved_flag_post = post(&db.pool, t, author, None).await;
    set_setting(&db.pool, "dont_feed_the_trolls_threshold", INTEGER, "2").await;

    // does not show a message for unflagged posts
    assert!(
        trolls_message(&st, user, details("reply", Some(t), Some(unflagged_post)))
            .await
            .is_none()
    );

    // shows a message when the replier has already flagged the post
    flag(&db.pool, self_flagged_post, user, "").await;
    let message = trolls_message(
        &st,
        user,
        details("reply", Some(t), Some(self_flagged_post)),
    )
    .await
    .expect("a message");
    assert_eq!(message["id"], "dont_feed_the_trolls");
    assert_eq!(message["wait_for_typing"], false);
    assert_eq!(message["extraClass"], "urgent");
    assert!(
        message["body"]
            .as_str()
            .unwrap()
            .contains("This post was flagged for moderator attention.")
    );

    // shows a message when replying to flagged topic (first post)
    flag(&db.pool, original_post, user, "").await;
    assert!(
        trolls_message(&st, user, details("reply", Some(t), None))
            .await
            .is_some()
    );

    // does not show a message when not enough others have flagged the post
    flag(&db.pool, under_flagged_post, other_user, "").await;
    assert!(
        trolls_message(
            &st,
            user,
            details("reply", Some(t), Some(under_flagged_post))
        )
        .await
        .is_none()
    );

    // shows a message when enough others have already flagged the post
    flag(&db.pool, over_flagged_post, other_user, "").await;
    flag(&db.pool, over_flagged_post, third_user, "").await;
    assert!(
        trolls_message(
            &st,
            user,
            details("reply", Some(t), Some(over_flagged_post))
        )
        .await
        .is_some()
    );

    // safely returns from not finding a post
    assert!(
        trolls_message(&st, user, details("reply", None, None))
            .await
            .is_none()
    );

    // does not show a message when the flag has already been resolved
    set_setting(&db.pool, "dont_feed_the_trolls_threshold", INTEGER, "1").await;
    flag(
        &db.pool,
        resolved_flag_post,
        other_user,
        "disagreed_at = now() - interval '1 hour'",
    )
    .await;
    assert!(
        trolls_message(
            &st,
            user,
            details("reply", Some(t), Some(resolved_flag_post))
        )
        .await
        .is_none()
    );

    // returns no message when the user cannot see the topic
    let private = private_topic(&db.pool, author).await;
    let private_post = post(&db.pool, private, author, None).await;
    flag(&db.pool, private_post, other_user, "").await;
    assert!(
        trolls_message(
            &st,
            user,
            details("reply", Some(private), Some(private_post))
        )
        .await
        .is_none()
    );
}

// describe ".check_get_a_room"

struct Room {
    user: i32,
    topic: i32,
    op: i32,
    other_user_reply: i32,
    first_reply: i32,
    second_reply: i32,
}

/// The spec's fabrications and before: the user's two replies to the
/// OP's author, a third user's, and the settings.
async fn room(db: &TestDb, name: &str) -> Room {
    let user = fabricate_user(&db.pool, name).await;
    let other_user = fabricate_user(&db.pool, &format!("{name}_other")).await;
    let third_user = fabricate_user(&db.pool, &format!("{name}_third")).await;
    let t = topic(&db.pool, other_user, Some(4), "now()").await;
    let op = post(&db.pool, t, other_user, None).await;
    let other_user_reply = post(&db.pool, t, third_user, Some(other_user)).await;
    let first_reply = post(&db.pool, t, user, Some(other_user)).await;
    let second_reply = post(&db.pool, t, user, Some(other_user)).await;
    set_setting(&db.pool, "educate_until_posts", INTEGER, "10").await;
    stats(&db.pool, user, 0, 11).await;
    set_setting(&db.pool, "get_a_room_threshold", INTEGER, "2").await;
    set_setting(&db.pool, "personal_message_enabled_groups", GROUP_LIST, "0").await;
    // The spec's environment: the upcoming change that ends the everyone
    // shortcut (granular pseudogroups) is not promoted there.
    set_setting(
        &db.pool,
        "granular_anonymous_and_logged_in_groups_permissions",
        BOOL,
        "f",
    )
    .await;
    Room {
        user,
        topic: t,
        op,
        other_user_reply,
        first_reply,
        second_reply,
    }
}

async fn room_message(
    st: &AppState,
    user: i32,
    d: Details,
    min_users_posted: i64,
) -> Option<Value> {
    let mut c = ctx(st, user).await;
    let finder = Finder::new(&mut c.conn, st, &c.settings, &c.guardian, d)
        .await
        .unwrap();
    finder
        .check_get_a_room(&mut c.conn, min_users_posted)
        .await
        .unwrap()
}

#[tokio::test]
async fn get_a_room_not_when_the_user_cannot_send_private_messages() {
    let (db, st) = setup().await;
    let r = room(&db, "room_nopm").await;
    let group: i32 = sqlx::query_scalar(
        "INSERT INTO groups (name, created_at, updated_at) VALUES ('room_pm_group', now(), now()) RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    set_setting(
        &db.pool,
        "personal_message_enabled_groups",
        GROUP_LIST,
        &group.to_string(),
    )
    .await;
    assert!(
        room_message(&st, r.user, details("reply", Some(r.topic), Some(r.op)), 2)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn get_a_room_not_for_new_topics_without_a_topic_or_in_a_read_restricted_category() {
    let (db, st) = setup().await;
    let r = room(&db, "room_scope").await;
    assert!(
        room_message(&st, r.user, details("createTopic", None, None), 2)
            .await
            .is_none()
    );
    assert!(
        room_message(&st, r.user, details("reply", None, None), 2)
            .await
            .is_none()
    );

    // does not give a message if the topic's category is read_restricted
    exec(
        &db.pool,
        "UPDATE categories SET read_restricted = TRUE WHERE id = 4",
    )
    .await;
    room_message(&st, r.user, details("reply", Some(r.topic), Some(r.op)), 2).await;
    assert!(!logged(&db.pool, r.user, "notified_about_get_a_room").await);
}

#[tokio::test]
async fn get_a_room_on_a_reply() {
    let (db, st) = setup().await;
    let r = room(&db, "room_reply").await;
    let reply = || details("reply", Some(r.topic), Some(r.op));

    // does not give a message to users who are still in the 'education' phase
    stats(&db.pool, r.user, 0, 9).await;
    assert!(room_message(&st, r.user, reply(), 2).await.is_none());
    stats(&db.pool, r.user, 0, 11).await;

    // does not notify users with too few posts
    set_setting(&db.pool, "get_a_room_threshold", INTEGER, "10").await;
    assert!(room_message(&st, r.user, reply(), 2).await.is_none());
    set_setting(&db.pool, "get_a_room_threshold", INTEGER, "2").await;

    // doesn't notify when replying to a different user
    assert!(
        room_message(
            &st,
            r.user,
            details("reply", Some(r.topic), Some(r.other_user_reply)),
            2
        )
        .await
        .is_none()
    );

    // with a default min_users_posted value
    assert!(room_message(&st, r.user, reply(), 5).await.is_none());

    // doesn't notify you if the posts aren't all to the same person
    exec(
        &db.pool,
        &format!(
            "UPDATE posts SET reply_to_user_id = {} WHERE id = {}",
            r.user, r.first_reply
        ),
    )
    .await;
    assert!(room_message(&st, r.user, reply(), 2).await.is_none());

    // doesn't notify you of posts to yourself
    exec(
        &db.pool,
        &format!(
            "UPDATE posts SET reply_to_user_id = {} WHERE id = {}",
            r.user, r.second_reply
        ),
    )
    .await;
    assert!(room_message(&st, r.user, reply(), 2).await.is_none());
    let op_user: i32 = sqlx::query_scalar("SELECT user_id FROM posts WHERE id = $1")
        .bind(r.op)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    exec(
        &db.pool,
        &format!(
            "UPDATE posts SET reply_to_user_id = {op_user} WHERE id IN ({}, {})",
            r.first_reply, r.second_reply
        ),
    )
    .await;

    // notifies the user if no notice was shown in the current topic
    history(&db.pool, r.user, "notified_about_get_a_room", r.topic + 1).await;
    let message = room_message(&st, r.user, reply(), 2)
        .await
        .expect("a message");
    assert_eq!(message["id"], "get_a_room");
    assert_eq!(message["wait_for_typing"], true);
    assert_eq!(message["templateName"], "get-a-room");
    assert_eq!(message["extraClass"], "education-message get-a-room");
    assert_eq!(message["reply_username"], "room_reply_other");
    let body = message["body"].as_str().unwrap();
    assert!(body.contains("room_reply_other</a> 2 times"), "{body}");
    assert!(logged(&db.pool, r.user, "notified_about_get_a_room").await);

    // doesn't notify a user it has already notified about sequential replies
    assert!(room_message(&st, r.user, reply(), 2).await.is_none());
}

#[tokio::test]
async fn get_a_room_not_in_a_message() {
    let (db, st) = setup().await;
    let r = room(&db, "room_pm").await;
    exec(
        &db.pool,
        &format!(
            "UPDATE topics SET category_id = NULL, archetype = 'private_message' WHERE id = {}",
            r.topic
        ),
    )
    .await;
    exec(
        &db.pool,
        &format!(
            "INSERT INTO topic_allowed_users (topic_id, user_id, created_at, updated_at) \
             VALUES ({}, {}, now(), now())",
            r.topic, r.user
        ),
    )
    .await;
    assert!(
        room_message(&st, r.user, details("reply", Some(r.topic), Some(r.op)), 2)
            .await
            .is_none()
    );
}

// context "when editing a post"

#[tokio::test]
async fn editing_a_post_returns_nothing() {
    let (db, st) = setup().await;
    let user = fabricate_user(&db.pool, "editing").await;
    let t = topic(&db.pool, user, Some(4), "now()").await;
    post(&db.pool, t, user, None).await;
    set_setting(&db.pool, "educate_until_posts", INTEGER, "2").await;
    let mut c = ctx(&st, user).await;
    let finder = Finder::new(
        &mut c.conn,
        &st,
        &c.settings,
        &c.guardian,
        details("edit", None, None),
    )
    .await
    .unwrap();
    assert!(finder.find(&mut c.conn).await.unwrap().is_none());
}

// describe "#user_not_seen_in_a_while"

async fn unseen_users(db: &TestDb, prefix: &str) -> [String; 3] {
    let names = [
        format!("{prefix}_1"),
        format!("{prefix}_2"),
        format!("{prefix}_3"),
    ];
    for (name, ago) in names.iter().zip(["3 years", "2 years", "6 months"]) {
        let id = fabricate_user(&db.pool, name).await;
        exec(
            &db.pool,
            &format!("UPDATE users SET last_seen_at = now() - interval '{ago}' WHERE id = {id}"),
        )
        .await;
    }
    names
}

#[tokio::test]
async fn user_not_seen_in_a_while() {
    let (db, st) = setup().await;
    let names = unseen_users(&db, "unseen").await;
    set_setting(&db.pool, "pm_warn_user_last_seen_months_ago", INTEGER, "24").await;
    let c = ctx(&st, 1).await;
    let mut conn = st.pool.acquire().await.unwrap();
    let found = messages::user_not_seen_in_a_while(&mut conn, &c.settings, &names)
        .await
        .unwrap();
    assert_eq!(found, [names[0].clone(), names[1].clone()]);

    // accounts for pm_warn_user_last_seen_months_ago site setting
    set_setting(&db.pool, "pm_warn_user_last_seen_months_ago", INTEGER, "30").await;
    let c = ctx(&st, 1).await;
    let found = messages::user_not_seen_in_a_while(&mut conn, &c.settings, &names)
        .await
        .unwrap();
    assert_eq!(found, [names[0].clone()]);
}

// discourse-solved: describe ".check_topic_is_solved"

async fn solved_message(st: &AppState, user: i32, d: Details) -> Option<Value> {
    let mut c = ctx(st, user).await;
    let finder = Finder::new(&mut c.conn, st, &c.settings, &c.guardian, d)
        .await
        .unwrap();
    discourse_rs::plugins::solved::composer_message::check_topic_is_solved(&mut c.conn, &finder)
        .await
        .unwrap()
}

#[tokio::test]
async fn topic_is_solved() {
    let (db, st) = setup().await;
    let user = fabricate_user(&db.pool, "solved_replier").await;
    let author = fabricate_user(&db.pool, "solved_author").await;
    let answerer = fabricate_user(&db.pool, "solved_answerer").await;
    set_setting(&db.pool, "solved_enabled", BOOL, "t").await;
    set_setting(&db.pool, "disable_solved_education_message", BOOL, "f").await;
    set_setting(&db.pool, "allow_solved_on_all_topics", BOOL, "t").await;
    let t = topic(&db.pool, author, Some(4), "now()").await;
    post(&db.pool, t, author, None).await;
    let answer = post(&db.pool, t, answerer, None).await;

    // does not show message without a topic id
    assert!(
        solved_message(&st, user, details("createTopic", None, None))
            .await
            .is_none()
    );
    assert!(
        solved_message(&st, user, details("reply", None, None))
            .await
            .is_none()
    );

    // does not show message if topic is not solved
    assert!(
        solved_message(&st, user, details("reply", Some(t), None))
            .await
            .is_none()
    );

    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, body) = admin
        .json(
            Method::POST,
            "/solution/accept.json",
            &format!("id={answer}"),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    // shows message if the topic is solved
    let message = solved_message(&st, user, details("reply", Some(t), None))
        .await
        .expect("a message");
    assert_eq!(message["id"], "solved_topic");
    assert_eq!(message["templateName"], "education");
    assert_eq!(message["wait_for_typing"], true);
    assert_eq!(message["extraClass"], "education-message");
    assert_eq!(message["hide_if_whisper"], true);
    let body = message["body"].as_str().unwrap();
    assert!(body.contains("This topic has been solved"), "{body}");
    assert!(
        body.contains("href=\"http://test.localhost/new-topic\""),
        "{body}"
    );

    // does not show message if disable_solved_education_message is true
    set_setting(&db.pool, "disable_solved_education_message", BOOL, "t").await;
    assert!(
        solved_message(&st, user, details("reply", Some(t), None))
            .await
            .is_none()
    );
}

// ComposerMessagesController

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn send(&mut self, method: Method, path: &str, body: &str) -> (u16, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest")
            .header(header::ACCEPT, "application/json");
        if !self.cookies.is_empty() {
            let cookie: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookie.join("; "));
        }
        if let Some(token) = &self.csrf {
            request = request.header("x-csrf-token", token.as_str());
        }
        if !body.is_empty() {
            request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap();
            let pair = v.split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn json(&mut self, method: Method, path: &str, body: &str) -> (u16, Value) {
        let (status, text) = self.send(method, path, body).await;
        let value = serde_json::from_str(&text).unwrap_or_else(|_| panic!("{status} {text}"));
        (status, value)
    }

    async fn anonymous(state: AppState) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client.json(Method::GET, "/session/csrf.json", "").await;
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client::anonymous(state).await;
        let (status, body) = client
            .send(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        client
    }
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The controller spec's topic and first post, ten years old.
async fn old_topic(pool: &PgPool) -> i32 {
    let author = fabricate_user(pool, &format!("old_author_{}", rand_suffix(pool).await)).await;
    let t = topic(pool, author, Some(4), "now() - interval '10 years'").await;
    let op = post(pool, t, author, None).await;
    exec(
        pool,
        &format!("UPDATE posts SET created_at = now() - interval '10 years' WHERE id = {op}"),
    )
    .await;
    t
}

async fn rand_suffix(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT nextval('users_id_seq')")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A reply through POST /posts, its links extracted as PostCreator does;
/// its post processing (ProcessPost, which extracts them again from the
/// processed post) is left queued, as in the reference's request specs.
async fn reply(client: &mut Client, topic_id: i32, raw: &str) -> i32 {
    let (status, body) = client
        .json(
            Method::POST,
            "/posts.json",
            &format!("topic_id={topic_id}&raw={}", urlencode(raw)),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    body["id"].as_i64().unwrap() as i32
}

#[tokio::test]
async fn index_requires_login() {
    let (_db, st) = setup().await;
    let mut anon = Client::anonymous(st.clone()).await;
    let (status, _) = anon.send(Method::GET, "/composer_messages.json", "").await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn index_delegates_args_to_the_finder() {
    let (db, st) = setup().await;
    let t = old_topic(&db.pool).await;
    fabricate_user(&db.pool, "ctrl_member").await;
    let mut client = Client::logged_in(st.clone(), "ctrl_member").await;

    let (status, body) = client
        .json(Method::GET, "/composer_messages.json", "")
        .await;
    assert_eq!(status, 200, "{body}");

    let (status, body) = client
        .json(
            Method::GET,
            &format!("/composer_messages.json?topic_id={t}&post_id=333&composer_action=reply"),
            "",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["composer_messages"][0]["id"], "education");
    assert_eq!(body["__rest_serializer"], "1");
}

#[tokio::test]
async fn duplicate_lookup_leaves_out_hidden_posts() {
    let (db, st) = setup().await;
    let t = old_topic(&db.pool).await;
    let poster = fabricate_user(&db.pool, "dup_poster").await;
    refresh_auto_groups(&db.pool, poster).await;
    fabricate_user(&db.pool, "dup_member").await;
    let mut poster = Client::logged_in(st.clone(), "dup_poster").await;
    let shown = reply(
        &mut poster,
        t,
        "Check out https://example.com/public-doc for details",
    )
    .await;
    let hidden = reply(
        &mut poster,
        t,
        "Check out https://example.com/private-doc for details",
    )
    .await;
    // reply.hide!(PostActionType.types[:spam])
    exec(
        &db.pool,
        &format!("UPDATE posts SET hidden = TRUE, hidden_at = now(), hidden_reason_id = 1 WHERE id = {hidden}"),
    )
    .await;

    let mut member = Client::logged_in(st.clone(), "dup_member").await;
    let (status, body) = member
        .json(
            Method::GET,
            &format!("/composer_messages.json?topic_id={t}&composer_action=reply"),
            "",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let lookup = &body["extras"]["duplicate_lookup"];
    assert!(lookup.get("example.com/private-doc").is_none(), "{lookup}");
    let found = &lookup["example.com/public-doc"];
    assert_eq!(found["domain"], "example.com");
    assert_eq!(found["username"], "dup_poster");
    let post_number: i64 =
        sqlx::query_scalar("SELECT post_number::bigint FROM posts WHERE id = $1")
            .bind(shown)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(found["post_number"], post_number);
    assert!(found["posted_at"].as_str().unwrap().ends_with('Z'));
}

#[tokio::test]
async fn duplicate_lookup_leaves_out_restricted_internal_links() {
    let (db, st) = setup().await;
    let t = old_topic(&db.pool).await;
    let member = fabricate_user(&db.pool, "restricted_member").await;
    // user.change_trust_level!(TrustLevel[1])
    refresh_auto_groups(&db.pool, member).await;
    let staff_author = fabricate_user(&db.pool, "restricted_author").await;
    // a restricted category (staff only), a message and a hidden post
    let restricted = topic(&db.pool, 1, Some(3), "now()").await;
    exec(
        &db.pool,
        &format!(
            "UPDATE topics SET title = 'Secured category secret title', slug = 'secured-category-secret-title' WHERE id = {restricted}"
        ),
    )
    .await;
    post(&db.pool, restricted, 1, None).await;
    let pm = topic(&db.pool, staff_author, None, "now()").await;
    exec(
        &db.pool,
        &format!(
            "UPDATE topics SET title = 'Private message secret title', slug = 'private-message-secret-title' WHERE id = {pm}"
        ),
    )
    .await;
    post(&db.pool, pm, staff_author, None).await;
    let visible = topic(&db.pool, staff_author, Some(4), "now()").await;
    post(&db.pool, visible, staff_author, None).await;
    let hidden_target = post(&db.pool, visible, staff_author, None).await;
    exec(
        &db.pool,
        &format!("UPDATE posts SET hidden = TRUE, hidden_at = now(), hidden_reason_id = 1 WHERE id = {hidden_target}"),
    )
    .await;

    let restricted_url = format!("http://test.localhost/t/{restricted}");
    let pm_url = format!("http://test.localhost/t/{pm}");
    let hidden_target_url = format!("http://test.localhost/t/a-topic/{visible}/2");
    let mut client = Client::logged_in(st.clone(), "restricted_member").await;
    let created = reply(
        &mut client,
        t,
        &format!("Check out {restricted_url}, {pm_url}, and {hidden_target_url}"),
    )
    .await;

    let mut urls: Vec<String> =
        sqlx::query_scalar("SELECT url FROM topic_links WHERE post_id = $1 ORDER BY url")
            .bind(created)
            .fetch_all(&db.pool)
            .await
            .unwrap();
    urls.sort();
    let mut expected = vec![
        restricted_url.clone(),
        pm_url.clone(),
        hidden_target_url.clone(),
    ];
    expected.sort();
    assert_eq!(urls, expected);
    let reflections: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM topic_links WHERE topic_id = ANY($1) AND link_post_id = $2 AND reflection",
    )
    .bind(vec![restricted, pm])
    .bind(created)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(reflections, 0);

    let (status, text) = client
        .send(
            Method::GET,
            &format!("/composer_messages.json?topic_id={t}&composer_action=reply"),
            "",
        )
        .await;
    assert_eq!(status, 200, "{text}");
    let body: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(body["extras"]["duplicate_lookup"], serde_json::json!({}));
    assert!(!text.contains("secured-category-secret-title"));
    assert!(!text.contains("private-message-secret-title"));
}

#[tokio::test]
async fn duplicate_lookup_leaves_out_posts_converted_to_whispers() {
    let (db, st) = setup().await;
    let t = old_topic(&db.pool).await;
    fabricate_user(&db.pool, "whisper_member").await;
    set_setting(&db.pool, "whispers_allowed_groups", GROUP_LIST, "3").await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let created = reply(
        &mut admin,
        t,
        "Check out https://staff.example.com/private-doc for details",
    )
    .await;
    let (status, text) = admin
        .send(
            Method::PUT,
            &format!("/posts/{created}/post_type.json"),
            "post_type=4",
        )
        .await;
    assert_eq!(status, 200, "{text}");

    let mut member = Client::logged_in(st.clone(), "whisper_member").await;
    let (status, body) = member
        .json(
            Method::GET,
            &format!("/composer_messages.json?topic_id={t}&composer_action=reply"),
            "",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body["extras"]["duplicate_lookup"]
            .get("staff.example.com/private-doc")
            .is_none(),
        "{body}"
    );
    // still there for staff, who see whispers
    let (_, body) = admin
        .json(
            Method::GET,
            &format!("/composer_messages.json?topic_id={t}&composer_action=reply"),
            "",
        )
        .await;
    assert!(
        body["extras"]["duplicate_lookup"]
            .get("staff.example.com/private-doc")
            .is_some(),
        "{body}"
    );
}

#[tokio::test]
async fn user_not_seen_in_a_while_requires_login_and_usernames() {
    let (db, st) = setup().await;
    let names = unseen_users(&db, "ctrl_unseen").await;
    let query = names
        .iter()
        .map(|n| format!("usernames[]={n}"))
        .collect::<Vec<_>>()
        .join("&");
    let mut anon = Client::anonymous(st.clone()).await;
    let (status, _) = anon
        .send(
            Method::GET,
            &format!("/composer_messages/user_not_seen_in_a_while.json?{query}"),
            "",
        )
        .await;
    assert_eq!(status, 403);

    fabricate_user(&db.pool, "ctrl_unseen_member").await;
    set_setting(&db.pool, "pm_warn_user_last_seen_months_ago", INTEGER, "24").await;
    let mut client = Client::logged_in(st.clone(), "ctrl_unseen_member").await;
    let (status, _) = client
        .send(
            Method::GET,
            "/composer_messages/user_not_seen_in_a_while.json",
            "",
        )
        .await;
    assert_eq!(status, 400);

    // returns users that have not been seen recently
    let (status, body) = client
        .json(
            Method::GET,
            &format!("/composer_messages/user_not_seen_in_a_while.json?{query}"),
            "",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["user_count"], 2);
    assert_eq!(body["usernames"], serde_json::json!([names[0], names[1]]));
    assert!(
        body["time_ago"].as_str().unwrap().ends_with("2 years ago"),
        "{body}"
    );

    // accounts for pm_warn_user_last_seen_months_ago site setting
    set_setting(&db.pool, "pm_warn_user_last_seen_months_ago", INTEGER, "30").await;
    let (status, body) = client
        .json(
            Method::GET,
            &format!("/composer_messages/user_not_seen_in_a_while.json?{query}"),
            "",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["user_count"], 1);
    assert_eq!(body["usernames"], serde_json::json!([names[0]]));
}
