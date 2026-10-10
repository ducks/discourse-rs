//! What chat's notification jobs alert: NotifyMentioned and NotifyWatching
//! publish to the member's /chat/notification-alert, which only they hear.

mod common;

use common::{TestDb, config, state};
use discourse_rs::config::RailsEnv;
use discourse_rs::site_settings::SiteSettings;
use serde_json::{Value, json};

async fn message(db: &TestDb, text: &str) {
    for sql in [
        format!(
            "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
             VALUES (1, 2, 3, now(), now(), '{text}', '<p>{text}</p>', 1, 3)"
        ),
        "UPDATE chat_channels SET last_message_id = 1 WHERE id = 2".into(),
    ] {
        sqlx::query(&sql).execute(&db.pool).await.unwrap();
    }
}

async fn alerts(
    st: &discourse_rs::AppState,
    from: pg_bus::Position,
    user_id: i32,
    want: usize,
) -> Vec<Value> {
    heard(
        st,
        from,
        &format!("/chat/notification-alert/{user_id}"),
        user_id,
        want,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn watching_members_hear_each_new_message() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    message(&db, "for :heart: watchers").await;
    // user0 wants every message; user2 joins but doesn't.
    sqlx::query("UPDATE user_chat_channel_memberships SET notification_level = 2 WHERE chat_channel_id = 2 AND user_id = 2")
        .execute(&db.pool)
        .await
        .unwrap();
    let from = st.bus.now().await.unwrap();
    let mut conn = st.pool.acquire().await.unwrap();
    let settings = SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap();
    discourse_rs::plugins::chat::notify::notify_watching(
        &st,
        &mut conn,
        &settings,
        &json!({ "chat_message_id": 1, "except_user_ids": [3], "timestamp": "2026-10-10 00:00:00 UTC" }),
    )
    .await
    .unwrap();
    let heard = alerts(&st, from, 2, 1).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    let alert = &heard[0];
    assert_eq!(alert["notification_type"], 30);
    assert_eq!(alert["username"], "user1");
    assert_eq!(alert["post_url"], "/chat/c/-/2/1");
    assert_eq!(
        alert["translated_title"],
        "user1 sent a message in \"General\""
    );
    assert_eq!(alert["excerpt"], "for ❤ watchers");
    assert_eq!(
        alert["action_data"],
        json!({ "channel_id": 2, "message_id": 1 })
    );
    // The sender is excepted.
    assert!(alerts(&st, from, 3, 0).await.is_empty());

    // A member who has read past it, or mutes the channel, hears nothing.
    sqlx::query("UPDATE user_chat_channel_memberships SET muted = true WHERE chat_channel_id = 2 AND user_id = 2")
        .execute(&db.pool)
        .await
        .unwrap();
    let from = st.bus.now().await.unwrap();
    discourse_rs::plugins::chat::notify::notify_watching(
        &st,
        &mut conn,
        &settings,
        &json!({ "chat_message_id": 1, "except_user_ids": [3] }),
    )
    .await
    .unwrap();
    assert!(alerts(&st, from, 2, 0).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mentioned_member_is_notified_and_alerted() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    message(&db, "hey @user0").await;
    sqlx::query(
        "INSERT INTO chat_mentions (chat_message_id, type, target_id, created_at, updated_at) VALUES (1, 'Chat::UserMention', 2, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let from = st.bus.now().await.unwrap();
    let mut conn = st.pool.acquire().await.unwrap();
    let settings = SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap();
    discourse_rs::plugins::chat::notify::notify_mentioned(
        &st,
        &mut conn,
        &settings,
        &json!({
            "chat_message_id": 1,
            "to_notify_ids_map": { "direct_mentions": [2] },
            "already_notified_user_ids": [],
            "timestamp": "2020-01-01 00:00:00 UTC",
        }),
    )
    .await
    .unwrap();
    let heard = alerts(&st, from, 2, 1).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert_eq!(heard[0]["notification_type"], 29);
    assert_eq!(
        heard[0]["translated_title"],
        "user1 mentioned you in \"General\""
    );
    assert_eq!(heard[0]["post_url"], "/chat/c/general/2/1");
    let linked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chat_mention_notifications cmn JOIN notifications n ON n.id = cmn.notification_id \
         WHERE n.user_id = 2 AND n.notification_type = 29 AND n.high_priority",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(linked, 1);
}

/// What the sender hears on the channel of their mentions' fate.
async fn notices(st: &discourse_rs::AppState, text: &str, setup: &[&str]) -> Vec<Value> {
    sqlx::query(&format!(
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         VALUES (1, 2, 3, now(), now(), '{text}', '', 1, 3)"
    ))
    .execute(&st.pool)
    .await
    .unwrap();
    for sql in setup {
        sqlx::query(sql).execute(&st.pool).await.unwrap();
    }
    let from = st.bus.now().await.unwrap();
    let mut conn = st.pool.acquire().await.unwrap();
    let settings = SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap();
    discourse_rs::plugins::chat::create::process_message(
        st,
        &mut conn,
        &settings,
        &json!({ "chat_message_id": 1 }),
    )
    .await
    .unwrap();
    heard(st, from, "/chat/2", 3, 2)
        .await
        .into_iter()
        .filter(|d| d["type"] == "notice")
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sender_hears_who_their_mentions_missed() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    // user2 can join General but hasn't.
    let heard = notices(&st, "@user2 come here", &[]).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert_eq!(heard[0]["notice_type"], "mention_without_membership");
    assert_eq!(heard[0]["data"]["user_ids"], json!([4]));
    assert_eq!(
        heard[0]["data"]["text"],
        "user2 has not joined this channel."
    );
    assert_eq!(heard[0]["data"]["message_id"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn channel_wide_mentions_where_disallowed_are_noticed() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let heard = notices(
        &st,
        "@all standup",
        &["UPDATE chat_channels SET allow_channel_wide_mentions = false WHERE id = 2"],
    )
    .await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert_eq!(
        heard[0]["text_content"],
        "@here and @all mentions are disabled in this channel."
    );
}

/// The bus's messages on a channel from a position, as the user hears
/// them: waited for, since a message shows only once every older write
/// transaction in the cluster has ended (other tests' too).
async fn heard(
    st: &discourse_rs::AppState,
    from: pg_bus::Position,
    channel: &str,
    user_id: i32,
    want: usize,
) -> Vec<Value> {
    let mut conn = st.pool.acquire().await.unwrap();
    let tags = discourse_rs::bus::tags(&mut conn, Some(user_id))
        .await
        .unwrap();
    let filter = pg_bus::Filter {
        channels: vec![channel.to_string()],
        tags,
    };
    let mut out = Vec::new();
    for _ in 0..50 {
        out = st
            .bus
            .backlog(from, &filter, 10)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.data)
            .collect();
        if out.len() >= want {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    out
}
