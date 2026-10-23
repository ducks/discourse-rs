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
/// until a newer one waits, and then replaces the first reminder.
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
    // A newer one replaces the first reminder: trashed, and a new one sent.
    sqlx::query(&reviewable(9302, 50))
        .execute(&db.pool)
        .await
        .unwrap();
    jobs::run_scheduled(&app, jobs::group_messages::PENDING_REVIEWABLES_REMINDER)
        .await
        .unwrap();
    assert_eq!(reminders().await, 2);
    let live: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM topics WHERE user_id = -1 AND subtype = 'system_message' \
         AND title = 'There are items in the review queue that need reviewing' AND deleted_at IS NULL",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(live, 1);
}

/// Upload#destroy of an upload an optimized video was made from: the
/// video's row, its reference and its upload go with it (OptimizedVideo
/// #destroy), both files to the tombstone. No recording can make an
/// optimized video (MediaConvert's job does).
#[tokio::test]
async fn clean_up_uploads_destroys_an_uploads_optimized_video() {
    let db = TestDb::new().await;
    let public = std::env::temp_dir().join(format!("discourse-rs-video-{}", std::process::id()));
    let original = public.join("uploads/default/original/1X");
    std::fs::create_dir_all(&original).unwrap();
    for name in ["o.mov", "v.mp4"] {
        std::fs::write(original.join(name), b"x").unwrap();
    }
    let mut config = recorded_config();
    config.public_dir = public.clone();
    let app = state(db.pool.clone(), config).await;
    for sql in [
        "INSERT INTO uploads (id, user_id, original_filename, filesize, url, sha1, extension, created_at, updated_at) VALUES \
         (9711, 3, 'o.mov', 1, '/uploads/default/original/1X/o.mov', '0000000000000000000000000000000000009711', 'mov', \
          now() - interval '3 days', now() - interval '3 days'), \
         (9712, 3, 'o_converted.mp4', 1, '/uploads/default/original/1X/v.mp4', '0000000000000000000000000000000000009712', 'mp4', \
          now() - interval '3 days', now() - interval '3 days')",
        "INSERT INTO optimized_videos (upload_id, optimized_upload_id, adapter, created_at, updated_at) \
         VALUES (9711, 9712, 'aws_mediaconvert', now(), now())",
        "INSERT INTO upload_references (upload_id, target_type, target_id, created_at, updated_at) \
         VALUES (9712, 'Upload', 9711, now(), now())",
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
    jobs::run_scheduled(&app, jobs::upload_cleanup::CLEAN_UP_UPLOADS)
        .await
        .unwrap();
    let left: Vec<i32> = sqlx::query_scalar("SELECT id FROM uploads WHERE id IN (9711, 9712)")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    assert!(left.is_empty(), "{left:?}");
    let rows: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM optimized_videos WHERE upload_id = 9711) \
              + (SELECT COUNT(*) FROM upload_references WHERE upload_id = 9712)",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(rows, 0);
    let tombstone = public.join("uploads/tombstone/default/original/1X");
    assert!(tombstone.join("o.mov").exists());
    assert!(tombstone.join("v.mp4").exists());
    let _ = std::fs::remove_dir_all(&public);
}

/// Destroying an optimized video's own upload leaves the video's foreign
/// key to it, which Upload#destroy does not clear: the delete fails, the
/// job with it, and the row stays, as in Rails.
#[tokio::test]
async fn clean_up_uploads_fails_on_an_optimized_videos_upload() {
    let db = TestDb::new().await;
    let public = std::env::temp_dir().join(format!("discourse-rs-video-fk-{}", std::process::id()));
    let mut config = recorded_config();
    config.public_dir = public.clone();
    let app = state(db.pool.clone(), config).await;
    for sql in [
        "INSERT INTO uploads (id, user_id, original_filename, filesize, url, sha1, extension, created_at, updated_at) VALUES \
         (9721, 3, 'o.mov', 1, '/uploads/default/original/1X/o.mov', '0000000000000000000000000000000000009721', 'mov', \
          now() - interval '3 days', now() - interval '3 days'), \
         (9722, 3, 'o_converted.mp4', 1, '/uploads/default/original/1X/v.mp4', '0000000000000000000000000000000000009722', 'mp4', \
          now() - interval '3 days', now() - interval '3 days')",
        "INSERT INTO optimized_videos (upload_id, optimized_upload_id, adapter, created_at, updated_at) \
         VALUES (9721, 9722, 'aws_mediaconvert', now(), now())",
        "INSERT INTO upload_references (upload_id, target_type, target_id, created_at, updated_at) \
         VALUES (9721, 'Post', 35, now(), now())",
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
    let error = jobs::run_scheduled(&app, jobs::upload_cleanup::CLEAN_UP_UPLOADS)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("foreign key"), "{error}");
    let left: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM uploads WHERE id IN (9721, 9722) ORDER BY id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(left, vec![9721, 9722]);
    let _ = std::fs::remove_dir_all(&public);
}

/// post_process_videos: a video container whose upload has an optimized
/// video shows it, the original kept in data-original-video-src, and the
/// post is saved (and both uploads linked).
#[tokio::test]
async fn process_post_swaps_in_an_optimized_video() {
    let db = TestDb::new().await;
    let app = state(db.pool.clone(), recorded_config()).await;
    let post_id: i32 =
        sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = 35 AND post_number = 2")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let original = "/uploads/default/original/1X/0000000000000000000000000000000000009731.mov";
    let optimized = "/uploads/default/original/1X/0000000000000000000000000000000000009732.mp4";
    for sql in [
        format!(
            "INSERT INTO uploads (id, user_id, original_filename, filesize, url, sha1, extension, created_at, updated_at) VALUES \
             (9731, 3, 'o.mov', 1, '{original}', '0000000000000000000000000000000000009731', 'mov', now(), now()), \
             (9732, 3, 'o_converted.mp4', 1, '{optimized}', '0000000000000000000000000000000000009732', 'mp4', now(), now())"
        ),
        "INSERT INTO optimized_videos (upload_id, optimized_upload_id, adapter, created_at, updated_at) \
         VALUES (9731, 9732, 'aws_mediaconvert', now(), now())"
            .to_string(),
        format!(
            "UPDATE posts SET cooked = '<p>A clip</p>\n<div class=\"video-placeholder-container\" data-video-src=\"{original}\">\n  </div>', \
             updated_at = now() - interval '1 day' WHERE id = {post_id}"
        ),
    ] {
        sqlx::query(&sql).execute(&db.pool).await.unwrap();
    }
    let mut conn = db.pool.acquire().await.unwrap();
    jobs::enqueue(&mut conn, "process_post", json!({ "post_id": post_id }))
        .await
        .unwrap();
    drop(conn);
    jobs::drain(&app).await.unwrap();
    let (cooked, fresh): (String, bool) = sqlx::query_as(
        "SELECT cooked, updated_at > now() - interval '1 hour' FROM posts WHERE id = $1",
    )
    .bind(post_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(
        cooked.contains(&format!(
            "data-video-src=\"{optimized}\" data-original-video-src=\"{original}\""
        )),
        "{cooked}"
    );
    assert!(fresh, "the post is saved");
    let linked: Vec<i64> = sqlx::query_scalar(
        "SELECT upload_id FROM upload_references WHERE target_type = 'Post' AND target_id = $1 \
         ORDER BY upload_id",
    )
    .bind(post_id)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(linked, vec![9731, 9732]);
    let failed: Option<String> = sqlx::query_scalar(
        "SELECT last_error FROM discourse_rs.jobs WHERE last_error IS NOT NULL LIMIT 1",
    )
    .fetch_optional(&db.pool)
    .await
    .unwrap();
    assert_eq!(failed, None);
}
