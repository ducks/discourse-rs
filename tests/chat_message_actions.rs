//! The message actions after moderation: quoting the selection (POST
//! /chat/:id/quote), the reaction users popover's list, stopping a stream,
//! block interactions, the composer's group mention hints, the duplicate
//! message exemptions, other spellings of a target date and the author's
//! name.

#[path = "common/chat_client.rs"]
mod chat_client;
mod common;

use chat_client::Client;
use common::{TestDb, config, set_setting, state};
use discourse_rs::config::RailsEnv;
use serde_json::json;

/// General's messages: user0's two, then user1's.
async fn messages(db: &TestDb) {
    for sql in [
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         VALUES (1, 2, 2, '2026-10-01 10:00:00', '2026-10-01 10:00:00', 'first', '<p>first</p>', 1, 2), \
                (2, 2, 2, '2026-10-01 10:01:00', '2026-10-01 10:01:00', 'second', '<p>second</p>', 1, 2), \
                (3, 2, 3, '2026-10-01 10:02:00', '2026-10-01 10:02:00', 'third', '<p>third</p>', 1, 3)",
        "SELECT setval('chat_messages_id_seq', 3)",
        "UPDATE chat_channels SET last_message_id = 3 WHERE id = 2",
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
}

#[tokio::test]
async fn the_selection_is_quoted_as_a_transcript() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    let mut user = Client::logged_in(st, "user1").await;
    let (status, body) = user
        .request(
            "POST",
            "/chat/2/quote",
            "message_ids[]=3&message_ids[]=1&message_ids[]=2",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["success"], "OK");
    assert_eq!(
        body["markdown"],
        "[chat quote=\"user0;1;2026-10-01T10:00:00Z\" channel=\"General\" channelId=\"2\" multiQuote=\"true\" chained=\"true\"]\n[/chat]\n\n[chat quote=\"user1;3;2026-10-01T10:02:00Z\" chained=\"true\"]\n[/chat]\n"
    );
    let (status, body) = user
        .request("POST", "/chat/2/quote", "message_ids[]=1")
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        body["markdown"],
        "[chat quote=\"user0;1;2026-10-01T10:00:00Z\" channel=\"General\" channelId=\"2\"]\n[/chat]\n"
    );
    let (status, _) = user.request("POST", "/chat/2/quote", "").await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn the_reaction_users_list_pages_through_reactions() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    sqlx::query(
        "INSERT INTO chat_message_reactions (chat_message_id, user_id, emoji, created_at, updated_at) VALUES \
         (1, 3, 'heart', now() - interval '2 minutes', now()), (1, 1, '+1', now() - interval '1 minute', now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let mut user = Client::logged_in(st, "user1").await;
    let (status, body) = user.get("/chat/2/1/reactions-users?emoji=heart").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["total_rows"], 2);
    let users = body["users"].as_array().unwrap();
    // Every reaction, whichever emoji was asked for, as the query leaves
    // the filter out.
    assert_eq!(users.len(), 2);
    assert_eq!(users[0]["username"], "user1");
    assert_eq!(users[0]["reaction"], "heart");
    assert_eq!(users[1]["reaction"], "+1");
    assert!(users[0]["avatar_template"].is_string());
    let (_, body) = user.get("/chat/2/1/reactions-users?limit=1&page=1").await;
    assert_eq!(body["users"].as_array().unwrap().len(), 1);
    assert_eq!(body["users"][0]["reaction"], "+1");
    let (status, _) = user.get("/chat/2/99/reactions-users").await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn the_replied_to_member_stops_a_stream() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    sqlx::query("UPDATE chat_messages SET streaming = TRUE, in_reply_to_id = 3 WHERE id = 2")
        .execute(&db.pool)
        .await
        .unwrap();
    let mut user = Client::logged_in(st.clone(), "user1").await;
    // The page offers them the button.
    let (_, html) = user.page("/chat/c/general/2").await;
    assert!(html.contains("<div class=\"chat-message-container -streaming"));
    assert!(html.contains("<div class=\"stop-streaming-btn-container\"><button class=\"btn btn-icon-text stop-streaming-btn\" type=\"button\">"));
    let (status, body) = user
        .request("DELETE", "/chat/api/channels/2/messages/2/streaming", "")
        .await;
    assert_eq!((status, body), (200, json!({"success": "OK"})));
    let streaming: bool = sqlx::query_scalar("SELECT streaming FROM chat_messages WHERE id = 2")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert!(!streaming);
    // Someone else's stream of someone else's message.
    sqlx::query("UPDATE chat_messages SET streaming = TRUE, in_reply_to_id = NULL WHERE id = 1")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, _) = user
        .request("DELETE", "/chat/api/channels/2/messages/1/streaming", "")
        .await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn a_block_button_press_is_recorded() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    sqlx::query(
        "UPDATE chat_messages SET blocks = '[{\"type\":\"actions\",\"elements\":[{\"type\":\"button\",\"action_id\":\"approve\",\"value\":\"yes\",\"text\":{\"type\":\"plain_text\",\"text\":\"Approve\"}}]}]' WHERE id = 1",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let mut user = Client::logged_in(st, "user1").await;
    let (status, body) = user
        .request(
            "POST",
            "/chat/api/channels/2/messages/1/interactions",
            "action_id=approve",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["interaction"],
        json!({
            "user": {"id": 3, "username": "user1"},
            "channel": {"id": 2, "title": "General"},
            "message": {"id": 1, "text": "first", "user_id": 2},
            "action": {"type": "button", "action_id": "approve", "text": {"type": "plain_text", "text": "Approve"}},
        })
    );
    let saved: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM chat_message_interactions WHERE user_id = 3 AND chat_message_id = 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(saved, 1);
    let (status, _) = user
        .request(
            "POST",
            "/chat/api/channels/2/messages/1/interactions",
            "action_id=nope",
        )
        .await;
    assert_eq!(status, 404);
    let (status, body) = user
        .request("POST", "/chat/api/channels/2/messages/1/interactions", "")
        .await;
    assert_eq!(status, 400);
    assert_eq!(body["errors"], json!(["Action can't be blank"]));
}

#[tokio::test]
async fn group_mention_hints_sort_the_names() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    sqlx::query(
        "INSERT INTO groups (id, name, created_at, updated_at, visibility_level, mentionable_level, user_count) \
         VALUES (90, 'closed', now(), now(), 0, 0, 3), (91, 'crowd', now(), now(), 0, 99, 5000)",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    set_setting(&db.pool, "max_users_notified_per_group_mention", 3, "100").await;
    let mut user = Client::logged_in(st, "user1").await;
    let (status, body) = user
        .get("/chat/api/mentions/groups.json?mentions[]=closed&mentions[]=crowd&mentions[]=someone")
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["unreachable"], json!(["closed"]));
    assert_eq!(body["over_members_limit"], json!(["crowd"]));
    assert_eq!(body["invalid"], json!(["someone"]));
    let (status, _) = user.get("/chat/api/mentions/groups.json").await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn a_two_member_dm_may_repeat_itself() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user = Client::logged_in(st, "user1").await;
    let (status, body) = user
        .request(
            "POST",
            "/chat/api/direct-message-channels",
            "target_usernames[]=user0&upsert=true",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let dm = body["channel"]["id"].as_i64().unwrap();
    for _ in 0..2 {
        let (status, body) = user
            .request("POST", &format!("/chat/{dm}"), "message=hello%20again")
            .await;
        assert_eq!(status, 200, "{body}");
    }
    // In General the second is a duplicate.
    let (status, _) = user
        .request("POST", "/chat/2", "message=hello%20general")
        .await;
    assert_eq!(status, 200);
    let (status, body) = user
        .request("POST", "/chat/2", "message=hello%20general")
        .await;
    assert_eq!(status, 422, "{body}");
}

#[tokio::test]
async fn a_target_date_in_another_spelling_is_read() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    let mut user = Client::logged_in(st, "user1").await;
    let (status, body) = user
        .get("/chat/api/channels/2/messages?target_date=Oct%201%202026%2010%3A01")
        .await;
    assert_eq!(status, 200, "{body}");
    let ids: Vec<i64> = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, vec![1, 2, 3]);
}

#[tokio::test]
async fn the_author_line_puts_names_first_when_the_site_does() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    sqlx::query("UPDATE users SET name = 'Zero Person' WHERE id = 2")
        .execute(&db.pool)
        .await
        .unwrap();
    set_setting(&db.pool, "enable_names", 5, "t").await;
    set_setting(&db.pool, "display_name_on_posts", 5, "t").await;
    set_setting(&db.pool, "prioritize_username_in_ux", 5, "f").await;
    let mut user = Client::logged_in(st, "user1").await;
    let (_, html) = user.page("/chat/c/general/2").await;
    assert!(html.contains("<button class=\"chat-message-info__username__name\" data-user-card=\"user0\" type=\"button\">Zero Person</button>"));
    set_setting(&db.pool, "prioritize_username_in_ux", 5, "t").await;
    let (_, html) = user.page("/chat/c/general/2").await;
    assert!(html.contains("data-user-card=\"user0\" type=\"button\">user0</button>"));
}
