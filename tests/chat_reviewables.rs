//! Chat::ReviewableMessage in the review queue: the list and page draw it
//! beside post flags (Chat::ReviewableMessageSerializer, chat's
//! reviewable/chat-message.gjs), the type filter takes it, and its actions
//! settle it (chat's reviewable_chat_message_spec), a staff flag with
//! take_action agrees at once, and flag stats past the limit are truncated.

#[path = "common/chat_client.rs"]
mod chat_client;
mod common;

use chat_client::Client;
use common::{TestDb, config, heard, state};
use discourse_rs::config::RailsEnv;
use serde_json::{Value, json};

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

async fn exec(db: &TestDb, sql: &str) {
    sqlx::query(sql).execute(&db.pool).await.unwrap();
}

async fn flag_id(db: &TestDb, name: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM flags WHERE name_key = $1")
        .bind(name)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

/// Fabricate(:chat_reviewable_message, target: message 1, created_by:
/// user2) with its spam score by user1.
async fn fabricated(db: &TestDb) -> i64 {
    let spam = flag_id(db, "spam").await;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO reviewables (type, type_source, status, created_by_id, reviewable_by_moderator, score, \
                                  potential_spam, target_id, target_type, target_created_by_id, version, \
                                  force_review, created_at, updated_at) \
         VALUES ('ReviewableChatMessage', 'chat', 0, 4, TRUE, 1.0, FALSE, 1, 'ChatMessage', 2, 0, FALSE, now(), now()) \
         RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO reviewable_scores (reviewable_id, user_id, reviewable_score_type, status, score, created_at, updated_at) \
         VALUES ($1, 3, $2, 0, 1.0, now(), now())",
    )
    .bind(id)
    .bind(spam as i32)
    .execute(&db.pool)
    .await
    .unwrap();
    id
}

/// UserSilencer.silence of user0 by the system user, for `reviewable_id`.
async fn silence(db: &TestDb, reviewable_id: Option<i64>, reason: &str) {
    sqlx::query("UPDATE users SET silenced_till = now() + interval '10 minutes' WHERE id = 2")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, details, reviewable_id, created_at, updated_at) \
         VALUES (30, -1, 2, $1, $2, now(), now())",
    )
    .bind(reason)
    .bind(reviewable_id)
    .execute(&db.pool)
    .await
    .unwrap();
}

/// UserSilencer.unsilence of user0 by the moderator.
async fn unsilence(db: &TestDb) {
    exec(db, "UPDATE users SET silenced_till = NULL WHERE id = 2").await;
    exec(
        db,
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, created_at, updated_at) \
         VALUES (31, 4, 2, now(), now())",
    )
    .await;
}

async fn perform(moderator: &mut Client, id: i64, action: &str) -> (u16, Value) {
    let version: i32 = {
        let (_, doc) = moderator.get("/review.json?status=all").await;
        doc["reviewables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .map(|r| r["version"].as_i64().unwrap() as i32)
            .unwrap()
    };
    moderator
        .request(
            "PUT",
            &format!("/review/{id}/perform/{action}?version={version}"),
            "",
        )
        .await
}

async fn status_of(db: &TestDb, id: i64) -> i32 {
    sqlx::query_scalar("SELECT status FROM reviewables WHERE id = $1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn message_deleted(db: &TestDb) -> bool {
    sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM chat_messages WHERE id = 1")
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn silenced(db: &TestDb) -> bool {
    sqlx::query_scalar(
        "SELECT silenced_till IS NOT NULL AND silenced_till > now() FROM users WHERE id = 2",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

/// user2 moderates.
async fn setup() -> (TestDb, discourse_rs::AppState) {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    exec(&db, "UPDATE users SET moderator = TRUE WHERE id = 4").await;
    (db, st)
}

/// The doc's rows of `table` by id.
fn rows<'a>(doc: &'a Value, table: &str, ids: &Value) -> Vec<&'a Value> {
    ids.as_array()
        .unwrap()
        .iter()
        .map(|id| {
            doc[table]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["id"] == *id)
                .unwrap()
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_queue_lists_a_chat_flag_beside_a_post_flag() {
    let (db, st) = setup().await;
    let spam = flag_id(&db, "spam").await;
    let mut user = Client::logged_in(st.clone(), "user1").await;
    let (status, body) = user
        .request(
            "POST",
            "/chat/api/channels/2/messages/1/flags",
            &format!("flag_type_id={spam}"),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = user
        .request(
            "POST",
            "/post_actions.json",
            "id=34&post_action_type_id=4&flag_topic=false",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let chat_id: i64 =
        sqlx::query_scalar("SELECT id FROM reviewables WHERE target_type = 'ChatMessage'")
            .fetch_one(&db.pool)
            .await
            .unwrap();

    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, doc) = admin.get("/review.json").await;
    assert_eq!(status, 200, "{doc}");
    let reviewables = doc["reviewables"].as_array().unwrap();
    assert_eq!(reviewables.len(), 2, "{doc}");
    let chat = reviewables.iter().find(|r| r["id"] == chat_id).unwrap();
    assert_eq!(chat["type"], "ReviewableChatMessage");
    assert_eq!(chat["type_source"], "chat");
    assert_eq!(chat["target_type"], "ChatMessage");
    assert_eq!(chat["target_id"], 1);
    assert_eq!(chat["chatmessage_id"], 1);
    assert_eq!(chat["cooked"], "<p>buy stuff</p>");
    assert_eq!(
        chat["payload"],
        json!({"message_cooked": "<p>buy stuff</p>"})
    );
    assert_eq!(chat["created_from_flag"], true);
    assert_eq!(chat["can_edit"], false);
    assert_eq!(chat["topic_id"], Value::Null);
    assert_eq!(chat["target_created_by_id"], 2);
    assert!(chat.get("target_url").is_none());
    assert_eq!(chat["chat_channel"]["id"], 2);
    assert_eq!(chat["chat_channel"]["chatable_type"], "Category");
    assert_eq!(chat["chat_channel"]["slug"], "general");

    // build_combined_actions: chat's labels, the message's action names.
    let bundles = rows(&doc, "bundled_actions", &chat["bundled_action_ids"]);
    assert_eq!(
        bundles.iter().map(|b| b["id"].clone()).collect::<Vec<_>>(),
        vec![
            json!(format!("{chat_id}-agree")),
            json!(format!("{chat_id}-disagree"))
        ]
    );
    assert_eq!(bundles[0]["label"], "Yes");
    let agree = rows(&doc, "actions", &bundles[0]["action_ids"]);
    assert_eq!(
        agree
            .iter()
            .map(|a| a["server_action"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!("agree_and_delete"),
            json!("agree_and_keep_message"),
            json!("agree_and_suspend"),
            json!("agree_and_silence")
        ]
    );
    assert_eq!(
        agree[0]["id"],
        format!("{chat_id}-chat_message-agree_and_delete")
    );
    assert_eq!(agree[0]["action_name"], "chat_message-agree_and_delete");
    assert_eq!(agree[0]["label"], "Delete Message");
    assert_eq!(
        agree[0]["description"],
        "Delete the message so that users cannot see it."
    );
    assert_eq!(agree[2]["client_action"], "suspend");
    let disagree = rows(&doc, "actions", &bundles[1]["action_ids"]);
    assert_eq!(
        disagree
            .iter()
            .map(|a| (a["server_action"].clone(), a["label"].clone()))
            .collect::<Vec<_>>(),
        vec![
            (json!("disagree"), json!("Keep message")),
            (json!("ignore"), json!("Ignore flag")),
            (
                json!("delete_and_agree"),
                json!("Ignore flag and delete message")
            )
        ]
    );

    let meta = &doc["meta"];
    assert!(
        meta["reviewable_types"]
            .as_array()
            .unwrap()
            .contains(&json!("Chat::ReviewableMessage"))
    );
    assert_eq!(meta["unknown_reviewable_types_and_sources"], json!([]));
    assert_eq!(meta["reviewable_count"], 2);

    // The type filter takes the class's name.
    let (status, doc) = admin
        .get("/review.json?type=Chat%3A%3AReviewableMessage")
        .await;
    assert_eq!(status, 200);
    assert_eq!(doc["reviewables"].as_array().unwrap().len(), 1);
    assert_eq!(doc["reviewables"][0]["id"], chat_id);
    let (status, _) = admin.get("/review.json?type=ReviewableChatMessage").await;
    assert_eq!(status, 400);
    // The ids filter, which review.js redraws an acted on item with.
    let (status, doc) = admin
        .get(&format!("/review.json?ids%5B%5D={chat_id}&status=all"))
        .await;
    assert_eq!(status, 200);
    assert_eq!(doc["reviewables"].as_array().unwrap().len(), 1);
    assert_eq!(doc["meta"]["ids"], json!([chat_id.to_string()]));
    let (_, html) = admin
        .page(&format!(
            "/review?ids%5B%5D={chat_id}&status=all&priority=any"
        ))
        .await;
    assert!(html.contains("reviewable-chat-message"));
    assert!(!html.contains("review-item reviewable-flagged-post"));

    // The user menu's queue.
    let (_, menu) = admin.get("/review/user-menu-list.json").await;
    assert_eq!(menu["reviewables"].as_array().unwrap().len(), 2);
    assert_eq!(menu["reviewable_count"], 2);

    // The page: the post flag and the chat flag.
    let (status, html) = admin.page("/review").await;
    assert_eq!(status, 200);
    assert!(
        html.contains("review-item reviewable-flagged-post"),
        "{html}"
    );
    assert!(html.contains(&format!(
        "<div class=\"review-item reviewable-chat-message\" data-reviewable-id=\"{chat_id}\">"
    )));
    assert!(html.contains("<span class=\"review-item__flag-label\">Chat flagged as</span>"));
    assert!(html.contains(
        "<div class=\"review-item__meta-label\">Channel</div><div class=\"review-item__meta-topic-title\"><a href=\"/chat/c/general/2/1\"><div class=\"chat-channel-title is-category\">"
    ));
    assert!(
        html.contains(
            "<div class=\"review-item__post-content\"><p>buy stuff</p></div></div></div>"
        )
    );
    assert!(html.contains(
        "class=\"select-kit single-select dropdown-select-box reviewable-action-dropdown btn-icon-text chat-message-agree-and-delete\""
    ));
    assert!(html.contains("data-value=\"chat_message-agree_and_delete\""));
    // The type filter's row.
    assert!(html.contains("Flagged Chat Message"));
    assert!(html.contains("\"server_action\":\"agree_and_delete\""));

    // Filtered to chat, only the chat flag draws.
    let (_, html) = admin.page("/review?type=Chat%3A%3AReviewableMessage").await;
    assert!(html.contains("reviewable-chat-message"));
    assert!(!html.contains("review-item reviewable-flagged-post"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trashed_or_hard_deleted_message_still_lists() {
    let (db, st) = setup().await;
    let id = fabricated(&db).await;
    exec(
        &db,
        "UPDATE reviewables SET payload = '{\"message_cooked\": \"<p>buy stuff</p>\"}' WHERE target_id = 1",
    )
    .await;
    exec(
        &db,
        "UPDATE chat_messages SET deleted_at = now(), deleted_by_id = 2 WHERE id = 1",
    )
    .await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (_, doc) = admin.get("/review.json").await;
    let chat = &doc["reviewables"][0];
    // The target is gone (Trashable), the payload keeps the message.
    assert_eq!(chat["target_id"], Value::Null);
    assert!(chat.get("cooked").is_none());
    assert!(chat.get("chatmessage_id").is_none());
    assert_eq!(chat["chat_channel"]["id"], 2);
    let actions: Vec<Value> = doc["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["server_action"].clone())
        .collect();
    assert_eq!(
        actions,
        vec![
            json!("agree_and_restore"),
            json!("agree_and_keep_deleted"),
            json!("agree_and_suspend"),
            json!("agree_and_silence"),
            json!("disagree_and_restore"),
            json!("ignore")
        ]
    );
    let (_, html) = admin.page("/review").await;
    assert!(html.contains("<a href=\"/chat/c/general/2\">"));
    assert!(html.contains("<p>buy stuff</p>"));

    // Hard deleted: no channel, only ignore (chat's reviewables_spec).
    exec(&db, "DELETE FROM chat_messages WHERE id = 1").await;
    let (_, doc) = admin.get("/review.json").await;
    assert_eq!(doc["reviewables"][0]["chat_channel"], Value::Null);
    let actions = doc["actions"].as_array().unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["server_action"], "ignore");
    assert_eq!(actions[0]["label"], "Ignore flag");
    assert_eq!(actions[0]["icon"], "up-right-from-square");
    let (_, html) = admin.page("/review").await;
    assert!(html.contains("<div class=\"review-item__post-content\"><p>buy stuff</p>"));
    let mut moderator = Client::logged_in(st, "user2").await;
    let (status, body) = perform(&mut moderator, id, "ignore").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(status_of(&db, id).await, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn agreeing_keeps_deletes_or_restores_the_message() {
    let (db, st) = setup().await;
    let mut moderator = Client::logged_in(st, "user2").await;

    // agree_and_keep agrees with the flag and doesn't delete the message
    let id = fabricated(&db).await;
    let (status, body) = perform(&mut moderator, id, "agree_and_keep_message").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(status_of(&db, id).await, 1);
    assert!(!message_deleted(&db).await);
    assert_eq!(
        body["reviewable_perform_result"]["remove_reviewable_ids"],
        json!([id])
    );
    let agreed: i32 = sqlx::query_scalar("SELECT flags_agreed FROM user_stats WHERE user_id = 3")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(agreed, 1);
    exec(&db, "DELETE FROM reviewables").await;

    // agree_and_delete agrees with the flag and deletes the message
    let id = fabricated(&db).await;
    let (status, body) = perform(&mut moderator, id, "agree_and_delete").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(status_of(&db, id).await, 1);
    let deleted_by: Option<i32> =
        sqlx::query_scalar("SELECT deleted_by_id FROM chat_messages WHERE id = 1")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(deleted_by, Some(4));
    exec(&db, "DELETE FROM reviewables").await;

    // agree_and_keep_deleted agrees with the flag and keeps the message
    // deleted
    let id = fabricated(&db).await;
    let (status, body) = perform(&mut moderator, id, "agree_and_keep_deleted").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(status_of(&db, id).await, 1);
    assert!(message_deleted(&db).await);
    exec(&db, "DELETE FROM reviewables").await;

    // agree_and_restore agrees with the flag and restores the message
    let id = fabricated(&db).await;
    let (status, body) = perform(&mut moderator, id, "agree_and_restore").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(status_of(&db, id).await, 1);
    assert!(!message_deleted(&db).await);
    exec(&db, "DELETE FROM reviewables").await;

    // An alias of agree_and_delete performs it.
    let id = fabricated(&db).await;
    let (status, body) = perform(&mut moderator, id, "delete_and_agree").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(status_of(&db, id).await, 1);
    assert!(message_deleted(&db).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn disagreeing_and_ignoring_leave_or_restore_the_message() {
    let (db, st) = setup().await;
    let mut moderator = Client::logged_in(st, "user2").await;

    // perform_disagree disagrees with the flag and does nothing
    let id = fabricated(&db).await;
    let (status, body) = perform(&mut moderator, id, "disagree").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(status_of(&db, id).await, 2);
    let (disagreed, score_status): (i32, i32) = sqlx::query_as(
        "SELECT us.flags_disagreed, rs.status FROM user_stats us JOIN reviewable_scores rs ON rs.user_id = us.user_id \
         WHERE rs.reviewable_id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((disagreed, score_status), (1, 2));
    exec(&db, "DELETE FROM reviewables").await;

    // perform_disagree_and_restore disagrees with the flag and restores
    // the message
    exec(
        &db,
        "UPDATE chat_messages SET deleted_at = now(), deleted_by_id = 2 WHERE id = 1",
    )
    .await;
    let id = fabricated(&db).await;
    let (status, body) = perform(&mut moderator, id, "disagree_and_restore").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(status_of(&db, id).await, 2);
    assert!(!message_deleted(&db).await);
    exec(&db, "DELETE FROM reviewables").await;

    // perform_ignore ignores the flag and does nothing
    let id = fabricated(&db).await;
    let (status, body) = perform(&mut moderator, id, "ignore").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(status_of(&db, id).await, 3);
    assert!(!message_deleted(&db).await);
    let ignored: i32 = sqlx::query_scalar("SELECT flags_ignored FROM user_stats WHERE user_id = 3")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(ignored, 1);

    // Settled, nothing is offered.
    let (status, _) = moderator
        .request(
            "PUT",
            &format!("/review/{id}/perform/disagree?version=1"),
            "",
        )
        .await;
    assert_eq!(status, 403);
}

#[tokio::test(flavor = "multi_thread")]
async fn disagreeing_lifts_a_silence_this_flag_brought() {
    let (db, st) = setup().await;
    let mut moderator = Client::logged_in(st, "user2").await;

    // when the flagged message author was silenced for this message:
    // perform_disagree unsilences the user, attributed to the moderator
    // and the reviewable
    let id = fabricated(&db).await;
    silence(&db, Some(id), "auto silenced").await;
    let (status, body) = perform(&mut moderator, id, "disagree").await;
    assert_eq!(status, 200, "{body}");
    assert!(!silenced(&db).await);
    let (acting, reviewable): (Option<i32>, Option<i64>) = sqlx::query_as(
        "SELECT acting_user_id, reviewable_id FROM user_histories WHERE action = 31 AND target_user_id = 2",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((acting, reviewable), (Some(4), Some(id)));
    exec(&db, "DELETE FROM reviewables").await;
    exec(&db, "DELETE FROM user_histories WHERE target_user_id = 2").await;

    // perform_disagree_and_restore unsilences the user
    exec(
        &db,
        "UPDATE chat_messages SET deleted_at = now(), deleted_by_id = 2 WHERE id = 1",
    )
    .await;
    let id = fabricated(&db).await;
    silence(&db, Some(id), "auto silenced").await;
    let (status, body) = perform(&mut moderator, id, "disagree_and_restore").await;
    assert_eq!(status, 200, "{body}");
    assert!(!silenced(&db).await);
    exec(&db, "DELETE FROM reviewables").await;
    exec(&db, "DELETE FROM user_histories WHERE target_user_id = 2").await;

    // when the flagged message author was silenced for something else:
    // perform_disagree leaves the user silenced
    let id = fabricated(&db).await;
    silence(&db, None, "unrelated reason").await;
    let (status, body) = perform(&mut moderator, id, "disagree").await;
    assert_eq!(status, 200, "{body}");
    assert!(silenced(&db).await);
    exec(&db, "DELETE FROM reviewables").await;
    exec(&db, "DELETE FROM user_histories WHERE target_user_id = 2").await;
    exec(&db, "UPDATE users SET silenced_till = NULL WHERE id = 2").await;

    // when a silence from this flag was already lifted and a new one
    // replaced it: perform_disagree leaves the later, unrelated silence
    let id = fabricated(&db).await;
    silence(&db, Some(id), "auto silenced").await;
    unsilence(&db).await;
    silence(&db, None, "unrelated reason").await;
    let (status, body) = perform(&mut moderator, id, "disagree").await;
    assert_eq!(status, 200, "{body}");
    assert!(silenced(&db).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn unsilencing_the_author_leaves_the_flag_pending() {
    let (db, st) = setup().await;
    let mut moderator = Client::logged_in(st.clone(), "user2").await;
    let id = fabricated(&db).await;

    // is not offered when the author is not silenced
    let (_, doc) = moderator.get("/review.json").await;
    assert!(
        !doc["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["server_action"] == "unsilence_user")
    );
    let (status, _) = moderator
        .request(
            "PUT",
            &format!("/review/{id}/perform/unsilence_user?version=0"),
            "",
        )
        .await;
    assert_eq!(status, 403);

    // when the author is silenced: offered with a translated label, set
    // apart from the actions that resolve the flag
    silence(&db, None, "auto silenced").await;
    let (_, doc) = moderator.get("/review.json").await;
    let unsilence = doc["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["server_action"] == "unsilence_user")
        .unwrap();
    assert_eq!(unsilence["label"], "Unsilence author");
    assert_eq!(unsilence["completed_message"], "Author unsilenced.");
    let secondary: Vec<&Value> = doc["bundled_actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["secondary"] == true)
        .collect();
    assert_eq!(secondary.len(), 1);
    assert_eq!(
        secondary[0]["action_ids"],
        json!([format!("{id}-chat_message-unsilence_user")])
    );
    // penalty_effect_for: the author has a penalty.
    assert_eq!(unsilence["penalty_effect"], "lifts_penalty");
    let disagree = doc["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["server_action"] == "disagree")
        .unwrap();
    assert_eq!(disagree["penalty_effect"], "retains_penalty");

    // is not offered to a user who cannot unsilence the author
    let mut member = Client::logged_in(st, "user1").await;
    let (status, _) = member
        .request(
            "PUT",
            &format!("/review/{id}/perform/unsilence_user?version=0"),
            "",
        )
        .await;
    assert_eq!(status, 403);

    // lifts the silence without resolving the flag, attributed to the
    // moderator and the reviewable
    let (status, body) = perform(&mut moderator, id, "unsilence_user").await;
    assert_eq!(status, 200, "{body}");
    assert!(!silenced(&db).await);
    assert_eq!(status_of(&db, id).await, 0);
    assert_eq!(
        body["reviewable_perform_result"]["remove_reviewable_ids"],
        json!([])
    );
    let (acting, reviewable): (Option<i32>, Option<i64>) = sqlx::query_as(
        "SELECT acting_user_id, reviewable_id FROM user_histories WHERE action = 31 AND target_user_id = 2",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((acting, reviewable), (Some(4), Some(id)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_staff_flag_taking_action_deletes_the_message_at_once() {
    let (db, st) = setup().await;
    let spam = flag_id(&db, "spam").await;
    let from = st.bus.now().await.unwrap();
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, body) = admin
        .request(
            "POST",
            "/chat/api/channels/2/messages/1/flags",
            &format!("flag_type_id={spam}&take_action=true"),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, version, deleted_by, bonus, score_status): (i32, i32, Option<i32>, f64, i32) =
        sqlx::query_as(
            "SELECT r.status, r.version, m.deleted_by_id, s.take_action_bonus, s.status FROM reviewables r \
             JOIN chat_messages m ON m.id = r.target_id JOIN reviewable_scores s ON s.reviewable_id = r.id",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        (status, version, deleted_by, bonus, score_status),
        (1, 1, Some(1), 5.0, 1)
    );
    // publish_delete!, not publish_flag!.
    let heard = heard(&st, from, "/chat/2", 3, 1).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert_eq!(heard[0]["type"], "delete");
    assert_eq!(heard[0]["deleted_id"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn flag_stats_past_the_limit_are_truncated() {
    let (db, st) = setup().await;
    exec(
        &db,
        "UPDATE user_stats SET flags_agreed = 100 WHERE user_id = 3",
    )
    .await;
    let id = fabricated(&db).await;
    let mut moderator = Client::logged_in(st.clone(), "user2").await;
    let (status, body) = perform(&mut moderator, id, "agree_and_keep_message").await;
    assert_eq!(status, 200, "{body}");
    let (job, args): (i64, Value) = sqlx::query_as(
        "SELECT id, args FROM discourse_rs.jobs WHERE name = 'truncate_user_flag_stats'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(args, json!({"user_ids": [3]}));
    // Jobs::TruncateUserFlagStats recounts from the last hundred post
    // flags alone: user1 has none.
    discourse_rs::jobs::perform_now(&st, job).await.unwrap();
    let stats: (i32, i32, i32) = sqlx::query_as(
        "SELECT flags_agreed, flags_disagreed, flags_ignored FROM user_stats WHERE user_id = 3",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(stats, (0, 0, 0));
}
