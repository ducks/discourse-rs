//! Writing posts: lib/post_creator.rb (with lib/topic_creator.rb and the
//! model callbacks they trigger), lib/post_revisor.rb, and the model
//! helpers both lean on. Rails' jobs, MessageBus and Redis side effects
//! are left out; what the database ends up holding is what
//! tests/writes.rs compares.

pub mod create;
pub mod links;
pub mod revise;
pub mod revisions;
pub mod search_index;
pub mod text;
pub mod validate;

use sqlx::PgConnection;

use crate::AppError;
use crate::config::Config;
use crate::i18n::I18n;
use crate::pretty_text::Host;
use crate::site_settings::SiteSettings;

/// What posting reads from the application.
pub struct Ctx<'a> {
    pub host: &'a Host,
    pub settings: &'a SiteSettings,
    pub config: &'a Config,
    pub i18n: &'a I18n,
}

impl Ctx<'_> {
    /// `I18n.t(key, count:)` with Rails' English pluralization.
    pub fn t_count(&self, key: &str, count: i64) -> String {
        let form = if count == 1 { "one" } else { "other" };
        let n = count.to_string();
        self.i18n
            .t_with(&format!("{key}.{form}"), &[("count", &n)])
            .unwrap_or_else(|| key.to_string())
    }

    pub fn t(&self, key: &str) -> String {
        self.i18n.t(key).unwrap_or(key).to_string()
    }

    /// `errors.full_messages` for an attribute error.
    pub fn full_message(&self, attribute_key: &str, message: &str) -> String {
        let attribute = self.t(attribute_key);
        format!("{attribute} {message}")
    }
}

/// `Post.types`
pub mod post_types {
    pub const REGULAR: i32 = 1;
    pub const MODERATOR_ACTION: i32 = 2;
    pub const SMALL_ACTION: i32 = 3;
    pub const WHISPER: i32 = 4;
}

/// `Post::BAKED_VERSION`
pub const BAKED_VERSION: i32 = 2;

/// `UserAction` types this slice writes.
pub mod user_actions {
    pub const NEW_TOPIC: i32 = 4;
    pub const REPLY: i32 = 5;
    pub const NEW_PRIVATE_MESSAGE: i32 = 12;
}

/// `TopicUser.notification_levels`
pub mod notification_levels {
    pub const REGULAR: i32 = 1;
    pub const TRACKING: i32 = 2;
    pub const WATCHING: i32 = 3;
}

/// `TopicUser.notification_reasons`
pub mod notification_reasons {
    pub const CREATED_TOPIC: i32 = 1;
    pub const USER_CHANGED: i32 = 2;
    pub const CREATED_POST: i32 = 4;
    pub const AUTO_WATCH_CATEGORY: i32 = 6;
    pub const AUTO_TRACK_CATEGORY: i32 = 8;
    pub const AUTO_WATCH_TAG: i32 = 10;
    pub const AUTO_TRACK_TAG: i32 = 12;
}

/// `DraftSequence.next!(user, key)` for a human user.
pub async fn next_draft_sequence(
    conn: &mut PgConnection,
    user_id: i32,
    key: &str,
) -> Result<i64, AppError> {
    if user_id <= 0 {
        return Ok(0);
    }
    let sequence: i64 = sqlx::query_scalar(
        "INSERT INTO draft_sequences (user_id, draft_key, sequence) VALUES ($1, $2, 1) \
         ON CONFLICT (user_id, draft_key) DO UPDATE SET sequence = draft_sequences.sequence + 1 \
         WHERE draft_sequences.user_id = $1 AND draft_sequences.draft_key = $2 \
         RETURNING sequence",
    )
    .bind(user_id)
    .bind(key)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query("DELETE FROM drafts WHERE user_id = $1 AND draft_key = $2")
        .bind(user_id)
        .bind(key)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "UPDATE user_stats SET draft_count = (SELECT COUNT(*) FROM drafts WHERE user_id = $1) \
         WHERE user_id = $1",
    )
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    Ok(sequence)
}

/// `DraftSequence.current(user, key)`
pub async fn current_draft_sequence(
    conn: &mut PgConnection,
    user_id: i32,
    key: &str,
) -> Result<i64, AppError> {
    if user_id <= 0 {
        return Ok(0);
    }
    let sequence: Option<i64> = sqlx::query_scalar(
        "SELECT sequence FROM draft_sequences WHERE user_id = $1 AND draft_key = $2",
    )
    .bind(user_id)
    .bind(key)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(sequence.unwrap_or(0))
}

/// One column assignment for `TopicUser.change`.
pub enum TopicUserAttr {
    Posted(bool),
    LastReadPostNumber(i32),
    LastPostedAtNow,
    Bookmarked(bool),
    LastEmailedPostNumber(i32),
    NotificationLevel(i32, i32),
}

/// `TopicUser.change(user_id, topic_id, attrs)`: update the row, or create
/// it (`create_missing_record`) with the category/tag/auto-track level.
pub async fn change_topic_user(
    conn: &mut PgConnection,
    user_id: i32,
    topic_id: i32,
    attrs: &[TopicUserAttr],
) -> Result<(), AppError> {
    let mut sets = Vec::new();
    let mut level = None;
    for a in attrs {
        match a {
            TopicUserAttr::Posted(v) => sets.push(format!("posted = {v}")),
            TopicUserAttr::LastReadPostNumber(n) => {
                sets.push(format!("last_read_post_number = {n}"))
            }
            TopicUserAttr::LastPostedAtNow => {
                sets.push("last_posted_at = clock_timestamp()".into())
            }
            TopicUserAttr::Bookmarked(v) => sets.push(format!("bookmarked = {v}")),
            TopicUserAttr::LastEmailedPostNumber(n) => {
                sets.push(format!("last_emailed_post_number = {n}"))
            }
            TopicUserAttr::NotificationLevel(l, reason) => {
                level = Some((*l, *reason));
                sets.push(format!(
                    "notification_level = {l}, notifications_reason_id = {reason}, \
                     notifications_changed_at = clock_timestamp()"
                ));
            }
        }
    }
    let updated = sqlx::query(&format!(
        "UPDATE topic_users SET {} WHERE topic_id = $1 AND user_id = $2",
        sets.join(", ")
    ))
    .bind(topic_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if updated > 0 {
        return Ok(());
    }

    let mut columns: Vec<String> = Vec::new();
    let mut values: Vec<String> = Vec::new();
    for a in attrs {
        match a {
            TopicUserAttr::Posted(v) => {
                columns.push("posted".into());
                values.push(v.to_string());
            }
            TopicUserAttr::LastReadPostNumber(n) => {
                columns.push("last_read_post_number".into());
                values.push(n.to_string());
            }
            TopicUserAttr::LastPostedAtNow => {
                columns.push("last_posted_at".into());
                values.push("clock_timestamp()".into());
            }
            TopicUserAttr::Bookmarked(v) => {
                columns.push("bookmarked".into());
                values.push(v.to_string());
            }
            TopicUserAttr::LastEmailedPostNumber(n) => {
                columns.push("last_emailed_post_number".into());
                values.push(n.to_string());
            }
            TopicUserAttr::NotificationLevel(..) => {}
        }
    }
    let level = match level {
        Some(l) => Some(l),
        None => missing_record_level(conn, user_id, topic_id).await?,
    };
    if let Some((l, reason)) = level {
        columns.push("notification_level".into());
        values.push(l.to_string());
        if let Some(reason) = Some(reason).filter(|r| *r != 0) {
            columns.push("notifications_reason_id".into());
            values.push(reason.to_string());
            columns.push("notifications_changed_at".into());
            values.push("clock_timestamp()".into());
        }
    }
    sqlx::query(&format!(
        "INSERT INTO topic_users (user_id, topic_id, first_visited_at, last_visited_at{}) \
         VALUES ($1, $2, clock_timestamp(), clock_timestamp(){})",
        columns.iter().map(|c| format!(", {c}")).collect::<String>(),
        values.iter().map(|v| format!(", {v}")).collect::<String>(),
    ))
    .bind(user_id)
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `create_missing_record`'s level without one given: the watched or
/// tracked category or tag, else tracking when the user auto-tracks
/// immediately. A reason of 0 sets the level alone.
async fn missing_record_level(
    conn: &mut PgConnection,
    user_id: i32,
    topic_id: i32,
) -> Result<Option<(i32, i32)>, AppError> {
    let (category_level, tag_level, pm, auto_track_after): (
        Option<i32>,
        Option<i32>,
        bool,
        Option<i32>,
    ) = sqlx::query_as(
        "SELECT \
           (SELECT notification_level FROM category_users WHERE user_id = $1 \
              AND category_id IN (SELECT category_id FROM topics WHERE id = $2) \
              AND notification_level IN (3, 2) ORDER BY notification_level DESC LIMIT 1), \
           (SELECT notification_level FROM tag_users WHERE user_id = $1 \
              AND tag_id IN (SELECT tag_id FROM topic_tags WHERE topic_id = $2) \
              AND notification_level IN (3, 2) ORDER BY notification_level DESC LIMIT 1), \
           EXISTS (SELECT 1 FROM topics WHERE id = $2 AND archetype = 'private_message'), \
           (SELECT auto_track_topics_after_msecs FROM user_options WHERE user_id = $1)",
    )
    .bind(user_id)
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    if let Some(c) = category_level {
        if tag_level.is_none_or(|t| t <= c) {
            let reason = if c == notification_levels::WATCHING {
                notification_reasons::AUTO_WATCH_CATEGORY
            } else {
                notification_reasons::AUTO_TRACK_CATEGORY
            };
            return Ok(Some((c, reason)));
        }
    }
    if let Some(t) = tag_level {
        let reason = if t == notification_levels::WATCHING {
            notification_reasons::AUTO_WATCH_TAG
        } else {
            notification_reasons::AUTO_TRACK_TAG
        };
        return Ok(Some((t, reason)));
    }
    if pm {
        return Err(crate::Unsupported("topic_users for messages").into());
    }
    // auto_track_topics_after_msecs <= total_msecs_viewed (0): immediately.
    let after = match auto_track_after {
        Some(a) => a,
        None => return Err(crate::Unsupported("users without user_options").into()),
    };
    if after == 0 {
        return Ok(Some((notification_levels::TRACKING, 0)));
    }
    Ok(None)
}

/// `TopicUser.auto_notification`
pub async fn auto_notification(
    conn: &mut PgConnection,
    user_id: i32,
    topic_id: i32,
    reason: i32,
    level: i32,
) -> Result<(), AppError> {
    let should_change: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topic_users WHERE user_id = $1 AND topic_id = $2 \
           AND (notifications_reason_id IS NULL OR (notification_level < $3 AND notification_level > $4)))",
    )
    .bind(user_id)
    .bind(topic_id)
    .bind(level)
    .bind(notification_levels::REGULAR)
    .fetch_one(&mut *conn)
    .await?;
    if should_change {
        change_topic_user(
            conn,
            user_id,
            topic_id,
            &[TopicUserAttr::NotificationLevel(level, reason)],
        )
        .await?;
    }
    Ok(())
}

/// `PostTiming.record_timing`
pub async fn record_timing(
    conn: &mut PgConnection,
    topic_id: i32,
    user_id: i32,
    post_number: i32,
    msecs: i32,
) -> Result<(), AppError> {
    let updated = sqlx::query(
        "UPDATE post_timings SET msecs = msecs + $4 \
         WHERE topic_id = $1 AND user_id = $2 AND post_number = $3",
    )
    .bind(topic_id)
    .bind(user_id)
    .bind(post_number)
    .bind(msecs)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if updated > 0 {
        return Ok(());
    }
    let inserted = sqlx::query(
        "INSERT INTO post_timings (topic_id, user_id, post_number, msecs) \
         SELECT $1, $2, $3, $4 ON CONFLICT DO NOTHING",
    )
    .bind(topic_id)
    .bind(user_id)
    .bind(post_number)
    .bind(msecs)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Ok(());
    }
    sqlx::query("UPDATE posts SET reads = reads + 1 WHERE topic_id = $1 AND post_number = $2")
        .bind(topic_id)
        .bind(post_number)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "UPDATE user_stats SET posts_read_count = posts_read_count + 1 WHERE user_id = $1 \
         AND NOT EXISTS (SELECT 1 FROM topics WHERE id = $2 AND archetype = 'private_message')",
    )
    .bind(user_id)
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `UserAction.log_action!` for the row types posting writes (no like
/// counts to bump).
pub async fn log_user_action(
    conn: &mut PgConnection,
    action_type: i32,
    user_id: i32,
    target_topic_id: i32,
    target_post_id: i32,
    created_at: chrono::NaiveDateTime,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO user_actions (action_type, user_id, acting_user_id, target_topic_id, target_post_id, \
                                   created_at, updated_at) \
         SELECT $1, $2, $2, $3, $4, $5, clock_timestamp() \
         WHERE NOT EXISTS (SELECT 1 FROM user_actions WHERE action_type = $1 AND user_id = $2 \
           AND acting_user_id = $2 AND target_topic_id = $3 AND target_post_id = $4)",
    )
    .bind(action_type)
    .bind(user_id)
    .bind(target_topic_id)
    .bind(target_post_id)
    .bind(created_at)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
