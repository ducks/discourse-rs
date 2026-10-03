//! `Topic#add_moderator_post` for a small action (`add_small_action`, and
//! TopicStatusUpdater's status posts): PostCreator with skip_validations
//! for an empty post of type small_action carrying an action code.
//!
//! Small actions are PostCreator's special case throughout: the post number
//! comes from the posts alone (the topic's counters stay), the topic keeps
//! its last poster and word count and bumps only when asked, the author's
//! post count stays, the after-create jobs (topic tracking state, mailing
//! lists) and the search index are skipped.

use serde_json::json;
use sqlx::PgConnection;

use super::{
    BAKED_VERSION, Ctx, TopicUserAttr, change_topic_user, current_draft_sequence,
    notification_levels, notification_reasons, post_types, record_timing, user_actions,
};
use crate::{AppError, Unsupported};

/// A small action post to add.
pub struct SmallAction<'a> {
    pub user_id: i32,
    pub topic_id: i32,
    pub action_code: &'a str,
    /// `bump:` (TopicStatusUpdater bumps only when opening a topic).
    pub bump: bool,
}

/// Creates the post and its trail, then `increment!(:moderator_posts_count)`.
/// Returns the post's id.
pub async fn add(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    action: &SmallAction<'_>,
) -> Result<i32, AppError> {
    let (topic_id, user_id) = (action.topic_id, action.user_id);
    let (archetype, category_id): (String, Option<i32>) =
        sqlx::query_as("SELECT archetype, category_id FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
    if archetype == "private_message" {
        return Err(Unsupported("small action posts in messages").into());
    }

    // build_post_stats: the topic draft's saves.
    let draft_key = format!("topic_{topic_id}");
    let sequence = current_draft_sequence(&mut *conn, user_id, &draft_key).await?;
    let drafts_saved: i32 = sqlx::query_scalar(
        "SELECT revisions FROM drafts WHERE sequence = $1 AND user_id = $2 AND draft_key = $3 LIMIT 1",
    )
    .bind(sequence)
    .bind(user_id)
    .bind(&draft_key)
    .fetch_optional(&mut *conn)
    .await?
    .unwrap_or(0);
    let notice = super::create::post_notice(&mut *conn, ctx, user_id).await?;

    // Topic.next_post_number for a small action: the posts' highest plus
    // one, the topic's counters untouched.
    let post_number: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(post_number), 0) + 1 FROM posts WHERE topic_id = $1",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    let (post_id, created_at): (i32, chrono::NaiveDateTime) = sqlx::query_as(
        "INSERT INTO posts (user_id, topic_id, post_number, raw, cooked, post_type, action_code, created_at, \
                            updated_at, last_editor_id, word_count, sort_order, last_version_at, baked_at, \
                            baked_version, wiki, quote_count) \
         VALUES ($1, $2, $3, '', '', $4, $5, clock_timestamp(), clock_timestamp(), $1, 0, $3, \
                 clock_timestamp(), clock_timestamp(), $6, FALSE, 0) \
         RETURNING id, created_at",
    )
    .bind(user_id)
    .bind(topic_id)
    .bind(post_number)
    .bind(post_types::SMALL_ACTION)
    .bind(action.action_code)
    .bind(BAKED_VERSION)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO post_stats (post_id, drafts_saved, typing_duration_msecs, composer_open_duration_msecs, \
                                 created_at, updated_at) \
         VALUES ($1, $2, 0, 0, clock_timestamp(), clock_timestamp())",
    )
    .bind(post_id)
    .bind(drafts_saved)
    .execute(&mut *conn)
    .await?;
    if let Some(notice) = notice {
        sqlx::query(
            "INSERT INTO post_custom_fields (post_id, name, value, created_at, updated_at) \
             VALUES ($1, 'notice', $2, clock_timestamp(), clock_timestamp())",
        )
        .bind(post_id)
        .bind(notice.to_string())
        .execute(&mut *conn)
        .await?;
    }
    // UserActionManager.post_created: a reply.
    super::log_user_action(
        &mut *conn,
        user_actions::REPLY,
        user_id,
        topic_id,
        post_id,
        created_at,
    )
    .await?;

    // track_topic
    change_topic_user(
        &mut *conn,
        user_id,
        topic_id,
        &[
            TopicUserAttr::Posted(true),
            TopicUserAttr::LastReadPostNumber(post_number),
            TopicUserAttr::LastPostedAtNow,
        ],
    )
    .await?;
    record_timing(&mut *conn, topic_id, user_id, post_number, 5000).await?;
    let replying_level: Option<i32> = sqlx::query_scalar(
        "SELECT notification_level_when_replying FROM user_options WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?
    .flatten();
    super::auto_notification(
        &mut *conn,
        user_id,
        topic_id,
        notification_reasons::CREATED_POST,
        replying_level.unwrap_or(notification_levels::TRACKING),
    )
    .await?;

    // update_topic_stats: a small action changes only the time, and the
    // bump when asked.
    sqlx::query(
        "UPDATE topics SET updated_at = clock_timestamp(), bumped_at = CASE WHEN $3 THEN $2 ELSE bumped_at END \
         WHERE id = $1",
    )
    .bind(topic_id)
    .bind(created_at)
    .bind(action.bump)
    .execute(&mut *conn)
    .await?;
    // update_topic_auto_close
    let timers: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topic_timers WHERE topic_id = $1 AND deleted_at IS NULL)",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    if timers {
        return Err(Unsupported("topic timers").into());
    }
    // update_user_counts: only the first post's date (no count for a
    // small action, no last_posted_at).
    sqlx::query(
        "UPDATE user_stats SET first_post_created_at = COALESCE(first_post_created_at, $2) WHERE user_id = $1",
    )
    .bind(user_id)
    .bind(created_at)
    .execute(&mut *conn)
    .await?;
    // delete_owned_bookmarks (on_owner_reply), then topic_users.bookmarked
    sqlx::query(
        "DELETE FROM bookmarks WHERE id IN (SELECT bookmarks.id FROM bookmarks \
           LEFT JOIN posts ON posts.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Post' \
           LEFT JOIN topics ON (topics.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Topic') \
                            OR (topics.id = posts.topic_id) \
           WHERE bookmarks.user_id = $1 AND (topics.id = $2 OR posts.topic_id = $2) \
             AND posts.deleted_at IS NULL AND topics.deleted_at IS NULL \
             AND bookmarks.auto_delete_preference = 2)",
    )
    .bind(user_id)
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    let bookmarked: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM bookmarks \
           LEFT JOIN posts ON posts.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Post' \
           LEFT JOIN topics ON (topics.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Topic') \
                            OR (topics.id = posts.topic_id) \
           WHERE bookmarks.user_id = $1 AND (topics.id = $2 OR posts.topic_id = $2) \
             AND posts.deleted_at IS NULL AND topics.deleted_at IS NULL)",
    )
    .bind(user_id)
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    change_topic_user(
        &mut *conn,
        user_id,
        topic_id,
        &[TopicUserAttr::Bookmarked(bookmarked)],
    )
    .await?;

    // PostJobsEnqueuer: no after-create jobs for a small action.
    crate::jobs::enqueue(
        &mut *conn,
        "post_alert",
        json!({"post_id": post_id, "new_record": true, "options": null}),
    )
    .await?;
    crate::jobs::enqueue(
        &mut *conn,
        "feature_topic_users",
        json!({"topic_id": topic_id}),
    )
    .await?;
    crate::jobs::enqueue(
        &mut *conn,
        "process_post",
        json!({"bypass_bump": false, "cooking_options": null, "new_post": true, "post_id": post_id, "skip_pull_hotlinked_images": false}),
    )
    .await?;

    // track_latest_on_category; the search index skips an empty raw.
    if let Some(category_id) = category_id {
        sqlx::query("UPDATE categories SET latest_post_id = $2 WHERE id = $1")
            .bind(category_id)
            .bind(post_id)
            .execute(&mut *conn)
            .await?;
    }
    // add_moderator_post
    sqlx::query(
        "UPDATE topics SET moderator_posts_count = moderator_posts_count + 1 WHERE id = $1",
    )
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    Ok(post_id)
}
