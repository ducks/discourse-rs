//! Changing a sent chat message: Chat::UpdateMessage (PUT
//! /chat/api/channels/:channel_id/messages/:message_id), Chat::TrashMessage
//! (DELETE there), Chat::RestoreMessage (PUT .../restore) and
//! Chat::MessageReactor (PUT /chat/:chat_channel_id/react/:message_id).
//!
//! Not ported yet, and refused: uploads, direct message channels, threads
//! proper (threading on, forced threads), bots, and messages of deleted
//! users or channels.

use chrono::NaiveDateTime;
use serde_json::{Value, json};

use super::channels::{ChannelRow, Context};
use crate::{AppError, Unsupported};

/// `Chat::MessageReactor::MAX_REACTIONS_LIMIT`
const MAX_REACTIONS_LIMIT: i64 = 30;

pub enum Outcome {
    Done(Value),
    NotFound,
    /// `Discourse::InvalidAccess`, with its custom message key if any.
    Forbidden(Option<String>),
    /// `Discourse::InvalidParameters`
    InvalidParameters,
    /// `ActionController::ParameterMissing`
    ParamMissing(&'static str),
    /// The contract's errors (400).
    Invalid(Vec<String>),
    /// A policy's reason (422).
    Unprocessable(String),
    /// `ActiveRecord::RecordInvalid` from a save!: the model's errors (422).
    RecordInvalid(Vec<String>),
}

#[derive(sqlx::FromRow)]
struct MessageRow {
    id: i64,
    user_id: Option<i32>,
    message: Option<String>,
    cooked: Option<String>,
    excerpt: Option<String>,
    cooked_version: Option<i32>,
    last_editor_id: i32,
    created_at: NaiveDateTime,
    deleted_by_id: Option<i32>,
    thread_id: Option<i64>,
}

const MESSAGE_COLUMNS: &str = "id, user_id, message, cooked, excerpt, cooked_version, last_editor_id, \
     created_at, deleted_by_id, thread_id";

/// ActiveModel's integer cast of a param: digits first, else nil.
fn cast_id(id: &str) -> Option<i64> {
    let numeric = id
        .trim_start()
        .trim_start_matches(['+', '-'])
        .starts_with(|c: char| c.is_ascii_digit());
    numeric.then(|| crate::ruby::to_i(id))
}

fn t(i18n: &crate::i18n::I18n, key: &str) -> String {
    i18n.t(key).unwrap_or(key).to_string()
}

/// `Chat::Channel#latest_not_deleted_message_id`: the channel's latest
/// live message, threads counted by their original message, before the
/// anchor when given.
pub(crate) async fn latest_not_deleted_message_id(
    conn: &mut sqlx::PgConnection,
    channel_id: i64,
    anchor: Option<i64>,
) -> Result<Option<i64>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT chat_messages.id FROM chat_messages \
         LEFT JOIN chat_threads original_message_threads ON original_message_threads.original_message_id = chat_messages.id \
         WHERE chat_channel_id = $1 AND deleted_at IS NULL \
           AND (chat_messages.thread_id IS NULL OR original_message_threads.id IS NOT NULL) \
           AND ($2::bigint IS NULL OR chat_messages.id < $2) \
         ORDER BY chat_messages.created_at DESC, chat_messages.id DESC LIMIT 1",
    )
    .bind(channel_id)
    .bind(anchor)
    .fetch_optional(&mut *conn)
    .await?)
}

impl Context<'_> {
    /// The live channel a message is in; a deleted one is refused, as
    /// Rails fails on it.
    async fn message_channel(&mut self, channel_id: i64) -> Result<Option<ChannelRow>, AppError> {
        let sql = format!(
            "SELECT {} FROM chat_channels WHERE id = $1",
            super::channels::CHANNEL_COLUMNS
        );
        let channel: Option<ChannelRow> = sqlx::query_as(&sql)
            .bind(channel_id)
            .fetch_optional(&mut *self.conn)
            .await?;
        let deleted: bool = sqlx::query_scalar(
            "SELECT COALESCE((SELECT deleted_at IS NOT NULL FROM chat_channels WHERE id = $1), FALSE)",
        )
        .bind(channel_id)
        .fetch_one(&mut *self.conn)
        .await?;
        if deleted {
            return Err(Unsupported("messages of deleted chat channels").into());
        }
        if let Some(c) = &channel
            && c.chatable_type != "Category"
        {
            return Err(Unsupported("chat direct messages").into());
        }
        Ok(channel)
    }

    /// `Chat::Message.find_by(id:, chat_channel_id:)`, deleted ones too
    /// when asked (`with_deleted`).
    async fn find_message(
        &mut self,
        message_id: Option<i64>,
        channel_id: i64,
        with_deleted: bool,
    ) -> Result<Option<MessageRow>, AppError> {
        let Some(message_id) = message_id else {
            return Ok(None);
        };
        let sql = format!(
            "SELECT {MESSAGE_COLUMNS} FROM chat_messages WHERE id = $1 AND chat_channel_id = $2{}",
            if with_deleted {
                ""
            } else {
                " AND deleted_at IS NULL"
            }
        );
        Ok(sqlx::query_as(&sql)
            .bind(message_id)
            .bind(channel_id)
            .fetch_optional(&mut *self.conn)
            .await?)
    }

    /// `can_modify_channel_message?` (and `can_create_channel_message?`,
    /// the same rule): staff in open or closed channels, others in open.
    fn can_modify(&self, channel: &ChannelRow) -> bool {
        let status = channel_status(channel);
        status == "open" || (self.guardian.is_staff() && status == "closed")
    }

    /// `can_post_in_chatable?` for a category channel.
    async fn can_post_in(&mut self, channel: &ChannelRow) -> Result<bool, AppError> {
        if !self.guardian.is_authenticated() {
            return Ok(false);
        }
        Ok(self.guardian.is_admin()
            || !self
                .post_allowed_category_ids(&[channel.chatable_id])
                .await?
                .is_empty())
    }

    /// `can_preview_chat_channel?`
    async fn can_preview(&mut self, channel: &ChannelRow) -> Result<bool, AppError> {
        self.can_see_category(channel.chatable_id).await
    }

    /// The guardian's user, refused when a bot.
    fn acting_user(&self) -> Result<Option<crate::session::current::SessionUser>, AppError> {
        let Some(user) = self.guardian.user().cloned() else {
            return Ok(None);
        };
        if user.id <= 0 {
            return Err(Unsupported("chat changes made by bots").into());
        }
        Ok(Some(user))
    }

    /// `Chat::Thread#update_last_message_id!` and
    /// `Chat::Channel#update_last_message_id!`: their latest live message,
    /// saved when it changed.
    async fn update_last_message_ids(
        &mut self,
        channel_id: i64,
        thread_id: Option<i64>,
        now: NaiveDateTime,
    ) -> Result<(), AppError> {
        if let Some(thread_id) = thread_id {
            let latest: Option<i64> = sqlx::query_scalar(
                "SELECT id FROM chat_messages WHERE chat_channel_id = $1 AND thread_id = $2 \
                   AND deleted_at IS NULL ORDER BY created_at DESC, id DESC LIMIT 1",
            )
            .bind(channel_id)
            .bind(thread_id)
            .fetch_optional(&mut *self.conn)
            .await?;
            sqlx::query(
                "UPDATE chat_threads SET last_message_id = $2, updated_at = $3 \
                 WHERE id = $1 AND last_message_id IS DISTINCT FROM $2",
            )
            .bind(thread_id)
            .bind(latest)
            .bind(now)
            .execute(&mut *self.conn)
            .await?;
        }
        let latest = latest_not_deleted_message_id(&mut *self.conn, channel_id, None).await?;
        sqlx::query(
            "UPDATE chat_channels SET last_message_id = $2, updated_at = $3 \
             WHERE id = $1 AND last_message_id IS DISTINCT FROM $2",
        )
        .bind(channel_id)
        .bind(latest)
        .bind(now)
        .execute(&mut *self.conn)
        .await?;
        Ok(())
    }

    /// The thread's reply count changed (`increment_replies_count_cache`
    /// and `decrement_replies_count_cache`): the count in Redis (not kept
    /// here), and the job that writes it down.
    async fn thread_reply_count_changed(&mut self, thread_id: Option<i64>) -> Result<(), AppError> {
        if let Some(thread_id) = thread_id {
            crate::jobs::enqueue_in(
                &mut *self.conn,
                5,
                super::create::UPDATE_THREAD_REPLY_COUNT_JOB,
                json!({ "thread_id": thread_id }),
            )
            .await?;
        }
        Ok(())
    }

    /// A thread that is neither on nor forced: hidden, as a reply in a
    /// channel without threading makes. Proper threads are refused.
    async fn hidden_thread(
        &mut self,
        channel: &ChannelRow,
        thread_id: Option<i64>,
    ) -> Result<Option<i64>, AppError> {
        let Some(thread_id) = thread_id else {
            return Ok(None);
        };
        let force: bool = sqlx::query_scalar("SELECT force FROM chat_threads WHERE id = $1")
            .bind(thread_id)
            .fetch_one(&mut *self.conn)
            .await?;
        if channel.threading_enabled || force {
            return Err(Unsupported("chat threads").into());
        }
        Ok(Some(thread_id))
    }

    /// Chat::UpdateMessage, the message edited by the guardian's user.
    pub async fn update_message(
        &mut self,
        host: &crate::pretty_text::Host,
        bus: &pg_bus::Bus,
        channel_id: Option<i64>,
        message_id: &str,
        message: Option<String>,
        upload_ids: &[String],
    ) -> Result<Outcome, AppError> {
        let settings = self.settings;
        let Some(user) = self.acting_user()? else {
            return Ok(Outcome::Forbidden(None));
        };
        // The contract.
        let mut errors = Vec::new();
        if message_id.trim().is_empty() {
            errors.push("Message can't be blank".to_string());
        }
        if channel_id.is_none() {
            errors.push("Channel can't be blank".to_string());
        }
        let raw = message.unwrap_or_default();
        if upload_ids.is_empty() && raw.trim().is_empty() {
            errors.push("Message can't be blank".to_string());
        }
        let max = settings.get("chat_maximum_message_length")?.to_i();
        if raw.chars().count() as i64 > max {
            errors.push(format!("Message is too long (maximum is {max} characters)"));
        }
        if !errors.is_empty() {
            return Ok(Outcome::Invalid(errors));
        }
        let Some(channel_id) = channel_id else {
            return Ok(Outcome::NotFound);
        };
        let new_message = if raw.trim().is_empty() {
            raw.clone()
        } else {
            crate::posting::text::clean_message(&raw, true)
        };

        // model :message, model :uploads
        let Some(old) = self
            .find_message(cast_id(message_id), channel_id, false)
            .await?
        else {
            return Ok(Outcome::NotFound);
        };
        let Some(channel) = self.message_channel(channel_id).await? else {
            return Ok(Outcome::NotFound);
        };
        if !upload_ids.is_empty() {
            return Err(Unsupported("chat message uploads").into());
        }
        // model :membership
        let has_membership: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM user_chat_channel_memberships WHERE user_id = $1 AND chat_channel_id = $2)",
        )
        .bind(user.id)
        .bind(channel.id)
        .fetch_one(&mut *self.conn)
        .await?;
        if !has_membership {
            return Ok(Outcome::NotFound);
        }
        // policy :can_edit_message: can_post_in_chatable? and can_edit_chat?
        let can_edit = self.can_post_in(&channel).await?
            && self.can_preview(&channel).await?
            && ((old.user_id == Some(user.id) && !self.guardian.is_silenced())
                || self.guardian.is_admin());
        if !can_edit {
            return Ok(Outcome::Forbidden(None));
        }
        // policy :channel_allows_message_modification
        if !self.can_modify(&channel) {
            return Ok(Outcome::Unprocessable(t(
                self.i18n,
                &format!(
                    "chat.errors.channel_modify_message_disallowed.{}",
                    channel_status(&channel)
                ),
            )));
        }
        let thread_id = self.hidden_thread(&channel, old.thread_id).await?;

        // modify_message: cooked as the editor, under the author's name.
        let author_id = old
            .user_id
            .ok_or(Unsupported("chat messages of deleted users"))?;
        let author: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
            .bind(author_id)
            .fetch_optional(&mut *self.conn)
            .await?
            .ok_or(Unsupported("chat messages of deleted users"))?;
        let cooked = super::cook::cook(host, &new_message, user.id, &author).await?;
        // update_excerpt
        let excerpt = super::messages::build_excerpt_for(&new_message, &cooked)?;
        // save_message: the model's validations.
        if sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM watched_words WHERE action = 1)",
        )
        .fetch_one(&mut *self.conn)
        .await?
        {
            return Err(Unsupported("watched words that block chat messages").into());
        }
        let now = crate::clock::now_naive();
        let mut model_errors = Vec::new();
        if cooked.chars().count() > 20_000 {
            model_errors.push("Cooked is too long (maximum is 20000 characters)".to_string());
        }
        let message_changed = old.message.as_deref() != Some(new_message.as_str());
        if message_changed {
            // Chat::DuplicateMessageValidator, the author's.
            let duplicate: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM chat_messages WHERE chat_channel_id = $1 AND user_id = $2 \
                   AND deleted_at IS NULL AND created_at >= $3 - interval '10 seconds' AND LOWER(message) = $4)",
            )
            .bind(channel.id)
            .bind(author_id)
            .bind(now)
            .bind(new_message.trim().to_lowercase())
            .fetch_one(&mut *self.conn)
            .await?;
            if duplicate {
                model_errors.push(t(self.i18n, "chat.errors.duplicate_message"));
            }
        }
        let min = settings.get("chat_minimum_message_length")?.to_i();
        if (new_message.chars().count() as i64) < min {
            model_errors.push(
                t(
                    self.i18n,
                    if min == 1 {
                        "chat.errors.minimum_length_not_met.one"
                    } else {
                        "chat.errors.minimum_length_not_met.other"
                    },
                )
                .replace("%{count}", &min.to_string()),
            );
        }
        if !model_errors.is_empty() {
            return Ok(Outcome::RecordInvalid(model_errors));
        }
        let changed = message_changed
            || old.cooked.as_deref() != Some(cooked.as_str())
            || old.excerpt.as_deref() != Some(excerpt.as_str())
            || old.cooked_version != Some(super::cook::BAKED_VERSION)
            || old.last_editor_id != user.id;
        if changed {
            sqlx::query(
                "UPDATE chat_messages SET message = $2, cooked = $3, cooked_version = $4, \
                   last_editor_id = $5, excerpt = $6, updated_at = $7 WHERE id = $1",
            )
            .bind(old.id)
            .bind(&new_message)
            .bind(&cooked)
            .bind(super::cook::BAKED_VERSION)
            .bind(user.id)
            .bind(&excerpt)
            .bind(now)
            .execute(&mut *self.conn)
            .await?;
        }

        // save_revision: past the grace period, or more changed than it
        // allows.
        let prev = old.message.clone().unwrap_or_default();
        let grace = settings.get("chat_editing_grace_period")?.to_i();
        let since_created = now.and_utc().timestamp() - old.created_at.and_utc().timestamp();
        let revise = since_created > grace || {
            let max_edited = if self.guardian.has_trust_level(2) {
                settings
                    .get("chat_editing_grace_period_max_diff_high_trust")?
                    .to_i()
            } else {
                settings
                    .get("chat_editing_grace_period_max_diff_low_trust")?
                    .to_i()
            };
            let edited = crate::discourse_diff::diff_size(&prev, &new_message)
                .ok_or(Unsupported("chat edits too large to diff"))?;
            edited as i64 > max_edited
        };
        let edit_timestamp = if revise {
            sqlx::query(
                "INSERT INTO chat_message_revisions (chat_message_id, old_message, new_message, user_id, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $5)",
            )
            .bind(old.id)
            .bind(&prev)
            .bind(&new_message)
            .bind(user.id)
            .bind(now)
            .execute(&mut *self.conn)
            .await?;
            now
        } else {
            crate::clock::now_naive()
        };

        // publish: publish_edit!, the processing job, the thread's preview.
        super::publisher::publish_message(self, bus, &channel, old.id, "edit").await?;
        crate::jobs::enqueue(
            &mut *self.conn,
            super::create::PROCESS_MESSAGE_JOB,
            json!({
                "chat_message_id": old.id,
                "edit_timestamp": edit_timestamp.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string(),
            }),
        )
        .await?;
        if let Some(thread_id) = thread_id {
            super::publisher::publish_thread_metadata(self, bus, &channel, thread_id).await?;
        }
        // index_message (deferred in Rails, made here).
        let urls = crate::url::Urls {
            config: self.config,
            settings,
        };
        let base = crate::posting::search_index::BaseUrls {
            base_path: self.base_path,
            base_url_no_prefix: urls.base_url_no_prefix()?,
            store: crate::file_store::FileStore::for_site(self.config, settings)?,
        };
        crate::posting::search_index::index_chat_message(
            &mut *self.conn,
            settings,
            &base,
            old.id,
            &new_message,
            &cooked,
        )
        .await?;
        Ok(Outcome::Done(
            json!({ "success": "OK", "message_id": old.id }),
        ))
    }

    /// The trash and restore contract: both ids, as integers.
    fn ids_contract(message_id: Option<i64>, channel_id: Option<i64>) -> Option<Outcome> {
        let mut errors = Vec::new();
        if message_id.is_none() {
            errors.push("Message can't be blank".to_string());
        }
        if channel_id.is_none() {
            errors.push("Channel can't be blank".to_string());
        }
        (!errors.is_empty()).then_some(Outcome::Invalid(errors))
    }

    /// Chat::TrashMessage, by the guardian's user.
    pub async fn trash_message(
        &mut self,
        bus: &pg_bus::Bus,
        channel_id: Option<i64>,
        message_id: Option<i64>,
    ) -> Result<Outcome, AppError> {
        if let Some(refused) = Self::ids_contract(message_id, channel_id) {
            return Ok(refused);
        }
        let Some(user) = self.acting_user()? else {
            return Ok(Outcome::Forbidden(None));
        };
        let channel_id = channel_id.unwrap_or_default();
        let Some(message) = self.find_message(message_id, channel_id, false).await? else {
            return Ok(Outcome::NotFound);
        };
        let Some(channel) = self.message_channel(channel_id).await? else {
            return Ok(Outcome::NotFound);
        };
        // policy :invalid_access (can_delete_chat?)
        let allowed = !self.guardian.is_silenced()
            && self.can_modify(&channel)
            && (self.guardian.is_admin() || self.can_preview(&channel).await?)
            && if message.user_id == Some(user.id) {
                self.settings.get("max_post_deletions_per_day")?.to_i() >= 1
            } else {
                self.can_moderate(&channel).await?
            };
        if !allowed {
            return Ok(Outcome::Forbidden(None));
        }
        let thread_id = self.hidden_thread(&channel, message.thread_id).await?;
        let now = crate::clock::now_naive();

        // trash_message
        sqlx::query("UPDATE chat_messages SET deleted_at = $2, deleted_by_id = $3 WHERE id = $1")
            .bind(message.id)
            .bind(now)
            .bind(user.id)
            .execute(&mut *self.conn)
            .await?;
        // destroy_pin
        let pinned = sqlx::query("DELETE FROM chat_pinned_messages WHERE chat_message_id = $1")
            .bind(message.id)
            .execute(&mut *self.conn)
            .await?
            .rows_affected()
            > 0;
        // destroy_notifications, each notified user's count refreshed.
        let notified: Vec<i32> = sqlx::query_scalar(
            "DELETE FROM notifications WHERE id IN (SELECT notifications.id FROM chat_mentions \
               INNER JOIN chat_mention_notifications ON chat_mention_notifications.chat_mention_id = chat_mentions.id \
               INNER JOIN notifications ON notifications.id = chat_mention_notifications.notification_id \
               WHERE chat_mentions.chat_message_id = $1) RETURNING user_id",
        )
        .bind(message.id)
        .fetch_all(&mut *self.conn)
        .await?;
        // update_last_message_ids
        self.update_last_message_ids(channel.id, thread_id, now)
            .await?;
        // update_tracking_state: Chat::Action::ResetUserLastReadChannelMessage
        // and ResetUserLastReadThreadMessage.
        sqlx::query(
            "WITH cte AS (SELECT chat_channels.id AS chat_channel_id, last_message_id FROM chat_channels \
               WHERE chat_channels.id = $2) \
             UPDATE user_chat_channel_memberships SET last_read_message_id = cte.last_message_id FROM cte \
             WHERE user_chat_channel_memberships.last_read_message_id = $1 \
               AND cte.chat_channel_id = user_chat_channel_memberships.chat_channel_id",
        )
        .bind(message.id)
        .bind(channel.id)
        .execute(&mut *self.conn)
        .await?;
        sqlx::query(
            "UPDATE user_chat_channel_memberships SET last_read_message_id = NULL WHERE last_read_message_id = $1",
        )
        .bind(message.id)
        .execute(&mut *self.conn)
        .await?;
        if let Some(thread_id) = message.thread_id {
            sqlx::query(
                "WITH cte AS (SELECT * FROM (SELECT id, thread_id, row_number() OVER ( \
                     PARTITION BY thread_id ORDER BY created_at DESC, id DESC) AS row_number \
                   FROM chat_messages WHERE deleted_at IS NULL AND thread_id = $2 AND chat_messages.id NOT IN ( \
                     SELECT original_message_id FROM chat_threads WHERE thread_id = $2)) AS recent_messages \
                   WHERE recent_messages.row_number = 1) \
                 UPDATE user_chat_thread_memberships SET last_read_message_id = cte.id FROM cte \
                 WHERE user_chat_thread_memberships.last_read_message_id = $1 \
                   AND cte.thread_id = user_chat_thread_memberships.thread_id",
            )
            .bind(message.id)
            .bind(thread_id)
            .execute(&mut *self.conn)
            .await?;
            sqlx::query(
                "UPDATE user_chat_thread_memberships SET last_read_message_id = NULL WHERE last_read_message_id = $1",
            )
            .bind(message.id)
            .execute(&mut *self.conn)
            .await?;
        }
        // update_thread_reply_cache
        self.thread_reply_count_changed(thread_id).await?;

        // after_commit refresh_notification_count, then publish_events.
        let mut refreshed: Vec<i32> = Vec::new();
        for id in notified {
            if !refreshed.contains(&id) {
                crate::bus::publish_notifications_state(bus, &mut *self.conn, self.settings, id)
                    .await?;
                refreshed.push(id);
            }
        }
        super::publisher::publish_delete(self, bus, &channel, message.id).await?;
        if pinned {
            super::publisher::publish_unpin(self, bus, &channel, message.id, user.id).await?;
        }
        if let Some(thread_id) = thread_id {
            super::publisher::publish_thread_metadata(self, bus, &channel, thread_id).await?;
        }
        Ok(Outcome::Done(json!({ "success": "OK" })))
    }

    /// Chat::RestoreMessage, by the guardian's user.
    pub async fn restore_message(
        &mut self,
        bus: &pg_bus::Bus,
        channel_id: Option<i64>,
        message_id: Option<i64>,
    ) -> Result<Outcome, AppError> {
        if let Some(refused) = Self::ids_contract(message_id, channel_id) {
            return Ok(refused);
        }
        let Some(user) = self.acting_user()? else {
            return Ok(Outcome::Forbidden(None));
        };
        let channel_id = channel_id.unwrap_or_default();
        let Some(message) = self.find_message(message_id, channel_id, true).await? else {
            return Ok(Outcome::NotFound);
        };
        let Some(channel) = self.message_channel(channel_id).await? else {
            return Ok(Outcome::NotFound);
        };
        // policy :invalid_access: can post or moderate, and can_restore_chat?
        let moderator = self.can_moderate(&channel).await?;
        let allowed = (self.can_post_in(&channel).await? || moderator)
            && !self.guardian.is_silenced()
            && self.can_modify(&channel)
            && (self.guardian.is_admin() || self.can_preview(&channel).await?)
            && if message.user_id == Some(user.id) {
                message.deleted_by_id == Some(user.id) || moderator
            } else {
                moderator
            };
        if !allowed {
            return Ok(Outcome::Forbidden(None));
        }
        let thread_id = self.hidden_thread(&channel, message.thread_id).await?;
        let now = crate::clock::now_naive();
        sqlx::query(
            "UPDATE chat_messages SET deleted_at = NULL, deleted_by_id = NULL WHERE id = $1",
        )
        .bind(message.id)
        .execute(&mut *self.conn)
        .await?;
        self.update_last_message_ids(channel.id, thread_id, now)
            .await?;
        self.thread_reply_count_changed(thread_id).await?;
        super::publisher::publish_message(self, bus, &channel, message.id, "restore").await?;
        if let Some(thread_id) = thread_id {
            super::publisher::publish_thread_metadata(self, bus, &channel, thread_id).await?;
        }
        Ok(Outcome::Done(json!({ "success": "OK" })))
    }

    /// ChatController#react and Chat::MessageReactor#react!.
    pub async fn react(
        &mut self,
        bus: &pg_bus::Bus,
        channel_id_or_slug: &str,
        message_id: &str,
        emoji: Option<&str>,
        react_action: Option<&str>,
    ) -> Result<Outcome, AppError> {
        // set_channel_and_chatable_with_access_check
        let Some(channel) = self.find_by_id_or_slug(channel_id_or_slug).await? else {
            return Ok(Outcome::NotFound);
        };
        if channel.chatable_type != "Category" {
            return Err(Unsupported("chat direct messages").into());
        }
        if !self.can_join(&channel, None).await? {
            return Ok(Outcome::Forbidden(None));
        }
        // params.require(%i[message_id emoji react_action])
        fn present(v: Option<&str>) -> Option<&str> {
            v.filter(|v| !v.trim().is_empty())
        }
        if present(Some(message_id)).is_none() {
            return Ok(Outcome::ParamMissing("message_id"));
        }
        let Some(emoji) = present(emoji) else {
            return Ok(Outcome::ParamMissing("emoji"));
        };
        let Some(react_action) = present(react_action) else {
            return Ok(Outcome::ParamMissing("react_action"));
        };
        // ensure_can_react! (can_create_chat_message?)
        let Some(user) = self.acting_user()? else {
            return Ok(Outcome::Forbidden(None));
        };
        if !self
            .guardian
            .can_create_post_anywhere(&mut *self.conn, self.settings)
            .await?
        {
            return Ok(Outcome::Forbidden(None));
        }
        let emoji = discourse_markdown::emoji::DATA
            .unicode
            .get(emoji)
            .cloned()
            .unwrap_or_else(|| emoji.to_string());
        // validate_channel_status!
        if !self.can_modify(&channel) {
            return Ok(Outcome::Forbidden(Some(format!(
                "chat.errors.channel_modify_message_disallowed.{}",
                channel_status(&channel)
            ))));
        }
        // validate_reaction!
        let custom: std::collections::HashSet<String> =
            sqlx::query_scalar("SELECT name FROM custom_emojis")
                .fetch_all(&mut *self.conn)
                .await?
                .into_iter()
                .collect();
        let add = match react_action {
            "add" => true,
            "remove" => false,
            _ => return Ok(Outcome::InvalidParameters),
        };
        if !crate::plugins::reactions::emoji_exists(&emoji, &custom) {
            return Ok(Outcome::InvalidParameters);
        }
        // ensure_chat_message!
        let Some(message) = self
            .find_message(cast_id(message_id), channel.id, false)
            .await?
        else {
            return Ok(Outcome::NotFound);
        };
        // validate_max_reactions!
        if add {
            let (distinct, has): (i64, bool) = sqlx::query_as(
                "SELECT COUNT(DISTINCT emoji), COALESCE(bool_or(emoji = $2), FALSE) \
                 FROM chat_message_reactions WHERE chat_message_id = $1",
            )
            .bind(message.id)
            .bind(&emoji)
            .fetch_one(&mut *self.conn)
            .await?;
            if distinct >= MAX_REACTIONS_LIMIT && !has {
                return Ok(Outcome::Forbidden(Some(
                    "chat.errors.max_reactions_limit_reached".into(),
                )));
            }
        }
        // enforce_channel_membership!, create_reaction
        let now = crate::clock::now_naive();
        if add {
            let following: bool = sqlx::query_scalar(
                "SELECT COALESCE((SELECT following FROM user_chat_channel_memberships \
                   WHERE user_id = $1 AND chat_channel_id = $2), FALSE)",
            )
            .bind(user.id)
            .bind(channel.id)
            .fetch_one(&mut *self.conn)
            .await?;
            if !following {
                return Ok(Outcome::Forbidden(Some(
                    "chat.errors.user_not_in_channel".into(),
                )));
            }
            sqlx::query(
                "INSERT INTO chat_message_reactions (chat_message_id, user_id, emoji, created_at, updated_at) \
                 SELECT $1, $2, $3, $4, $4 WHERE NOT EXISTS (SELECT 1 FROM chat_message_reactions \
                   WHERE chat_message_id = $1 AND user_id = $2 AND emoji = $3)",
            )
            .bind(message.id)
            .bind(user.id)
            .bind(&emoji)
            .bind(now)
            .execute(&mut *self.conn)
            .await?;
        } else {
            sqlx::query(
                "DELETE FROM chat_message_reactions WHERE chat_message_id = $1 AND user_id = $2 AND emoji = $3",
            )
            .bind(message.id)
            .bind(user.id)
            .bind(&emoji)
            .execute(&mut *self.conn)
            .await?;
        }
        super::publisher::publish_reaction(
            self,
            bus,
            &channel,
            message.id,
            react_action,
            user.id,
            &emoji,
        )
        .await?;
        Ok(Outcome::Done(json!({ "success": "OK" })))
    }
}

fn channel_status(channel: &ChannelRow) -> &'static str {
    super::channels::STATUSES
        .get(channel.status as usize)
        .copied()
        .unwrap_or("open")
}

impl Context<'_> {
    /// Chat::RebakeMessage (PUT /chat/:chat_channel_id/:message_id/rebake):
    /// the message cooked again by its processing job.
    pub async fn rebake_message(
        &mut self,
        channel_id: Option<i64>,
        message_id: Option<i64>,
    ) -> Result<Outcome, AppError> {
        let mut errors = Vec::new();
        if message_id.is_none() {
            errors.push("Message can't be blank".to_string());
        }
        if channel_id.is_none() {
            errors.push("Chat channel can't be blank".to_string());
        }
        if !errors.is_empty() {
            return Ok(Outcome::Invalid(errors));
        }
        // model :channel, policy :can_access_channel
        let sql = format!(
            "SELECT {} FROM chat_channels WHERE id = $1 AND deleted_at IS NULL",
            super::channels::CHANNEL_COLUMNS
        );
        let channel: Option<ChannelRow> = sqlx::query_as(&sql)
            .bind(channel_id)
            .fetch_optional(&mut *self.conn)
            .await?;
        let Some(channel) = channel else {
            return Ok(Outcome::NotFound);
        };
        if channel.chatable_type != "Category" {
            return Err(Unsupported("chat direct messages").into());
        }
        if !self.can_join(&channel, None).await? {
            return Ok(Outcome::Forbidden(None));
        }
        // model :message, policy :can_rebake
        let Some(message) = self.find_message(message_id, channel.id, true).await? else {
            return Ok(Outcome::NotFound);
        };
        if !(self.can_modify(&channel)
            && (self.guardian.is_staff() || self.guardian.has_trust_level(4)))
        {
            return Ok(Outcome::Forbidden(None));
        }
        crate::jobs::enqueue(
            &mut *self.conn,
            super::create::PROCESS_MESSAGE_JOB,
            json!({ "chat_message_id": message.id, "invalidate_oneboxes": true }),
        )
        .await?;
        Ok(Outcome::Done(json!({ "success": "OK" })))
    }
}
