//! Port of lib/topics_bulk_action.rb: an operation over a list of topics
//! for one user, answered with the topics it changed (sorted) and the
//! errors it counted.

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::params;
use crate::posting::{Ctx, TopicUserAttr, change_topic_user};
use crate::topic_guardian::TopicCtx;
use crate::{AppError, Unsupported};

/// `TopicsBulkAction.operations`
pub const OPERATIONS: [&str; 23] = [
    "change_category",
    "close",
    "archive",
    "change_notification_level",
    "destroy_post_timing",
    "dismiss_posts",
    "delete",
    "unlist",
    "archive_messages",
    "move_messages_to_inbox",
    "change_tags",
    "append_tags",
    "remove_tags",
    "manage_tags",
    "relist",
    "dismiss_topics",
    "reset_bump_dates",
    "pin",
    "unpin",
    "convert_to_public_topic",
    "convert_to_private_message",
    "enable_nested_view",
    "disable_nested_view",
];

/// How `perform!` ends.
pub enum Performed {
    Done {
        /// `@changed_ids.sort`
        topic_ids: Vec<i32>,
        /// `@errors`: each message with its count, in first-seen order.
        errors: Vec<(String, i64)>,
    },
    /// `Discourse::InvalidParameters` with its message.
    InvalidParameters(String),
}

/// One run of an operation (`TopicsBulkAction.new(user, ids, operation)`).
pub struct BulkAction<'a, 'b> {
    pub conn: &'a mut PgConnection,
    pub ctx: &'a Ctx<'b>,
    pub guardian: &'a Guardian,
    pub topic_ids: Vec<i32>,
    pub operation: Map<String, Value>,
    changed: Vec<i32>,
    errors: Vec<(String, i64)>,
}

impl<'a, 'b> BulkAction<'a, 'b> {
    pub fn new(
        conn: &'a mut PgConnection,
        ctx: &'a Ctx<'b>,
        guardian: &'a Guardian,
        topic_ids: Vec<i32>,
        operation: Map<String, Value>,
    ) -> Self {
        BulkAction {
            conn,
            ctx,
            guardian,
            topic_ids,
            operation,
            changed: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn user_id(&self) -> Result<i32, AppError> {
        Ok(self
            .guardian
            .user_id()
            .ok_or(Unsupported("bulk actions anonymously"))?)
    }

    fn op(&self, key: &str) -> Option<String> {
        self.operation.get(key).and_then(params::scalar)
    }

    /// `perform!`
    pub async fn perform(mut self) -> Result<Performed, AppError> {
        let kind = self.op("type").unwrap_or_default();
        if !OPERATIONS.contains(&kind.as_str()) {
            return Ok(Performed::InvalidParameters("operation".into()));
        }
        let result = match kind.as_str() {
            "dismiss_posts" => self.dismiss_posts().await,
            "dismiss_topics" => self.dismiss_topics().await,
            "destroy_post_timing" => self.destroy_post_timing().await,
            "change_notification_level" => self.change_notification_level().await,
            _ => return Err(Unsupported("this topic bulk action (188g)").into()),
        };
        if let Err(message) = result? {
            return Ok(Performed::InvalidParameters(message));
        }
        let mut topic_ids = self.changed;
        topic_ids.sort_unstable();
        Ok(Performed::Done {
            topic_ids,
            errors: self.errors,
        })
    }

    /// `guardian.can_see?(topic)` for a topic that exists.
    async fn can_see(&mut self, topic_id: i32) -> Result<bool, AppError> {
        let s = self.ctx.settings;
        let Some(topic) = TopicCtx::load(&mut *self.conn, s, self.guardian, topic_id).await? else {
            return Ok(false);
        };
        let secure = self
            .guardian
            .secure_category_ids(&mut *self.conn, s)
            .await?;
        Ok(self.guardian.can_see_topic(s, &topic, true, &secure)?)
    }

    /// `topics`: the live topics among the ids.
    async fn topics(&mut self) -> Result<Vec<i32>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT id FROM topics WHERE id = ANY($1) AND deleted_at IS NULL ORDER BY id",
        )
        .bind(&self.topic_ids)
        .fetch_all(&mut *self.conn)
        .await
    }

    /// The user's unread channel (TopicTrackingState.publish_dismiss_new*).
    async fn publish_unread(
        &mut self,
        message_type: &str,
        topic_ids: &[i32],
    ) -> Result<(), AppError> {
        let user_id = self.user_id()?;
        let mut sorted = topic_ids.to_vec();
        sorted.sort_unstable();
        self.ctx
            .bus
            .publish(
                &mut *self.conn,
                &format!("/unread/{user_id}"),
                &json!({"message_type": message_type, "payload": {"topic_ids": sorted}}),
                Some(&[crate::bus::user_tag(user_id)]),
            )
            .await?;
        Ok(())
    }

    /// `dismiss_posts`: every given topic read up to its last post (the
    /// staff count for whisperers); all the ids count as changed.
    async fn dismiss_posts(&mut self) -> Result<Result<(), String>, AppError> {
        let user_id = self.user_id()?;
        let column = if self.guardian.is_whisperer(self.ctx.settings)? {
            "highest_staff_post_number"
        } else {
            "highest_post_number"
        };
        sqlx::query(&format!(
            "UPDATE topic_users tu SET last_read_post_number = t.{column} FROM topics t \
             WHERE t.id = tu.topic_id AND tu.user_id = $1 AND t.id = ANY($2)"
        ))
        .bind(user_id)
        .bind(&self.topic_ids)
        .execute(&mut *self.conn)
        .await?;
        let ids = self.topic_ids.clone();
        self.publish_unread("dismiss_new_posts", &ids).await?;
        self.changed.extend(ids);
        Ok(Ok(()))
    }

    /// `dismiss_topics`: the given topics still new to the user (since
    /// dismiss_topics_since_date, never read, newest first, up to
    /// max_new_topics, visible to them) dismissed.
    async fn dismiss_topics(&mut self) -> Result<Result<(), String>, AppError> {
        let user_id = self.user_id()?;
        let since = dismiss_topics_since_date(&mut *self.conn, self.ctx, user_id).await?;
        let candidates: Vec<i32> = sqlx::query_scalar(
            "SELECT topics.id FROM topics \
             LEFT JOIN topic_users ON topic_users.topic_id = topics.id AND topic_users.user_id = $1 \
             WHERE topics.deleted_at IS NULL AND topics.id = ANY($2) AND topics.created_at >= $3 \
               AND topic_users.last_read_post_number IS NULL \
             ORDER BY topics.created_at DESC LIMIT $4",
        )
        .bind(user_id)
        .bind(&self.topic_ids)
        .bind(since)
        .bind(self.ctx.settings.get("max_new_topics")?.to_i())
        .fetch_all(&mut *self.conn)
        .await?;
        let mut ids = Vec::new();
        for id in candidates {
            if self.can_see(id).await? {
                ids.push(id);
            }
        }
        if !ids.is_empty() {
            let (now,): (NaiveDateTime,) = sqlx::query_as("SELECT clock_timestamp()::timestamp")
                .fetch_one(&mut *self.conn)
                .await?;
            for id in &ids {
                sqlx::query(
                    "INSERT INTO dismissed_topic_users (topic_id, user_id, created_at) VALUES ($1, $2, $3) \
                     ON CONFLICT DO NOTHING",
                )
                .bind(id)
                .bind(user_id)
                .bind(now)
                .execute(&mut *self.conn)
                .await?;
            }
            self.publish_unread("dismiss_new", &ids).await?;
        }
        self.changed = ids;
        Ok(Ok(()))
    }

    /// `destroy_post_timing`: the user's last read post forgotten in each
    /// topic they can see.
    async fn destroy_post_timing(&mut self) -> Result<Result<(), String>, AppError> {
        let user_id = self.user_id()?;
        let whisperer = self.guardian.is_whisperer(self.ctx.settings)?;
        for topic_id in self.topics().await? {
            if !self.can_see(topic_id).await? {
                continue;
            }
            crate::read_tracking::destroy_last_for(&mut *self.conn, user_id, whisperer, topic_id)
                .await?;
            self.changed.push(topic_id);
        }
        Ok(Ok(()))
    }

    /// `change_notification_level`
    async fn change_notification_level(&mut self) -> Result<Result<(), String>, AppError> {
        let user_id = self.user_id()?;
        let Some(level) = self
            .op("notification_level_id")
            .filter(|l| !l.trim().is_empty())
        else {
            return Ok(Err("notification_level_id".into()));
        };
        let level = crate::ruby::to_i(&level) as i32;
        for topic_id in self.topics().await? {
            if !self.can_see(topic_id).await? {
                continue;
            }
            change_topic_user(
                &mut *self.conn,
                user_id,
                topic_id,
                &[TopicUserAttr::NotificationLevel(
                    level,
                    crate::posting::notification_reasons::USER_CHANGED,
                )],
            )
            .await?;
            crate::bus::publish_notification_level_change(
                self.ctx.bus,
                &mut *self.conn,
                user_id,
                topic_id,
                level,
                Some(crate::posting::notification_reasons::USER_CHANGED),
            )
            .await?;
            self.changed.push(topic_id);
        }
        Ok(Ok(()))
    }
}

/// `dismiss_topics_since_date`: the user's new-topic window (last visit,
/// always, or a number of minutes), no earlier than their creation or
/// min_new_topics_time.
pub async fn dismiss_topics_since_date(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
) -> Result<NaiveDateTime, AppError> {
    let s = ctx.settings;
    let (duration, previous_visit_at, created_at): (
        Option<i32>,
        Option<NaiveDateTime>,
        NaiveDateTime,
    ) = sqlx::query_as(
        "SELECT uo.new_topic_duration_minutes, u.previous_visit_at, u.created_at \
             FROM users u LEFT JOIN user_options uo ON uo.user_id = u.id WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    let minutes = match duration {
        Some(m) => i64::from(m),
        None => s.get("default_other_new_topic_duration_minutes")?.to_i(),
    };
    let (now,): (NaiveDateTime,) = sqlx::query_as("SELECT clock_timestamp()::timestamp")
        .fetch_one(&mut *conn)
        .await?;
    let setting_date = match minutes {
        -2 => previous_visit_at.unwrap_or(created_at),
        -1 => created_at,
        m => now - chrono::Duration::minutes(m),
    };
    let min_new = chrono::DateTime::from_timestamp(s.get("min_new_topics_time")?.to_i(), 0)
        .map(|t| t.naive_utc())
        .unwrap_or_default();
    Ok(setting_date.max(created_at).max(min_new))
}
