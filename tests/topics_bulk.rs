//! PUT /topics/bulk: TopicsBulkAction#delete against topics#destroy, and
//! #change_category with a blank, zero, missing or uncategorized category.

use crate::common;

use common::chat_client::Client;
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use serde_json::{Value, json};
use sqlx::PgPool;

const INTEGER: i32 = 3;
const BOOL: i32 = 5;

/// Seeded users: admin (1, staff), user0 (2), user1 (3).
const ADMIN: i32 = 1;
/// Topics in General (4): 35 by user0 with replies, 5 by system, 39 by
/// admin. 38 is user1's, alone in Sub General (34).
const BY_USER0: i32 = 35;
const BY_SYSTEM: i32 = 5;
const BY_USER1: i32 = 38;
const BY_ADMIN: i32 = 39;
const GENERAL: i32 = 4;
const SUB_GENERAL: i32 = 34;
const UNCATEGORIZED: i32 = 1;
/// `UserHistory.actions[:delete_topic]`
const DELETE_TOPIC: i32 = 18;

async fn site(db: &TestDb) -> AppState {
    state(db.pool.clone(), config(RailsEnv::Test, &[])).await
}

async fn bulk(client: &mut Client, body: &str) -> (u16, Value) {
    client.request("PUT", "/topics/bulk.json", body).await
}

async fn rows(pool: &PgPool, sql: &str) -> Value {
    sqlx::query_scalar(&format!(
        "SELECT COALESCE(jsonb_agg(to_jsonb(r)), '[]') FROM ({sql}) r"
    ))
    .fetch_one(pool)
    .await
    .unwrap()
}

/// What a topic's deletion leaves behind, without its times.
async fn deletion_effects(pool: &PgPool) -> Value {
    json!({
        "topics": rows(pool, "SELECT id, deleted_at IS NOT NULL AS deleted, deleted_by_id, closed, \
                                     posts_count, highest_post_number FROM topics ORDER BY id").await,
        "posts": rows(pool, "SELECT id, deleted_at IS NOT NULL AS deleted, deleted_by_id, user_deleted, \
                                    raw, version FROM posts ORDER BY id").await,
        "user_stats": rows(pool, "SELECT user_id, topic_count, post_count FROM user_stats ORDER BY user_id").await,
        "categories": rows(pool, "SELECT id, topic_count, post_count, latest_topic_id, latest_post_id \
                                  FROM categories ORDER BY id").await,
        "user_actions": rows(pool, "SELECT action_type, user_id, acting_user_id, target_topic_id, target_post_id \
                                    FROM user_actions ORDER BY id").await,
        "user_histories": rows(pool, "SELECT action, acting_user_id, target_user_id, topic_id, post_id, details \
                                      FROM user_histories ORDER BY id").await,
    })
}

async fn category_of(pool: &PgPool, topic_id: i32) -> Option<i32> {
    sqlx::query_scalar("SELECT category_id FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Staff deleting their own topic with another's: both go as topics#destroy
/// deletes each, the staff log kept for the other's only.
#[tokio::test]
async fn staff_delete_their_own_topics_and_others() {
    let db = TestDb::new().await;
    let mut admin = Client::logged_in(site(&db).await, "admin").await;
    let (status, body) = bulk(
        &mut admin,
        &format!("topic_ids[]={BY_ADMIN}&topic_ids[]={BY_USER1}&operation[type]=delete"),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"topic_ids": [BY_USER1, BY_ADMIN]}));

    for topic_id in [BY_ADMIN, BY_USER1] {
        let (deleted, by): (bool, Option<i32>) = sqlx::query_as(
            "SELECT deleted_at IS NOT NULL, deleted_by_id FROM topics WHERE id = $1",
        )
        .bind(topic_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!((deleted, by), (true, Some(ADMIN)), "topic {topic_id}");
        let (deleted, by): (bool, Option<i32>) = sqlx::query_as(
            "SELECT deleted_at IS NOT NULL, deleted_by_id FROM posts WHERE topic_id = $1 AND post_number = 1",
        )
        .bind(topic_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(
            (deleted, by),
            (true, Some(ADMIN)),
            "first post of {topic_id}"
        );
    }
    // perform_delete logs when `@user.id != @post.user_id`, and the bulk
    // action passes no context.
    let logged: Vec<(i32, Option<i32>, Option<String>)> = sqlx::query_as(
        "SELECT acting_user_id, topic_id, context FROM user_histories \
         WHERE action = $1 AND topic_id IN ($2, $3) ORDER BY id",
    )
    .bind(DELETE_TOPIC)
    .bind(BY_ADMIN)
    .bind(BY_USER1)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(logged, vec![(ADMIN, Some(BY_USER1), None)]);

    // The same two topics through DELETE /t/:id.
    let single = TestDb::new().await;
    let mut admin = Client::logged_in(site(&single).await, "admin").await;
    for topic_id in [BY_USER1, BY_ADMIN] {
        let (status, body) = admin
            .request("DELETE", &format!("/t/{topic_id}.json"), "")
            .await;
        assert_eq!(status, 200, "{body}");
    }
    let expected = deletion_effects(&single.pool).await;
    let got = deletion_effects(&db.pool).await;
    for (table, rows) in expected.as_object().unwrap() {
        assert_eq!(&got[table], rows, "{table}");
    }
    // The deleted topics' counters did move.
    let before = deletion_effects(&TestDb::new().await.pool).await;
    assert_ne!(before["user_stats"], got["user_stats"]);
    assert_ne!(before["categories"], got["categories"]);
}

/// A member's own topic: `can_delete_topic?` wants it theirs, a day old at
/// most and without replies, and PostDestroyer then marks it for deletion
/// (delete_removed_posts_after is 24) as topics#destroy does.
#[tokio::test]
async fn a_member_bulk_deletes_their_own_topic() {
    let db = TestDb::new().await;
    let mut user1 = Client::logged_in(site(&db).await, "user1").await;
    let body = format!(
        "topic_ids[]={BY_USER1}&topic_ids[]={BY_USER0}&topic_ids[]={BY_ADMIN}&operation[type]=delete"
    );

    // Older than a day: left alone, as the others' are.
    let before = deletion_effects(&db.pool).await;
    let (status, reply) = bulk(&mut user1, &body).await;
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply, json!({"topic_ids": []}));
    assert_eq!(deletion_effects(&db.pool).await, before);

    let single = TestDb::new().await;
    for pool in [&db.pool, &single.pool] {
        sqlx::query("UPDATE topics SET created_at = now() WHERE id = $1")
            .bind(BY_USER1)
            .execute(pool)
            .await
            .unwrap();
    }
    let (status, reply) = bulk(&mut user1, &body).await;
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply, json!({"topic_ids": [BY_USER1]}));
    let (deleted, closed): (bool, bool) =
        sqlx::query_as("SELECT deleted_at IS NOT NULL, closed FROM topics WHERE id = $1")
            .bind(BY_USER1)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!((deleted, closed), (false, true));
    let (deleted, user_deleted, raw): (bool, bool, String) = sqlx::query_as(
        "SELECT deleted_at IS NOT NULL, user_deleted, raw FROM posts WHERE topic_id = $1 AND post_number = 1",
    )
    .bind(BY_USER1)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        (deleted, user_deleted, raw.as_str()),
        (false, true, "(topic deleted by author)")
    );

    let mut user1 = Client::logged_in(site(&single).await, "user1").await;
    let (status, reply) = user1
        .request("DELETE", &format!("/t/{BY_USER1}.json"), "")
        .await;
    assert_eq!(status, 200, "{reply}");
    assert_eq!(
        deletion_effects(&db.pool).await,
        deletion_effects(&single.pool).await
    );
}

/// With allow_uncategorized_topics off nothing moves to Uncategorized,
/// asked for by leaving the category out, as 0 or by its id; a blank
/// category matches no topic; a missing one is Rails' RecordNotFound.
#[tokio::test]
async fn change_category_without_uncategorized_topics() {
    let db = TestDb::new().await;
    // The seeds leave uncategorized_category_id at -1.
    set_setting(&db.pool, "uncategorized_category_id", INTEGER, "1").await;
    set_setting(&db.pool, "allow_uncategorized_topics", BOOL, "f").await;
    let mut admin = Client::logged_in(site(&db).await, "admin").await;
    let topics = format!("topic_ids[]={BY_USER0}&topic_ids[]={BY_USER1}");
    for category in [
        String::new(),
        "&operation[category_id]=".to_string(),
        "&operation[category_id]=0".to_string(),
        format!("&operation[category_id]={UNCATEGORIZED}"),
    ] {
        let (status, body) = bulk(
            &mut admin,
            &format!("{topics}&operation[type]=change_category{category}"),
        )
        .await;
        assert_eq!(status, 200, "{category}: {body}");
        assert_eq!(body, json!({"topic_ids": []}), "{category}");
        assert_eq!(category_of(&db.pool, BY_USER0).await, Some(GENERAL));
        assert_eq!(category_of(&db.pool, BY_USER1).await, Some(SUB_GENERAL));
    }
    let (status, _) = bulk(
        &mut admin,
        &format!("{topics}&operation[type]=change_category&operation[category_id]=999"),
    )
    .await;
    assert_eq!(status, 500);
    assert_eq!(category_of(&db.pool, BY_USER0).await, Some(GENERAL));
}

/// With allow_uncategorized_topics on: no category, 0 and the
/// uncategorized category's id all move a topic there; a blank one still
/// matches no topic.
#[tokio::test]
async fn change_category_to_uncategorized() {
    let db = TestDb::new().await;
    // The seeds leave uncategorized_category_id at -1.
    set_setting(&db.pool, "uncategorized_category_id", INTEGER, "1").await;
    set_setting(&db.pool, "allow_uncategorized_topics", BOOL, "t").await;
    let mut admin = Client::logged_in(site(&db).await, "admin").await;

    let (status, body) = bulk(
        &mut admin,
        &format!("topic_ids[]={BY_USER0}&operation[type]=change_category&operation[category_id]="),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"topic_ids": []}));
    assert_eq!(category_of(&db.pool, BY_USER0).await, Some(GENERAL));

    for (topic_id, category) in [
        (BY_USER0, String::new()),
        (BY_USER1, "&operation[category_id]=0".to_string()),
        (
            BY_SYSTEM,
            format!("&operation[category_id]={UNCATEGORIZED}"),
        ),
    ] {
        let (status, body) = bulk(
            &mut admin,
            &format!("topic_ids[]={topic_id}&operation[type]=change_category{category}"),
        )
        .await;
        assert_eq!(status, 200, "{category}: {body}");
        assert_eq!(body, json!({"topic_ids": [topic_id]}), "{category}");
        assert_eq!(
            category_of(&db.pool, topic_id).await,
            Some(UNCATEGORIZED),
            "{category}"
        );
    }

    // By its id a topic already there is not updatable.
    let (status, body) = bulk(
        &mut admin,
        &format!(
            "topic_ids[]={BY_USER0}&operation[type]=change_category&operation[category_id]={UNCATEGORIZED}"
        ),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"topic_ids": []}));

    let (status, _) = bulk(
        &mut admin,
        &format!(
            "topic_ids[]={BY_USER0}&operation[type]=change_category&operation[category_id]=999"
        ),
    )
    .await;
    assert_eq!(status, 500);
    assert_eq!(category_of(&db.pool, BY_USER0).await, Some(UNCATEGORIZED));
}

/// A close's small action and PostCreator for a new topic both lock the
/// category's row and the author's user_stats row. A bulk close holding
/// one transaction over topics in two categories keeps the first one's
/// user_stats while it waits for the second category, against the same
/// user's new topic there: each topic's close commits on its own, as
/// TopicsBulkAction wraps nothing.
#[tokio::test]
async fn closing_races_the_closers_new_topics_in_the_categories_without_deadlock() {
    const ROUNDS: usize = 30;
    const TOPICS: [i32; 3] = [BY_USER0, BY_ADMIN, BY_USER1];
    let db = TestDb::new().await;
    let st = site(&db).await;
    let mut closer = Client::logged_in(st.clone(), "admin").await;
    let mut in_general = Client::logged_in(st.clone(), "admin").await;
    let mut in_sub_general = Client::logged_in(st.clone(), "admin").await;
    let close = TOPICS
        .iter()
        .map(|id| format!("topic_ids[]={id}&"))
        .collect::<String>()
        + "operation[type]=close";
    let topic = |category: i32, round: usize| {
        format!(
            "title=Racing+the+bulk+close+in+category+{category}+round+{round}\
             &raw=A+new+topic+by+the+closer+during+a+bulk+close,+round+{round}.&category={category}"
        )
    };
    let mut failed = Vec::new();
    for round in 0..ROUNDS {
        // Reopened every round, so each one adds its small actions.
        sqlx::query("UPDATE topics SET closed = FALSE WHERE id = ANY($1)")
            .bind(&TOPICS[..])
            .execute(&db.pool)
            .await
            .unwrap();
        let (general, sub_general) = (topic(GENERAL, round), topic(SUB_GENERAL, round));
        let (closed, first, second) = tokio::join!(
            bulk(&mut closer, &close),
            in_general.request("POST", "/posts.json", &general),
            in_sub_general.request("POST", "/posts.json", &sub_general),
        );
        if closed.0 == 200 && closed.1["topic_ids"] != json!([BY_USER0, BY_USER1, BY_ADMIN]) {
            failed.push(format!("round {round} close changed {}", closed.1));
        }
        for (what, (status, body)) in [("close", closed), ("topic", first), ("topic", second)] {
            if status != 200 {
                failed.push(format!("round {round} {what}: {status} {body}"));
            }
        }
    }
    assert!(
        failed.is_empty(),
        "{} failures in {ROUNDS} rounds:\n{}",
        failed.len(),
        failed.join("\n")
    );
    let (closed, small_actions, topics): (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM topics WHERE id = ANY($1) AND closed), \
                (SELECT COUNT(*) FROM posts WHERE topic_id = ANY($1) AND action_code = 'closed.enabled'), \
                (SELECT COUNT(*) FROM topics WHERE title LIKE 'Racing the bulk close in category %')",
    )
    .bind(&TOPICS[..])
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        (closed, small_actions, topics),
        (3, (ROUNDS * 3) as i64, (ROUNDS * 2) as i64)
    );
}

/// A 500's cause is only logged: show it with the failure.
fn show_errors() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::ERROR)
        .with_test_writer()
        .try_init();
}

async fn topic_counts(pool: &PgPool) -> (i32, i32) {
    sqlx::query_as(
        "SELECT (SELECT topic_count FROM categories WHERE id = $1), \
                (SELECT topic_count FROM categories WHERE id = $2)",
    )
    .bind(GENERAL)
    .bind(SUB_GENERAL)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Topic#changed_to_category counts the old category down, then the new
/// one up. A topic going from General to Sub General and another coming
/// the other way each hold the row the other counts next, unless both
/// rows are taken in one order first.
#[tokio::test]
async fn opposite_category_moves_race_without_deadlock() {
    const ROUNDS: usize = 30;
    show_errors();
    let db = TestDb::new().await;
    let st = site(&db).await;
    let mut one = Client::logged_in(st.clone(), "admin").await;
    // Two editors: one editor's moves queue on their user_stats row, which
    // counts the edit first.
    sqlx::query("UPDATE users SET moderator = TRUE WHERE username = 'user1'")
        .execute(&db.pool)
        .await
        .unwrap();
    let mut other = Client::logged_in(st.clone(), "user1").await;
    let before = topic_counts(&db.pool).await;
    let out = format!("/t/-/{BY_USER0}.json");
    let back = format!("/t/-/{BY_USER1}.json");
    let to = |category: i32| format!("category_id={category}");
    let mut failed = Vec::new();
    for round in 0..ROUNDS {
        for (first, second) in [(SUB_GENERAL, GENERAL), (GENERAL, SUB_GENERAL)] {
            let (first_body, second_body) = (to(first), to(second));
            let (moved_out, moved_back) = tokio::join!(
                one.request("PUT", &out, &first_body),
                other.request("PUT", &back, &second_body),
            );
            for (to, (status, body)) in [(first, moved_out), (second, moved_back)] {
                if status != 200 {
                    failed.push(format!("round {round} to {to}: {status} {body}"));
                }
            }
        }
    }
    assert!(
        failed.is_empty(),
        "{} failures in {ROUNDS} rounds:\n{}",
        failed.len(),
        failed.join("\n")
    );
    let categories: Vec<(i32, Option<i32>)> =
        sqlx::query_as("SELECT id, category_id FROM topics WHERE id = ANY($1) ORDER BY id")
            .bind(&[BY_USER0, BY_USER1][..])
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        categories,
        [(BY_USER0, Some(GENERAL)), (BY_USER1, Some(SUB_GENERAL))]
    );
    assert_eq!(topic_counts(&db.pool).await, before);
}

/// PostMover counts the new topic into its category first and leaves its
/// small action in the original topic last, which writes that topic's
/// category: posts split from General into Sub General and others split
/// the other way lock the two rows in opposite orders.
#[tokio::test]
async fn opposite_splits_into_new_topics_race_without_deadlock() {
    const ROUNDS: usize = 20;
    show_errors();
    let db = TestDb::new().await;
    let st = site(&db).await;
    let mut one = Client::logged_in(st.clone(), "admin").await;
    let mut other = Client::logged_in(st.clone(), "admin").await;
    let before = topic_counts(&db.pool).await;
    let mut failed = Vec::new();
    for round in 0..ROUNDS {
        let from = [BY_USER0, BY_USER1].map(|id| format!("/t/{id}/move-posts.json"));
        let mut replies = Vec::new();
        for topic in [BY_USER0, BY_USER1] {
            let (status, body) = one
                .request(
                    "POST",
                    "/posts.json",
                    &format!(
                        "topic_id={topic}&raw=A+reply+to+split+out+of+topic+{topic},+round+{round}."
                    ),
                )
                .await;
            assert_eq!(status, 200, "round {round} reply in {topic}: {body}");
            replies.push(body["id"].as_i64().unwrap());
        }
        let split = |reply: i64, category: i32| {
            format!(
                "post_ids[]={reply}&title=Split+into+category+{category}+in+round+{round}\
                 &category_id={category}"
            )
        };
        let (out, back) = (split(replies[0], SUB_GENERAL), split(replies[1], GENERAL));
        let (from_general, from_sub_general) = tokio::join!(
            one.request("POST", &from[0], &out),
            other.request("POST", &from[1], &back),
        );
        for (from, (status, body)) in [(GENERAL, from_general), (SUB_GENERAL, from_sub_general)] {
            if status != 200 || body["success"] != json!(true) {
                failed.push(format!("round {round} from {from}: {status} {body}"));
            }
        }
    }
    assert!(
        failed.is_empty(),
        "{} failures in {ROUNDS} rounds:\n{}",
        failed.len(),
        failed.join("\n")
    );
    let split: Vec<(i32, i64)> = sqlx::query_as(
        "SELECT category_id, COUNT(*) FROM topics WHERE title LIKE 'Split into category %' \
         GROUP BY category_id ORDER BY category_id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        split,
        [(GENERAL, ROUNDS as i64), (SUB_GENERAL, ROUNDS as i64)]
    );
    let after = topic_counts(&db.pool).await;
    assert_eq!(after, (before.0 + ROUNDS as i32, before.1 + ROUNDS as i32));
}

/// Jobs::PeriodicalUpdates rewrites each category's featured topics, and so
/// does a new topic in the category, holding the category's row. Without
/// that row the job's inserts meet the topic's on cat_featured_threads and
/// the job fails.
#[tokio::test]
async fn featuring_topics_races_new_topics_in_the_categories() {
    use std::sync::atomic::{AtomicBool, Ordering};

    const ROUNDS: usize = 20;
    show_errors();
    let db = TestDb::new().await;
    let st = site(&db).await;
    let mut first = Client::logged_in(st.clone(), "admin").await;
    let mut second = Client::logged_in(st.clone(), "admin").await;
    let mut third = Client::logged_in(st.clone(), "admin").await;
    let mut conn = db.pool.acquire().await.unwrap();
    let s = discourse_rs::site_settings::SiteSettings::load(
        &mut conn,
        &st.site_setting_defs,
        &st.config.globals,
    )
    .await
    .unwrap();
    let topic = |category: i32, n: usize, round: usize| {
        format!(
            "title=Racing+the+featured+topics+of+category+{category}+topic+{n}+round+{round}\
             &raw=A+new+topic+while+the+periodical+updates+feature+topics,+{n}+of+round+{round}.\
             &category={category}"
        )
    };
    let mut failed = Vec::new();
    let mut runs = 0;
    for round in 0..ROUNDS {
        let bodies = [
            topic(GENERAL, 1, round),
            topic(GENERAL, 2, round),
            topic(SUB_GENERAL, 3, round),
        ];
        let done = AtomicBool::new(false);
        // The job over and over until the round's topics are in.
        let job = async {
            let mut errors = Vec::new();
            while !done.load(Ordering::SeqCst) {
                runs += 1;
                if let Err(e) =
                    discourse_rs::jobs::periodical::feature_topics(&st, &mut conn, &s).await
                {
                    errors.push(format!("round {round} job: {e:?}"));
                }
            }
            errors
        };
        let topics = async {
            let created = tokio::join!(
                first.request("POST", "/posts.json", &bodies[0]),
                second.request("POST", "/posts.json", &bodies[1]),
                third.request("POST", "/posts.json", &bodies[2]),
            );
            done.store(true, Ordering::SeqCst);
            created
        };
        let (errors, created) = tokio::join!(job, topics);
        failed.extend(errors);
        for (status, body) in [created.0, created.1, created.2] {
            if status != 200 {
                failed.push(format!("round {round} topic: {status} {body}"));
            }
        }
    }
    assert!(
        failed.is_empty(),
        "{} failures in {ROUNDS} rounds ({runs} runs of the job):\n{}",
        failed.len(),
        failed.join("\n")
    );
    // Each category's list whole: one row a topic, ranked from 0 with no
    // gaps, each for a topic that is listed.
    let listed = rows(
        &db.pool,
        "SELECT category_id, topic_id, rank FROM category_featured_topics \
         ORDER BY category_id, rank, topic_id",
    )
    .await;
    let (broken, unlisted): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM (SELECT 1 FROM category_featured_topics GROUP BY category_id \
                                      HAVING COUNT(*) <> COUNT(DISTINCT topic_id) \
                                          OR COUNT(*) <> COUNT(DISTINCT rank) \
                                          OR COUNT(*) <> MAX(rank) + 1) r), \
                (SELECT COUNT(*) FROM category_featured_topics f \
                 WHERE NOT EXISTS (SELECT 1 FROM topics t WHERE t.id = f.topic_id \
                                     AND t.visible AND t.deleted_at IS NULL))",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((broken, unlisted), (0, 0), "{listed}");
    let topics: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM topics WHERE title LIKE 'Racing the featured topics of category %'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(topics, (ROUNDS * 3) as i64);
}
