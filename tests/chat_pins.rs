//! Pinned chat messages: POST and DELETE .../messages/:id/pin with their
//! publishes, GET .../pins (Chat::ChannelPinsSerializer) and PUT
//! .../pins/read.

#[path = "common/chat_client.rs"]
mod chat_client;
mod common;

use chat_client::Client;
use common::{TestDb, config, heard, set_setting, state};
use discourse_rs::config::RailsEnv;
use serde_json::json;

async fn messages(db: &TestDb) {
    for sql in [
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         VALUES (1, 2, 2, now() - interval '50 minutes', now() - interval '50 minutes', 'first **one**', '<p>first <strong>one</strong></p>', 1, 2), \
                (2, 2, 3, now() - interval '40 minutes', now() - interval '40 minutes', 'see [this](https://example.com/x) and more', '<p>see <a href=\"https://example.com/x\">this</a> and more</p>', 1, 3)",
        "SELECT setval('chat_messages_id_seq', 2)",
        "UPDATE chat_channels SET last_message_id = 2 WHERE id = 2",
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
    set_setting(&db.pool, "chat_pinned_messages", 5, "t").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn staff_pin_and_unpin_with_publishes() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;

    let from = st.bus.now().await.unwrap();
    let (status, body) = admin
        .request("POST", "/chat/api/channels/2/messages/2/pin", "")
        .await;
    assert_eq!((status, body), (200, json!({"success": "OK"})));
    let heard_pin = heard(&st, from, "/chat/2", 1, 1).await;
    assert_eq!(heard_pin.len(), 1, "{heard_pin:?}");
    assert_eq!(heard_pin[0]["type"], "pin");
    assert_eq!(heard_pin[0]["chat_message_id"], 2);
    assert_eq!(heard_pin[0]["pinned_by_id"], 1);
    assert_eq!(heard_pin[0]["pinned_message_count"], 1);

    // Pinned already.
    let (status, body) = admin
        .request("POST", "/chat/api/channels/2/messages/2/pin", "")
        .await;
    assert_eq!(status, 422);
    assert_eq!(body["error"], "This message is already pinned.");

    // The list, its excerpt keeping links.
    let (status, body) = admin.get("/chat/api/channels/2/pins").await;
    assert_eq!(status, 200, "{body}");
    let pins = body["pinned_messages"].as_array().unwrap();
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0]["chat_message_id"], 2);
    assert_eq!(
        pins[0]["excerpt"],
        "see <a href=\"https://example.com/x\">this</a> and more"
    );
    assert_eq!(pins[0]["pinned_by"]["username"], "admin");
    assert_eq!(pins[0]["message"]["id"], 2);
    assert_eq!(pins[0]["message"]["pinned"], true);
    let keys: Vec<&String> = pins[0].as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "id",
            "chat_message_id",
            "pinned_at",
            "excerpt",
            "pinned_by",
            "message"
        ]
    );
    assert_eq!(body["membership"]["chat_channel_id"], 2);

    let from = st.bus.now().await.unwrap();
    let (status, _) = admin
        .request("DELETE", "/chat/api/channels/2/messages/2/pin", "")
        .await;
    assert_eq!(status, 200);
    let heard_unpin = heard(&st, from, "/chat/2", 1, 1).await;
    assert_eq!(heard_unpin[0]["type"], "unpin");
    assert_eq!(heard_unpin[0]["unpinned_by_id"], 1);
    assert_eq!(heard_unpin[0]["pinned_message_count"], 0);
    let (status, body) = admin
        .request("DELETE", "/chat/api/channels/2/messages/2/pin", "")
        .await;
    assert_eq!(status, 422);
    assert_eq!(body["error"], "This message is not pinned.");

    // No such message.
    let (status, _) = admin
        .request("POST", "/chat/api/channels/2/messages/99/pin", "")
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn members_outside_the_pinning_groups_cant_pin() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    let mut user = Client::logged_in(st, "user1").await;
    let (status, _) = user
        .request("POST", "/chat/api/channels/2/messages/1/pin", "")
        .await;
    assert_eq!(status, 403);
    // They still see the pins and mark them read.
    let (status, body) = user.get("/chat/api/channels/2/pins").await;
    assert_eq!(status, 200);
    assert_eq!(body["pinned_messages"], json!([]));
    let (status, body) = user
        .request("PUT", "/chat/api/channels/2/pins/read", "")
        .await;
    assert_eq!((status, body), (200, json!({"success": "OK"})));
    let viewed: bool = sqlx::query_scalar(
        "SELECT last_viewed_pins_at IS NOT NULL FROM user_chat_channel_memberships WHERE user_id = 3 AND chat_channel_id = 2",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(viewed);
    // The staff channel's pins are not theirs to see.
    let (status, _) = user.get("/chat/api/channels/1/pins").await;
    assert_eq!(status, 403);
    let (status, _) = user.get("/chat/api/channels/abc/pins").await;
    assert_eq!(status, 400);

    set_setting(&db.pool, "chat_pinned_messages", 5, "f").await;
    let (status, _) = user.get("/chat/api/channels/2/pins").await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn the_twentieth_pin_is_the_last() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    sqlx::query(
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         SELECT g, 2, 2, now(), now(), 'm' || g, '<p>m' || g || '</p>', 1, 2 FROM generate_series(10, 29) g",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO chat_pinned_messages (chat_message_id, chat_channel_id, pinned_by_id, created_at, updated_at) \
         SELECT g, 2, 1, now(), now() FROM generate_series(10, 29) g",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, body) = admin
        .request("POST", "/chat/api/channels/2/messages/1/pin", "")
        .await;
    assert_eq!(status, 422);
    assert_eq!(
        body["error"],
        "This channel has reached the maximum number of pinned messages (20)."
    );
}

#[tokio::test]
async fn the_pins_route_lists_them_beside_the_channel() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    sqlx::query(
        "INSERT INTO chat_pinned_messages (chat_message_id, chat_channel_id, pinned_by_id, created_at, updated_at) \
         VALUES (1, 2, 1, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let mut user = Client::logged_in(st.clone(), "user1").await;
    let (status, html) = user.page("/chat/c/general/2/pins").await;
    assert_eq!(status, 200);
    assert!(html.contains("class=\"main-chat-outlet has-side-panel-expanded chat-view\""));
    assert!(html.contains(
        "<div class=\"chat-side-panel\"><div class=\"c-routes --channel-pins c-channel-pins\">"
    ));
    assert!(html.contains("<span class=\"c-navbar__title-text\">"));
    assert!(html.contains("class=\"c-navbar__close-pins-button btn-transparent btn btn-icon no-text\" title=\"Close pinned messages\" href=\"/chat/c/general/2\""));
    // Pinned by admin, never seen: unseen.
    assert!(html.contains("<div class=\"chat-pinned-message__pinned-by\"><svg class=\""));
    assert!(html.contains("chat-pinned-message__pinned-by-icon"));
    assert!(html.contains("<span>Pinned by admin</span>"));
    assert!(html.contains("<a class=\"chat-pinned-message__link\" href=\"/chat/c/general/2/1\"><div class=\"chat-message-container -persisted -processed -not-interactive\" data-id=\"1\""));
    assert!(html.contains("<span class=\"chat-message-info__username__name\">user0</span>"));
    assert!(html.contains("<span aria-hidden=\"true\" class=\"chat-user-avatar__container\">"));
    // Members outside the pinning groups get no unpin button.
    assert!(!html.contains("chat-pinned-message__unpin"));

    let mut admin = Client::logged_in(st, "admin").await;
    let (_, html) = admin.page("/chat/c/general/2/pins").await;
    assert!(html.contains("<span>Pinned by you</span>"));
    assert!(html.contains("class=\"btn no-text btn-icon btn-transparent chat-pinned-message__unpin\" aria-label=\"Unpin message\""));

    // A wrong slug goes to the channel's own, still the pins.
    let (status, _) = admin.page("/chat/c/wrong/2/pins").await;
    assert_eq!(status, 302);
    set_setting(&db.pool, "chat_pinned_messages", 5, "f").await;
    let (status, _) = admin.page("/chat/c/general/2/pins").await;
    assert_eq!(status, 404);
}
