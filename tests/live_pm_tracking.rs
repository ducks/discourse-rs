//! PrivateMessageTopicTrackingState's messages: a new message, a reply
//! and a read, each on the users' own channels, as the fixtures' message
//! 42 (user1 to user0; post 49 is user0's reply) has them.

mod common;

use common::{TestDb, bus_messages, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use discourse_rs::pm_tracking_state as pm;
use discourse_rs::site_settings::SiteSettings;
use serde_json::{Value, json};

const USER0: i32 = 2;
const USER1: i32 = 3;

async fn settings(st: &AppState) -> SiteSettings {
    let mut conn = st.pool.acquire().await.unwrap();
    SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap()
}

fn channel(user_id: i32) -> String {
    format!("/private-message-topic-tracking-state/user/{user_id}")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_message_reaches_its_recipients_not_its_author() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let from = st.bus.now().await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    pm::publish_new(&st.bus, &mut tx, 42).await.unwrap();
    tx.commit().await.unwrap();
    let (c0, c1) = (channel(USER0), channel(USER1));
    let messages = bus_messages(&st, from, &[&c0, &c1]).await;
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0].channel, c0);
    assert_eq!(messages[0].audience, Some(vec![format!("user:{USER0}")]));
    assert_eq!(
        messages[0].data,
        json!({
            "message_type": "new_topic",
            "topic_id": 42,
            "payload": {
                "last_read_post_number": Value::Null,
                "highest_post_number": 1,
                "group_ids": [],
                "created_by_user_id": USER1,
            },
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_is_unread_for_the_others_tracking_it() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let s = settings(&st).await;
    let from = st.bus.now().await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    pm::publish_unread(&st.bus, &s, &mut tx, 49).await.unwrap();
    tx.commit().await.unwrap();
    let (c0, c1) = (channel(USER0), channel(USER1));
    let messages = bus_messages(&st, from, &[&c0, &c1]).await;
    // user0 wrote it; user1 watches with post 1 read.
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0].channel, c1);
    assert_eq!(
        messages[0].data,
        json!({
            "topic_id": 42,
            "message_type": "unread",
            "payload": {
                "last_read_post_number": 1,
                "highest_post_number": 2,
                "notification_level": 3,
                "group_ids": [],
                "created_by_user_id": USER0,
            },
        })
    );
    // Muted, or never read a message from before their new-topic start:
    // left out.
    sqlx::query(
        "UPDATE topic_users SET notification_level = 1 WHERE topic_id = 42 AND user_id = $1",
    )
    .bind(USER1)
    .execute(&db.pool)
    .await
    .unwrap();
    let from = st.bus.now().await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    pm::publish_unread(&st.bus, &s, &mut tx, 49).await.unwrap();
    tx.commit().await.unwrap();
    assert!(bus_messages(&st, from, &[&c1]).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_goes_to_the_reader() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let s = settings(&st).await;
    let from = st.bus.now().await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    pm::publish_read(&st.bus, &s, &mut tx, 42, Some(2), USER1, Some(3))
        .await
        .unwrap();
    pm::publish_read(&st.bus, &s, &mut tx, 42, None, USER0, None)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let (c0, c1) = (channel(USER0), channel(USER1));
    let messages = bus_messages(&st, from, &[&c0, &c1]).await;
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert_eq!(messages[0].channel, c1);
    assert_eq!(
        messages[0].data,
        json!({
            "message_type": "read",
            "topic_id": 42,
            "payload": {"last_read_post_number": 2, "notification_level": 3, "highest_post_number": 2},
        })
    );
    assert_eq!(
        messages[1].data["payload"]["last_read_post_number"],
        Value::Null
    );
}
