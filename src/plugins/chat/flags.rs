//! Flagging a chat message: Chat::FlagMessage (POST
//! /chat/api/channels/:channel_id/messages/:message_id/flags) through
//! Chat::ReviewQueue#flag_message: the message's Chat::ReviewableMessage
//! (Reviewable.needs_review!) and the flag's score, the auto silence a
//! high enough score brings (chat_auto_silence_from_flags_duration over
//! Chat::ReviewableMessage.score_to_silence_user), and publish_flag!: the
//! flagger hears self_flagged, staff hear flag.
//!
//! Not ported, and refused: the companion message a notify flag with a
//! message sends, the transcript a direct message's flag sends the
//! moderators, uploads in the payload, take_action (agree_and_delete) and
//! queue_for_review.

use chrono::NaiveDateTime;
use serde_json::{Value, json};

use super::channels::{ChannelRow, Context};
use crate::reviewables::{self, Flagger, InReview, NewScore};
use crate::{AppError, Unsupported};

/// `ReviewableHistory.types`
const HISTORY_CREATED: i32 = 0;
const HISTORY_TRANSITIONED: i32 = 1;

pub struct FlagParams {
    pub message_id: Option<i64>,
    pub channel_id: Option<i64>,
    pub flag_type_id: Option<i64>,
    pub message: Option<String>,
    pub is_warning: bool,
    pub take_action: bool,
    pub queue_for_review: bool,
}

pub enum Flagged {
    /// Flagged; the author to silence after the commit (the reviewable's
    /// id with it), as enforce_auto_silence_threshold does.
    Done(Option<(i32, i64)>),
    NotFound,
    /// The contract's errors (400).
    Invalid(Vec<String>),
    /// `Discourse::InvalidParameters.new(:flag_type)`
    InvalidFlagType,
    /// The flag step's errors (render_json_error, 422).
    Failed(Vec<String>),
}

#[derive(sqlx::FromRow)]
struct Message {
    id: i64,
    user_id: Option<i32>,
    cooked: Option<String>,
    chat_channel_id: i64,
}

impl Context<'_> {
    /// Chat::FlagMessage, by the guardian's user.
    pub async fn flag_message(
        &mut self,
        bus: &pg_bus::Bus,
        p: &FlagParams,
    ) -> Result<Flagged, AppError> {
        // The contract.
        let mut errors = Vec::new();
        if p.message_id.is_none() {
            errors.push("Message can't be blank".to_string());
        }
        if p.channel_id.is_none() {
            errors.push("Channel can't be blank".to_string());
        }
        // ReviewableScore.types: the flags and the score types.
        let flag_type: Option<String> = match p.flag_type_id {
            Some(id) => {
                sqlx::query_scalar("SELECT name_key FROM flags WHERE id = $1 AND id <> 2")
                    .bind(id as i32)
                    .fetch_optional(&mut *self.conn)
                    .await?
            }
            None => None,
        };
        if flag_type.is_none() {
            errors.push("Flag type is not included in the list".to_string());
        }
        let (Some(message_id), Some(channel_id), Some(flag_type), Some(flag_type_id)) =
            (p.message_id, p.channel_id, flag_type, p.flag_type_id)
        else {
            return Ok(Flagged::Invalid(errors));
        };

        // model :message
        let message: Option<Message> = sqlx::query_as(
            "SELECT id, user_id, cooked, chat_channel_id FROM chat_messages \
             WHERE id = $1 AND chat_channel_id = $2 AND deleted_at IS NULL",
        )
        .bind(message_id)
        .bind(channel_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some(message) = message else {
            return Ok(Flagged::NotFound);
        };
        let sql = format!(
            "SELECT {} FROM chat_channels WHERE id = $1 AND deleted_at IS NULL",
            super::channels::CHANNEL_COLUMNS
        );
        let channel: ChannelRow = sqlx::query_as(&sql)
            .bind(message.chat_channel_id)
            .fetch_optional(&mut *self.conn)
            .await?
            .ok_or(Unsupported("flags in deleted chat channels"))?;
        let Some(user) = self.guardian.user().cloned() else {
            return Ok(Flagged::NotFound);
        };
        let staff = self.guardian.is_staff();

        // policy :can_flag_message_in_channel
        if !self.can_join(&channel, None).await?
            || !self.can_flag_chat_message(&channel, &message).await?
            || (!staff && (p.take_action || p.queue_for_review))
            || (flag_type == "notify_user" && p.is_warning && !staff)
        {
            return Ok(Flagged::NotFound);
        }

        // Chat::ReviewQueue#flag_message
        let notify_user = flag_type == "notify_user";
        let notify = notify_user || flag_type == "notify_moderators";
        let dm = channel.chatable_type == "DirectMessage";
        if dm && notify {
            return Ok(Flagged::InvalidFlagType);
        }
        let existing: Option<(i64, i32, NaiveDateTime)> = sqlx::query_as(
            "SELECT id, status, updated_at FROM reviewables WHERE target_type = $1 AND target_id = $2 \
             ORDER BY id LIMIT 1",
        )
        .bind(super::bookmarks::POLYMORPHIC_NAME)
        .bind(message.id)
        .fetch_optional(&mut *self.conn)
        .await?;
        if let Some(existing) = existing
            && !self
                .can_flag_again(existing, message.id, user.id, flag_type_id, &flag_type)
                .await?
        {
            return Ok(Flagged::Failed(vec![
                self.i18n
                    .t("chat.reviewables.message_already_handled")
                    .unwrap_or("chat.reviewables.message_already_handled")
                    .to_string(),
            ]));
        }
        let has_message = p.message.as_deref().is_some_and(|m| !m.trim().is_empty());
        if has_message && !dm && notify {
            return Err(Unsupported("the message a chat notify flag sends").into());
        }
        if dm {
            let earlier: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM chat_messages WHERE chat_channel_id = $1 AND id < $2 \
                   AND deleted_at IS NULL)",
            )
            .bind(channel.id)
            .bind(message.id)
            .fetch_one(&mut *self.conn)
            .await?;
            if earlier {
                return Err(Unsupported(
                    "the transcript a direct message flag sends the moderators",
                )
                .into());
            }
        }
        if p.queue_for_review {
            return Err(Unsupported("queueing a chat message for review as staff").into());
        }
        if notify_user {
            return Ok(Flagged::Done(None));
        }
        let uploads: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM upload_references WHERE target_type = $1 AND target_id = $2)",
        )
        .bind(super::bookmarks::POLYMORPHIC_NAME)
        .bind(message.id)
        .fetch_one(&mut *self.conn)
        .await?;
        if uploads {
            return Err(Unsupported("flags of chat messages with uploads").into());
        }
        if p.take_action {
            return Err(Unsupported("taking action on a chat flag (agree_and_delete)").into());
        }

        let payload = json!({ "message_cooked": message.cooked });
        let reviewable = self
            .message_needs_review(&message, user.id, flag_type == "spam", &payload)
            .await?;
        let created_at = crate::clock::now_naive();
        let hooks = crate::web_hooks::Payloads {
            settings: self.settings,
            i18n: self.i18n,
            config: self.config,
        };
        reviewables::add_score(
            &mut *self.conn,
            &hooks,
            &reviewable,
            &Flagger {
                id: user.id,
                staff,
                trust_level: user.trust_level,
            },
            &NewScore {
                score_type: flag_type_id,
                created_at,
                take_action: false,
                reason: None,
                context: None,
                meta_topic_id: None,
            },
        )
        .await?;

        // enforce_auto_silence_threshold, the silence itself left to the
        // caller once this commits.
        let silence = self.auto_silence(&reviewable, message.user_id).await?;

        // publish_flag!: self_flagged to the flagger, flag to staff.
        bus.publish(
            &mut *self.conn,
            &format!("/chat/{}", channel.id),
            &json!({
                "type": "self_flagged",
                "user_flag_status": reviewables::PENDING,
                "chat_message_id": message.id,
            }),
            Some(&[crate::bus::user_tag(user.id)]),
        )
        .await?;
        bus.publish(
            &mut *self.conn,
            &format!("/chat/{}", channel.id),
            &json!({
                "type": "flag",
                "chat_message_id": message.id,
                "reviewable_id": reviewable.id,
            }),
            Some(&[crate::bus::group_tag(
                crate::guardian::auto_groups::STAFF as i32,
            )]),
        )
        .await?;
        Ok(Flagged::Done(silence))
    }

    /// `can_flag_chat_message?`: a live message with an author who isn't
    /// the viewer (nor staff, unless allow_flagging_staff), flagged by
    /// someone unsilenced in chat_message_flag_allowed_groups, in a channel
    /// whose messages can change and that they can join.
    async fn can_flag_chat_message(
        &mut self,
        channel: &ChannelRow,
        message: &Message,
    ) -> Result<bool, AppError> {
        let g = self.guardian;
        let Some(author_id) = message.user_id else {
            return Ok(false);
        };
        let author: Option<bool> =
            sqlx::query_scalar("SELECT admin OR moderator FROM users WHERE id = $1")
                .bind(author_id)
                .fetch_optional(&mut *self.conn)
                .await?;
        let Some(author_staff) = author else {
            return Ok(false);
        };
        if !g.is_authenticated()
            || (author_staff && !self.settings.get("allow_flagging_staff")?.truthy())
            || g.user_id() == Some(author_id)
            || g.is_silenced()
            || !g.in_setting_groups(self.settings, "chat_message_flag_allowed_groups")?
        {
            return Ok(false);
        }
        let status = channel.status;
        let modifiable = if g.is_staff() {
            status == 0 || status == 2
        } else {
            status == 0
        };
        Ok(modifiable && self.can_join(channel, None).await?)
    }

    /// `can_flag_again?`: no pending flag of the flagger's (a notify
    /// moderators flag always goes, then), none of this type pending, and
    /// the message not reviewed lately or edited since.
    async fn can_flag_again(
        &mut self,
        (reviewable_id, status, updated_at): (i64, i32, NaiveDateTime),
        message_id: i64,
        flagger_id: i32,
        flag_type_id: i64,
        flag_type: &str,
    ) -> Result<bool, AppError> {
        let (flagger_pending, flag_used): (bool, bool) = sqlx::query_as(
            "SELECT COALESCE(bool_or(user_id = $2 AND status = $4), FALSE), \
                    COALESCE(bool_or(reviewable_score_type = $3 AND status = $4), FALSE) \
             FROM reviewable_scores WHERE reviewable_id = $1",
        )
        .bind(reviewable_id)
        .bind(flagger_id)
        .bind(flag_type_id as i32)
        .bind(reviewables::PENDING)
        .fetch_one(&mut *self.conn)
        .await?;
        if !flagger_pending && flag_type == "notify_moderators" {
            return Ok(true);
        }
        let cooldown = self.settings.get("cooldown_hours_until_reflag")?.to_i();
        let handled_recently = !(status == reviewables::PENDING
            || updated_at < crate::clock::now_naive() - chrono::Duration::hours(cooldown));
        let edited_since: bool = sqlx::query_scalar(
            "SELECT COALESCE((SELECT updated_at > $2 FROM chat_message_revisions \
               WHERE chat_message_id = $1 ORDER BY id DESC LIMIT 1), FALSE)",
        )
        .bind(message_id)
        .bind(updated_at)
        .fetch_one(&mut *self.conn)
        .await?;
        Ok(!flag_used && !flagger_pending && (!handled_recently || edited_since))
    }

    /// `Chat::ReviewableMessage.needs_review!` and the update of its
    /// target_created_by and payload: the message's reviewable set back to
    /// pending (its score recalculated and a transition logged when it had
    /// been reviewed), or a new one.
    async fn message_needs_review(
        &mut self,
        message: &Message,
        created_by_id: i32,
        potential_spam: bool,
        payload: &Value,
    ) -> Result<InReview, AppError> {
        let hooks = crate::web_hooks::Payloads {
            settings: self.settings,
            i18n: self.i18n,
            config: self.config,
        };
        let old: Option<(i64, i32, f64)> = sqlx::query_as(
            "UPDATE reviewables SET status = $1, \
                    potential_spam = COALESCE($3, reviewables.potential_spam) \
             FROM reviewables AS old_reviewables \
             WHERE reviewables.target_id = $2 AND reviewables.target_type = $4 \
               AND reviewables.type = 'Chat::ReviewableMessage' AND old_reviewables.id = reviewables.id \
             RETURNING reviewables.id, old_reviewables.status, old_reviewables.score",
        )
        .bind(reviewables::PENDING)
        .bind(message.id)
        .bind(potential_spam.then_some(true))
        .bind(super::bookmarks::POLYMORPHIC_NAME)
        .fetch_optional(&mut *self.conn)
        .await?;
        let (id, loaded_score) = match old {
            Some((id, old_status, loaded_score)) => {
                if old_status != reviewables::PENDING {
                    reviewables::recalculate_score(&mut *self.conn, &hooks, id).await?;
                    self.log_history(id, HISTORY_TRANSITIONED, created_by_id)
                        .await?;
                }
                sqlx::query(
                    "UPDATE reviewables SET target_created_by_id = $2, \
                            payload = COALESCE(payload, '{}'::jsonb) || $3::jsonb, updated_at = clock_timestamp() \
                     WHERE id = $1",
                )
                .bind(id)
                .bind(message.user_id)
                .bind(payload)
                .execute(&mut *self.conn)
                .await?;
                (id, loaded_score)
            }
            None => {
                let id: i64 = sqlx::query_scalar(
                    "INSERT INTO reviewables (type, type_source, status, created_by_id, reviewable_by_moderator, \
                                              reviewable_by_group_id, category_id, topic_id, score, potential_spam, \
                                              target_id, target_type, target_created_by_id, payload, version, \
                                              latest_score, force_review, potentially_illegal, created_at, updated_at) \
                     VALUES ('Chat::ReviewableMessage', 'core', $1, $2, TRUE, NULL, NULL, NULL, 0, $3, $4, $5, $6, \
                             $7::jsonb, 0, NULL, FALSE, FALSE, clock_timestamp(), clock_timestamp()) \
                     RETURNING id",
                )
                .bind(reviewables::PENDING)
                .bind(created_by_id)
                .bind(potential_spam)
                .bind(message.id)
                .bind(super::bookmarks::POLYMORPHIC_NAME)
                .bind(message.user_id)
                .bind(payload)
                .fetch_one(&mut *self.conn)
                .await?;
                self.log_history(id, HISTORY_CREATED, created_by_id).await?;
                crate::web_hooks::enqueue_reviewable_hooks(
                    &mut *self.conn,
                    &hooks,
                    "reviewable_created",
                    id,
                )
                .await?;
                crate::jobs::enqueue(
                    &mut *self.conn,
                    "notify_reviewable",
                    json!({ "reviewable_id": id }),
                )
                .await?;
                (id, 0.0)
            }
        };
        Ok(InReview {
            id,
            loaded_score,
            topic_id: None,
        })
    }

    async fn log_history(&mut self, id: i64, kind: i32, by: i32) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO reviewable_histories (reviewable_id, reviewable_history_type, status, created_by_id, \
                                               edited, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, NULL, clock_timestamp(), clock_timestamp())",
        )
        .bind(id)
        .bind(kind)
        .bind(reviewables::PENDING)
        .bind(by)
        .execute(&mut *self.conn)
        .await?;
        Ok(())
    }

    /// `enforce_auto_silence_threshold`: the author, unless an admin or
    /// silenced already, when the reviewable's score passes
    /// `sensitivity_score(chat_silence_user_sensitivity, scale: 0.6)`.
    async fn auto_silence(
        &mut self,
        reviewable: &InReview,
        author_id: Option<i32>,
    ) -> Result<Option<(i32, i64)>, AppError> {
        if self
            .settings
            .get("chat_auto_silence_from_flags_duration")?
            .to_i()
            == 0
        {
            return Ok(None);
        }
        let score: f64 = sqlx::query_scalar("SELECT score FROM reviewables WHERE id = $1")
            .bind(reviewable.id)
            .fetch_one(&mut *self.conn)
            .await?;
        let to_silence = reviewables::sensitivity_score(
            &mut *self.conn,
            self.settings,
            "chat_silence_user_sensitivity",
            0.6,
        )
        .await?;
        if score <= to_silence {
            return Ok(None);
        }
        let Some(author_id) = author_id else {
            return Ok(None);
        };
        let author: Option<(bool, Option<NaiveDateTime>)> =
            sqlx::query_as("SELECT admin, silenced_till FROM users WHERE id = $1")
                .bind(author_id)
                .fetch_optional(&mut *self.conn)
                .await?;
        match author {
            Some((false, silenced_till))
                if silenced_till.is_none_or(|t| t <= crate::clock::now_naive()) =>
            {
                Ok(Some((author_id, reviewable.id)))
            }
            _ => Ok(None),
        }
    }
}
