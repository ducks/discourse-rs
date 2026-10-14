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
use crate::AppError;

/// A small action post to add.
pub struct SmallAction<'a> {
    pub user_id: i32,
    pub topic_id: i32,
    pub action_code: &'a str,
    /// `bump:` (TopicStatusUpdater bumps only when opening a topic).
    pub bump: bool,
    /// `add_small_action`'s `who`: the action_code_who custom field.
    pub who: Option<&'a str>,
}

/// Creates the post and its trail, then `increment!(:moderator_posts_count)`.
/// Returns the post's id.
pub async fn add(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    action: &SmallAction<'_>,
) -> Result<i32, AppError> {
    add_with_message(conn, ctx, action, None, false).await
}

/// `add_moderator_post(user, message, post_type: small_action, silent:)`:
/// the small action with a raw message (cooked, its words counted), and
/// `silent` leaving the topic's bump alone.
pub async fn add_with_message(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    action: &SmallAction<'_>,
    message: Option<&str>,
    silent: bool,
) -> Result<i32, AppError> {
    let (topic_id, user_id) = (action.topic_id, action.user_id);
    let raw = message.unwrap_or("").trim_end().to_string();
    let cooked = if raw.is_empty() {
        String::new()
    } else {
        crate::pretty_text::cook(
            ctx.host,
            &raw,
            &crate::pretty_text::MarkdownOptions {
                topic_id: Some(i64::from(topic_id)),
                post_id: None,
                user_id: Some(i64::from(user_id)),
                force_quote_link: false,
                omit_nofollow: false,
                chat: false,
            },
        )
        .await?
    };
    let words = super::text::word_count(&raw);
    let (archetype, category_id): (String, Option<i32>) =
        sqlx::query_as("SELECT archetype, category_id FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
    let private_message = archetype == "private_message";

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
         VALUES ($1, $2, $3, $7, $8, $4, $5, clock_timestamp(), clock_timestamp(), $1, $9, $3, \
                 clock_timestamp(), clock_timestamp(), $6, FALSE, 0) \
         RETURNING id, created_at",
    )
    .bind(user_id)
    .bind(topic_id)
    .bind(post_number)
    .bind(post_types::SMALL_ACTION)
    .bind(action.action_code)
    .bind(BAKED_VERSION)
    .bind(&raw)
    .bind(&cooked)
    .bind(words)
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
    if let Some(who) = action.who.filter(|w| !w.is_empty()) {
        sqlx::query(
            "INSERT INTO post_custom_fields (post_id, name, value, created_at, updated_at) \
             VALUES ($1, 'action_code_who', $2, clock_timestamp(), clock_timestamp())",
        )
        .bind(post_id)
        .bind(who)
        .execute(&mut *conn)
        .await?;
    }
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
    // UserActionManager.post_created: a reply, or in a message one row per
    // allowed user (small actions are not filtered out).
    if private_message {
        let allowed: Vec<i32> = sqlx::query_scalar(
            "SELECT user_id FROM topic_allowed_users WHERE topic_id = $1 ORDER BY id",
        )
        .bind(topic_id)
        .fetch_all(&mut *conn)
        .await?;
        for allowed_user in allowed {
            let kind = if allowed_user == user_id {
                user_actions::NEW_PRIVATE_MESSAGE
            } else {
                user_actions::GOT_PRIVATE_MESSAGE
            };
            super::log_user_action_by(
                &mut *conn,
                kind,
                allowed_user,
                user_id,
                topic_id,
                post_id,
                created_at,
            )
            .await?;
        }
    } else {
        super::log_user_action(
            &mut *conn,
            user_actions::REPLY,
            user_id,
            topic_id,
            post_id,
            created_at,
        )
        .await?;
    }

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
    let staged: bool = sqlx::query_scalar("SELECT staged FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    if staged {
        // auto_notification_for_staging
        change_topic_user(
            &mut *conn,
            user_id,
            topic_id,
            &[TopicUserAttr::NotificationLevel(
                notification_levels::WATCHING,
                notification_reasons::AUTO_WATCH,
            )],
        )
        .await?;
    } else if !private_message {
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
    }

    // update_topic_stats: a small action changes only the time, and the
    // bump when asked.
    sqlx::query(
        "UPDATE topics SET updated_at = clock_timestamp(), bumped_at = CASE WHEN $3 THEN $2 ELSE bumped_at END \
         WHERE id = $1",
    )
    .bind(topic_id)
    .bind(created_at)
    .bind(action.bump && !silent)
    .execute(&mut *conn)
    .await?;
    crate::topic_timer::update_topic_auto_close(&mut *conn, ctx, topic_id).await?;
    // update_user_counts: only the first post's date (no count for a
    // small action, no last_posted_at).
    let first_post = sqlx::query(
        "UPDATE user_stats SET first_post_created_at = $2 \
         WHERE user_id = $1 AND first_post_created_at IS NULL",
    )
    .bind(user_id)
    .bind(created_at)
    .execute(&mut *conn)
    .await?
    .rows_affected()
        > 0;
    // UserStat's after_save
    if first_post {
        crate::badge_granter::queue_user_change(&mut *conn, ctx.settings, user_id).await?;
    }
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
    // unarchive_message (ensure_in_allowed_users skips small actions)
    if private_message {
        let users: Vec<i32> =
            sqlx::query_scalar("SELECT user_id FROM user_archived_messages WHERE topic_id = $1")
                .bind(topic_id)
                .fetch_all(&mut *conn)
                .await?;
        for archiver in users {
            crate::archived_messages::user_move_to_inbox(ctx.bus, &mut *conn, archiver, topic_id)
                .await?;
        }
        let groups: Vec<i32> =
            sqlx::query_scalar("SELECT group_id FROM group_archived_messages WHERE topic_id = $1")
                .bind(topic_id)
                .fetch_all(&mut *conn)
                .await?;
        for group_id in groups {
            crate::archived_messages::group_move_to_inbox(
                ctx.bus,
                &mut *conn,
                group_id,
                topic_id,
                Some(user_id),
            )
            .await?;
        }
    }

    // PostCreator#trigger_after_events: post_created's web hooks.
    crate::web_hooks::enqueue_post_hooks(
        &mut *conn,
        &crate::web_hooks::Payloads {
            settings: ctx.settings,
            i18n: ctx.i18n,
            config: ctx.config,
        },
        "post_created",
        post_id,
        crate::topic_view::Held {
            topic_loaded: true,
            just_created: true,
            topic_posts_count: None,
        },
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
    crate::badge_granter::queue(
        &mut *conn,
        ctx.settings,
        crate::badge_granter::Trigger::PostRevision,
        &[post_id],
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
    if !raw.is_empty() {
        index_message(&mut *conn, ctx, topic_id, post_id, &cooked, private_message).await?;
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

/// Post#index_search (after commit) for a small action with a message.
async fn index_message(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    post_id: i32,
    cooked: &str,
    private_message: bool,
) -> Result<(), AppError> {
    let s = ctx.settings;
    let (title, category_name, tags): (String, Option<String>, Option<String>) =
        sqlx::query_as(&format!(
            "SELECT t.title, (SELECT name FROM categories WHERE id = t.category_id), {} \
             FROM topics t WHERE t.id = $1",
            super::search_index::topic_tag_names_sql("t.id")
        ))
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
    let urls = crate::url::Urls {
        config: ctx.config,
        settings: s,
    };
    let base_path = ctx.config.globals.relative_url_root().to_string();
    super::search_index::index_post(
        conn,
        s,
        &super::search_index::BaseUrls {
            base_path: &base_path,
            base_url_no_prefix: urls.base_url_no_prefix()?,
            store: crate::file_store::FileStore::for_site(urls.config, s)?,
        },
        &super::search_index::PostIndex {
            post_id,
            topic_id,
            is_first_post: false,
            topic_title: &title,
            category_name: category_name.as_deref(),
            tag_names: tags.as_deref(),
            cooked,
            private_message,
        },
    )
    .await
}
