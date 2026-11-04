//! Archiving a chat channel, ported from chat's specs for
//! ChannelsArchivesController (requests/chat/api), ChannelArchiveService
//! (lib/chat) and Jobs::Chat::ChannelArchive: the request and its refusals,
//! the archive row and the job, the batches quoted into a new or existing
//! topic (threads split with their ranges), the destination's status, the
//! members unfollowed, the system messages, a failure kept and retried.

use crate::common;
use crate::common::chat_client;

use chat_client::Client;
use common::{TestDb, bus_messages, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use discourse_rs::plugins::chat::archive;
use discourse_rs::site_settings::SiteSettings;
use serde_json::json;

const NEW_TOPIC: &str =
    "archive[type]=new_topic&archive[title]=This+will+be+a+new+topic&archive[category_id]=4";

async fn exec(db: &TestDb, sql: &str) {
    sqlx::query(sql).execute(&db.pool).await.unwrap();
}

async fn scalar<T>(db: &TestDb, sql: &str) -> T
where
    T: for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres> + Send + Unpin,
{
    sqlx::query_scalar(sql).fetch_one(&db.pool).await.unwrap()
}

/// The channel's messages as Fabricate makes them, one second apart.
struct Messages {
    at: i64,
}

impl Messages {
    fn new() -> Messages {
        Messages { at: 0 }
    }

    /// `count` messages in General by user0..user2 in turn, in the thread
    /// given; their ids.
    async fn create(&mut self, db: &TestDb, count: usize, thread: Option<i64>) -> Vec<i64> {
        let mut ids = Vec::new();
        for i in 0..count {
            self.at += 1;
            let user = 2 + (i as i32 % 3);
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO chat_messages (chat_channel_id, user_id, created_at, updated_at, message, \
                   cooked, cooked_version, last_editor_id, thread_id) \
                 VALUES (2, $1, '2026-10-01'::timestamp + make_interval(secs => $2), \
                         '2026-10-01'::timestamp + make_interval(secs => $2), $3, $4, 1, $1, $5) \
                 RETURNING id",
            )
            .bind(user)
            .bind(self.at as f64)
            .bind(format!("message {}", self.at))
            .bind(format!("<p>message {}</p>", self.at))
            .bind(thread)
            .fetch_one(&db.pool)
            .await
            .unwrap();
            ids.push(id);
        }
        exec(
            db,
            "UPDATE chat_channels SET last_message_id = (SELECT MAX(id) FROM chat_messages WHERE chat_channel_id = 2) WHERE id = 2",
        )
        .await;
        ids
    }

    /// `create_threaded_messages`: an original message, its thread and
    /// count - 1 replies.
    async fn threaded(&mut self, db: &TestDb, count: usize, title: Option<&str>) {
        let om = self.create(db, 1, None).await[0];
        let thread: i64 = sqlx::query_scalar(
            "INSERT INTO chat_threads (channel_id, original_message_id, original_message_user_id, title, \
               replies_count, created_at, updated_at) \
             SELECT 2, id, user_id, $2, $3, created_at, created_at FROM chat_messages WHERE id = $1 RETURNING id",
        )
        .bind(om)
        .bind(title)
        .bind(count as i32 - 1)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        exec(
            db,
            &format!("UPDATE chat_messages SET thread_id = {thread} WHERE id = {om}"),
        )
        .await;
        self.create(db, count - 1, Some(thread)).await;
    }
}

async fn archiving_allowed(db: &TestDb) {
    set_setting(&db.pool, "chat_allow_archiving_channels", 5, "t").await;
}

async fn settings(st: &AppState) -> SiteSettings {
    let mut conn = st.pool.acquire().await.unwrap();
    SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap()
}

/// The archive the admin starts with `params`, and its id.
async fn start(st: &AppState, params: &str) -> i64 {
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, body) = admin
        .request("POST", "/chat/api/channels/2/archives", params)
        .await;
    assert_eq!(status, 200, "{body}");
    sqlx::query_scalar("SELECT id FROM chat_channel_archives WHERE chat_channel_id = 2")
        .fetch_one(&st.pool)
        .await
        .unwrap()
}

async fn run(st: &AppState, id: i64, per_post: usize) -> Result<(), discourse_rs::AppError> {
    archive::execute_with(st, &settings(st).await, id, per_post).await
}

/// The destination's posts after the first ones: their raw and author.
async fn posts(db: &TestDb, topic_id: i32) -> Vec<(i32, String, i32)> {
    sqlx::query_as(
        "SELECT post_number, raw, user_id FROM posts WHERE topic_id = $1 AND deleted_at IS NULL ORDER BY post_number",
    )
    .bind(topic_id)
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

async fn destination(db: &TestDb) -> i32 {
    scalar::<Option<i32>>(
        db,
        "SELECT destination_topic_id FROM chat_channel_archives WHERE chat_channel_id = 2",
    )
    .await
    .expect("a destination topic")
}

async fn live_messages(db: &TestDb) -> i64 {
    scalar(
        db,
        "SELECT COUNT(*) FROM chat_messages WHERE chat_channel_id = 2 AND deleted_at IS NULL",
    )
    .await
}

async fn status(db: &TestDb) -> i32 {
    scalar(db, "SELECT status FROM chat_channels WHERE id = 2").await
}

#[tokio::test]
async fn the_request_is_refused_as_rails_refuses_it() {
    let db = TestDb::new().await;
    archiving_allowed(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    // Not staff.
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status_code, _) = user1
        .request("POST", "/chat/api/channels/2/archives", NEW_TOPIC)
        .await;
    assert_eq!(status_code, 403);
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    // No archive, or no type.
    let (status_code, body) = admin
        .request("POST", "/chat/api/channels/2/archives", "")
        .await;
    assert_eq!(status_code, 400, "{body}");
    let (status_code, _) = admin
        .request("POST", "/chat/api/channels/2/archives", "type=new_topic")
        .await;
    assert_eq!(status_code, 400);
    let (status_code, _) = admin
        .request(
            "POST",
            "/chat/api/channels/2/archives",
            "archive[title]=no+type",
        )
        .await;
    assert_eq!(status_code, 400);
    // A new topic without a title, an existing one without an id.
    let (status_code, _) = admin
        .request(
            "POST",
            "/chat/api/channels/2/archives",
            "archive[type]=new_topic",
        )
        .await;
    assert_eq!(status_code, 400);
    let (status_code, _) = admin
        .request(
            "POST",
            "/chat/api/channels/2/archives",
            "archive[type]=existing_topic",
        )
        .await;
    assert_eq!(status_code, 400);
    // A channel that can't be made read only.
    exec(&db, "UPDATE chat_channels SET status = 3 WHERE id = 2").await;
    let (status_code, body) = admin
        .request("POST", "/chat/api/channels/2/archives", NEW_TOPIC)
        .await;
    assert_eq!(status_code, 403);
    assert_eq!(
        body["errors"][0],
        "You are not permitted to view the requested resource."
    );
    exec(&db, "UPDATE chat_channels SET status = 0 WHERE id = 2").await;
    // A missing channel.
    let (status_code, _) = admin
        .request("POST", "/chat/api/channels/99/archives", NEW_TOPIC)
        .await;
    assert_eq!(status_code, 404);
    // Archiving off.
    set_setting(&db.pool, "chat_allow_archiving_channels", 5, "f").await;
    let (status_code, _) = admin
        .request(
            "POST",
            "/chat/api/channels/2/archives",
            "archive[type]=existing_topic&archive[topic_id]=1",
        )
        .await;
    assert_eq!(status_code, 403);
    assert_eq!(
        scalar::<i64>(&db, "SELECT COUNT(*) FROM chat_channel_archives").await,
        0
    );
}

#[tokio::test]
async fn an_existing_topic_the_member_cannot_reach_is_refused() {
    let db = TestDb::new().await;
    archiving_allowed(&db).await;
    // user3 a moderator, the topic in Staff's restricted category.
    exec(
        &db,
        "UPDATE users SET moderator = TRUE WHERE username = 'user3'",
    )
    .await;
    exec(
        &db,
        "DELETE FROM group_users WHERE group_id = 3 AND user_id = 5",
    )
    .await;
    let topic: i32 = scalar(
        &db,
        "INSERT INTO topics (title, fancy_title, slug, user_id, last_post_user_id, category_id, archetype, \
           created_at, updated_at, bumped_at, posts_count, highest_post_number) \
         VALUES ('A private staff topic here', 'A private staff topic here', 'a-private-staff-topic-here', 1, 1, 3, 'regular', \
           now(), now(), now(), 0, 0) RETURNING id",
    )
    .await;
    exec(
        &db,
        "UPDATE category_groups SET group_id = 1 WHERE category_id = 3",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut moderator = Client::logged_in(st.clone(), "user3").await;
    let params = format!("archive[type]=existing_topic&archive[topic_id]={topic}");
    let (status_code, body) = moderator
        .request("POST", "/chat/api/channels/2/archives", &params)
        .await;
    assert_eq!(status_code, 403, "{body}");
    exec(
        &db,
        &format!("UPDATE topics SET closed = TRUE WHERE id = {topic}"),
    )
    .await;
    let (status_code, _) = moderator
        .request("POST", "/chat/api/channels/2/archives", &params)
        .await;
    assert_eq!(status_code, 403);
    // A topic that doesn't exist.
    let (status_code, _) = moderator
        .request(
            "POST",
            "/chat/api/channels/2/archives",
            "archive[type]=existing_topic&archive[topic_id]=9999",
        )
        .await;
    assert_eq!(status_code, 403);
    assert_eq!(
        scalar::<i64>(&db, "SELECT COUNT(*) FROM chat_channel_archives").await,
        0
    );
}

#[tokio::test]
async fn an_archive_starts_read_only_with_its_row_and_job() {
    let db = TestDb::new().await;
    archiving_allowed(&db).await;
    let mut messages = Messages::new();
    let ids = messages.create(&db, 4, None).await;
    // A trashed message isn't counted.
    exec(
        &db,
        &format!(
            "UPDATE chat_messages SET deleted_at = now() WHERE id = {}",
            ids[3]
        ),
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let from = st.bus.now().await.unwrap();
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status_code, body) = admin
        .request(
            "POST",
            "/chat/api/channels/2/archives",
            &format!("{NEW_TOPIC}&archive[tags][]=news&archive[tags][]=gossip"),
        )
        .await;
    assert_eq!(status_code, 200, "{body}");
    assert_eq!(body, json!({"success": "OK"}));
    assert_eq!(status(&db).await, 1);
    type Row = (
        i64,
        i32,
        Option<String>,
        Option<i32>,
        Option<Vec<String>>,
        i32,
        i32,
    );
    let row: Row = sqlx::query_as(
        "SELECT id, archived_by_id, destination_topic_title, destination_category_id, destination_tags, \
           total_messages, archived_messages FROM chat_channel_archives WHERE chat_channel_id = 2",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(row.1, 1);
    assert_eq!(row.2.as_deref(), Some("This will be a new topic"));
    assert_eq!(row.3, Some(4));
    assert_eq!(row.4, Some(vec!["news".to_string(), "gossip".to_string()]));
    assert_eq!((row.5, row.6), (3, 0));
    let jobs: Vec<(String, serde_json::Value)> =
        sqlx::query_as("SELECT name, args FROM discourse_rs.jobs ORDER BY id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert!(
        jobs.iter().any(
            |(n, a)| n == "Jobs::Chat::ChannelArchive" && a["chat_channel_archive_id"] == row.0
        ),
        "{jobs:?}"
    );
    // The status change logged and published.
    let details: String = scalar(
        &db,
        "SELECT new_value FROM user_histories WHERE custom_type = 'chat_channel_status_change' AND acting_user_id = 1",
    )
    .await;
    assert_eq!(details, "read_only");
    let published = bus_messages(&st, from, &["/chat/channel-status"]).await;
    assert!(
        published
            .iter()
            .any(|m| m.data == json!({"chat_channel_id": 2, "status": "read_only"})),
        "{published:?}"
    );
    // A second request finds the archive, which hasn't failed.
    let (status_code, _) = admin
        .request("POST", "/chat/api/channels/2/archives", NEW_TOPIC)
        .await;
    assert_eq!(status_code, 403);
    assert_eq!(
        scalar::<i64>(&db, "SELECT COUNT(*) FROM chat_channel_archives").await,
        1
    );
}

#[tokio::test]
async fn an_invalid_new_topic_answers_its_errors() {
    let db = TestDb::new().await;
    archiving_allowed(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status_code, body) = admin
        .request(
            "POST",
            "/chat/api/channels/2/archives",
            "archive[type]=new_topic&archive[title]=Some+new+topic+with+too+many+emoji+:joy:+:sob:+:tada:&archive[category_id]=4",
        )
        .await;
    assert_eq!(status_code, 400, "{body}");
    assert_eq!(
        body["errors"],
        json!(["Title can't have more than 1 emoji"])
    );
    assert_eq!(
        scalar::<i64>(&db, "SELECT COUNT(*) FROM chat_channel_archives").await,
        0
    );
    assert_eq!(status(&db).await, 0);
}

#[tokio::test]
async fn an_existing_topic_starts_an_archive() {
    let db = TestDb::new().await;
    archiving_allowed(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let topic: i32 = scalar(
        &db,
        "SELECT id FROM topics WHERE archetype = 'regular' AND category_id = 4 AND deleted_at IS NULL \
           AND NOT closed AND NOT archived ORDER BY id LIMIT 1",
    )
    .await;
    let id = start(
        &st,
        &format!("archive[type]=existing_topic&archive[topic_id]={topic}"),
    )
    .await;
    assert_eq!(status(&db).await, 1);
    assert_eq!(destination(&db).await, topic);
    let jobs: i64 = scalar(
        &db,
        &format!(
            "SELECT COUNT(*) FROM discourse_rs.jobs WHERE name = 'Jobs::Chat::ChannelArchive' \
             AND (args->>'chat_channel_archive_id')::bigint = {id}"
        ),
    )
    .await;
    assert_eq!(jobs, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_topic_gets_the_messages_in_batches_and_the_channel_is_archived() {
    let db = TestDb::new().await;
    archiving_allowed(&db).await;
    let mut messages = Messages::new();
    let ids = messages.create(&db, 50, None).await;
    let reaction_message = *ids.last().unwrap();
    exec(
        &db,
        &format!(
            "INSERT INTO chat_message_reactions (chat_message_id, user_id, emoji, created_at, updated_at) \
             VALUES ({reaction_message}, 3, '+1', now(), now())"
        ),
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let id = start(
        &st,
        &format!("{NEW_TOPIC}&archive[tags][]=news&archive[tags][]=gossip"),
    )
    .await;
    let from = st.bus.now().await.unwrap();
    run(&st, id, 5).await.unwrap();

    let topic = destination(&db).await;
    let (title, category, user, archived, excerpt): (String, i32, i32, bool, Option<String>) =
        sqlx::query_as(
            "SELECT title, category_id, user_id, archived, excerpt FROM topics WHERE id = $1",
        )
        .bind(topic)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(title, "This will be a new topic");
    assert_eq!(category, 4);
    assert_eq!(user, -1);
    assert!(archived);
    assert_eq!(excerpt, None);
    let mut tags: Vec<String> = sqlx::query_scalar(
        "SELECT tags.name FROM topic_tags JOIN tags ON tags.id = topic_tags.tag_id WHERE topic_id = $1",
    )
    .bind(topic)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    tags.sort();
    assert_eq!(tags, ["gossip", "news"]);
    let posts = posts(&db, topic).await;
    assert_eq!(posts.len(), 11);
    assert_eq!(
        posts[0].1,
        "This topic is an archive of the [General](http://test.localhost/chat/c/general/2) chat channel."
    );
    let mut with_reactions = 0;
    for (_, raw, user) in &posts[1..] {
        assert!(raw.contains("[chat"), "{raw}");
        assert!(raw.contains("noLink=\"true\""), "{raw}");
        assert_eq!(*user, -1);
        if raw.contains(&format!(";{reaction_message};")) {
            assert!(raw.contains("reactions=\"+1:user1\""), "{raw}");
            with_reactions += 1;
        }
    }
    assert_eq!(with_reactions, 1);
    // The messages' text, in order, five a post.
    assert!(
        posts[1].1.contains("\nmessage 1\n[/chat]"),
        "{}",
        posts[1].1
    );
    assert!(posts[10].1.contains("message 50"), "{}", posts[10].1);

    let (archived_messages, error): (i32, Option<String>) = sqlx::query_as(
        "SELECT archived_messages, archive_error FROM chat_channel_archives WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((archived_messages, error), (50, None));
    assert_eq!(status(&db).await, 3);
    assert_eq!(live_messages(&db).await, 0);
    assert_eq!(
        scalar::<i64>(
            &db,
            "SELECT COUNT(*) FROM chat_messages WHERE chat_channel_id = 2 AND deleted_by_id = 1"
        )
        .await,
        50
    );

    // The archiver's message, with the channel's hashtag cooked.
    let (pm_title, pm_raw, pm_cooked): (String, String, String) = sqlx::query_as(
        "SELECT t.title, p.raw, p.cooked FROM topics t JOIN posts p ON p.topic_id = t.id AND p.post_number = 1 \
         JOIN topic_allowed_users tau ON tau.topic_id = t.id AND tau.user_id = 1 \
         WHERE t.archetype = 'private_message' ORDER BY t.id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(pm_title, "Chat channel archive completed successfully");
    assert!(
        pm_raw.contains(&format!(
            "#general::channel has been completed successfully. The messages were copied into the topic [This will be a new topic](http://test.localhost/t/this-will-be-a-new-topic/{topic})"
        )),
        "{pm_raw}"
    );
    assert!(
        pm_cooked.contains("class=\"hashtag-cooked\""),
        "{pm_cooked}"
    );
    assert!(pm_cooked.contains("data-type=\"channel\""), "{pm_cooked}");
    assert!(
        pm_cooked.contains("data-ref=\"general::channel\""),
        "{pm_cooked}"
    );

    // The status: archived (logged, published) and the archive's success.
    let published = bus_messages(
        &st,
        from,
        &["/chat/channel-status", "/chat/channel-archive-status"],
    )
    .await;
    assert!(
        published
            .iter()
            .any(|m| m.data == json!({"chat_channel_id": 2, "status": "archived"})),
        "{published:?}"
    );
    assert!(
        published
            .iter()
            .any(|m| m.channel == "/chat/channel-archive-status"
                && m.data
                    == json!({
                        "chat_channel_id": 2,
                        "archive_failed": false,
                        "archive_completed": true,
                        "archived_messages": 50,
                        "total_messages": 50,
                        "archive_topic_id": topic,
                    })),
        "{published:?}"
    );
    // The channel as staff read it: a completed archive.
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (_, body) = admin.request("GET", "/chat/api/channels/2", "").await;
    assert_eq!(body["channel"]["status"], "archived");
    assert_eq!(body["channel"]["archive_completed"], true);
    assert_eq!(body["channel"]["archive_topic_id"], topic);
}

#[tokio::test(flavor = "multi_thread")]
async fn threads_are_split_across_posts_with_their_ranges() {
    let db = TestDb::new().await;
    archiving_allowed(&db).await;
    exec(
        &db,
        "UPDATE chat_channels SET threading_enabled = TRUE WHERE id = 2",
    )
    .await;
    let mut messages = Messages::new();
    messages.create(&db, 2, None).await;
    messages.threaded(&db, 6, Some("a new thread")).await;
    messages.create(&db, 7, None).await;
    messages.threaded(&db, 3, None).await;
    messages
        .threaded(&db, 27, Some("another long thread"))
        .await;
    messages.create(&db, 10, None).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let id = start(&st, NEW_TOPIC).await;
    run(&st, id, 5).await.unwrap();

    let topic = destination(&db).await;
    let posts = posts(&db, topic).await;
    assert_eq!(posts.len(), 14);
    for (number, raw, user) in &posts[1..] {
        match number {
            2 => {
                assert!(raw.contains("a new thread"), "{raw}");
                assert!(raw.contains("messages 1 to 2 of 5"), "{raw}");
            }
            3 => {
                assert!(raw.contains("a new thread"), "{raw}");
                assert!(raw.contains("messages 3 to 5 of 5"), "{raw}");
            }
            5 => assert!(raw.contains("threadTitle=\"Thread\""), "{raw}"),
            10 => {
                assert!(raw.contains("another long thread"), "{raw}");
                assert!(raw.contains("messages 17 to 20 of 26"), "{raw}");
            }
            _ => {}
        }
        assert!(raw.contains("[chat"), "{raw}");
        assert!(raw.contains("noLink=\"true\""), "{raw}");
        assert_eq!(*user, -1);
    }
    assert_eq!(
        scalar::<i32>(
            &db,
            &format!("SELECT archived_messages FROM chat_channel_archives WHERE id = {id}")
        )
        .await,
        55
    );
    assert_eq!(status(&db).await, 3);
    assert_eq!(live_messages(&db).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn post_validations_and_uploads() {
    let db = TestDb::new().await;
    archiving_allowed(&db).await;
    set_setting(&db.pool, "max_post_length", 3, "1").await;
    let mut messages = Messages::new();
    let ids = messages.create(&db, 3, None).await;
    let sha1 = "b".repeat(40);
    let upload: i32 = sqlx::query_scalar(
        "INSERT INTO uploads (user_id, original_filename, filesize, width, height, url, sha1, extension, \
           created_at, updated_at) VALUES (2, 'cat.png', 2048, 10, 20, $1, $2, 'png', now(), now()) RETURNING id",
    )
    .bind(format!("/uploads/default/original/1X/{sha1}.png"))
    .bind(&sha1)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    exec(
        &db,
        &format!(
            "INSERT INTO upload_references (upload_id, target_type, target_id, created_at, updated_at) \
             VALUES ({upload}, 'ChatMessage', {}, now(), now())",
            ids[2]
        ),
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let id = start(&st, NEW_TOPIC).await;
    run(&st, id, 100).await.unwrap();
    assert_eq!(status(&db).await, 3);
    let topic = destination(&db).await;
    let posts = posts(&db, topic).await;
    assert_eq!(posts.len(), 2);
    assert!(
        posts[1]
            .1
            .contains("message 3\n\n![cat.png|10x20](upload://"),
        "{}",
        posts[1].1
    );
    let references: i64 = scalar(
        &db,
        &format!(
            "SELECT COUNT(*) FROM upload_references WHERE target_type = 'Post' AND target_id = \
             (SELECT id FROM posts WHERE topic_id = {topic} AND post_number = 2)"
        ),
    )
    .await;
    assert_eq!(references, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_topic_that_cannot_be_made_fails_the_archive() {
    let db = TestDb::new().await;
    archiving_allowed(&db).await;
    let mut messages = Messages::new();
    messages.create(&db, 3, None).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let id = start(&st, NEW_TOPIC).await;
    exec(
        &db,
        "UPDATE chat_channel_archives SET destination_topic_title = 'Wow this is the new title :tada: :joy:'",
    )
    .await;
    let from = st.bus.now().await.unwrap();
    run(&st, id, 100).await.unwrap();
    let (error, topic): (Option<String>, Option<i32>) = sqlx::query_as(
        "SELECT archive_error, destination_topic_id FROM chat_channel_archives WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(error.as_deref(), Some("Title can't have more than 1 emoji"));
    assert_eq!(topic, None);
    assert_eq!(status(&db).await, 1);
    assert_eq!(live_messages(&db).await, 3);
    let (pm_title, pm_raw): (String, String) = sqlx::query_as(
        "SELECT t.title, p.raw FROM topics t JOIN posts p ON p.topic_id = t.id AND p.post_number = 1 \
         WHERE t.archetype = 'private_message' ORDER BY t.id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(pm_title, "Chat channel archive failed");
    assert!(
        pm_raw.contains("Title can't have more than 1 emoji"),
        "{pm_raw}"
    );
    assert!(
        pm_raw.contains("No messages have been archived"),
        "{pm_raw}"
    );
    let published = bus_messages(&st, from, &["/chat/channel-archive-status"]).await;
    assert_eq!(
        published.last().map(|m| m.data.clone()),
        Some(json!({
            "chat_channel_id": 2,
            "archive_failed": true,
            "archive_completed": false,
            "archived_messages": 0,
            "total_messages": 3,
            "archive_topic_id": null,
        }))
    );
    // The channel as staff read it, with the retry the status bar offers.
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (_, body) = admin.request("GET", "/chat/api/channels/2", "").await;
    assert_eq!(body["channel"]["archive_failed"], true);
    let (_, page) = admin.page("/chat/c/general/2").await;
    assert!(page.contains("chat-channel-retry-archive"), "retry offered");
}

#[tokio::test(flavor = "multi_thread")]
async fn members_are_unfollowed() {
    let db = TestDb::new().await;
    archiving_allowed(&db).await;
    let mut messages = Messages::new();
    let ids = messages.create(&db, 3, None).await;
    exec(
        &db,
        "INSERT INTO user_chat_channel_memberships (user_id, chat_channel_id, following, created_at, updated_at) \
         SELECT u, 2, TRUE, now(), now() FROM unnest(ARRAY[2, 3, 4]) u \
         ON CONFLICT (user_id, chat_channel_id) DO UPDATE SET following = TRUE",
    )
    .await;
    exec(
        &db,
        &format!(
            "UPDATE user_chat_channel_memberships SET last_read_message_id = {} WHERE user_id = 4 AND chat_channel_id = 2",
            ids[0]
        ),
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let id = start(&st, NEW_TOPIC).await;
    run(&st, id, 100).await.unwrap();
    assert_eq!(
        scalar::<i64>(
            &db,
            "SELECT COUNT(*) FROM user_chat_channel_memberships WHERE chat_channel_id = 2 AND following"
        )
        .await,
        0
    );
    // The channel's last live message: none left.
    assert_eq!(
        scalar::<Option<i64>>(
            &db,
            "SELECT last_read_message_id FROM user_chat_channel_memberships WHERE user_id = 4 AND chat_channel_id = 2"
        )
        .await,
        None
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_destination_topic_status_setting() {
    for (setting, archived, closed) in [
        ("archived", true, false),
        ("open", false, false),
        ("closed", false, true),
    ] {
        let db = TestDb::new().await;
        archiving_allowed(&db).await;
        set_setting(
            &db.pool,
            "chat_archive_destination_topic_status",
            7,
            setting,
        )
        .await;
        let mut messages = Messages::new();
        messages.create(&db, 3, None).await;
        let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
        let id = start(&st, NEW_TOPIC).await;
        run(&st, id, 2).await.unwrap();
        let topic = destination(&db).await;
        let got: (bool, bool) = sqlx::query_as("SELECT archived, closed FROM topics WHERE id = $1")
            .bind(topic)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(got, (archived, closed), "{setting}");
        assert_eq!(posts(&db, topic).await.len(), 3, "{setting}");
        assert_eq!(status(&db).await, 3, "{setting}");
    }
}

/// A regular topic in General with three posts, by the admin.
async fn existing_topic(st: &AppState) -> i32 {
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status_code, body) = admin
        .request(
            "POST",
            "/posts.json",
            "title=An+existing+topic+for+the+archive&raw=The+first+post+of+the+existing+topic&category=4",
        )
        .await;
    assert_eq!(status_code, 200, "{body}");
    let topic = body["topic_id"].as_i64().unwrap() as i32;
    for raw in ["A second post in the topic", "A third post in the topic"] {
        let (status_code, body) = admin
            .request(
                "POST",
                "/posts.json",
                &format!("topic_id={topic}&raw={}", raw.replace(' ', "+")),
            )
            .await;
        assert_eq!(status_code, 200, "{body}");
    }
    topic
}

#[tokio::test(flavor = "multi_thread")]
async fn an_existing_topic_gets_the_messages_and_keeps_its_status() {
    let db = TestDb::new().await;
    archiving_allowed(&db).await;
    let mut messages = Messages::new();
    messages.create(&db, 50, None).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let topic = existing_topic(&st).await;
    let id = start(
        &st,
        &format!("archive[type]=existing_topic&archive[topic_id]={topic}"),
    )
    .await;
    run(&st, id, 5).await.unwrap();
    let posts = posts(&db, topic).await;
    assert_eq!(posts.len(), 13);
    for (_, raw, user) in &posts[3..] {
        assert!(raw.contains("[chat"), "{raw}");
        assert!(raw.contains("noLink=\"true\""), "{raw}");
        assert_eq!(*user, -1);
    }
    let (title, archived, closed): (String, bool, bool) =
        sqlx::query_as("SELECT title, archived, closed FROM topics WHERE id = $1")
            .bind(topic)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(title, "An existing topic for the archive");
    assert!(!archived && !closed);
    assert_eq!(status(&db).await, 3);
    assert_eq!(live_messages(&db).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failure_is_kept_told_and_retried() {
    let db = TestDb::new().await;
    archiving_allowed(&db).await;
    let mut messages = Messages::new();
    messages.create(&db, 35, None).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let topic = existing_topic(&st).await;
    let id = start(
        &st,
        &format!("archive[type]=existing_topic&archive[topic_id]={topic}"),
    )
    .await;
    // Every post to the topic fails, as the spec's stubbed create_post.
    exec(
        &db,
        "CREATE FUNCTION archive_test_error() RETURNS trigger AS $$ \
         BEGIN RAISE EXCEPTION 'this is a test error'; END $$ LANGUAGE plpgsql",
    )
    .await;
    exec(
        &db,
        &format!(
            "CREATE TRIGGER archive_test_error BEFORE INSERT ON posts FOR EACH ROW \
             WHEN (NEW.topic_id = {topic}) EXECUTE FUNCTION archive_test_error()"
        ),
    )
    .await;
    let err = run(&st, id, 5).await.unwrap_err();
    assert!(err.to_string().contains("this is a test error"), "{err}");
    let error: Option<String> = scalar(
        &db,
        &format!("SELECT archive_error FROM chat_channel_archives WHERE id = {id}"),
    )
    .await;
    let error = error.expect("the error kept");
    assert!(error.contains("this is a test error"), "{error}");
    let (pm_title, pm_raw): (String, String) = sqlx::query_as(
        "SELECT t.title, p.raw FROM topics t JOIN posts p ON p.topic_id = t.id AND p.post_number = 1 \
         JOIN topic_allowed_users tau ON tau.topic_id = t.id AND tau.user_id = 1 \
         WHERE t.archetype = 'private_message' ORDER BY t.id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(pm_title, "Chat channel archive failed");
    assert!(pm_raw.contains("0 messages have been archived"), "{pm_raw}");
    assert!(
        pm_raw.contains("Visit the channel at http://test.localhost/chat/c/general/2 to retry"),
        "{pm_raw}"
    );

    // The retry: not for a member, then the job enqueued again.
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status_code, _) = user1
        .request("POST", "/chat/api/channels/2/archives", "")
        .await;
    assert_eq!(status_code, 403);
    exec(
        &db,
        "DELETE FROM discourse_rs.jobs WHERE name = 'Jobs::Chat::ChannelArchive'",
    )
    .await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status_code, body) = admin
        .request("POST", "/chat/api/channels/2/archives", "")
        .await;
    assert_eq!(status_code, 200, "{body}");
    assert_eq!(
        scalar::<i64>(
            &db,
            "SELECT COUNT(*) FROM discourse_rs.jobs WHERE name = 'Jobs::Chat::ChannelArchive'"
        )
        .await,
        1
    );
    exec(&db, "DROP TRIGGER archive_test_error ON posts").await;
    // The job, as the worker runs it.
    discourse_rs::jobs::drain(&st).await.unwrap();
    let (archived_messages, error): (i32, Option<String>) = sqlx::query_as(
        "SELECT archived_messages, archive_error FROM chat_channel_archives WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((archived_messages, error), (35, None));
    // The job posts the default hundred messages a post: one.
    assert_eq!(posts(&db, topic).await.len(), 4);
    assert_eq!(status(&db).await, 3);
    // Complete now: the retry is refused.
    let (status_code, _) = admin
        .request("POST", "/chat/api/channels/2/archives", "")
        .await;
    assert_eq!(status_code, 403);
}

#[tokio::test]
async fn a_retry_needs_a_failed_archive_and_a_read_only_channel() {
    let db = TestDb::new().await;
    archiving_allowed(&db).await;
    exec(&db, "UPDATE chat_channels SET status = 1 WHERE id = 2").await;
    exec(
        &db,
        "INSERT INTO chat_channel_archives (chat_channel_id, destination_topic_title, archived_by_id, \
           total_messages, created_at, updated_at) VALUES (2, 'test archive topic title', 1, 10, now(), now())",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    // Not failed.
    let (status_code, _) = admin
        .request("POST", "/chat/api/channels/2/archives", "")
        .await;
    assert_eq!(status_code, 403);
    // Failed, but the channel isn't read only.
    exec(
        &db,
        "UPDATE chat_channel_archives SET archive_error = 'bad stuff', archived_messages = 1",
    )
    .await;
    exec(&db, "UPDATE chat_channels SET status = 0 WHERE id = 2").await;
    let (status_code, _) = admin
        .request("POST", "/chat/api/channels/2/archives", "")
        .await;
    assert_eq!(status_code, 403);
    exec(&db, "UPDATE chat_channels SET status = 1 WHERE id = 2").await;
    let (status_code, body) = admin
        .request("POST", "/chat/api/channels/2/archives", "")
        .await;
    assert_eq!(status_code, 200, "{body}");
    assert_eq!(body, json!({"success": "OK"}));
    assert_eq!(
        scalar::<i64>(
            &db,
            "SELECT COUNT(*) FROM discourse_rs.jobs WHERE name = 'Jobs::Chat::ChannelArchive'"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn the_job_marks_a_complete_archive_and_skips_a_missing_one() {
    let db = TestDb::new().await;
    exec(&db, "UPDATE chat_channels SET status = 1 WHERE id = 2").await;
    let id: i64 = scalar(
        &db,
        "INSERT INTO chat_channel_archives (chat_channel_id, destination_topic_title, destination_category_id, \
           archived_by_id, total_messages, archived_messages, created_at, updated_at) \
         VALUES (2, 'This will be the archive topic', 4, 1, 10, 10, now(), now()) RETURNING id",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let topics: i64 = scalar(&db, "SELECT COUNT(*) FROM topics").await;
    archive::channel_archive_job(&st, &json!({"chat_channel_archive_id": 999}))
        .await
        .unwrap();
    assert_eq!(status(&db).await, 1);
    archive::channel_archive_job(&st, &json!({"chat_channel_archive_id": id}))
        .await
        .unwrap();
    assert_eq!(
        scalar::<i64>(&db, "SELECT COUNT(*) FROM topics").await,
        topics
    );
    assert_eq!(status(&db).await, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_settings_offer_the_archive_row_to_staff_when_archiving_is_allowed() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (_, page) = admin.page("/chat/c/general/2/info/settings").await;
    assert!(!page.contains("archive-btn"), "archiving is off");
    archiving_allowed(&db).await;
    let (_, page) = admin.page("/chat/c/general/2/info/settings").await;
    assert!(page.contains("archive-btn"), "the archive row");
    assert!(page.contains("Archive channel"));
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (_, page) = user1.page("/chat/c/general/2/info/settings").await;
    assert!(!page.contains("archive-btn"), "not for members");
    exec(&db, "UPDATE chat_channels SET status = 1 WHERE id = 2").await;
    let (_, page) = admin.page("/chat/c/general/2/info/settings").await;
    assert!(!page.contains("archive-btn"), "not while read only");
}
