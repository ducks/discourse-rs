//! The job queue: what runs, what waits, what fails and why.

mod common;

use common::{TestDb, recorded_config, state};
use discourse_rs::jobs;
use serde_json::json;

#[tokio::test]
async fn due_jobs_run_and_unported_ones_fail_with_the_reason() {
    let db = TestDb::new().await;
    let app = state(db.pool.clone(), recorded_config()).await;
    let mut conn = db.pool.acquire().await.unwrap();
    jobs::enqueue(
        &mut conn,
        "post_update_topic_tracking_state",
        json!({"post_id": 1}),
    )
    .await
    .unwrap();
    jobs::enqueue(&mut conn, "no_such_job", json!({}))
        .await
        .unwrap();
    jobs::enqueue_in(
        &mut conn,
        600,
        "post_update_topic_tracking_state",
        json!({"post_id": 2}),
    )
    .await
    .unwrap();
    drop(conn);

    assert_eq!(jobs::drain(&app).await.unwrap(), 2);

    let rows: Vec<(String, Option<String>, bool)> = sqlx::query_as(
        "SELECT name, last_error, failed_at IS NOT NULL FROM discourse_rs.jobs ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2, "the completed job is deleted: {rows:?}");
    assert_eq!(rows[0].0, "no_such_job");
    assert!(rows[0].2, "an unported job fails at once");
    assert!(rows[0].1.as_deref().unwrap().contains("no_such_job"));
    assert_eq!(rows[1].0, "post_update_topic_tracking_state");
    assert!(!rows[1].2, "the delayed job waits");
}

/// CreateRecentPostSearchIndexes can't be recorded (Rails' recorder runs
/// jobs in a transaction, where CREATE INDEX CONCURRENTLY can't run): past
/// the enable size, the offset is the search_recent_posts_size'th newest
/// regular post, and the partial index is built and swapped in.
#[tokio::test]
async fn recent_post_search_index_sets_the_offset_and_swaps_the_index() {
    let db = TestDb::new().await;
    for (name, value) in [
        ("search_enable_recent_regular_posts_offset_size", "5"),
        ("search_recent_posts_size", "3"),
    ] {
        sqlx::query(
            "INSERT INTO site_settings (name, data_type, value, created_at, updated_at) \
             VALUES ($1, 3, $2, now(), now())",
        )
        .bind(name)
        .bind(value)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    let app = state(db.pool.clone(), recorded_config()).await;
    let expected: i32 = sqlx::query_scalar(
        "SELECT post_id FROM post_search_data WHERE NOT private_message \
         ORDER BY post_id DESC LIMIT 1 OFFSET 2",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    jobs::run_scheduled(&app, jobs::search_jobs::CREATE_RECENT_POST_SEARCH_INDEXES)
        .await
        .unwrap();
    let value: String = sqlx::query_scalar(
        "SELECT value FROM site_settings WHERE name = 'search_recent_regular_posts_offset_post_id'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(value, expected.to_string());
    let index: Option<String> = sqlx::query_scalar(
        "SELECT indexdef FROM pg_indexes WHERE indexname = 'idx_recent_regular_post_search_data'",
    )
    .fetch_optional(&db.pool)
    .await
    .unwrap();
    let index = index.expect("the index is swapped in");
    assert!(index.contains(&format!("post_id >= {expected}")), "{index}");
    let temp: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = 'temp_idx_recent_regular_post_search_data')",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(!temp, "the temporary index is renamed");
}

/// CleanUpUploads runs at most once per half grace period (the
/// LAST_UPLOAD_CLEANUP key), which no recording can show.
#[tokio::test]
async fn clean_up_uploads_waits_half_the_grace_period() {
    let db = TestDb::new().await;
    let public = std::env::temp_dir().join(format!("discourse-rs-cleanup-{}", std::process::id()));
    let mut config = recorded_config();
    config.public_dir = public.clone();
    let app = state(db.pool.clone(), config).await;
    let orphan = |id: i32| {
        format!(
            "INSERT INTO uploads (id, user_id, original_filename, filesize, url, sha1, extension, created_at, updated_at) \
             VALUES ({id}, 3, 'o.png', 1, '/uploads/default/original/1X/{id}.png', '{id:040x}', 'png', \
                     now() - interval '3 days', now() - interval '3 days')"
        )
    };
    sqlx::query(&orphan(9701)).execute(&db.pool).await.unwrap();
    jobs::run_scheduled(&app, jobs::upload_cleanup::CLEAN_UP_UPLOADS)
        .await
        .unwrap();
    sqlx::query(&orphan(9702)).execute(&db.pool).await.unwrap();
    jobs::run_scheduled(&app, jobs::upload_cleanup::CLEAN_UP_UPLOADS)
        .await
        .unwrap();
    let left: Vec<i32> = sqlx::query_scalar("SELECT id FROM uploads WHERE id > 9700 ORDER BY id")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        left,
        vec![9702],
        "the second run within 24 hours keeps the new orphan"
    );
    let _ = std::fs::remove_dir_all(&public);
}

/// PurgeDeletedUploads: `find -mtime +30`, a whole 31 days old removed, a
/// day younger kept.
#[tokio::test]
async fn purge_deleted_uploads_removes_files_past_the_grace_period() {
    let db = TestDb::new().await;
    let public = std::env::temp_dir().join(format!("discourse-rs-purge-{}", std::process::id()));
    let tombstone = public.join("uploads/tombstone/default/original/1X");
    std::fs::create_dir_all(&tombstone).unwrap();
    let day = std::time::Duration::from_secs(86_400);
    let now = std::time::SystemTime::now();
    for (name, age) in [
        ("old.png", day * 31 + std::time::Duration::from_secs(60)),
        (
            "young.png",
            day * 30 + std::time::Duration::from_secs(23 * 3600),
        ),
    ] {
        let path = tombstone.join(name);
        std::fs::write(&path, b"x").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(now - age)
            .unwrap();
    }
    let mut config = recorded_config();
    config.public_dir = public.clone();
    let app = state(db.pool.clone(), config).await;
    jobs::run_scheduled(&app, jobs::upload_cleanup::PURGE_DELETED_UPLOADS)
        .await
        .unwrap();
    assert!(!tombstone.join("old.png").exists());
    assert!(tombstone.join("young.png").exists());
    let _ = std::fs::remove_dir_all(&public);
}

/// PendingReviewablesReminder remembers the newest reviewable it reminded
/// of (a redis key no recording can seed): a second run sends nothing
/// until a newer one waits.
#[tokio::test]
async fn reviewables_reminder_waits_for_a_newer_reviewable() {
    let db = TestDb::new().await;
    let app = state(db.pool.clone(), recorded_config()).await;
    let reviewable = |id: i32, target: i32| {
        format!(
            "INSERT INTO reviewables (id, type, type_source, status, created_by_id, reviewable_by_moderator, \
               category_id, topic_id, score, potential_spam, target_id, target_type, target_created_by_id, payload, \
               version, latest_score, force_review, potentially_illegal, created_at, updated_at) \
             VALUES ({id}, 'ReviewableFlaggedPost', 'core', 0, 3, TRUE, 4, 35, 10, FALSE, {target}, 'Post', 2, '{{}}', 0, \
               now() - interval '3 days', FALSE, FALSE, now() - interval '3 days', now() - interval '3 days')"
        )
    };
    let reminders = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM topics WHERE user_id = -1 AND subtype = 'system_message' \
             AND title = 'There are items in the review queue that need reviewing'",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap()
    };
    sqlx::query(&reviewable(9301, 52))
        .execute(&db.pool)
        .await
        .unwrap();
    jobs::run_scheduled(&app, jobs::group_messages::PENDING_REVIEWABLES_REMINDER)
        .await
        .unwrap();
    assert_eq!(reminders().await, 1);
    jobs::run_scheduled(&app, jobs::group_messages::PENDING_REVIEWABLES_REMINDER)
        .await
        .unwrap();
    assert_eq!(reminders().await, 1);
    // A newer one would replace the first reminder, which needs
    // PostDestroyer in messages: refused rather than sent twice.
    sqlx::query(&reviewable(9302, 53))
        .execute(&db.pool)
        .await
        .unwrap();
    let refused = jobs::run_scheduled(&app, jobs::group_messages::PENDING_REVIEWABLES_REMINDER)
        .await
        .unwrap_err();
    assert!(refused.is_unsupported(), "{refused}");
    assert_eq!(reminders().await, 1);
}
