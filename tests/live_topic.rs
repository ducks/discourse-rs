//! `/topic/<id>` messages (Post#publish_change_to_clients!, Topic.
//! publish_stats_to_clients!) and who hears them
//! (Topic#secure_audience_publish_messages).

mod common;

use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use discourse_rs::site_settings::SiteSettings;
use pg_bus::{Filter, Message, Position};
use serde_json::json;
use sqlx::PgPool;

/// Publishes the change in its own transaction, as a write path would.
async fn publish(st: &AppState, post_id: i32, kind: &str, skip_stats: bool) {
    let mut conn = st.pool.acquire().await.unwrap();
    let settings = SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap();
    drop(conn);
    let host = discourse_rs::pretty_text::Host::from_state(st);
    let ctx = discourse_rs::posting::Ctx {
        host: &host,
        settings: &settings,
        config: &st.config,
        i18n: &st.i18n,
        bus: &st.bus,
    };
    let mut tx = st.pool.begin().await.unwrap();
    discourse_rs::bus::publish_post_change(
        &ctx,
        &mut tx,
        post_id,
        kind,
        Default::default(),
        skip_stats,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

/// Everything on the topic's channel after `from`, whoever it is for.
async fn sent(st: &AppState, from: Position, topic_id: i32) -> Vec<Message> {
    common::bus_messages(st, from, &[&format!("/topic/{topic_id}")]).await
}

/// A reply in a public regular topic: (post id, topic id, category id).
async fn public_reply(pool: &PgPool) -> (i32, i32, i32) {
    sqlx::query_as(
        "SELECT p.id, p.topic_id, t.category_id FROM posts p JOIN topics t ON t.id = p.topic_id \
         JOIN categories c ON c.id = t.category_id \
         WHERE t.archetype = 'regular' AND p.post_number > 1 AND p.post_type = 1 \
           AND p.deleted_at IS NULL AND t.deleted_at IS NULL AND NOT c.read_restricted \
         ORDER BY p.id LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_public_topic_tells_everyone_with_its_stats() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (post_id, topic_id, _) = public_reply(&db.pool).await;
    let (post_number, posts_count): (i32, i32) = sqlx::query_as(
        "SELECT p.post_number, t.posts_count FROM posts p JOIN topics t ON t.id = p.topic_id \
         WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let from = st.bus.now().await.unwrap();

    publish(&st, post_id, "created", false).await;
    let messages = sent(&st, from, topic_id).await;
    assert_eq!(messages.len(), 2, "{messages:?}");
    let change = &messages[0];
    assert_eq!(change.audience, None, "public: everyone, anonymous too");
    assert_eq!(change.data["type"], "created");
    assert_eq!(change.data["id"], post_id);
    assert_eq!(change.data["post_number"], post_number);
    assert!(change.data["username"].is_string(), "{}", change.data);
    let stats = &messages[1];
    assert_eq!(stats.data["type"], "stats");
    assert_eq!(stats.data["id"], topic_id);
    assert_eq!(stats.data["posts_count"], posts_count);
    assert!(stats.data["last_poster"]["username"].is_string());

    // Revisions carry no username and no stats.
    let from = st.bus.now().await.unwrap();
    publish(&st, post_id, "revised", false).await;
    let messages = sent(&st, from, topic_id).await;
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0].data["type"], "revised");
    assert!(messages[0].data.get("username").is_none());

    // skip_topic_stats leaves the stats out.
    let from = st.bus.now().await.unwrap();
    publish(&st, post_id, "recovered", true).await;
    let messages = sent(&st, from, topic_id).await;
    assert_eq!(messages.len(), 1, "{messages:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restricted_category_tells_its_groups_only() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (post_id, topic_id, category_id) = public_reply(&db.pool).await;
    sqlx::query("UPDATE categories SET read_restricted = TRUE WHERE id = $1")
        .bind(category_id)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM category_groups WHERE category_id = $1")
        .bind(category_id)
        .execute(&db.pool)
        .await
        .unwrap();

    // No groups: MessageBus would publish to nobody, so nothing goes out.
    let from = st.bus.now().await.unwrap();
    publish(&st, post_id, "created", false).await;
    assert!(sent(&st, from, topic_id).await.is_empty());

    sqlx::query(
        "INSERT INTO category_groups (category_id, group_id, permission_type, created_at, updated_at) \
         VALUES ($1, 3, 1, now(), now())",
    )
    .bind(category_id)
    .execute(&db.pool)
    .await
    .unwrap();
    let from = st.bus.now().await.unwrap();
    publish(&st, post_id, "created", false).await;
    let messages = sent(&st, from, topic_id).await;
    assert_eq!(messages.len(), 2, "{messages:?}");
    for m in &messages {
        assert_eq!(m.audience, Some(vec!["group:3".to_string()]));
    }
    // An anonymous viewer holds no tags and hears none of it.
    let heard = st
        .bus
        .backlog(
            from,
            &Filter {
                channels: vec![format!("/topic/{topic_id}")],
                tags: vec![],
            },
            100,
        )
        .await
        .unwrap();
    assert!(heard.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn whispers_and_messages_go_to_who_may_read_them() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let staff: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM users WHERE id > 0 AND (admin OR moderator) ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert!(!staff.is_empty());

    // A whisper: human staff and the author.
    let (post_id, topic_id, _) = public_reply(&db.pool).await;
    let author: i32 =
        sqlx::query_scalar("UPDATE posts SET post_type = 4 WHERE id = $1 RETURNING user_id")
            .bind(post_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let from = st.bus.now().await.unwrap();
    publish(&st, post_id, "revised", false).await;
    let messages = sent(&st, from, topic_id).await;
    assert_eq!(messages.len(), 1);
    let mut expected: Vec<i32> = staff.clone();
    expected.push(author);
    expected.sort();
    expected.dedup();
    let expected: Vec<String> = expected
        .into_iter()
        .map(discourse_rs::bus::user_tag)
        .collect();
    assert_eq!(messages[0].audience, Some(expected));

    // A message: human staff and the participants.
    let (pm_post, pm_topic): (i32, i32) = sqlx::query_as(
        "SELECT p.id, t.id FROM posts p JOIN topics t ON t.id = p.topic_id \
         WHERE t.archetype = 'private_message' AND t.deleted_at IS NULL \
           AND EXISTS (SELECT 1 FROM topic_allowed_users a WHERE a.topic_id = t.id AND a.user_id > 0) \
         ORDER BY p.id LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let participants: Vec<i32> =
        sqlx::query_scalar("SELECT user_id FROM topic_allowed_users WHERE topic_id = $1")
            .bind(pm_topic)
            .fetch_all(&db.pool)
            .await
            .unwrap();
    let from = st.bus.now().await.unwrap();
    publish(&st, pm_post, "revised", false).await;
    let messages = sent(&st, from, pm_topic).await;
    assert_eq!(messages.len(), 1);
    let audience = messages[0].audience.clone().unwrap();
    for id in staff.iter().chain(&participants) {
        assert!(
            audience.contains(&discourse_rs::bus::user_tag(*id)),
            "{id} missing from {audience:?}"
        );
    }
    assert_eq!(messages[0].data["type"], json!("revised"));
}
