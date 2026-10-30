//! Chat moderation: flagging a message (Chat::FlagMessage into a
//! Chat::ReviewableMessage, the auto silence and the flag publishes),
//! select mode's bulk delete (Chat::TrashMessages) and moving messages to
//! another channel (Chat::MessageMover).

#[path = "common/chat_client.rs"]
mod chat_client;
mod common;

use chat_client::Client;
use common::{TestDb, config, heard, state};
use discourse_rs::config::RailsEnv;
use serde_json::json;

/// General's messages: user0's two, user1's one.
async fn messages(db: &TestDb) {
    for sql in [
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         VALUES (1, 2, 2, now() - interval '50 minutes', now() - interval '50 minutes', 'buy stuff', '<p>buy stuff</p>', 1, 2), \
                (2, 2, 2, now() - interval '40 minutes', now() - interval '40 minutes', 'more stuff', '<p>more stuff</p>', 1, 2), \
                (3, 2, 3, now() - interval '30 minutes', now() - interval '30 minutes', 'mine', '<p>mine</p>', 1, 3)",
        "SELECT setval('chat_messages_id_seq', 3)",
        "UPDATE chat_channels SET last_message_id = 3 WHERE id = 2",
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
}

async fn flag_id(db: &TestDb, name: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM flags WHERE name_key = $1")
        .bind(name)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_flag_opens_a_reviewable_and_tells_the_flagger_and_staff() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    let spam = flag_id(&db, "spam").await;
    let mut user = Client::logged_in(st.clone(), "user1").await;

    let from = st.bus.now().await.unwrap();
    let (status, body) = user
        .request(
            "POST",
            "/chat/api/channels/2/messages/1/flags",
            &format!("flag_type_id={spam}"),
        )
        .await;
    assert_eq!((status, body), (200, json!({"success": "OK"})));
    let (kind, target_type, by, target_by, spam_flag, payload): (
        String,
        String,
        i32,
        Option<i32>,
        bool,
        serde_json::Value,
    ) = sqlx::query_as(
        "SELECT type, target_type, created_by_id, target_created_by_id, potential_spam, payload::json \
         FROM reviewables WHERE target_id = 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(kind, "Chat::ReviewableMessage");
    assert_eq!(target_type, "ChatMessage");
    assert_eq!((by, target_by, spam_flag), (3, Some(2), true));
    assert_eq!(payload, json!({"message_cooked": "<p>buy stuff</p>"}));
    let scores: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM reviewable_scores WHERE user_id = 3 AND reviewable_score_type = $1",
    )
    .bind(spam)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(scores, 1);

    let mine = heard(&st, from, "/chat/2", 3, 1).await;
    assert_eq!(
        mine,
        vec![json!({"type": "self_flagged", "user_flag_status": 0, "chat_message_id": 1})]
    );
    let staff = heard(&st, from, "/chat/2", 1, 1).await;
    assert_eq!(staff.len(), 1, "{staff:?}");
    assert_eq!(staff[0]["type"], "flag");
    assert_eq!(staff[0]["chat_message_id"], 1);

    // Flagged already, pending: handled.
    let (status, body) = user
        .request(
            "POST",
            "/chat/api/channels/2/messages/1/flags",
            &format!("flag_type_id={spam}"),
        )
        .await;
    assert_eq!(status, 422);
    assert_eq!(
        body["errors"][0],
        "Thanks, but we've already reviewed this message and determined it does not need to be flagged again."
    );
}

#[tokio::test]
async fn flags_the_member_cant_make_are_refused() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    let spam = flag_id(&db, "spam").await;
    let mut user = Client::logged_in(st, "user1").await;
    // Their own message.
    let (status, _) = user
        .request(
            "POST",
            "/chat/api/channels/2/messages/3/flags",
            &format!("flag_type_id={spam}"),
        )
        .await;
    assert_eq!(status, 404);
    // Taking action is staff's.
    let (status, _) = user
        .request(
            "POST",
            "/chat/api/channels/2/messages/1/flags",
            &format!("flag_type_id={spam}&take_action=true"),
        )
        .await;
    assert_eq!(status, 404);
    let (status, body) = user
        .request(
            "POST",
            "/chat/api/channels/2/messages/1/flags",
            "flag_type_id=999",
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(
        body["errors"],
        json!(["Flag type is not included in the list"])
    );
    // notify_user without a message flags nothing.
    let notify_user = flag_id(&db, "notify_user").await;
    let (status, _) = user
        .request(
            "POST",
            "/chat/api/channels/2/messages/1/flags",
            &format!("flag_type_id={notify_user}"),
        )
        .await;
    assert_eq!(status, 200);
    let reviewables: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reviewables")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(reviewables, 0);
}

#[tokio::test]
async fn a_staff_flag_scoring_past_the_threshold_silences_the_author() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    sqlx::query("UPDATE users SET trust_level = 1 WHERE id = 2")
        .execute(&db.pool)
        .await
        .unwrap();
    let inappropriate = flag_id(&db, "inappropriate").await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, _) = admin
        .request(
            "POST",
            "/chat/api/channels/2/messages/1/flags",
            &format!("flag_type_id={inappropriate}"),
        )
        .await;
    assert_eq!(status, 200);
    let (silenced, reviewable): (bool, Option<i64>) = sqlx::query_as(
        "SELECT silenced_till > now(), (SELECT reviewable_id FROM user_histories WHERE target_user_id = 2 \
           AND action = 30 ORDER BY id DESC LIMIT 1) FROM users WHERE id = 2",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(silenced);
    assert!(reviewable.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn selected_messages_are_deleted_together() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    let mut user = Client::logged_in(st.clone(), "user1").await;
    // Not theirs to delete.
    let (status, _) = user
        .request(
            "DELETE",
            "/chat/api/channels/2/messages",
            "message_ids[]=1&message_ids[]=3",
        )
        .await;
    assert_eq!(status, 403);
    let (status, body) = user
        .request("DELETE", "/chat/api/channels/2/messages", "")
        .await;
    assert_eq!(status, 400);
    assert_eq!(
        body["errors"],
        json!(["Message ids is too short (minimum is 1 character)"])
    );

    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let from = st.bus.now().await.unwrap();
    let (status, body) = admin
        .request(
            "DELETE",
            "/chat/api/channels/2/messages",
            "message_ids[]=1&message_ids[]=2",
        )
        .await;
    assert_eq!((status, body), (200, json!({"success": "OK"})));
    let deleted: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM chat_messages WHERE deleted_at IS NOT NULL AND deleted_by_id = 1 ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(deleted, vec![1, 2]);
    let bus = heard(&st, from, "/chat/2", 1, 1).await;
    assert_eq!(bus[0]["type"], "bulk_delete");
    assert_eq!(bus[0]["deleted_ids"], json!(["1", "2"]));
    // Nothing left of them to delete.
    let (status, _) = admin
        .request("DELETE", "/chat/api/channels/2/messages", "message_ids[]=1")
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn messages_move_to_another_public_channel() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    sqlx::query("INSERT INTO chat_message_reactions (chat_message_id, user_id, emoji, created_at, updated_at) VALUES (1, 3, 'heart', now(), now())")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE chat_messages SET in_reply_to_id = 1 WHERE id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let mut user = Client::logged_in(st.clone(), "user1").await;
    let (status, _) = user
        .request(
            "POST",
            "/chat/api/channels/2/messages/moves",
            "move[message_ids][]=1&move[destination_channel_id]=1",
        )
        .await;
    assert_eq!(status, 403);

    let mut admin = Client::logged_in(st, "admin").await;
    let (status, body) = admin
        .request(
            "POST",
            "/chat/api/channels/2/messages/moves",
            "move[message_ids][]=1&move[message_ids][]=2&move[destination_channel_id]=1",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["success"], "OK");
    assert_eq!(body["destination_channel_id"], 1);
    let first = body["first_moved_message_id"].as_i64().unwrap();
    let moved: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, message FROM chat_messages WHERE chat_channel_id = 1 AND deleted_at IS NULL ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        moved,
        vec![
            (first, "buy stuff".into()),
            (first + 1, "more stuff".into())
        ]
    );
    let reaction: i64 =
        sqlx::query_scalar("SELECT chat_message_id FROM chat_message_reactions WHERE user_id = 3")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(reaction, first);
    let (gone, reply): (i64, Option<i64>) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM chat_messages WHERE id IN (1, 2) AND deleted_at IS NOT NULL), \
                (SELECT in_reply_to_id FROM chat_messages WHERE id = 3)",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((gone, reply), (2, None));
    // The system user's placeholder in the source.
    let placeholder: String = sqlx::query_scalar(
        "SELECT message FROM chat_messages WHERE chat_channel_id = 2 AND user_id = -1 ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(
        placeholder.starts_with("@admin moved 2 messages to the ["),
        "{placeholder}"
    );
    assert!(placeholder.ends_with(&format!("(/chat/c/-/1/{first}) channel.")));

    // Nothing left to move.
    let (status, body) = admin
        .request(
            "POST",
            "/chat/api/channels/2/messages/moves",
            "move[message_ids][]=1&move[destination_channel_id]=1",
        )
        .await;
    assert_eq!(status, 422);
    assert_eq!(
        body["errors"],
        json!(["No messages were found with the provided message IDs."])
    );
}

#[tokio::test]
async fn the_channel_page_carries_select_mode_and_the_flag_modal() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    let mut user = Client::logged_in(st.clone(), "user1").await;
    let (status, html) = user.page("/chat/c/general/2").await;
    assert_eq!(status, 200);
    assert!(html.contains("<template class=\"chat-selection-template\""));
    assert!(html.contains("<div class=\"chat-selection-management\"><div class=\"chat-selection-management__buttons\"><button class=\"btn btn-icon-text btn-default\" id=\"chat-quote-btn\" type=\"button\">"));
    // Moving is for those who moderate.
    assert!(!html.contains("chat-move-to-channel-btn"));
    assert!(html.contains("data-available-flags=\""));
    assert!(html.contains("<template class=\"chat-flag-modal\" data-src=\""));
    assert!(html.contains("<script type=\"application/json\" id=\"flag-modal-config\">"));
    assert!(html.contains("\"chat.flagging.action\":\"Flag message\""));

    let mut admin = Client::logged_in(st, "admin").await;
    let (_, html) = admin.page("/chat/c/general/2").await;
    assert!(html.contains("id=\"chat-move-to-channel-btn\""));
}

#[tokio::test]
async fn staff_settle_a_flagged_message_from_the_queue() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    let spam = flag_id(&db, "spam").await;
    let mut user = Client::logged_in(st.clone(), "user1").await;
    for message in [1, 2] {
        let (status, flagged) = user
            .request(
                "POST",
                &format!("/chat/api/channels/2/messages/{message}/flags"),
                &format!("flag_type_id={spam}"),
            )
            .await;
        assert_eq!(status, 200, "{flagged}");
    }
    let id = |message: i64| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT id FROM reviewables WHERE target_id = $1")
                .bind(message)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let (first, second) = (id(1).await, id(2).await);
    let mut admin = Client::logged_in(st, "admin").await;
    // An action the message doesn't offer.
    let (status, refused) = admin
        .request(
            "PUT",
            &format!("/review/{first}/perform/agree_and_restore?version=0"),
            "",
        )
        .await;
    assert_eq!(status, 403, "{refused}");
    let (status, body) = admin
        .request(
            "PUT",
            &format!("/review/{first}/perform/agree_and_delete?version=0"),
            "",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["reviewable_perform_result"]["remove_reviewable_ids"],
        json!([first])
    );
    let (status, deleted_by, score_status, agreed): (i32, Option<i32>, i32, i32) = sqlx::query_as(
        "SELECT r.status, m.deleted_by_id, s.status, us.flags_agreed FROM reviewables r \
         JOIN chat_messages m ON m.id = r.target_id JOIN reviewable_scores s ON s.reviewable_id = r.id \
         JOIN user_stats us ON us.user_id = s.user_id WHERE r.id = $1",
    )
    .bind(first)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        (status, deleted_by, score_status, agreed),
        (1, Some(1), 1, 1)
    );

    // A stale version conflicts.
    let (status, _) = admin
        .request(
            "PUT",
            &format!("/review/{second}/perform/disagree?version=5"),
            "",
        )
        .await;
    assert_eq!(status, 409);
    let (status, _) = admin
        .request(
            "PUT",
            &format!("/review/{second}/perform/disagree?version=0"),
            "",
        )
        .await;
    assert_eq!(status, 200);
    let (status, deleted): (i32, bool) = sqlx::query_as(
        "SELECT r.status, m.deleted_at IS NOT NULL FROM reviewables r JOIN chat_messages m ON m.id = r.target_id WHERE r.id = $1",
    )
    .bind(second)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((status, deleted), (2, false));
}
