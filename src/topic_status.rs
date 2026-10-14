//! `Topic#update_status` -> TopicStatusUpdater for closing, opening,
//! archiving, unarchiving, listing, unlisting, pinning and unpinning a
//! topic: the column, the small action post saying so, what the change
//! takes with it (featured topics, the category's count, hot scores), and
//! the staff action log.
//!
//! Refused: pinning until a time, and
//! `autoclosed` (not a status the controller takes).

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::posting::small_action::{self, SmallAction};
use crate::{AppError, Unsupported};

/// `UserHistory.actions`
const TOPIC_CLOSED: i32 = 91;
const TOPIC_OPENED: i32 = 92;
const TOPIC_ARCHIVED: i32 = 93;
const TOPIC_UNARCHIVED: i32 = 94;
/// `Topic.visibility_reasons`
const MANUALLY_UNLISTED: i32 = 3;
const MANUALLY_RELISTED: i32 = 4;

/// `TopicStatusUpdater::Status` names the controller takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status {
    Closed,
    Archived,
    Visible,
    Pinned,
    PinnedGlobally,
}

impl Status {
    /// `check_for_status_presence`
    pub fn parse(s: &str) -> Option<Status> {
        Some(match s {
            "closed" => Status::Closed,
            "archived" => Status::Archived,
            "visible" => Status::Visible,
            "pinned" => Status::Pinned,
            "pinned_globally" => Status::PinnedGlobally,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Status::Closed => "closed",
            Status::Archived => "archived",
            Status::Visible => "visible",
            Status::Pinned => "pinned",
            Status::PinnedGlobally => "pinned_globally",
        }
    }
}

/// How a status update ends when it isn't a server error.
pub enum Outcome {
    Done,
    /// `Discourse::InvalidAccess` from the guardian (also for a missing
    /// topic, which the guardian refuses).
    Forbidden,
}

#[derive(sqlx::FromRow)]
struct Topic {
    id: i32,
    archetype: String,
    category_id: Option<i32>,
}

/// `TopicsController#status` from the topic lookup on.
#[allow(clippy::too_many_arguments)]
pub async fn update(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    topic_id: i32,
    category_id: Option<i32>,
    status: Status,
    enabled: bool,
    until: Option<&str>,
) -> Result<Outcome, AppError> {
    let s = ctx.settings;
    let topic: Option<Topic> = sqlx::query_as(
        "SELECT id, archetype, category_id FROM topics WHERE id = $1 AND deleted_at IS NULL \
           AND ($2::int IS NULL OR category_id = $2)",
    )
    .bind(topic_id)
    .bind(category_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(topic) = topic else {
        return Ok(Outcome::Forbidden);
    };
    let topic_ctx = crate::topic_guardian::TopicCtx::load(&mut *conn, s, guardian, topic.id)
        .await?
        .ok_or(Unsupported("loading a topic being moderated"))?;
    let secure = guardian.secure_category_ids(&mut *conn, s).await?;
    let can_see = guardian.can_see_topic(s, &topic_ctx, true, &secure)?;
    let allowed = match status {
        Status::Closed | Status::Archived | Status::Pinned => {
            guardian.can_perform_action_available_to_group_moderators(s, can_see)?
        }
        Status::Visible => {
            guardian.can_moderate(can_see)
                || guardian.can_perform_action_available_to_group_moderators(s, can_see)?
        }
        Status::PinnedGlobally => guardian.can_moderate(can_see),
    };
    if !allowed {
        return Ok(Outcome::Forbidden);
    }
    let user = guardian
        .user()
        .ok_or(Unsupported("changing a topic's status anonymously"))?;
    apply(conn, ctx, topic.id, user.id, status, enabled, until).await?;
    Ok(Outcome::Done)
}

/// `Topic#update_status(status, enabled, user)`: TopicStatusUpdater, the
/// staff action log, and a closed message's group archives, for a topic
/// that exists.
pub async fn apply(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    user_id: i32,
    status: Status,
    enabled: bool,
    until: Option<&str>,
) -> Result<(), AppError> {
    let s = ctx.settings;
    let topic: Topic =
        sqlx::query_as("SELECT id, archetype, category_id FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
    // @topic_timer: closing and opening act on the public timer.
    let topic_timer = crate::topic_timer::public_topic_timer(&mut *conn, topic.id).await?;

    // change
    let updated = match status {
        Status::Pinned | Status::PinnedGlobally => {
            if until.is_some_and(|u| !u.trim().is_empty()) {
                return Err(Unsupported("pinning until a time (the unpin job)").into());
            }
            // update_pinned: update_columns, then the scheduled unpin goes.
            sqlx::query(
                "UPDATE topics SET pinned_at = CASE WHEN $2 THEN clock_timestamp() END, \
                                   pinned_globally = $3, pinned_until = NULL WHERE id = $1",
            )
            .bind(topic.id)
            .bind(enabled)
            .bind(status == Status::PinnedGlobally)
            .execute(&mut *conn)
            .await?;
            sqlx::query(
                "DELETE FROM discourse_rs.jobs WHERE name = 'unpin_topic' AND args->>'topic_id' = $1::text",
            )
            .bind(topic.id)
            .execute(&mut *conn)
            .await?;
            true
        }
        Status::Closed | Status::Archived | Status::Visible => {
            let column = status.name();
            sqlx::query(&format!(
                "UPDATE topics SET {column} = $2 WHERE id = $1 AND {column} = NOT $2"
            ))
            .bind(topic.id)
            .bind(enabled)
            .execute(&mut *conn)
            .await?
            .rows_affected()
                > 0
        }
    };
    if status == Status::Visible && !enabled {
        // UserProfile.remove_featured_topic_from_all_profiles
        sqlx::query(
            "UPDATE user_profiles SET featured_topic_id = NULL WHERE featured_topic_id = $1",
        )
        .bind(topic.id)
        .execute(&mut *conn)
        .await?;
    }
    if status == Status::Visible && updated {
        // update_category_topic_count_by, but not for the category's
        // definition topic.
        if let Some(category_id) = topic.category_id {
            sqlx::query(
                "UPDATE categories SET topic_count = topic_count + $3 \
                 WHERE id = $1 AND (topic_id <> $2 OR topic_id IS NULL)",
            )
            .bind(category_id)
            .bind(topic.id)
            .bind(if enabled { 1 } else { -1 })
            .execute(&mut *conn)
            .await?;
        }
        // UserStatCountUpdater on the first post: its author's topic count,
        // not below zero.
        let author: Option<Option<i32>> =
            sqlx::query_scalar("SELECT user_id FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL")
                .bind(topic.id)
                .fetch_optional(&mut *conn)
                .await?;
        if let Some(Some(author)) = author {
            let sql = if enabled {
                "UPDATE user_stats SET topic_count = topic_count + 1 WHERE user_id = $1"
            } else {
                "UPDATE user_stats SET topic_count = topic_count - 1 WHERE user_id = $1 AND topic_count >= 1"
            };
            sqlx::query(sql).bind(author).execute(&mut *conn).await?;
        }
    }
    if status == Status::Visible {
        // topic.update(visibility_reason_id:), a validated save.
        let slug = crate::posting::topic_save::reassigned_slug(&mut *conn, s, topic.id).await?;
        sqlx::query(
            "UPDATE topics SET visibility_reason_id = $2, slug = $3, fancy_title = NULL, \
                               updated_at = clock_timestamp() \
             WHERE id = $1 AND (visibility_reason_id IS DISTINCT FROM $2 OR slug IS DISTINCT FROM $3 \
                                OR fancy_title IS NOT NULL)",
        )
        .bind(topic.id)
        .bind(if enabled {
            MANUALLY_RELISTED
        } else {
            MANUALLY_UNLISTED
        })
        .bind(&slug)
        .execute(&mut *conn)
        .await?;
    }
    // The public timer: closing finishes close timers, opening finishes
    // the open timer and takes the category's auto close again.
    if topic_timer.is_some() && status == Status::Closed {
        use crate::topic_timer::{delete_topic_timer, types};
        let by = Some(user_id);
        if enabled {
            delete_topic_timer(&mut *conn, topic.id, types::CLOSE, by).await?;
            delete_topic_timer(&mut *conn, topic.id, types::SILENT_CLOSE, by).await?;
        } else {
            delete_topic_timer(&mut *conn, topic.id, types::OPEN, by).await?;
            crate::topic_timer::inherit_auto_close_from_category(
                &mut *conn,
                ctx,
                topic.id,
                types::CLOSE,
            )
            .await?;
        }
    }
    // Featured topics go when a topic is closed, archived or unlisted.
    if (enabled && matches!(status, Status::Closed | Status::Archived))
        || (!enabled && status == Status::Visible)
    {
        sqlx::query("DELETE FROM category_featured_topics WHERE topic_id = $1")
            .bind(topic.id)
            .execute(&mut *conn)
            .await?;
    }
    if status == Status::Visible && !enabled {
        sqlx::query("DELETE FROM topic_hot_scores WHERE topic_id = $1")
            .bind(topic.id)
            .execute(&mut *conn)
            .await?;
    }

    // create_moderator_post_for
    if updated {
        let action_code = format!(
            "{}.{}",
            status.name(),
            if enabled { "enabled" } else { "disabled" }
        );
        small_action::add(
            &mut *conn,
            ctx,
            &SmallAction {
                user_id,
                topic_id: topic.id,
                action_code: &action_code,
                bump: status == Status::Closed && !enabled,
                who: None,
            },
        )
        .await?;
    }
    // DiscourseEvent :topic_status_updated (pins compare the pin instead,
    // and no plugin listens for them).
    if updated && !matches!(status, Status::Pinned | Status::PinnedGlobally) {
        crate::plugins::topic_status_updated(&mut *conn, s, topic.id, status.name(), enabled)
            .await?;
    }

    // StaffActionLogger
    let logged = match status {
        Status::Closed => Some(if enabled { TOPIC_CLOSED } else { TOPIC_OPENED }),
        Status::Archived => Some(if enabled {
            TOPIC_ARCHIVED
        } else {
            TOPIC_UNARCHIVED
        }),
        _ => None,
    };
    if let Some(action) = logged {
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, topic_id, admin_only, created_at, updated_at) \
             VALUES ($1, $2, $3, FALSE, clock_timestamp(), clock_timestamp())",
        )
        .bind(action)
        .bind(user_id)
        .bind(topic.id)
        .execute(&mut *conn)
        .await?;
    }
    // Topic#update_status: a message closed is archived for each of the
    // closer's groups it is to (with no acting user).
    if enabled && status == Status::Closed && topic.archetype == "private_message" {
        let groups: Vec<i32> = sqlx::query_scalar(
            "SELECT groups.id FROM groups \
             INNER JOIN topic_allowed_groups ON groups.id = topic_allowed_groups.group_id \
             WHERE topic_allowed_groups.topic_id = $1 \
               AND topic_allowed_groups.group_id IN (SELECT group_id FROM group_users WHERE user_id = $2)",
        )
        .bind(topic.id)
        .bind(user_id)
        .fetch_all(&mut *conn)
        .await?;
        for group_id in groups {
            crate::archived_messages::group_archive(ctx.bus, &mut *conn, group_id, topic.id, None)
                .await?;
        }
    }
    Ok(())
}

/// The response's body: success, with `TopicTimer.find_by(topic:)`, any
/// live timer of the topic (private ones too).
pub async fn response(conn: &mut PgConnection, topic_id: i32) -> Result<Value, AppError> {
    let timer = crate::topic_timer::any_topic_timer(conn, topic_id).await?;
    Ok(json!({
        "success": "OK",
        "topic_status_update": timer.map(|t| t.serialize()),
    }))
}
