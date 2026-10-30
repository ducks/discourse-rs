//! Sending a chat message (POST /chat/:chat_channel_id): Chat::CreateMessage,
//! and the job it enqueues, Jobs::Chat::ProcessMessage (the message cooked
//! again, its mentions upserted, its links extracted, Chat::Notifier's
//! mention and watching jobs enqueued, and the processed message published).
//!
//! A reply makes (or joins) the replied-to message's thread, as Rails does
//! in channels without threading too: the thread stays hidden there. Not
//! ported yet, and refused: blocks, threads proper (threading on,
//! forced threads, a thread_id), bots, group and channel-wide mentions,
//! oneboxes and images in a message. The controller's
//! Chat::MessageRateLimiter is in the route.

use chrono::NaiveDateTime;
use serde_json::{Value, json};

use super::channels::{ChannelRow, Context};
use crate::{AppError, Unsupported};

/// The job that processes a new or edited message.
pub const PROCESS_MESSAGE_JOB: &str = "Jobs::Chat::ProcessMessage";
/// The job that recounts a thread's replies.
pub const UPDATE_THREAD_REPLY_COUNT_JOB: &str = "Jobs::Chat::UpdateThreadReplyCount";

/// `Chat::NotificationLevels.all[:tracking]`
const THREAD_TRACKING: i32 = 2;

/// Chat::CreateMessage's params.
#[derive(Default)]
pub struct Params {
    /// The channel's id or slug.
    pub chat_channel_id: String,
    pub message: Option<String>,
    pub in_reply_to_id: Option<String>,
    pub staged_id: Option<String>,
    pub thread_id: Option<String>,
    pub upload_ids: Vec<String>,
    pub blocks: bool,
    pub client_created_at: Option<String>,
    /// `enforce_membership`: the sender joins the channel first (bots
    /// always do).
    pub enforce_membership: bool,
}

pub enum Outcome {
    Created(i64),
    NotFound,
    Forbidden,
    /// The contract's errors (400).
    Invalid(Vec<String>),
    /// A policy's reason or the model's errors (422).
    Unprocessable(String),
}

fn t(i18n: &crate::i18n::I18n, key: &str) -> String {
    i18n.t(key).unwrap_or(key).to_string()
}

impl Context<'_> {
    /// `Chat::Channel.find_by_id_or_slug`: a live channel by id, else by
    /// slug.
    pub(crate) async fn find_by_id_or_slug(
        &mut self,
        id_or_slug: &str,
    ) -> Result<Option<ChannelRow>, AppError> {
        let sql = format!(
            "SELECT {} FROM chat_channels WHERE deleted_at IS NULL \
               AND (id::text = $1 OR lower(slug) = lower($1)) ORDER BY (id::text = $1) DESC LIMIT 1",
            super::channels::CHANNEL_COLUMNS
        );
        Ok(sqlx::query_as(&sql)
            .bind(id_or_slug.trim())
            .fetch_optional(&mut *self.conn)
            .await?)
    }

    /// `fetch_uploads`: the uploads of these ids that are the user's own
    /// (user_uploads), as (id, original_filename); None when
    /// chat_allow_uploads is off.
    pub(crate) async fn own_uploads(
        &mut self,
        user_id: i32,
        upload_ids: &[String],
    ) -> Result<Option<Vec<(i64, String)>>, AppError> {
        if !self.settings.get("chat_allow_uploads")?.truthy() {
            return Ok(None);
        }
        let ids: Vec<i64> = upload_ids.iter().map(|id| crate::ruby::to_i(id)).collect();
        Ok(Some(
            sqlx::query_as(
                "SELECT uploads.id::int8, uploads.original_filename FROM uploads \
                 INNER JOIN user_uploads ON user_uploads.upload_id = uploads.id \
                 WHERE uploads.id::int8 = ANY($1) AND user_uploads.user_id = $2 ORDER BY uploads.id",
            )
            .bind(&ids)
            .bind(user_id)
            .fetch_all(&mut *self.conn)
            .await?,
        ))
    }

    /// The upload references a message's `uploads` (or `upload_ids=`)
    /// saves, one per upload.
    pub(crate) async fn attach_uploads(
        &mut self,
        message_id: i64,
        uploads: &[(i64, String)],
        now: NaiveDateTime,
    ) -> Result<(), AppError> {
        for (upload_id, _) in uploads {
            sqlx::query(
                "INSERT INTO upload_references (upload_id, target_type, target_id, created_at, updated_at) \
                 VALUES ($1, 'ChatMessage', $2, $3, $3)",
            )
            .bind(upload_id)
            .bind(message_id)
            .bind(now)
            .execute(&mut *self.conn)
            .await?;
        }
        Ok(())
    }

    /// Chat::CreateMessage, the message created by the guardian's user.
    pub async fn create_message(
        &mut self,
        host: &crate::pretty_text::Host,
        bus: &pg_bus::Bus,
        params: Params,
    ) -> Result<Outcome, AppError> {
        let g = self.guardian;
        let settings = self.settings;
        let Some(user) = g.user().cloned() else {
            return Ok(Outcome::Forbidden);
        };
        // policy :no_silenced_user, policy :accept_blocks
        if g.is_silenced() {
            return Ok(Outcome::Forbidden);
        }
        // User#bot?
        let bot = user.id <= 0;
        if params.blocks && !bot {
            return Ok(Outcome::Forbidden);
        }
        // The contract.
        let raw = params.message.clone().unwrap_or_default();
        let mut errors = Vec::new();
        if params.chat_channel_id.trim().is_empty() {
            errors.push("Chat channel can't be blank".to_string());
        }
        if raw.trim().is_empty() && params.upload_ids.is_empty() {
            errors.push("Message can't be blank".to_string());
        }
        let max = settings.get("chat_maximum_message_length")?.to_i();
        if raw.chars().count() as i64 > max {
            errors.push(format!("Message is too long (maximum is {max} characters)"));
        }
        if !errors.is_empty() {
            return Ok(Outcome::Invalid(errors));
        }
        // A blank message (with uploads) is kept as it came, uncleaned.
        let message = if raw.trim().is_empty() {
            raw.clone()
        } else {
            crate::posting::text::clean_message(&raw, true)
        };

        // model :channel, policy :can_post_in_channel
        let Some(channel) = self.find_by_id_or_slug(&params.chat_channel_id).await? else {
            return Ok(Outcome::NotFound);
        };
        // can_post_in_chatable?: always in a direct message.
        let dm = if channel.chatable_type == "DirectMessage" {
            Some(self.direct_message(channel.chatable_id).await?)
        } else {
            None
        };
        let can_post = dm.is_some()
            || g.is_admin()
            || !sqlx::query_scalar::<_, i32>(&format!(
                "SELECT categories.id FROM categories WHERE categories.id = $1 AND {}",
                super::categories_scoped_to(g, "1, 2")
            ))
            .bind(channel.chatable_id as i32)
            .fetch_all(&mut *self.conn)
            .await?
            .is_empty();
        if !can_post {
            return Ok(Outcome::Forbidden);
        }
        // enforce_membership: channel.add(user), and a DM's participant.
        if bot || params.enforce_membership {
            self.follow(&channel, user.id).await?;
            if let Some(dm) = &dm
                && !dm.users.iter().flatten().any(|u| u.id == user.id)
            {
                sqlx::query(
                    "INSERT INTO direct_message_users (direct_message_channel_id, user_id, created_at, updated_at) \
                     VALUES ($1, $2, $3, $3)",
                )
                .bind(channel.chatable_id)
                .bind(user.id)
                .bind(crate::clock::now_naive())
                .execute(&mut *self.conn)
                .await?;
            }
        }
        // model :membership (its following, for PublishAndFollow)
        let following: Option<bool> = sqlx::query_scalar(
            "SELECT following FROM user_chat_channel_memberships WHERE user_id = $1 AND chat_channel_id = $2",
        )
        .bind(user.id)
        .bind(channel.id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some(was_following) = following else {
            return Ok(Outcome::NotFound);
        };
        // policy :channel_allows_message_creation (can_create_channel_message?)
        let status = super::channels::STATUSES
            .get(channel.status as usize)
            .copied()
            .unwrap_or("open");
        let allowed = status == "open" || (g.is_staff() && status == "closed");
        if let Some(dm) = &dm {
            if let Some(reason) = self.dm_creation_refusal(dm, allowed).await? {
                return Ok(Outcome::Unprocessable(reason));
            }
        } else if !allowed {
            return Ok(Outcome::Unprocessable(t(
                self.i18n,
                &format!("chat.errors.channel_new_message_disallowed.{status}"),
            )));
        }
        // model :reply, policy :ensure_reply_consistency
        let reply: Option<(i64, i64, Option<i32>, Option<i64>)> = match params
            .in_reply_to_id
            .as_deref()
            .filter(|r| !r.trim().is_empty())
        {
            None => None,
            Some(id) => {
                let found: Option<(i64, i64, Option<i32>, Option<i64>)> = sqlx::query_as(
                    "SELECT id, chat_channel_id, user_id, thread_id FROM chat_messages \
                     WHERE id = $1 AND deleted_at IS NULL",
                )
                .bind(crate::ruby::to_i(id))
                .fetch_optional(&mut *self.conn)
                .await?;
                match found {
                    Some(r) if r.1 == channel.id => Some(r),
                    _ => return Ok(Outcome::NotFound),
                }
            }
        };
        // model :thread: a thread_id, or the reply's thread (made when it
        // has none).
        if params
            .thread_id
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty())
        {
            return Err(Unsupported("chat threads").into());
        }
        if channel.threading_enabled && reply.is_some() {
            return Err(Unsupported("chat threads").into());
        }
        let existing_thread = match reply.and_then(|r| r.3) {
            Some(thread_id) => {
                let force: bool =
                    sqlx::query_scalar("SELECT force FROM chat_threads WHERE id = $1")
                        .bind(thread_id)
                        .fetch_one(&mut *self.conn)
                        .await?;
                if force {
                    return Err(Unsupported("chat threads").into());
                }
                Some(thread_id)
            }
            None => None,
        };
        // model :uploads
        let uploads = self
            .own_uploads(user.id, &params.upload_ids)
            .await?
            .unwrap_or_default();
        // A nil message with upload_ids fails the model's length checks
        // (message.length on nil).
        if params.message.is_none() {
            return Err(Unsupported("a chat message with upload_ids and no message").into());
        }
        let only_uploads = message.trim().is_empty() && !uploads.is_empty();

        // model :message_instance: cooked, then the model's validations.
        let cooked = super::cook::cook(host, &message, user.id, &user.username).await?;
        // WatchedWordsValidator, unless the author is a bot.
        let blocked = if user.id > 0 {
            crate::word_watcher::block_error(&mut *self.conn, settings, self.i18n, &message).await?
        } else {
            None
        };
        let now = crate::clock::now_naive();
        let created_at = params
            .client_created_at
            .as_deref()
            .and_then(|c| chrono::DateTime::parse_from_rfc3339(c).ok())
            .map(|c| c.naive_utc())
            .filter(|c| (now - *c).num_seconds().abs() <= 60)
            .unwrap_or(now);
        // `validates :cooked, length:` comes before validate_message's
        // checks, so its error is the first.
        let mut model_errors: Vec<String> = Vec::new();
        if cooked.chars().count() > 20_000 {
            model_errors.push("Cooked is too long (maximum is 20000 characters)".into());
        }
        model_errors.extend(blocked);
        // Chat::DuplicateMessageValidator (an upload-only message can't be
        // one).
        let duplicate: bool = !only_uploads
            && sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM chat_messages WHERE chat_channel_id = $1 AND user_id = $2 \
               AND deleted_at IS NULL AND created_at >= $3 - interval '10 seconds' AND LOWER(message) = $4)",
            )
            .bind(channel.id)
            .bind(user.id)
            .bind(now)
            .bind(message.trim().to_lowercase())
            .fetch_one(&mut *self.conn)
            .await?;
        if duplicate {
            model_errors.push(t(self.i18n, "chat.errors.duplicate_message"));
        }
        let min = settings.get("chat_minimum_message_length")?.to_i();
        if uploads.is_empty() && (message.chars().count() as i64) < min {
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
            return Ok(Outcome::Unprocessable(model_errors.join(", ")));
        }

        // The transaction.
        let excerpt = super::messages::build_excerpt_with_uploads(
            &message,
            &cooked,
            uploads.first().map(|u| u.1.as_str()),
            crate::excerpt::Plugins::from_settings(settings)?,
        )?;
        let new_thread = match (reply, existing_thread) {
            (Some(r), None) => {
                let original_user =
                    r.2.ok_or(Unsupported("replies to deleted users' messages"))?;
                let id: i64 = sqlx::query_scalar(
                    "INSERT INTO chat_threads (channel_id, original_message_id, original_message_user_id, status, \
                       replies_count, force, created_at, updated_at) \
                     VALUES ($1, $2, $3, 0, 0, FALSE, $4, $4) RETURNING id",
                )
                .bind(channel.id)
                .bind(r.0)
                .bind(i64::from(original_user))
                .bind(now)
                .fetch_one(&mut *self.conn)
                .await?;
                Some(id)
            }
            _ => None,
        };
        let thread_id = existing_thread.or(new_thread);
        let message_id: i64 = sqlx::query_scalar(
            "INSERT INTO chat_messages (chat_channel_id, user_id, created_at, updated_at, message, cooked, \
               cooked_version, last_editor_id, in_reply_to_id, thread_id, streaming, excerpt, created_by_sdk) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $2, $8, $9, FALSE, $10, FALSE) RETURNING id",
        )
        .bind(channel.id)
        .bind(user.id)
        .bind(created_at)
        .bind(now)
        .bind(&message)
        .bind(&cooked)
        .bind(super::cook::BAKED_VERSION)
        .bind(reply.map(|r| r.0))
        .bind(thread_id)
        .bind(&excerpt)
        .fetch_one(&mut *self.conn)
        .await?;
        // The uploads' references, as `uploads:` saves them.
        self.attach_uploads(message_id, &uploads, now).await?;
        // The reply's autosaved thread.
        if let (Some(r), Some(new)) = (reply, new_thread) {
            sqlx::query("UPDATE chat_messages SET thread_id = $2, updated_at = $3 WHERE id = $1")
                .bind(r.0)
                .bind(new)
                .bind(now)
                .execute(&mut *self.conn)
                .await?;
        }
        // delete_drafts
        sqlx::query("DELETE FROM chat_drafts WHERE user_id = $1 AND chat_channel_id = $2")
            .bind(user.id)
            .bind(channel.id)
            .execute(&mut *self.conn)
            .await?;
        // post_process_thread
        if let (Some(thread_id), Some(r)) = (thread_id, reply) {
            sqlx::query(
                "UPDATE chat_threads SET last_message_id = $2, updated_at = $3 WHERE id = $1",
            )
            .bind(thread_id)
            .bind(message_id)
            .bind(now)
            .execute(&mut *self.conn)
            .await?;
            // increment_replies_count_cache: the count in Redis (not
            // kept here), and the job that writes it down.
            crate::jobs::enqueue_in(
                &mut *self.conn,
                5,
                UPDATE_THREAD_REPLY_COUNT_JOB,
                json!({ "thread_id": thread_id }),
            )
            .await?;
            match &dm {
                // Every participant joins a DM's thread, for unread
                // tracking, when the channel has no threading.
                Some(dm) if !channel.threading_enabled => {
                    for participant in dm.users.iter().flatten() {
                        self.add_to_thread(thread_id, i64::from(participant.id), now)
                            .await?;
                    }
                }
                _ => {
                    let original_user = r.2.map(i64::from).unwrap_or(0);
                    self.add_to_thread(thread_id, original_user, now).await?;
                    self.add_to_thread(thread_id, i64::from(user.id), now)
                        .await?;
                }
            }
            sqlx::query(
                "UPDATE user_chat_thread_memberships SET last_read_message_id = $3, updated_at = $4 \
                 WHERE thread_id = $1 AND user_id = $2",
            )
            .bind(thread_id)
            .bind(i64::from(user.id))
            .bind(message_id)
            .bind(now)
            .execute(&mut *self.conn)
            .await?;
        }
        // update_channel_last_message, update_membership_last_read (a
        // hidden thread's messages are the channel's).
        sqlx::query("UPDATE chat_channels SET last_message_id = $2, updated_at = $3 WHERE id = $1")
            .bind(channel.id)
            .bind(message_id)
            .bind(now)
            .execute(&mut *self.conn)
            .await?;
        sqlx::query(
            "UPDATE user_chat_channel_memberships SET last_read_message_id = $3, updated_at = $4 \
             WHERE user_id = $1 AND chat_channel_id = $2",
        )
        .bind(user.id)
        .bind(channel.id)
        .bind(message_id)
        .bind(now)
        .execute(&mut *self.conn)
        .await?;

        // process_direct_message_channel: PublishAndFollowDirectMessageChannel.
        if dm.is_some() {
            self.publish_and_follow(bus, &channel, user.id, g.is_staff(), was_following)
                .await?;
        }
        // process: publish_new!, then the processing job.
        super::publisher::publish_new(self, bus, &channel, message_id, params.staged_id.as_deref())
            .await?;
        // DiscourseEvent :chat_message_created: its web hooks.
        self.enqueue_message_hooks("chat_message_created", channel.id, message_id)
            .await?;
        crate::jobs::enqueue(
            &mut *self.conn,
            PROCESS_MESSAGE_JOB,
            json!({ "chat_message_id": message_id, "staged_id": params.staged_id }),
        )
        .await?;
        super::publisher::publish_user_tracking_state(self, bus, channel.id, message_id, thread_id)
            .await?;
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
            message_id,
            &message,
            &cooked,
        )
        .await?;
        Ok(Outcome::Created(message_id))
    }

    /// `ChannelMembershipManager#follow`: a following membership, made or
    /// turned on, and the count recalculated when it changed.
    pub(crate) async fn follow(
        &mut self,
        channel: &ChannelRow,
        user_id: i32,
    ) -> Result<(), AppError> {
        let following: Option<bool> = sqlx::query_scalar(
            "SELECT following FROM user_chat_channel_memberships WHERE user_id = $1 AND chat_channel_id = $2",
        )
        .bind(user_id)
        .bind(channel.id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let now = crate::clock::now_naive();
        match following {
            None => {
                sqlx::query(
                    "INSERT INTO user_chat_channel_memberships (user_id, chat_channel_id, following, created_at, updated_at) \
                     VALUES ($1, $2, TRUE, $3, $3)",
                )
                .bind(user_id)
                .bind(channel.id)
                .bind(now)
                .execute(&mut *self.conn)
                .await?;
            }
            Some(false) => {
                sqlx::query(
                    "UPDATE user_chat_channel_memberships SET following = TRUE, updated_at = $3 \
                     WHERE user_id = $1 AND chat_channel_id = $2",
                )
                .bind(user_id)
                .bind(channel.id)
                .bind(now)
                .execute(&mut *self.conn)
                .await?;
            }
            Some(true) => return Ok(()),
        }
        self.mark_user_count_stale(channel.id).await
    }

    /// `Chat::Channel::Policy::MessageCreation::DirectMessageStrategy`:
    /// the reason the actor may not post in the DM, None when they may.
    async fn dm_creation_refusal(
        &mut self,
        dm: &super::direct_messages::DirectMessage,
        can_create: bool,
    ) -> Result<Option<String>, AppError> {
        let g = self.guardian;
        let Some(actor) = g.user().cloned() else {
            return Ok(Some(t(
                self.i18n,
                "chat.errors.user_cannot_send_direct_messages",
            )));
        };
        let with = |key: &str, username: &str| {
            self.i18n
                .t_with(key, &[("username", username)])
                .unwrap_or_else(|| key.to_string())
        };
        if !can_create {
            return Ok(Some(t(
                self.i18n,
                "chat.errors.channel_new_message_disallowed.closed",
            )));
        }
        // can_send_direct_message?
        let bot = actor.id < 0;
        if !(g.is_staff() || bot || (dm.user_can_access(g) && !actor.suspended())) {
            return Ok(Some(t(
                self.i18n,
                "chat.errors.user_cannot_send_direct_messages",
            )));
        }
        // allowing_direct_messages?
        let allows: bool = sqlx::query_scalar(
            "SELECT COALESCE((SELECT allow_private_messages FROM user_options WHERE user_id = $1), TRUE)",
        )
        .bind(actor.id)
        .fetch_one(&mut *self.conn)
        .await?;
        if !allows {
            return Ok(Some(t(self.i18n, "chat.errors.actor_disallowed_dms")));
        }
        let participants: Vec<&super::direct_messages::DmUser> =
            dm.users.iter().flatten().collect();
        if participants.len() == 1 || dm.group || actor.id == -1 || bot {
            return Ok(None);
        }
        let Some(target) = participants.into_iter().find(|u| u.id != actor.id) else {
            return Ok(Some(t(
                self.i18n,
                "chat.errors.user_cannot_send_direct_messages",
            )));
        };
        let (muting, ignoring): (bool, bool) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM muted_users WHERE user_id = $1 AND muted_user_id = $2), \
                    EXISTS (SELECT 1 FROM ignored_users WHERE user_id = $1 AND ignored_user_id = $2)",
        )
        .bind(actor.id)
        .bind(target.id)
        .fetch_one(&mut *self.conn)
        .await?;
        if muting {
            return Ok(Some(with(
                "chat.errors.actor_muting_target_user",
                &target.username,
            )));
        }
        if ignoring {
            return Ok(Some(with(
                "chat.errors.actor_ignoring_target_user",
                &target.username,
            )));
        }
        // recipient_can_chat?
        let target_user = crate::session::current::SessionUser::load(&mut *self.conn, target.id)
            .await?
            .ok_or(Unsupported("a direct message to a user that is gone"))?;
        let target_guardian =
            crate::guardian::Guardian::for_user(&mut *self.conn, &target_user).await?;
        let can_chat = super::can_chat(&mut *self.conn, self.settings, &target_guardian).await?
            && target.chat_enabled.unwrap_or(false);
        if !can_chat {
            return Ok(Some(with("chat.errors.not_reachable", &target.username)));
        }
        // recipient_allows_direct_messages?
        if !g.is_staff() {
            let target_allows: bool = sqlx::query_scalar(
                "SELECT COALESCE((SELECT allow_private_messages FROM user_options WHERE user_id = $1), TRUE)",
            )
            .bind(target.id)
            .fetch_one(&mut *self.conn)
            .await?;
            let screener = crate::user_comm_screener::UserCommScreener::new(
                &mut *self.conn,
                crate::user_comm_screener::Actor {
                    id: actor.id,
                    staff: false,
                },
                &[target.id],
            )
            .await?;
            if !target_allows
                || screener.disallowing_pms_from_actor(target.id)?
                || target_user.suspended()
            {
                return Ok(Some(with(
                    "chat.errors.not_accepting_dms",
                    &target.username,
                )));
            }
        }
        Ok(None)
    }

    /// `Chat::Action::PublishAndFollowDirectMessageChannel`: the members
    /// not following who take communication from the actor (and the actor,
    /// when not following) follow, and each is told of the channel.
    async fn publish_and_follow(
        &mut self,
        bus: &pg_bus::Bus,
        channel: &ChannelRow,
        actor: i32,
        actor_staff: bool,
        actor_following: bool,
    ) -> Result<(), AppError> {
        let not_following: Vec<i32> = sqlx::query_scalar(
            "SELECT user_id FROM user_chat_channel_memberships WHERE chat_channel_id = $1 AND NOT following \
             ORDER BY id",
        )
        .bind(channel.id)
        .fetch_all(&mut *self.conn)
        .await?;
        let screener = crate::user_comm_screener::UserCommScreener::new(
            &mut *self.conn,
            crate::user_comm_screener::Actor {
                id: actor,
                staff: actor_staff,
            },
            &not_following,
        )
        .await?;
        let mut user_ids = screener.allowing_actor_communication();
        if !actor_following {
            user_ids.push(actor);
        }
        if user_ids.is_empty() {
            return Ok(());
        }
        sqlx::query(
            "UPDATE user_chat_channel_memberships SET following = TRUE \
             WHERE chat_channel_id = $1 AND user_id = ANY($2)",
        )
        .bind(channel.id)
        .bind(&user_ids)
        .execute(&mut *self.conn)
        .await?;
        super::auto_join::publish_new_channel(
            &mut *self.conn,
            bus,
            self.i18n,
            self.config,
            self.settings,
            channel.id,
            &user_ids,
        )
        .await
    }

    /// `Chat::Thread#add(user)`: a tracking membership, unless there is one.
    async fn add_to_thread(
        &mut self,
        thread_id: i64,
        user_id: i64,
        now: NaiveDateTime,
    ) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO user_chat_thread_memberships (user_id, thread_id, notification_level, created_at, updated_at) \
             SELECT $1, $2, $3, $4, $4 WHERE NOT EXISTS \
               (SELECT 1 FROM user_chat_thread_memberships WHERE user_id = $1 AND thread_id = $2)",
        )
        .bind(user_id)
        .bind(thread_id)
        .bind(THREAD_TRACKING)
        .bind(now)
        .execute(&mut *self.conn)
        .await?;
        Ok(())
    }
}

/// Jobs::Chat::ProcessMessage
pub async fn process_message(
    state: &crate::AppState,
    conn: &mut sqlx::PgConnection,
    settings: &crate::site_settings::SiteSettings,
    args: &Value,
) -> Result<(), AppError> {
    let Some(message_id) = args["chat_message_id"].as_i64() else {
        return Ok(());
    };
    #[derive(sqlx::FromRow)]
    struct Row {
        chat_channel_id: i64,
        user_id: Option<i32>,
        last_editor_id: i32,
        message: Option<String>,
        cooked: Option<String>,
        created_at: NaiveDateTime,
        deleted_at: Option<NaiveDateTime>,
    }
    let row: Option<Row> = sqlx::query_as(
        "SELECT chat_channel_id, user_id, last_editor_id, message, cooked, created_at, deleted_at \
         FROM chat_messages WHERE id = $1",
    )
    .bind(message_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(Row {
        chat_channel_id: channel_id,
        user_id,
        last_editor_id: editor_id,
        message,
        cooked: stored_cooked,
        created_at,
        deleted_at,
    }) = row
    else {
        return Ok(());
    };
    let user_id = user_id.ok_or(Unsupported("chat messages of deleted users"))?;
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    // Chat::MessageProcessor: cooked again, then the post processors,
    // which only change what holds oneboxes, images or videos.
    let host = crate::pretty_text::Host::from_state(state);
    let cooked = super::cook::cook(
        &host,
        message.as_deref().unwrap_or_default(),
        editor_id,
        &username,
    )
    .await?;
    if cooked.contains("class=\"onebox\"")
        || has_processed_images(
            &cooked,
            &crate::url::Urls {
                config: &state.config,
                settings,
            }
            .current_hostname_with_port()?,
        )
        || cooked.contains("<video")
    {
        return Err(Unsupported("oneboxes, images and videos in chat messages").into());
    }
    process_thumbnails(conn, settings, &state.config, message_id).await?;
    if stored_cooked.as_deref() != Some(cooked.as_str()) {
        sqlx::query("UPDATE chat_messages SET cooked = $2, cooked_version = $3, updated_at = $4 WHERE id = $1")
            .bind(message_id)
            .bind(&cooked)
            .bind(super::cook::BAKED_VERSION)
            .bind(crate::clock::now_naive())
            .execute(&mut *conn)
            .await?;
    }

    // upsert_mentions
    let parsed = super::mentions::parse(&cooked);
    let author = super::mentions::Author::load(&mut *conn, user_id)
        .await?
        .ok_or(Unsupported("chat messages of deleted users"))?;
    let now = crate::clock::now_naive();
    super::mentions::upsert(
        conn, &state.bus, settings, message_id, &parsed, &author, now,
    )
    .await?;

    // Chat::MessageLink.extract_from
    if deleted_at.is_none() {
        extract_links(conn, message_id, &cooked, now).await?;
    }

    // Chat::Notifier#notify_edit or #notify_new.
    let sql = format!(
        "SELECT {} FROM chat_channels WHERE id = $1",
        super::channels::CHANNEL_COLUMNS
    );
    let channel: ChannelRow = sqlx::query_as(&sql)
        .bind(channel_id)
        .fetch_one(&mut *conn)
        .await?;
    let notifier = super::mentions::Notifier {
        state,
        settings,
        channel: &channel,
        message_id,
        author: &author,
        parsed: &parsed,
    };
    if let Some(timestamp) = args.get("edit_timestamp") {
        let timestamp = match timestamp {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        notifier.notify_edit(conn, &timestamp).await?;
    } else if !args["skip_notifications"].as_bool().unwrap_or(false) {
        notifier.notify_new(conn, created_at).await?;
    }

    // Chat::Publisher.publish_processed!
    let anonymous = crate::guardian::Guardian::anonymous();
    {
        let mut cx = Context {
            conn: &mut *conn,
            settings,
            i18n: &state.i18n,
            guardian: &anonymous,
            base_path: state.config.globals.relative_url_root(),
            config: &state.config,
        };
        super::publisher::publish_message(&mut cx, &state.bus, &channel, message_id, "processed")
            .await?;
    }
    Ok(())
}

/// `Chat::MessageProcessor::IMG_FILETYPES`
const IMG_FILETYPES: [&str; 7] = ["jpg", "jpeg", "gif", "png", "heic", "heif", "webp"];

/// `Chat::MessageProcessor#process_thumbnails`: each image upload larger
/// than max_image_width by max_image_height gets its thumbnail and the
/// responsive ones, until the first that isn't (Rails returns there). The
/// optimized videos post_process_videos references are refused.
async fn process_thumbnails(
    conn: &mut sqlx::PgConnection,
    settings: &crate::site_settings::SiteSettings,
    config: &crate::config::Config,
    message_id: i64,
) -> Result<(), AppError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        #[sqlx(flatten)]
        upload: crate::optimized_images::Upload,
        thumbnail_width: Option<i32>,
        thumbnail_height: Option<i32>,
        optimized_video: bool,
    }
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT u.id, u.url, u.sha1, u.extension, u.original_filename, u.filesize, u.width, u.height, \
                u.animated, u.dominant_color, u.thumbnail_width, u.thumbnail_height, \
                EXISTS (SELECT 1 FROM optimized_videos v WHERE v.upload_id = u.id) AS optimized_video \
         FROM upload_references ur JOIN uploads u ON u.id = ur.upload_id \
         WHERE ur.target_type = 'ChatMessage' AND ur.target_id = $1 ORDER BY ur.id",
    )
    .bind(message_id)
    .fetch_all(&mut *conn)
    .await?;
    let video = |name: &str| {
        let lower = name.to_lowercase();
        ["mov", "mp4", "webm", "ogv", "m4v", "3gp", "avi", "mpeg"]
            .iter()
            .any(|e| lower.ends_with(&format!(".{e}")))
    };
    if rows
        .iter()
        .any(|r| r.optimized_video && video(&r.upload.original_filename))
    {
        return Err(Unsupported("optimized videos of chat message uploads").into());
    }
    if !settings.get("create_thumbnails")?.truthy() {
        return Ok(());
    }
    let store = crate::file_store::FileStore::for_site(config, settings)?;
    let max_width = settings.get("max_image_width")?.to_i();
    let max_height = settings.get("max_image_height")?.to_i();
    let min_ratio_to_crop = settings.get("min_ratio_to_crop")?.to_f();
    // each_responsive_ratio
    let mut ratios: Vec<f64> = settings
        .get("responsive_post_image_sizes")?
        .to_s()
        .split('|')
        .map(|r| r.trim().parse::<f64>().unwrap_or(0.0))
        .collect();
    ratios.sort_by(|a, b| a.total_cmp(b));
    for r in rows {
        let ext = r
            .upload
            .extension
            .clone()
            .unwrap_or_default()
            .to_lowercase();
        if !IMG_FILETYPES.contains(&ext.as_str()) {
            continue;
        }
        let (Some(width), Some(height)) = (r.upload.width, r.upload.height) else {
            return Err(Unsupported("a chat image upload without its dimensions").into());
        };
        if i64::from(width) <= max_width && i64::from(height) <= max_height {
            return Ok(());
        }
        let (Some(tw), Some(th)) = (r.thumbnail_width, r.thumbnail_height) else {
            return Err(Unsupported("a chat image upload without its thumbnail size").into());
        };
        let crop =
            min_ratio_to_crop > 0.0 && f64::from(width) / f64::from(height) < min_ratio_to_crop;
        crate::optimized_images::create_thumbnail(conn, settings, &store, &r.upload, tw, th, crop)
            .await?;
        for ratio in ratios.iter().filter(|r| **r > 1.0) {
            let (rw, rh) = (
                (f64::from(tw) * ratio) as i32,
                (f64::from(th) * ratio) as i32,
            );
            if rw <= width {
                crate::optimized_images::create_thumbnail(
                    conn, settings, &store, &r.upload, rw, rh, crop,
                )
                .await?;
            }
        }
    }
    Ok(())
}

/// Whether the message processor would change an image: an upload's
/// (data-base62-sha1, its lightbox) or one from another host than the
/// site's (hotlinked media). Emoji, data: images and the site's own, a
/// quote's avatar, it leaves as they are.
fn has_processed_images(cooked: &str, host: &str) -> bool {
    let dom = crate::pretty_text::cleanup::parse(cooked);
    crate::pretty_text::cleanup::all_elements(&dom)
        .iter()
        .any(|e| {
            if crate::pretty_text::cleanup::element_name(e) != Some("img")
                || crate::pretty_text::cleanup::has_class(e, "emoji")
            {
                return false;
            }
            let src = crate::pretty_text::cleanup::attr(e, "src").unwrap_or_default();
            let remote = src
                .strip_prefix("https:")
                .or_else(|| src.strip_prefix("http:"))
                .unwrap_or(&src)
                .strip_prefix("//")
                .and_then(|rest| rest.split('/').next())
                .is_some_and(|h| !h.eq_ignore_ascii_case(host));
            crate::pretty_text::cleanup::attr(e, "data-base62-sha1").is_some() || remote
        })
}

/// `Chat::MessageLink.extract_from`
async fn extract_links(
    conn: &mut sqlx::PgConnection,
    message_id: i64,
    cooked: &str,
    now: NaiveDateTime,
) -> Result<(), AppError> {
    let mut urls: Vec<String> = Vec::new();
    for link in crate::posting::links::extract_with_quotes(cooked)? {
        // UrlHelper.relaxed_parse: the url as written, with its scheme and
        // host; relative urls (mentions' profiles) have no host.
        let (scheme, rest) = match link.split_once(':') {
            Some((s, r))
                if !s.is_empty()
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)) =>
            {
                (s, r)
            }
            _ => ("", link.as_str()),
        };
        if scheme.eq_ignore_ascii_case("mailto") {
            continue;
        }
        let host = rest
            .strip_prefix("//")
            .and_then(|r| r.split(['/', '?', '#']).next())
            .and_then(|h| h.rsplit('@').next())
            .and_then(|h| h.split(':').next())
            .unwrap_or_default();
        if host.is_empty() || host.len() > 100 {
            continue;
        }
        let url: String = link.chars().take(500).collect();
        if urls.contains(&url) {
            continue;
        }
        sqlx::query(
            "INSERT INTO chat_message_links (chat_message_id, url, created_at, updated_at) \
             VALUES ($1, $2, $3, $3) ON CONFLICT (chat_message_id, url) DO NOTHING",
        )
        .bind(message_id)
        .bind(&url)
        .bind(now)
        .execute(&mut *conn)
        .await?;
        urls.push(url);
    }
    sqlx::query(
        "DELETE FROM chat_message_links WHERE chat_message_id = $1 AND NOT (url = ANY($2))",
    )
    .bind(message_id)
    .bind(&urls)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
