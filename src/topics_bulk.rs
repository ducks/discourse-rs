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
use crate::topic_status::{self, Status, StatusOpts};
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
    /// For the operations that save through their own transactions
    /// (revisions, conversions), as Rails commits each topic's.
    pub pool: &'a sqlx::PgPool,
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
        pool: &'a sqlx::PgPool,
        ctx: &'a Ctx<'b>,
        guardian: &'a Guardian,
        topic_ids: Vec<i32>,
        operation: Map<String, Value>,
    ) -> Self {
        BulkAction {
            conn,
            pool,
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
            "change_category" => self.change_category().await,
            "close" => self.status_change(Status::Closed, true).await,
            "archive" => self.status_change(Status::Archived, true).await,
            "unlist" => self.status_change(Status::Visible, false).await,
            "relist" => self.status_change(Status::Visible, true).await,
            "pin" => self.pin().await,
            "unpin" => self.unpin().await,
            "delete" => self.delete().await,
            "reset_bump_dates" => self.reset_bump_dates().await,
            "archive_messages" => self.archive_messages(true).await,
            "move_messages_to_inbox" => self.archive_messages(false).await,
            "convert_to_public_topic" => self.convert(true).await,
            "convert_to_private_message" => self.convert(false).await,
            "enable_nested_view" | "disable_nested_view" => self.toggle_nested_view().await,
            _ => return Err(Unsupported("tags in topic bulk actions (189)").into()),
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

    /// `topics`: the live topics among the ids, in the order Postgres reads
    /// the table (Rails iterates them without an order).
    async fn topics(&mut self) -> Result<Vec<i32>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT id FROM topics WHERE id = ANY($1) AND deleted_at IS NULL ORDER BY ctid",
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

/// `ActiveModel::Type::Boolean.new.cast`, as #bulk casts silent,
/// pinned_globally and remove_all_tags.
fn cast_bool(value: &str) -> bool {
    !matches!(
        value,
        "" | "0" | "f" | "F" | "false" | "FALSE" | "off" | "OFF"
    )
}

/// `Topic.visibility_reasons[:bulk_action]`
const BULK_ACTION: i32 = 5;

impl BulkAction<'_, '_> {
    fn flag(&self, key: &str, default: bool) -> bool {
        match self.op(key) {
            Some(v) => cast_bool(&v),
            None => default,
        }
    }

    fn count_error(&mut self, message: String, by: i64) {
        match self.errors.iter_mut().find(|(m, _)| *m == message) {
            Some((_, count)) => *count += by,
            None => self.errors.push((message, by)),
        }
    }

    async fn topic_ctx(&mut self, topic_id: i32) -> Result<Option<(TopicCtx, bool)>, AppError> {
        let s = self.ctx.settings;
        let Some(topic) = TopicCtx::load(&mut *self.conn, s, self.guardian, topic_id).await? else {
            return Ok(None);
        };
        let secure = self
            .guardian
            .secure_category_ids(&mut *self.conn, s)
            .await?;
        let can_see = self.guardian.can_see_topic(s, &topic, true, &secure)?;
        Ok(Some((topic, can_see)))
    }

    /// `guardian.can_moderate?(topic)`
    async fn can_moderate(&mut self, topic_id: i32) -> Result<bool, AppError> {
        Ok(match self.topic_ctx(topic_id).await? {
            Some((_, can_see)) => self.guardian.can_moderate(can_see),
            None => false,
        })
    }

    fn user(&self) -> Result<i32, AppError> {
        self.user_id()
    }

    /// `change_category`: through PostRevisor on each first post, silent
    /// unless asked otherwise, without a bump or the edit rate limit.
    async fn change_category(&mut self) -> Result<Result<(), String>, AppError> {
        let s = self.ctx.settings;
        let category = self.op("category_id").unwrap_or_default();
        let category_id = crate::ruby::to_i(&category) as i32;
        let updatable: Vec<i32> = sqlx::query_scalar(
            "SELECT id FROM topics WHERE id = ANY($1) AND deleted_at IS NULL AND category_id <> $2 ORDER BY ctid",
        )
        .bind(&self.topic_ids)
        .bind(category_id)
        .fetch_all(&mut *self.conn)
        .await?;
        if category_id > 0 {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE id = $1)")
                    .bind(category_id)
                    .fetch_one(&mut *self.conn)
                    .await?;
            if !exists {
                return Err(
                    Unsupported("a bulk move to a missing category (RecordNotFound)").into(),
                );
            }
            if !crate::posting::revise_topic::can_move_topic_to_category(
                &mut *self.conn,
                s,
                self.guardian,
                category_id,
            )
            .await?
            {
                let message = self
                    .ctx
                    .t("category.errors.move_topic_to_category_disallowed");
                self.count_error(message, updatable.len() as i64);
                return Ok(Ok(()));
            }
        }
        let silent = self.flag("silent", true);
        for topic_id in updatable {
            let Some((topic, can_see)) = self.topic_ctx(topic_id).await? else {
                continue;
            };
            let can_post_anywhere = self
                .guardian
                .can_create_post_anywhere(&mut *self.conn, s)
                .await?;
            if !self
                .guardian
                .can_edit_topic(s, &topic, can_see, can_post_anywhere)?
            {
                continue;
            }
            let has_first_post: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL)",
            )
            .bind(topic_id)
            .fetch_one(&mut *self.conn)
            .await?;
            if !has_first_post {
                continue;
            }
            let fields = crate::posting::revise_topic::TopicFields {
                category_id: Some(category_id),
                ..Default::default()
            };
            let opts = crate::posting::revise_topic::Opts {
                bypass_bump: true,
                bypass_rate_limiter: true,
                silent,
                ..Default::default()
            };
            match crate::posting::revise_topic::revise(
                self.pool,
                self.ctx,
                self.guardian,
                topic_id,
                &fields,
                opts,
            )
            .await?
            {
                crate::posting::revise_topic::Outcome::Revised => self.changed.push(topic_id),
                crate::posting::revise_topic::Outcome::Invalid(errors) => {
                    for message in errors {
                        self.count_error(message, 1);
                    }
                }
                crate::posting::revise_topic::Outcome::RateLimited(_) => {
                    return Err(Unsupported("the edit rate limit on a bulk revision").into());
                }
            }
        }
        Ok(Ok(()))
    }

    /// close, archive, unlist and relist: `update_status` on each topic the
    /// user may moderate (a close with the operation's message, the
    /// visibility changes as a bulk action).
    async fn status_change(
        &mut self,
        status: Status,
        enabled: bool,
    ) -> Result<Result<(), String>, AppError> {
        let user_id = self.user()?;
        let message = self.op("message").filter(|m| !m.is_empty());
        for topic_id in self.topics().await? {
            if !self.can_moderate(topic_id).await? {
                continue;
            }
            let opts = StatusOpts {
                message: if status == Status::Closed {
                    message.as_deref()
                } else {
                    None
                },
                visibility_reason_id: (status == Status::Visible).then_some(BULK_ACTION),
                ..Default::default()
            };
            topic_status::apply(
                &mut *self.conn,
                self.ctx,
                topic_id,
                user_id,
                status,
                enabled,
                &opts,
            )
            .await?;
            self.changed.push(topic_id);
        }
        Ok(Ok(()))
    }

    /// `pin`: pinned, or pinned globally, until the operation's time.
    async fn pin(&mut self) -> Result<Result<(), String>, AppError> {
        let user_id = self.user()?;
        let status = if self.flag("pinned_globally", false) {
            Status::PinnedGlobally
        } else {
            Status::Pinned
        };
        let until = self.op("pinned_until");
        for topic_id in self.topics().await? {
            if !self.can_moderate(topic_id).await? {
                continue;
            }
            let opts = StatusOpts {
                until: until.as_deref(),
                ..Default::default()
            };
            topic_status::apply(
                &mut *self.conn,
                self.ctx,
                topic_id,
                user_id,
                status,
                true,
                &opts,
            )
            .await?;
            self.changed.push(topic_id);
        }
        Ok(Ok(()))
    }

    /// `unpin`: each pinned topic unpinned as it was pinned.
    async fn unpin(&mut self) -> Result<Result<(), String>, AppError> {
        let user_id = self.user()?;
        for topic_id in self.topics().await? {
            if !self.can_moderate(topic_id).await? {
                continue;
            }
            let pin: (Option<NaiveDateTime>, bool) =
                sqlx::query_as("SELECT pinned_at, pinned_globally FROM topics WHERE id = $1")
                    .bind(topic_id)
                    .fetch_one(&mut *self.conn)
                    .await?;
            if pin.0.is_none() {
                continue;
            }
            let status = if pin.1 {
                Status::PinnedGlobally
            } else {
                Status::Pinned
            };
            topic_status::apply(
                &mut *self.conn,
                self.ctx,
                topic_id,
                user_id,
                status,
                false,
                &StatusOpts::default(),
            )
            .await?;
            self.changed.push(topic_id);
        }
        Ok(Ok(()))
    }

    /// `delete`: each first post the user may delete, through
    /// PostDestroyer.
    async fn delete(&mut self) -> Result<Result<(), String>, AppError> {
        let s = self.ctx.settings;
        let user_id = self.user()?;
        for topic_id in self.topics().await? {
            let Some((topic, _)) = self.topic_ctx(topic_id).await? else {
                continue;
            };
            if !self.guardian.can_delete_topic(s, &topic)? {
                continue;
            }
            let first: Option<(i32, Option<NaiveDateTime>, Option<i32>)> = sqlx::query_as(
                "SELECT id, deleted_at, user_id FROM posts WHERE topic_id = $1 ORDER BY post_number LIMIT 1",
            )
            .bind(topic_id)
            .fetch_optional(&mut *self.conn)
            .await?;
            let Some((post_id, deleted_at, author)) = first else {
                continue;
            };
            if deleted_at.is_some() {
                return Err(
                    Unsupported("bulk deleting a topic whose first post is deleted").into(),
                );
            }
            if !self.guardian.is_staff() || author == Some(user_id) {
                return Err(Unsupported("bulk deleting one's own topics").into());
            }
            crate::post_destroyer::destroy_by_staff_in(
                &mut *self.conn,
                self.ctx,
                self.guardian,
                post_id,
                &crate::post_destroyer::Request {
                    context: None,
                    reviewable_id: None,
                    deferred: None,
                },
            )
            .await?;
            self.changed.push(topic_id);
        }
        Ok(Ok(()))
    }

    /// `reset_bump_dates`: for staff and TL4, each visible topic bumped
    /// back to its last regular post.
    async fn reset_bump_dates(&mut self) -> Result<Result<(), String>, AppError> {
        if !(self.guardian.is_staff() || self.guardian.has_trust_level(4)) {
            return Ok(Ok(()));
        }
        for topic_id in self.topics().await? {
            if !self.can_see(topic_id).await? {
                continue;
            }
            crate::post_destroyer::reset_bumped_at(&mut *self.conn, topic_id).await?;
            self.changed.push(topic_id);
        }
        Ok(Ok(()))
    }

    /// `archive_messages` and `move_messages_to_inbox` for the user's own
    /// archive.
    async fn archive_messages(&mut self, archive: bool) -> Result<Result<(), String>, AppError> {
        let user_id = self.user()?;
        for topic_id in self.topics().await? {
            let Some((topic, can_see)) = self.topic_ctx(topic_id).await? else {
                continue;
            };
            if !can_see || !topic.private_message() {
                continue;
            }
            if archive {
                crate::archived_messages::user_archive(
                    self.ctx.bus,
                    &mut *self.conn,
                    user_id,
                    topic_id,
                )
                .await?;
            } else {
                crate::archived_messages::user_move_to_inbox(
                    self.ctx.bus,
                    &mut *self.conn,
                    user_id,
                    topic_id,
                )
                .await?;
            }
            self.changed.push(topic_id);
        }
        Ok(Ok(()))
    }

    /// `bulk_convert`: TopicConverter, silent unless asked otherwise, on
    /// each topic of the other kind the user may convert.
    async fn convert(&mut self, from_pm: bool) -> Result<Result<(), String>, AppError> {
        let s = self.ctx.settings;
        let silent = self.flag("silent", true);
        let category_id = self.op("category_id").map(|c| crate::ruby::to_i(&c) as i32);
        for topic_id in self.topics().await? {
            let Some((topic, _)) = self.topic_ctx(topic_id).await? else {
                continue;
            };
            if topic.private_message() != from_pm {
                continue;
            }
            let can_post_anywhere = self
                .guardian
                .can_create_post_anywhere(&mut *self.conn, s)
                .await?;
            let can_create_post = self
                .guardian
                .can_create_post(s, &topic, can_post_anywhere)?;
            if !self
                .guardian
                .can_convert_topic(s, &topic, can_create_post)?
            {
                continue;
            }
            let converted = if from_pm {
                crate::topic_converter::to_public(
                    self.pool,
                    self.ctx,
                    self.guardian,
                    topic_id,
                    category_id,
                    silent,
                )
                .await?
            } else {
                crate::topic_converter::to_private(
                    self.pool,
                    self.ctx,
                    self.guardian,
                    topic_id,
                    silent,
                )
                .await?
            };
            match converted {
                crate::topic_converter::Converted::Invalid(errors) => {
                    for message in errors {
                        self.count_error(message, 1);
                    }
                }
                crate::topic_converter::Converted::Done(_) => {
                    let archetype: String =
                        sqlx::query_scalar("SELECT archetype FROM topics WHERE id = $1")
                            .bind(topic_id)
                            .fetch_one(self.pool)
                            .await?;
                    if (archetype == "private_message") != from_pm {
                        self.changed.push(topic_id);
                    }
                }
            }
        }
        Ok(Ok(()))
    }

    /// `toggle_nested_view`: nothing unless nested replies are on and not
    /// the default.
    async fn toggle_nested_view(&mut self) -> Result<Result<(), String>, AppError> {
        let s = self.ctx.settings;
        if !s.get("nested_replies_enabled")?.truthy() || s.get("nested_replies_default")?.truthy() {
            return Ok(Ok(()));
        }
        Err(Unsupported("NestedTopic::Toggle (nested replies)").into())
    }
}
