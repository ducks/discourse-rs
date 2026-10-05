//! TopicTrackingState's messages: topic lists and the new and unread
//! counts kept live (`/new`, `/latest`, `/unread`, `/unread/<id>`), and
//! who hears them.

mod common;

use common::{TestDb, bus_messages, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use discourse_rs::site_settings::SiteSettings;
use discourse_rs::topic_tracking_state as tracking;
use serde_json::json;
use sqlx::PgPool;

async fn settings(st: &AppState) -> SiteSettings {
    let mut conn = st.pool.acquire().await.unwrap();
    SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap()
}

/// A reply in a public regular topic: (post id, topic id, category id,
/// author, post number).
async fn public_reply(pool: &PgPool) -> (i32, i32, i32, i32, i32) {
    sqlx::query_as(
        "SELECT p.id, p.topic_id, t.category_id, p.user_id, p.post_number FROM posts p \
         JOIN topics t ON t.id = p.topic_id JOIN categories c ON c.id = t.category_id \
         WHERE t.archetype = 'regular' AND p.post_number > 1 AND p.post_type = 1 \
           AND p.user_id > 0 AND p.deleted_at IS NULL AND t.deleted_at IS NULL \
           AND NOT c.read_restricted \
         ORDER BY p.id LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn new_and_latest_topics_reach_who_may_see_them() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let s = settings(&st).await;
    let (_, topic_id, category_id, _, _) = public_reply(&db.pool).await;
    let owner: i32 = sqlx::query_scalar("SELECT user_id FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();

    let from = st.bus.now().await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    tracking::publish_new(&st.bus, &s, &mut tx, topic_id)
        .await
        .unwrap();
    tracking::publish_latest(&st.bus, &s, &mut tx, topic_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let unread_owner = format!("/unread/{owner}");
    let messages = bus_messages(&st, from, &["/new", "/latest", &unread_owner]).await;
    assert_eq!(messages.len(), 3, "{messages:?}");
    let new = &messages[0];
    assert_eq!(new.channel, "/new");
    assert_eq!(new.audience, None);
    assert_eq!(new.data["message_type"], "new_topic");
    assert_eq!(new.data["topic_id"], topic_id);
    assert_eq!(new.data["payload"]["highest_post_number"], 1);
    assert_eq!(new.data["payload"]["created_in_new_period"], true);
    // publish_new ends with the author's read of the first post.
    let read = &messages[1];
    assert_eq!(read.channel, unread_owner);
    assert_eq!(read.audience, Some(vec![format!("user:{owner}")]));
    assert_eq!(read.data["message_type"], "read");
    assert_eq!(read.data["payload"]["last_read_post_number"], 1);
    let latest = &messages[2];
    assert_eq!(latest.channel, "/latest");
    assert_eq!(latest.audience, None);
    assert_eq!(latest.data["message_type"], "latest");
    assert!(latest.data["payload"]["bumped_at"].is_string());

    // A read-restricted category: admins and its groups.
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
    sqlx::query(
        "INSERT INTO category_groups (category_id, group_id, permission_type, created_at, updated_at) \
         VALUES ($1, 3, 1, now(), now())",
    )
    .bind(category_id)
    .execute(&db.pool)
    .await
    .unwrap();
    let from = st.bus.now().await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    tracking::publish_latest(&st.bus, &s, &mut tx, topic_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let messages = bus_messages(&st, from, &["/latest"]).await;
    assert_eq!(
        messages[0].audience,
        Some(vec!["group:1".to_string(), "group:3".to_string()])
    );

    // Deleting it tells the same audience. (A regular topic always has a
    // category, so secure_category_group_ids' admins-only case is moot.)
    let from = st.bus.now().await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    tracking::publish_delete(&st.bus, &mut tx, topic_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let messages = bus_messages(&st, from, &["/delete"]).await;
    assert_eq!(
        messages[0].audience,
        Some(vec!["group:1".to_string(), "group:3".to_string()])
    );
    assert_eq!(
        messages[0].data,
        json!({ "topic_id": topic_id, "message_type": "delete" })
    );

    // Messages are left to PrivateMessageTopicTrackingState.
    let pm: i32 = sqlx::query_scalar(
        "SELECT id FROM topics WHERE archetype = 'private_message' ORDER BY id LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let from = st.bus.now().await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    tracking::publish_latest(&st.bus, &s, &mut tx, pm)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(bus_messages(&st, from, &["/latest"]).await.is_empty());
}

/// The job a new post enqueues: the topic's watchers seen lately hear it
/// is unmuted, its muters that it is muted, its trackers other than the
/// author that it has an unread post, everyone that it was bumped.
#[tokio::test(flavor = "multi_thread")]
async fn a_reply_reaches_trackers_but_not_its_author() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (post_id, topic_id, _, author, post_number) = public_reply(&db.pool).await;
    let others: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM users WHERE id > 0 AND id <> $1 ORDER BY id LIMIT 3")
            .bind(author)
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(others.len(), 3, "the seed has enough users");
    let (tracker, regular, muter) = (others[0], others[1], others[2]);
    sqlx::query("DELETE FROM topic_users WHERE topic_id = $1")
        .bind(topic_id)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM category_users")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM tag_users")
        .execute(&db.pool)
        .await
        .unwrap();
    for (user, level) in [(author, 3), (tracker, 2), (regular, 1), (muter, 0)] {
        sqlx::query(
            "INSERT INTO topic_users (user_id, topic_id, notification_level) VALUES ($1, $2, $3)",
        )
        .bind(user)
        .bind(topic_id)
        .bind(level)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    sqlx::query("UPDATE users SET last_seen_at = now() WHERE id = ANY($1)")
        .bind(vec![author, tracker, regular, muter])
        .execute(&db.pool)
        .await
        .unwrap();

    let from = st.bus.now().await.unwrap();
    let job: i64 = sqlx::query_scalar(
        "INSERT INTO discourse_rs.jobs (name, args) VALUES ('post_update_topic_tracking_state', $1) \
         RETURNING id",
    )
    .bind(json!({ "post_id": post_id }))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    discourse_rs::jobs::perform_now(&st, job).await.unwrap();
    let left: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM discourse_rs.jobs WHERE id = $1)")
            .bind(job)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(!left, "the job ran and was removed");

    let messages = bus_messages(&st, from, &["/latest", "/unread"]).await;
    let types: Vec<(&str, &str)> = messages
        .iter()
        .map(|m| (m.channel.as_str(), m.data["message_type"].as_str().unwrap()))
        .collect();
    assert_eq!(
        types,
        vec![
            ("/latest", "unmuted"),
            ("/latest", "muted"),
            ("/unread", "unread"),
            ("/latest", "latest"),
        ]
    );
    let tags = |ids: &[i32]| -> Option<Vec<String>> {
        let mut ids = ids.to_vec();
        ids.sort();
        Some(ids.into_iter().map(discourse_rs::bus::user_tag).collect())
    };
    // Any level above muted counts as unmuted (User.watching_topic), the
    // author included.
    assert_eq!(messages[0].audience, tags(&[author, tracker, regular]));
    assert_eq!(messages[1].audience, tags(&[muter]));
    assert_eq!(messages[2].audience, tags(&[tracker]));
    assert_eq!(
        messages[2].data["payload"]["highest_post_number"],
        post_number
    );
    assert_eq!(messages[3].audience, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn reading_tells_the_reader_where_they_are() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let s = settings(&st).await;
    let (_, topic_id, _, author, _) = public_reply(&db.pool).await;
    let reader: i32 = sqlx::query_scalar(
        "SELECT u.id FROM users u WHERE u.id > 0 AND u.id <> $1 AND NOT u.admin AND NOT u.moderator \
           AND EXISTS (SELECT 1 FROM user_options o WHERE o.user_id = u.id) ORDER BY u.id LIMIT 1",
    )
    .bind(author)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM topic_users WHERE topic_id = $1 AND user_id = $2")
        .bind(topic_id)
        .bind(reader)
        .execute(&db.pool)
        .await
        .unwrap();
    let mut conn = db.pool.acquire().await.unwrap();
    let user = discourse_rs::session::current::SessionUser::load(&mut conn, reader)
        .await
        .unwrap()
        .unwrap();
    let guardian = discourse_rs::guardian::Guardian::for_user(&mut conn, &user)
        .await
        .unwrap();
    drop(conn);
    let read = |n: i64| {
        let (st, s, guardian) = (&st, &s, &guardian);
        async move {
            let from = st.bus.now().await.unwrap();
            let mut tx = st.pool.begin().await.unwrap();
            discourse_rs::read_tracking::process_timings(
                &st.bus,
                &mut tx,
                s,
                guardian,
                topic_id,
                1000,
                vec![(n, 1000)],
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
            let unread = format!("/unread/{reader}");
            let topic = format!("/topic/{topic_id}");
            bus_messages(st, from, &[&unread, &topic]).await
        }
    };

    // A first read: the position, then the new topic user's level.
    let messages = read(1).await;
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert_eq!(messages[0].channel, format!("/unread/{reader}"));
    assert_eq!(messages[0].audience, Some(vec![format!("user:{reader}")]));
    assert_eq!(messages[0].data["payload"]["last_read_post_number"], 1);
    assert_eq!(messages[1].channel, format!("/topic/{topic_id}"));
    assert_eq!(messages[1].audience, Some(vec![format!("user:{reader}")]));
    assert!(messages[1].data["notification_level_change"].is_number());

    // Reading further moves it; reading the same post again does not.
    let messages = read(2).await;
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0].data["payload"]["last_read_post_number"], 2);
    assert!(read(2).await.is_empty());
}
