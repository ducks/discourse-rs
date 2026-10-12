//! What presence says on the bus: a member entering (their first client)
//! and leaving (their last, or the sweep taking an expired one), to those
//! who may view the channel.

mod common;

use common::{TestDb, config, heard, state};
use discourse_rs::config::RailsEnv;
use discourse_rs::presence;
use discourse_rs::site_settings::SiteSettings;
use serde_json::json;

async fn enter(st: &discourse_rs::AppState, name: &str, user_id: i32, client_id: &str) {
    let mut tx = st.pool.begin().await.unwrap();
    let settings = SiteSettings::load(&mut tx, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap();
    let urls = discourse_rs::url::Urls {
        config: &st.config,
        settings: &settings,
    };
    let config = presence::config(&mut tx, &settings, name)
        .await
        .unwrap()
        .unwrap();
    presence::present(&mut tx, &st.bus, &urls, name, &config, user_id, client_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

async fn leave(st: &discourse_rs::AppState, name: &str, user_id: i32, client_id: &str) {
    let mut tx = st.pool.begin().await.unwrap();
    let settings = SiteSettings::load(&mut tx, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap();
    let urls = discourse_rs::url::Urls {
        config: &st.config,
        settings: &settings,
    };
    let config = presence::config(&mut tx, &settings, name)
        .await
        .unwrap()
        .unwrap();
    presence::leave(&mut tx, &st.bus, &urls, name, &config, user_id, client_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn members_are_announced_entering_and_leaving() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let from = st.bus.now().await.unwrap();
    // user1's two tabs in a public channel's reply presence: one entering,
    // then one leaving when the second tab goes too.
    enter(&st, "/chat-reply/2", 3, "a").await;
    enter(&st, "/chat-reply/2", 3, "b").await;
    enter(&st, "/chat-reply/2", 3, "a").await;
    leave(&st, "/chat-reply/2", 3, "a").await;
    leave(&st, "/chat-reply/2", 3, "b").await;
    let messages = heard(&st, from, "/presence/chat-reply/2", 4, 2).await;
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert_eq!(messages[0]["entering_users"][0]["username"], "user1");
    assert_eq!(messages[0]["entering_users"][0]["id"], 3);
    assert_eq!(messages[1], json!({ "leaving_user_ids": [3] }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restricted_channel_tells_only_its_groups() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let from = st.bus.now().await.unwrap();
    // Channel 1's category (Staff) is read restricted: staff hear it, user1
    // doesn't.
    enter(&st, "/chat-reply/1", 1, "a").await;
    let staff = heard(&st, from, "/presence/chat-reply/1", 1, 1).await;
    assert_eq!(staff.len(), 1);
    // Delivered by now (staff heard it), so one look is enough.
    let member = heard(&st, from, "/presence/chat-reply/1", 3, 0).await;
    assert!(member.is_empty(), "{member:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sweep_takes_expired_clients() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    // From before anyone entered: the bus position is its delivery horizon,
    // which other transactions in the cluster can hold behind a commit.
    let from = st.bus.now().await.unwrap();
    enter(&st, "/chat/online", 3, "a").await;
    enter(&st, "/chat/online", 4, "a").await;
    sqlx::query(
        "UPDATE discourse_rs.presence_clients SET expires_at = now() - interval '1 second' WHERE user_id = 3",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    presence::auto_leave_all(&st).await.unwrap();
    let messages = heard(&st, from, "/presence/chat/online", 4, 3).await;
    assert_eq!(messages.len(), 3, "{messages:?}");
    assert_eq!(messages[0]["entering_users"][0]["id"], 3);
    assert_eq!(messages[1]["entering_users"][0]["id"], 4);
    assert_eq!(messages[2], json!({ "leaving_user_ids": [3] }));
    let left: Vec<i32> = sqlx::query_scalar(
        "SELECT user_id FROM discourse_rs.presence_clients WHERE channel = '/chat/online' ORDER BY user_id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(left, vec![4]);
}
