//! Chat threads: Chat::Thread and its memberships
//! (Chat::UserChatThreadMembership), Chat::ThreadSerializer with its
//! preview, original message and participants (Chat::ThreadParticipantQuery),
//! Chat::ThreadUnreadsQuery's tracking, and the thread API
//! (Chat::Api::ChannelThreadsController and its siblings: the channel's
//! threads, a thread, its messages, title, read state, notification level,
//! title prompt and drafts; the member's threads).
//!
//! A thread shows when its channel has threading on or the thread is
//! forced (`Chat::Message#in_thread?`); a reply in a channel without
//! threading makes a hidden thread whose messages are the channel's.
//! The reply count is the thread's live replies, what Rails' Redis count
//! (replies_count_cache) holds once warm.

use std::collections::HashMap;

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::channels::{ChannelRow, Context};
use crate::{AppError, Unsupported};

/// `Chat::NotificationLevels.all`, by stored value.
pub const NOTIFICATION_LEVELS: [&str; 4] = ["muted", "normal", "tracking", "watching"];
pub const MUTED: i32 = 0;
pub const NORMAL: i32 = 1;
pub const TRACKING: i32 = 2;
pub const WATCHING: i32 = 3;
/// `Chat::Thread.statuses`
const STATUSES: [&str; 4] = ["open", "read_only", "closed", "archived"];
/// `Chat::Thread::MAX_TITLE_LENGTH`
pub const MAX_TITLE_LENGTH: usize = 100;
/// `LookupChannelThreads::THREADS_LIMIT` and LookupUserThreads'.
pub const THREADS_LIMIT: i64 = 10;
/// `Chat::ThreadUnreadsQuery::MAX_THREADS`
const MAX_THREADS: i64 = 500;
/// `Chat::ThreadParticipantQuery::MAX_PARTICIPANTS`
const MAX_PARTICIPANTS: usize = 10;
/// `Notification.types[:chat_mention]`
const CHAT_MENTION: i32 = 29;

/// A row of chat_threads.
#[derive(sqlx::FromRow, Clone)]
pub struct ThreadRow {
    pub id: i64,
    pub channel_id: i64,
    pub original_message_id: i64,
    pub original_message_user_id: i64,
    pub status: i32,
    pub title: Option<String>,
    pub force: bool,
    pub last_message_id: Option<i64>,
}

pub(crate) const THREAD_COLUMNS: &str = "chat_threads.id, chat_threads.channel_id, \
     chat_threads.original_message_id, chat_threads.original_message_user_id, chat_threads.status, \
     chat_threads.title, chat_threads.force, chat_threads.last_message_id";

/// A row of user_chat_thread_memberships.
#[derive(sqlx::FromRow, Clone)]
pub struct ThreadMembership {
    pub thread_id: i64,
    pub notification_level: i32,
    pub last_read_message_id: Option<i64>,
    pub thread_title_prompt_seen: bool,
}

/// A thread's participants as Chat::ThreadParticipantQuery gives them:
/// up to nine of the most prolific, the most recent last, and how many
/// wrote in it.
#[derive(Clone, Default)]
pub struct Participants {
    pub users: Vec<i32>,
    pub total_count: i64,
}

/// One thread's tracking, as Chat::ThreadUnreadsQuery counts it.
#[derive(Clone)]
pub struct ThreadUnread {
    pub thread_id: i64,
    pub channel_id: i64,
    pub unread_count: i64,
    pub mention_count: i64,
    pub watched_threads_unread_count: i64,
}

/// What Chat::ThreadSerializer is asked to include.
#[derive(Default, Clone, Copy)]
pub struct ThreadOpts<'a> {
    pub preview: bool,
    pub original_message: bool,
    pub channel: bool,
    pub membership: Option<&'a ThreadMembership>,
    pub participants: Option<&'a Participants>,
}

/// `Chat::Message#in_thread?` for a message of the channel in the thread.
pub fn shows(channel: &ChannelRow, thread: &ThreadRow) -> bool {
    channel.threading_enabled || thread.force
}

/// `Chat::BaseThreadMembershipSerializer`
pub fn membership_json(m: &ThreadMembership) -> Value {
    json!({
        "notification_level": m.notification_level,
        "thread_id": m.thread_id,
        "last_read_message_id": m.last_read_message_id,
        "thread_title_prompt_seen": m.thread_title_prompt_seen,
    })
}

fn iso8601(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

impl Context<'_> {
    /// `Chat::Thread.find_by(id:)`, and the channel when given.
    pub async fn find_thread(
        &mut self,
        id: i64,
        channel_id: Option<i64>,
    ) -> Result<Option<ThreadRow>, AppError> {
        let sql = format!(
            "SELECT {THREAD_COLUMNS} FROM chat_threads WHERE id = $1 AND ($2::bigint IS NULL OR channel_id = $2)"
        );
        Ok(sqlx::query_as(&sql)
            .bind(id)
            .bind(channel_id)
            .fetch_optional(&mut *self.conn)
            .await?)
    }

    pub(crate) async fn threads_by_id(
        &mut self,
        ids: &[i64],
    ) -> Result<HashMap<i64, ThreadRow>, AppError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let sql = format!("SELECT {THREAD_COLUMNS} FROM chat_threads WHERE id = ANY($1)");
        let rows: Vec<ThreadRow> = sqlx::query_as(&sql)
            .bind(ids)
            .fetch_all(&mut *self.conn)
            .await?;
        Ok(rows.into_iter().map(|t| (t.id, t)).collect())
    }

    /// The live channel a thread belongs to (`thread.channel`, trashed
    /// channels hidden by the default scope, where Rails fails).
    pub(crate) async fn thread_channel(
        &mut self,
        thread: &ThreadRow,
    ) -> Result<ChannelRow, AppError> {
        let sql = format!(
            "SELECT {} FROM chat_channels WHERE id = $1 AND deleted_at IS NULL",
            super::channels::CHANNEL_COLUMNS
        );
        let channel: Option<ChannelRow> = sqlx::query_as(&sql)
            .bind(thread.channel_id)
            .fetch_optional(&mut *self.conn)
            .await?;
        channel.ok_or_else(|| Unsupported("threads of deleted chat channels").into())
    }

    /// The viewer's memberships of these threads, by thread.
    pub(crate) async fn thread_memberships(
        &mut self,
        thread_ids: &[i64],
    ) -> Result<HashMap<i64, ThreadMembership>, AppError> {
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(HashMap::new());
        };
        if thread_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows: Vec<ThreadMembership> = sqlx::query_as(
            "SELECT thread_id, notification_level, last_read_message_id, thread_title_prompt_seen \
             FROM user_chat_thread_memberships WHERE user_id = $1 AND thread_id = ANY($2)",
        )
        .bind(i64::from(user_id))
        .bind(thread_ids)
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(rows.into_iter().map(|m| (m.thread_id, m)).collect())
    }

    async fn thread_membership(
        &mut self,
        thread_id: i64,
        user_id: i64,
    ) -> Result<Option<ThreadMembership>, AppError> {
        Ok(sqlx::query_as(
            "SELECT thread_id, notification_level, last_read_message_id, thread_title_prompt_seen \
             FROM user_chat_thread_memberships WHERE user_id = $1 AND thread_id = $2",
        )
        .bind(user_id)
        .bind(thread_id)
        .fetch_optional(&mut *self.conn)
        .await?)
    }

    /// `Chat::Thread.viewable_by_user(user).exists?`
    pub(crate) async fn user_has_threads(&mut self, user_id: i64) -> Result<bool, AppError> {
        user_has_threads(&mut *self.conn, user_id).await
    }

    /// `Chat::Thread#add(user, notification_level:)`
    pub(crate) async fn add_thread_member(
        &mut self,
        bus: &pg_bus::Bus,
        thread_id: i64,
        user_id: i64,
        notification_level: i32,
        now: NaiveDateTime,
    ) -> Result<(), AppError> {
        add_member(
            &mut *self.conn,
            bus,
            thread_id,
            user_id,
            notification_level,
            now,
        )
        .await
    }

    /// Chat::ThreadParticipantQuery
    pub(crate) async fn thread_participants(
        &mut self,
        thread_ids: &[i64],
    ) -> Result<HashMap<i64, Participants>, AppError> {
        let mut out: HashMap<i64, Participants> = HashMap::new();
        if thread_ids.is_empty() {
            return Ok(out);
        }
        let stats: Vec<(i64, i32)> = sqlx::query_as(
            "SELECT stats.thread_id, stats.user_id FROM ( \
               SELECT chat_messages.thread_id, chat_messages.user_id, COUNT(*) AS message_count \
               FROM chat_messages \
               INNER JOIN chat_threads ON chat_threads.id = chat_messages.thread_id \
               INNER JOIN user_chat_thread_memberships ON user_chat_thread_memberships.thread_id = chat_threads.id \
                 AND user_chat_thread_memberships.user_id = chat_messages.user_id \
               WHERE chat_messages.thread_id = ANY($1) AND chat_messages.deleted_at IS NULL \
               GROUP BY chat_messages.thread_id, chat_messages.user_id) AS stats \
             INNER JOIN users ON users.id = stats.user_id \
             ORDER BY stats.thread_id ASC, stats.message_count DESC, stats.user_id ASC",
        )
        .bind(thread_ids)
        .fetch_all(&mut *self.conn)
        .await?;
        let recent: Vec<(i64, i32)> = sqlx::query_as(
            "SELECT DISTINCT ON (thread_id) chat_messages.thread_id, chat_messages.user_id \
             FROM chat_messages \
             INNER JOIN chat_threads ON chat_threads.id = chat_messages.thread_id \
             INNER JOIN user_chat_thread_memberships ON user_chat_thread_memberships.thread_id = chat_threads.id \
               AND user_chat_thread_memberships.user_id = chat_messages.user_id \
             INNER JOIN users ON users.id = chat_messages.user_id \
             WHERE chat_messages.thread_id = ANY($1) AND chat_messages.deleted_at IS NULL \
             ORDER BY chat_messages.thread_id ASC, chat_messages.created_at DESC",
        )
        .bind(thread_ids)
        .fetch_all(&mut *self.conn)
        .await?;
        let recent: HashMap<i64, i32> = recent.into_iter().collect();
        for (thread_id, user_id) in stats {
            let p = out.entry(thread_id).or_default();
            if p.users.len() < MAX_PARTICIPANTS - 1 && recent.get(&thread_id) != Some(&user_id) {
                p.users.push(user_id);
            }
            p.total_count += 1;
        }
        for (thread_id, user_id) in recent {
            out.entry(thread_id).or_default().users.push(user_id);
        }
        Ok(out)
    }

    /// Core's BasicUserSerializer for a user, null when gone.
    pub(crate) async fn core_basic_user(
        &mut self,
        user_id: Option<i32>,
    ) -> Result<Value, AppError> {
        let row: Option<(i32, String, Option<String>, Option<i32>)> = match user_id {
            Some(id) => {
                sqlx::query_as(
                    "SELECT id, username, name, uploaded_avatar_id FROM users WHERE id = $1",
                )
                .bind(id)
                .fetch_optional(&mut *self.conn)
                .await?
            }
            None => None,
        };
        let enable_names = self.settings.get("enable_names")?.truthy();
        let mut out = Map::new();
        let Some((id, username, name, avatar)) = row else {
            // Chat::NullUser
            out.insert("id".into(), Value::Null);
            out.insert(
                "username".into(),
                json!(
                    self.i18n
                        .t("chat.deleted_chat_username")
                        .unwrap_or("deleted")
                ),
            );
            if enable_names {
                out.insert("name".into(), Value::Null);
            }
            out.insert(
                "avatar_template".into(),
                json!("/plugins/chat/images/deleted-chat-user-avatar.png"),
            );
            return Ok(Value::Object(out));
        };
        let logo = crate::admin_users::logo_small_url(&mut *self.conn, self.settings).await?;
        let urls = crate::url::Urls {
            config: self.config,
            settings: self.settings,
        };
        out.insert("id".into(), json!(id));
        out.insert("username".into(), json!(username));
        if enable_names {
            out.insert("name".into(), json!(name));
        }
        out.insert(
            "avatar_template".into(),
            json!(crate::avatar::avatar_template(
                &urls,
                id,
                &username,
                avatar,
                logo.as_deref()
            )?),
        );
        Ok(Value::Object(out))
    }

    /// The thread's live replies: what the reply count shows.
    pub(crate) async fn thread_reply_count(&mut self, thread: &ThreadRow) -> Result<i64, AppError> {
        Ok(sqlx::query_scalar(
            "SELECT COUNT(*) FROM chat_messages WHERE thread_id = $1 AND deleted_at IS NULL AND id <> $2",
        )
        .bind(thread.id)
        .bind(thread.original_message_id)
        .fetch_one(&mut *self.conn)
        .await?)
    }

    /// Chat::ThreadPreviewSerializer: the thread's last live message (the
    /// NullMessage without one), its reply count and participants.
    pub(crate) async fn thread_preview(
        &mut self,
        thread: &ThreadRow,
        participants: Option<&Participants>,
    ) -> Result<Value, AppError> {
        #[derive(sqlx::FromRow)]
        struct Last {
            id: i64,
            created_at: NaiveDateTime,
            user_id: Option<i32>,
            excerpt: Option<String>,
            message: Option<String>,
            cooked: Option<String>,
        }
        let last: Option<Last> = sqlx::query_as(
            "SELECT id, created_at, user_id, excerpt, message, cooked FROM chat_messages \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(thread.last_message_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let reply_count = self.thread_reply_count(thread).await?;
        // The NullMessage: now, no excerpt, id or user.
        let (created_at, excerpt, id, user_id) = match last {
            Some(last) => {
                let excerpt = match last.excerpt {
                    Some(e) => e,
                    None => super::messages::build_excerpt_for(
                        last.message.as_deref().unwrap_or_default(),
                        last.cooked.as_deref().unwrap_or_default(),
                        crate::excerpt::Plugins::from_settings(self.settings)?,
                    )?,
                };
                (
                    last.created_at,
                    json!(excerpt),
                    json!(last.id),
                    last.user_id,
                )
            }
            None => (crate::clock::now_naive(), Value::Null, Value::Null, None),
        };
        let mut out = Map::new();
        out.insert("last_reply_created_at".into(), json!(iso8601(created_at)));
        out.insert("last_reply_excerpt".into(), excerpt);
        out.insert("last_reply_id".into(), id);
        if let Some(p) = participants {
            out.insert("participant_count".into(), json!(p.total_count));
        }
        out.insert("reply_count".into(), json!(reply_count));
        if let Some(p) = participants {
            let mut users = Vec::new();
            for id in &p.users {
                users.push(self.core_basic_user(Some(*id)).await?);
            }
            out.insert("participant_users".into(), Value::Array(users));
        }
        out.insert(
            "last_reply_user".into(),
            self.core_basic_user(user_id).await?,
        );
        Ok(Value::Object(out))
    }

    /// Chat::ThreadOriginalMessageSerializer (deleted or not: the
    /// association reads with_deleted).
    async fn thread_original_message(&mut self, thread: &ThreadRow) -> Result<Value, AppError> {
        #[derive(sqlx::FromRow)]
        struct Om {
            id: i64,
            message: Option<String>,
            cooked: Option<String>,
            created_at: NaiveDateTime,
            excerpt: Option<String>,
            chat_channel_id: i64,
            user_id: Option<i32>,
        }
        let om: Om = sqlx::query_as(
            "SELECT id, message, cooked, created_at, excerpt, chat_channel_id, user_id \
             FROM chat_messages WHERE id = $1",
        )
        .bind(thread.original_message_id)
        .fetch_one(&mut *self.conn)
        .await?;
        let user_id = om
            .user_id
            .ok_or(Unsupported("threads of deleted users' messages"))?;
        let excerpt = match om.excerpt {
            Some(e) => e,
            None => super::messages::build_excerpt_for(
                om.message.as_deref().unwrap_or_default(),
                om.cooked.as_deref().unwrap_or_default(),
                crate::excerpt::Plugins::from_settings(self.settings)?,
            )?,
        };
        let max = self.settings.get("max_mentions_per_chat_message")?.to_i();
        let mut mentioned: Vec<i32> = sqlx::query_scalar(
            "SELECT chat_mentions.target_id FROM chat_mentions \
             JOIN users ON users.id = chat_mentions.target_id \
             WHERE chat_mentions.chat_message_id = $1 AND chat_mentions.type = 'Chat::UserMention' \
             ORDER BY chat_mentions.id LIMIT $2",
        )
        .bind(om.id)
        .bind(max)
        .fetch_all(&mut *self.conn)
        .await?;
        mentioned.sort_unstable();
        let mut mentioned_users = Vec::new();
        for id in mentioned {
            mentioned_users.push(self.chat_basic_user(Some(id)).await?);
        }
        Ok(json!({
            "id": om.id,
            "message": om.message,
            "cooked": om.cooked,
            "created_at": crate::topic_list::time_json(om.created_at),
            "excerpt": excerpt,
            "chat_channel_id": om.chat_channel_id,
            "deleted_at": null,
            "mentioned_users": mentioned_users,
            "user": self.chat_basic_user(Some(user_id)).await?,
        }))
    }

    /// Chat::BasicUserSerializer without a scope: core's, its can_chat
    /// and has_chat_enabled nil.
    async fn chat_basic_user(&mut self, user_id: Option<i32>) -> Result<Value, AppError> {
        let mut user = self.core_basic_user(user_id).await?;
        if let Value::Object(u) = &mut user {
            u.insert("can_chat".into(), Value::Null);
            u.insert("has_chat_enabled".into(), Value::Null);
        }
        Ok(user)
    }

    /// Chat::ThreadSerializer
    pub(crate) async fn thread_json(
        &mut self,
        thread: &ThreadRow,
        opts: ThreadOpts<'_>,
    ) -> Result<Value, AppError> {
        let mut out = Map::new();
        out.insert("id".into(), json!(thread.id));
        out.insert("title".into(), json!(thread.title));
        out.insert(
            "status".into(),
            json!(
                STATUSES
                    .get(thread.status as usize)
                    .copied()
                    .unwrap_or("open")
            ),
        );
        out.insert("channel_id".into(), json!(thread.channel_id));
        if opts.channel {
            let channel = self.thread_channel(thread).await?;
            out.insert(
                "channel".into(),
                Box::pin(self.channel(&channel, None, None, None)).await?,
            );
        }
        out.insert(
            "meta".into(),
            json!({"message_bus_last_ids": {"thread_message_bus_last_id": 0}}),
        );
        out.insert(
            "reply_count".into(),
            json!(self.thread_reply_count(thread).await?),
        );
        if let Some(m) = opts.membership {
            out.insert("current_user_membership".into(), membership_json(m));
        }
        if opts.preview {
            out.insert(
                "preview".into(),
                self.thread_preview(thread, opts.participants).await?,
            );
        }
        out.insert("last_message_id".into(), json!(thread.last_message_id));
        out.insert("force".into(), json!(thread.force));
        if opts.original_message {
            out.insert(
                "original_message".into(),
                self.thread_original_message(thread).await?,
            );
        }
        Ok(Value::Object(out))
    }

    /// Chat::ThreadUnreadsQuery for the viewer: their threads' unread,
    /// mention and watched counts in these channels and/or threads.
    pub(crate) async fn thread_unreads(
        &mut self,
        channel_ids: &[i64],
        thread_ids: &[i64],
        include_missing_memberships: bool,
        include_read: bool,
    ) -> Result<Vec<ThreadUnread>, AppError> {
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(Vec::new());
        };
        if channel_ids.is_empty() && thread_ids.is_empty() {
            return Ok(Vec::new());
        }
        let filters = format!(
            "{}{}",
            if channel_ids.is_empty() {
                ""
            } else {
                " AND chat_threads.channel_id = ANY($2)"
            },
            if thread_ids.is_empty() {
                ""
            } else {
                " AND chat_threads.id = ANY($3)"
            },
        );
        let mut sql = format!(
            "WITH limited_memberships AS ( \
               SELECT memberships.thread_id, memberships.last_read_message_id, memberships.notification_level, \
                      chat_threads.channel_id, chat_threads.original_message_id, chat_threads.force, \
                      chat_channels.threading_enabled, uccm.muted AS channel_muted \
               FROM user_chat_thread_memberships AS memberships \
               INNER JOIN chat_threads ON chat_threads.id = memberships.thread_id \
               INNER JOIN chat_channels ON chat_channels.id = chat_threads.channel_id \
               INNER JOIN user_chat_channel_memberships AS uccm \
                 ON uccm.chat_channel_id = chat_channels.id AND uccm.user_id = $1 \
               WHERE memberships.user_id = $1{filters} \
               ORDER BY chat_threads.last_message_id DESC NULLS LAST LIMIT {MAX_THREADS}) \
             SELECT CASE WHEN lm.notification_level = {TRACKING} THEN COALESCE(unread_calc.cnt, 0) ELSE 0 END AS unread_count, \
                    COALESCE(mention_calc.cnt, 0) AS mention_count, \
                    CASE WHEN lm.notification_level = {WATCHING} THEN COALESCE(unread_calc.cnt, 0) ELSE 0 END AS watched_threads_unread_count, \
                    lm.channel_id, lm.thread_id \
             FROM limited_memberships lm \
             LEFT JOIN LATERAL ( \
               SELECT COUNT(*) AS cnt FROM chat_messages cm \
               WHERE cm.thread_id = lm.thread_id AND cm.user_id != $1 \
                 AND cm.id > COALESCE(lm.last_read_message_id, 0) AND cm.deleted_at IS NULL \
                 AND cm.id != lm.original_message_id AND NOT lm.channel_muted \
                 AND (lm.threading_enabled OR lm.force) \
                 AND EXISTS (SELECT 1 FROM chat_messages om WHERE om.id = lm.original_message_id AND om.deleted_at IS NULL) \
             ) unread_calc ON true \
             LEFT JOIN LATERAL ( \
               SELECT COUNT(*) AS cnt FROM notifications n \
               INNER JOIN chat_messages cm ON cm.id = (n.data::json->>'chat_message_id')::bigint \
               WHERE n.user_id = $1 AND n.notification_type = {CHAT_MENTION} AND NOT n.read \
                 AND cm.thread_id = lm.thread_id AND cm.deleted_at IS NULL \
                 AND cm.id > COALESCE(lm.last_read_message_id, 0) \
                 AND lm.threading_enabled AND NOT lm.channel_muted \
             ) mention_calc ON true"
        );
        if !include_read {
            sql = format!(
                "SELECT * FROM ({sql}) AS thread_tracking \
                 WHERE (unread_count > 0 OR mention_count > 0 OR watched_threads_unread_count > 0)"
            );
        }
        if include_missing_memberships && include_read {
            sql.push_str(&format!(
                " UNION ALL \
                 SELECT 0, 0, 0, chat_threads.channel_id, chat_threads.id \
                 FROM chat_channels \
                 INNER JOIN chat_threads ON chat_threads.channel_id = chat_channels.id \
                 LEFT JOIN user_chat_thread_memberships ON user_chat_thread_memberships.thread_id = chat_threads.id \
                   AND user_chat_thread_memberships.user_id = $1 \
                 WHERE user_chat_thread_memberships.id IS NULL{filters} \
                 GROUP BY chat_threads.id LIMIT {MAX_THREADS}"
            ));
        }
        let rows: Vec<(i64, i64, i64, i64, i64)> = sqlx::query_as(&sql)
            .bind(i64::from(user_id))
            .bind(channel_ids)
            .bind(thread_ids)
            .fetch_all(&mut *self.conn)
            .await?;
        Ok(rows
            .into_iter()
            .map(
                |(
                    unread_count,
                    mention_count,
                    watched_threads_unread_count,
                    channel_id,
                    thread_id,
                )| {
                    ThreadUnread {
                        thread_id,
                        channel_id,
                        unread_count,
                        mention_count,
                        watched_threads_unread_count,
                    }
                },
            )
            .collect())
    }

    /// TrackingStateReport#thread_tracking for these threads
    /// (include_threads, include_read).
    pub(crate) async fn thread_tracking_for(
        &mut self,
        thread_ids: &[i64],
        include_missing_memberships: bool,
    ) -> Result<Map<String, Value>, AppError> {
        let mut out = Map::new();
        for t in self
            .thread_unreads(&[], thread_ids, include_missing_memberships, true)
            .await?
        {
            out.insert(
                t.thread_id.to_string(),
                json!({
                    "channel_id": t.channel_id,
                    "mention_count": t.mention_count,
                    "unread_count": t.unread_count,
                    "watched_threads_unread_count": t.watched_threads_unread_count,
                }),
            );
        }
        Ok(out)
    }

    /// TrackingStateReportQuery with include_threads, include_read: false
    /// and include_last_reply_details: each unread thread's last reply
    /// time, by channel (`thread_unread_overview_by_channel`).
    pub(crate) async fn unread_thread_overview(
        &mut self,
        channel_ids: &[i64],
    ) -> Result<Map<String, Value>, AppError> {
        let mut out = Map::new();
        let unreads = self.thread_unreads(channel_ids, &[], false, false).await?;
        let ids: Vec<i64> = unreads.iter().map(|t| t.thread_id).collect();
        let details: HashMap<i64, NaiveDateTime> = if ids.is_empty() {
            HashMap::new()
        } else {
            sqlx::query_as::<_, (i64, NaiveDateTime)>(
                "SELECT chat_threads.id, last_message.created_at FROM chat_threads \
                 INNER JOIN chat_messages AS last_message ON last_message.id = chat_threads.last_message_id \
                 WHERE chat_threads.id = ANY($1) AND last_message.deleted_at IS NULL",
            )
            .bind(&ids)
            .fetch_all(&mut *self.conn)
            .await?
            .into_iter()
            .collect()
        };
        for t in unreads {
            let entry = out
                .entry(t.channel_id.to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(threads) = entry {
                threads.insert(
                    t.thread_id.to_string(),
                    json!(
                        details
                            .get(&t.thread_id)
                            .map(|d| crate::topic_list::time_json(*d))
                    ),
                );
            }
        }
        Ok(out)
    }
}

/// `Chat::Thread.viewable_by_user(user)` as the FROM clause's joins and
/// conditions on chat_threads, the user bound as $1.
pub(crate) fn viewable_by_user_sql() -> String {
    format!(
        "INNER JOIN user_chat_thread_memberships ON user_chat_thread_memberships.thread_id = chat_threads.id \
         INNER JOIN chat_channels ON chat_channels.id = chat_threads.channel_id \
         INNER JOIN chat_messages viewable_om ON viewable_om.id = chat_threads.original_message_id \
           AND viewable_om.chat_channel_id = chat_threads.channel_id AND viewable_om.deleted_at IS NULL \
         INNER JOIN chat_messages viewable_lm ON viewable_lm.id = chat_threads.last_message_id \
           AND viewable_lm.chat_channel_id = chat_threads.channel_id AND viewable_lm.deleted_at IS NULL \
         INNER JOIN user_chat_channel_memberships ON user_chat_channel_memberships.chat_channel_id = chat_channels.id \
           AND user_chat_channel_memberships.user_id = $1 AND user_chat_channel_memberships.following \
         WHERE user_chat_thread_memberships.user_id = $1 \
           AND user_chat_thread_memberships.notification_level <> {MUTED} \
           AND chat_channels.threading_enabled AND chat_channels.status = 0 \
           AND chat_threads.replies_count > 0"
    )
}

/// `Chat::Thread.viewable_by_user(user).exists?`
pub(crate) async fn user_has_threads(
    conn: &mut PgConnection,
    user_id: i64,
) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM chat_threads {})",
        viewable_by_user_sql()
    ))
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?)
}

/// `Chat::Thread#add(user, notification_level:)`: the membership, made
/// when there is none; the member's first thread in a channel with
/// threading tells them they have threads now.
pub(crate) async fn add_member(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    thread_id: i64,
    user_id: i64,
    notification_level: i32,
    now: NaiveDateTime,
) -> Result<(), AppError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_chat_thread_memberships WHERE user_id = $1 AND thread_id = $2)",
    )
    .bind(user_id)
    .bind(thread_id)
    .fetch_one(&mut *conn)
    .await?;
    if exists {
        return Ok(());
    }
    let threading: bool = sqlx::query_scalar(
        "SELECT chat_channels.threading_enabled FROM chat_threads \
         JOIN chat_channels ON chat_channels.id = chat_threads.channel_id WHERE chat_threads.id = $1",
    )
    .bind(thread_id)
    .fetch_one(&mut *conn)
    .await?;
    let is_first = threading && !user_has_threads(&mut *conn, user_id).await?;
    sqlx::query(
        "INSERT INTO user_chat_thread_memberships (user_id, thread_id, notification_level, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $4)",
    )
    .bind(user_id)
    .bind(thread_id)
    .bind(notification_level)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    if is_first && notification_level != MUTED {
        // publish_user_has_threads!, after the commit (the bus's NOTIFY
        // goes out with it).
        let user_id = user_id as i32;
        bus.publish(
            &mut *conn,
            &format!("/chat/user-has-threads/{user_id}"),
            &json!({ "has_threads": true }),
            Some(&[crate::bus::user_tag(user_id)]),
        )
        .await?;
    }
    Ok(())
}

/// A thread action's outcome, as its controller answers it.
pub enum Outcome {
    Done(Value),
    NotFound,
    Forbidden,
    /// The contract's errors (400).
    Invalid(Vec<String>),
    /// `on_failure` (422), with a step's error when it has one.
    Unprocessable(Option<String>),
}

/// An integer param as ActiveModel casts it: blank is nil, else `to_i`.
pub fn cast_param(raw: Option<&str>) -> Option<i64> {
    raw.filter(|r| !r.trim().is_empty()).map(crate::ruby::to_i)
}

/// The listing services' limit and offset (`after_validation`): the limit
/// within 1..=10 (10 without one), the offset at least 0.
fn page(limit: Option<i64>, offset: Option<i64>) -> (i64, i64) {
    (
        limit.unwrap_or(THREADS_LIMIT).clamp(1, THREADS_LIMIT),
        offset.unwrap_or(0).max(0),
    )
}

impl Context<'_> {
    /// The live channel by id (`Chat::Channel.find_by`).
    async fn live_channel_by_id(&mut self, id: i64) -> Result<Option<ChannelRow>, AppError> {
        let sql = format!(
            "SELECT {} FROM chat_channels WHERE id = $1 AND deleted_at IS NULL",
            super::channels::CHANNEL_COLUMNS
        );
        Ok(sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(&mut *self.conn)
            .await?)
    }

    /// The threads serialized for a list (Chat::ThreadListSerializer's
    /// threads and tracking).
    async fn thread_list(
        &mut self,
        threads: &[ThreadRow],
    ) -> Result<(Vec<Value>, Value), AppError> {
        let ids: Vec<i64> = threads.iter().map(|t| t.id).collect();
        let tracking = self.thread_tracking_for(&ids, false).await?;
        let memberships = self.thread_memberships(&ids).await?;
        let participants = self.thread_participants(&ids).await?;
        let mut out = Vec::new();
        for thread in threads {
            let opts = ThreadOpts {
                preview: true,
                original_message: true,
                channel: true,
                membership: memberships.get(&thread.id),
                participants: participants.get(&thread.id),
            };
            out.push(self.thread_json(thread, opts).await?);
        }
        Ok((out, Value::Object(tracking)))
    }

    /// GET /chat/api/channels/:channel_id/threads: Chat::LookupChannelThreads
    /// (Chat::Action::FetchThreads): the channel's threads with replies,
    /// their original and last messages live, unread ones first, then by
    /// last reply.
    pub async fn channel_threads(
        &mut self,
        channel_id: i64,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Outcome, AppError> {
        if limit.is_some_and(|l| l > THREADS_LIMIT) {
            return Ok(Outcome::Invalid(vec![format!(
                "Limit must be less than or equal to {THREADS_LIMIT}"
            )]));
        }
        let (limit, offset) = page(limit, offset);
        let Some(channel) = self.live_channel_by_id(channel_id).await? else {
            return Ok(Outcome::NotFound);
        };
        if !channel.threading_enabled {
            return Ok(Outcome::NotFound);
        }
        if !self.can_join(&channel, None).await? {
            return Ok(Outcome::Forbidden);
        }
        let user_id = i64::from(self.guardian.user_id().unwrap_or(0));
        let sql = format!(
            "SELECT {THREAD_COLUMNS} FROM chat_threads \
             LEFT JOIN user_chat_thread_memberships ON chat_threads.id = user_chat_thread_memberships.thread_id \
               AND user_chat_thread_memberships.user_id = $2 \
               AND user_chat_thread_memberships.notification_level NOT IN ({MUTED}) \
             LEFT JOIN chat_messages AS last_message ON chat_threads.last_message_id = last_message.id \
             INNER JOIN chat_messages AS original_message ON chat_threads.original_message_id = original_message.id \
             WHERE chat_threads.channel_id = $1 \
               AND original_message.chat_channel_id = chat_threads.channel_id \
               AND original_message.deleted_at IS NULL \
               AND last_message.chat_channel_id = chat_threads.channel_id \
               AND last_message.deleted_at IS NULL \
               AND chat_threads.replies_count > 0 \
             ORDER BY CASE WHEN user_chat_thread_memberships.id IS NOT NULL \
                 AND (user_chat_thread_memberships.last_read_message_id IS NULL \
                   OR user_chat_thread_memberships.last_read_message_id < chat_threads.last_message_id) \
               THEN true ELSE false END DESC, last_message.created_at DESC \
             LIMIT $3 OFFSET $4"
        );
        let threads: Vec<ThreadRow> = sqlx::query_as(&sql)
            .bind(channel.id)
            .bind(user_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&mut *self.conn)
            .await?;
        if threads.is_empty() {
            return Ok(Outcome::Done(json!({"success": "OK", "threads": []})));
        }
        let (threads, tracking) = self.thread_list(&threads).await?;
        Ok(Outcome::Done(json!({
            "meta": {
                "load_more_url": format!("/chat/api/channels/{}/threads?offset={}", channel.id, offset + limit),
                "channel_id": channel.id,
            },
            "threads": threads,
            "tracking": tracking,
        })))
    }

    /// GET /chat/api/me/threads: Chat::LookupUserThreads, the member's
    /// viewable threads in the channels they follow, unread ones first,
    /// then by last reply.
    pub async fn user_threads(
        &mut self,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Outcome, AppError> {
        let (limit, offset) = page(limit, offset);
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(Outcome::Forbidden);
        };
        // ChannelFetcher.all_secured_channel_ids(guardian): the followed
        // ones among those the viewer may see.
        let allowed = super::allowed_channel_ids_sql(self.settings, self.guardian, false)?;
        let sql = format!(
            "SELECT {THREAD_COLUMNS} FROM chat_threads {} \
               AND chat_threads.channel_id IN (SELECT chat_channel_id FROM user_chat_channel_memberships \
                 WHERE user_chat_channel_memberships.user_id = $1 \
                   AND user_chat_channel_memberships.chat_channel_id IN ({allowed})) \
             ORDER BY CASE WHEN user_chat_thread_memberships.last_read_message_id IS NULL \
                 OR user_chat_thread_memberships.last_read_message_id < chat_threads.last_message_id \
               THEN 1 ELSE 0 END DESC, viewable_lm.created_at DESC \
             LIMIT $2 OFFSET $3",
            viewable_by_user_sql()
        );
        let threads: Vec<ThreadRow> = sqlx::query_as(&sql)
            .bind(i64::from(user_id))
            .bind(limit)
            .bind(offset)
            .fetch_all(&mut *self.conn)
            .await?;
        if threads.is_empty() {
            return Ok(Outcome::Done(json!({"success": "OK", "threads": []})));
        }
        let (threads, tracking) = self.thread_list(&threads).await?;
        Ok(Outcome::Done(json!({
            "meta": {
                "load_more_url": format!("/chat/api/me/threads?limit={limit}&offset={}", offset + limit),
            },
            "threads": threads,
            "tracking": tracking,
        })))
    }

    /// GET /chat/api/channels/:channel_id/threads/:thread_id:
    /// Chat::LookupThread.
    pub async fn show_thread(
        &mut self,
        channel_id: i64,
        thread_id: i64,
    ) -> Result<Outcome, AppError> {
        let Some(thread) = self.find_thread(thread_id, Some(channel_id)).await? else {
            return Ok(Outcome::NotFound);
        };
        let channel = self.thread_channel(&thread).await?;
        // policy :invalid_access, :threading_enabled_for_channel,
        // :original_message_not_deleted
        if !self.can_join(&channel, None).await? {
            return Ok(Outcome::Forbidden);
        }
        if !shows(&channel, &thread) {
            return Ok(Outcome::NotFound);
        }
        let om_deleted: bool =
            sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM chat_messages WHERE id = $1")
                .bind(thread.original_message_id)
                .fetch_one(&mut *self.conn)
                .await?;
        if om_deleted && !self.can_moderate(&channel).await? {
            return Ok(Outcome::NotFound);
        }
        let membership = self
            .thread_memberships(&[thread.id])
            .await?
            .remove(&thread.id);
        let participants = self.thread_participants(&[thread.id]).await?;
        let opts = ThreadOpts {
            preview: true,
            original_message: true,
            channel: false,
            membership: membership.as_ref(),
            participants: participants.get(&thread.id),
        };
        Ok(Outcome::Done(
            json!({ "thread": self.thread_json(&thread, opts).await? }),
        ))
    }

    /// PUT /chat/api/channels/:channel_id/threads/:thread_id:
    /// Chat::UpdateThread, the title only (cleared without one), by staff
    /// or the original message's author; the preview published.
    pub async fn update_thread(
        &mut self,
        bus: &pg_bus::Bus,
        thread_id: Option<i64>,
        title: Option<&str>,
    ) -> Result<Outcome, AppError> {
        let mut errors = Vec::new();
        if thread_id.is_none() {
            errors.push("Thread can't be blank".to_string());
        }
        if title.is_some_and(|t| t.chars().count() > MAX_TITLE_LENGTH) {
            errors.push(format!(
                "Title is too long (maximum is {MAX_TITLE_LENGTH} characters)"
            ));
        }
        if !errors.is_empty() {
            return Ok(Outcome::Invalid(errors));
        }
        if self.guardian.is_silenced() {
            return Ok(Outcome::Forbidden);
        }
        let Some(thread) = self.find_thread(thread_id.unwrap_or(0), None).await? else {
            return Ok(Outcome::NotFound);
        };
        let channel = self.thread_channel(&thread).await?;
        if !self.can_preview(&channel).await? {
            return Ok(Outcome::Forbidden);
        }
        // can_edit_thread?
        let user_id = i64::from(self.guardian.user_id().unwrap_or(0));
        if !(self.guardian.is_staff() || thread.original_message_user_id == user_id) {
            return Ok(Outcome::Forbidden);
        }
        if !channel.threading_enabled {
            return Ok(Outcome::NotFound);
        }
        if thread.title.as_deref() != title {
            sqlx::query("UPDATE chat_threads SET title = $2, updated_at = $3 WHERE id = $1")
                .bind(thread.id)
                .bind(title)
                .bind(crate::clock::now_naive())
                .execute(&mut *self.conn)
                .await?;
        }
        super::publisher::publish_thread_metadata(self, bus, &channel, thread.id).await?;
        Ok(Outcome::Done(json!({"success": "OK"})))
    }

    /// POST /chat/api/channels/:channel_id/threads: Chat::CreateThread, the
    /// message's thread (made with the title when it has none).
    pub async fn create_thread(
        &mut self,
        bus: &pg_bus::Bus,
        channel_id: Option<i64>,
        original_message_id: Option<i64>,
        title: Option<&str>,
    ) -> Result<Outcome, AppError> {
        let mut errors = Vec::new();
        if original_message_id.is_none() {
            errors.push("Original message can't be blank".to_string());
        }
        if channel_id.is_none() {
            errors.push("Channel can't be blank".to_string());
        }
        if title.is_some_and(|t| t.chars().count() > MAX_TITLE_LENGTH) {
            errors.push(format!(
                "Title is too long (maximum is {MAX_TITLE_LENGTH} characters)"
            ));
        }
        if !errors.is_empty() {
            return Ok(Outcome::Invalid(errors));
        }
        if self.guardian.is_silenced() {
            return Ok(Outcome::Forbidden);
        }
        let Some(channel) = self.live_channel_by_id(channel_id.unwrap_or(0)).await? else {
            return Ok(Outcome::NotFound);
        };
        if !self.can_preview(&channel).await? {
            return Ok(Outcome::Forbidden);
        }
        // policy :can_create_thread_in_channel
        let allowed = if channel.chatable_type == "DirectMessage" {
            let staff = self.guardian.is_staff();
            let status_ok = channel.status == 0 || (staff && channel.status == 2);
            let dm = self.direct_message(channel.chatable_id).await?;
            self.dm_creation_refusal(&dm, status_ok).await?.is_none()
        } else {
            self.can_join(&channel, None).await?
                && (channel.status == 0 || (self.guardian.is_staff() && channel.status == 2))
        };
        if !allowed {
            return Ok(Outcome::Forbidden);
        }
        if !channel.threading_enabled {
            return Ok(Outcome::NotFound);
        }
        let original: Option<(i64, Option<i32>, Option<i64>)> = sqlx::query_as(
            "SELECT id, user_id, thread_id FROM chat_messages \
             WHERE id = $1 AND chat_channel_id = $2 AND deleted_at IS NULL",
        )
        .bind(original_message_id)
        .bind(channel.id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some((om_id, om_user, om_thread)) = original else {
            return Ok(Outcome::Unprocessable(None));
        };
        let now = crate::clock::now_naive();
        let thread_id = match om_thread {
            Some(id) => id,
            None => {
                let om_user = om_user.ok_or(Unsupported("threads of deleted users' messages"))?;
                let id: i64 = sqlx::query_scalar(
                    "INSERT INTO chat_threads (channel_id, original_message_id, original_message_user_id, \
                       status, title, replies_count, force, last_message_id, created_at, updated_at) \
                     VALUES ($1, $2, $3, 0, $4, 0, FALSE, $2, $5, $5) RETURNING id",
                )
                .bind(channel.id)
                .bind(om_id)
                .bind(i64::from(om_user))
                .bind(title)
                .bind(now)
                .fetch_one(&mut *self.conn)
                .await?;
                // associate_thread_to_message
                sqlx::query(
                    "UPDATE chat_messages SET thread_id = $2, updated_at = $3 WHERE id = $1",
                )
                .bind(om_id)
                .bind(id)
                .bind(now)
                .execute(&mut *self.conn)
                .await?;
                id
            }
        };
        super::publisher::publish_thread_created(self, bus, &channel, om_id, thread_id).await?;
        let thread = self
            .find_thread(thread_id, None)
            .await?
            .ok_or(Unsupported("a thread gone while creating it"))?;
        let opts = ThreadOpts {
            original_message: true,
            ..ThreadOpts::default()
        };
        Ok(Outcome::Done(self.thread_json(&thread, opts).await?))
    }

    /// PUT /chat/api/channels/:channel_id/threads/:thread_id/read:
    /// Chat::UpdateUserThreadLastRead, up to the message (the thread's
    /// last without one), never back.
    pub async fn mark_thread_read(
        &mut self,
        bus: &pg_bus::Bus,
        channel_id: i64,
        thread_id: i64,
        message_id: Option<i64>,
    ) -> Result<Outcome, AppError> {
        let Some(thread) = self.find_thread(thread_id, Some(channel_id)).await? else {
            return Ok(Outcome::NotFound);
        };
        let channel = self.thread_channel(&thread).await?;
        if !self.can_join(&channel, None).await? {
            return Ok(Outcome::Forbidden);
        }
        let user_id = i64::from(self.guardian.user_id().unwrap_or(0));
        let Some(membership) = self.thread_membership(thread.id, user_id).await? else {
            return Ok(Outcome::Unprocessable(None));
        };
        let message: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM chat_messages WHERE id = $1 AND thread_id = $2 AND chat_channel_id = $3",
        )
        .bind(message_id.or(thread.last_message_id))
        .bind(thread.id)
        .bind(channel.id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some(message_id) = message else {
            return Ok(Outcome::NotFound);
        };
        // policy :ensure_valid_message
        if membership
            .last_read_message_id
            .is_some_and(|last| message_id < last)
        {
            return Ok(Outcome::Unprocessable(None));
        }
        // mark_associated_mentions_as_read: with a message id, the
        // channel's mentions up to it.
        sqlx::query(
            "UPDATE notifications SET read = TRUE WHERE id IN ( \
               SELECT notifications.id FROM notifications \
               JOIN chat_mention_notifications ON chat_mention_notifications.notification_id = notifications.id \
               JOIN chat_mentions ON chat_mentions.id = chat_mention_notifications.chat_mention_id \
               JOIN chat_messages ON chat_mentions.chat_message_id = chat_messages.id \
               WHERE notifications.notification_type = $1 AND notifications.user_id = $2 AND NOT notifications.read \
                 AND chat_messages.chat_channel_id = $3 AND chat_messages.id <= $4)",
        )
        .bind(CHAT_MENTION)
        .bind(user_id as i32)
        .bind(channel.id)
        .bind(message_id)
        .execute(&mut *self.conn)
        .await?;
        // mark_thread_read
        if membership.last_read_message_id != Some(message_id) {
            sqlx::query(
                "UPDATE user_chat_thread_memberships SET last_read_message_id = $3, updated_at = $4 \
                 WHERE thread_id = $1 AND user_id = $2",
            )
            .bind(thread.id)
            .bind(user_id)
            .bind(message_id)
            .bind(crate::clock::now_naive())
            .execute(&mut *self.conn)
            .await?;
        }
        super::publisher::publish_user_tracking_state(self, bus, &channel, message_id).await?;
        Ok(Outcome::Done(json!({"success": "OK"})))
    }

    /// PUT .../threads/:thread_id/notifications-settings/me:
    /// Chat::UpdateThreadNotificationSettings, joining the thread (read up
    /// to its last message) first when not a member.
    pub async fn set_thread_notification_level(
        &mut self,
        bus: &pg_bus::Bus,
        channel_id: i64,
        thread_id: i64,
        level: Option<i64>,
    ) -> Result<Outcome, AppError> {
        let Some(level) = level else {
            return Ok(Outcome::Invalid(vec![
                "Notification level can't be blank".into(),
                "Notification level is not included in the list".into(),
            ]));
        };
        if !(0..NOTIFICATION_LEVELS.len() as i64).contains(&level) {
            return Ok(Outcome::Invalid(vec![
                "Notification level is not included in the list".into(),
            ]));
        }
        let Some(thread) = self.find_thread(thread_id, Some(channel_id)).await? else {
            return Ok(Outcome::NotFound);
        };
        let channel = self.thread_channel(&thread).await?;
        if !self.can_preview(&channel).await? {
            return Ok(Outcome::Forbidden);
        }
        if !channel.threading_enabled {
            return Ok(Outcome::NotFound);
        }
        let user_id = i64::from(self.guardian.user_id().unwrap_or(0));
        let now = crate::clock::now_naive();
        if self.thread_membership(thread.id, user_id).await?.is_none() {
            self.add_thread_member(bus, thread.id, user_id, TRACKING, now)
                .await?;
            sqlx::query(
                "UPDATE user_chat_thread_memberships SET last_read_message_id = $3, updated_at = $4 \
                 WHERE thread_id = $1 AND user_id = $2",
            )
            .bind(thread.id)
            .bind(user_id)
            .bind(thread.last_message_id)
            .bind(now)
            .execute(&mut *self.conn)
            .await?;
        }
        sqlx::query(
            "UPDATE user_chat_thread_memberships SET notification_level = $3, updated_at = $4 \
             WHERE thread_id = $1 AND user_id = $2 AND notification_level <> $3",
        )
        .bind(thread.id)
        .bind(user_id)
        .bind(level as i32)
        .bind(now)
        .execute(&mut *self.conn)
        .await?;
        let membership = self
            .thread_membership(thread.id, user_id)
            .await?
            .ok_or(Unsupported("a thread membership gone while updating it"))?;
        Ok(Outcome::Done(
            json!({ "membership": membership_json(&membership) }),
        ))
    }

    /// POST .../threads/:thread_id/mark-thread-title-prompt-seen/me:
    /// Chat::MarkThreadTitlePromptSeen, joining the thread (normal) first
    /// when not a member.
    pub async fn mark_thread_title_prompt_seen(
        &mut self,
        bus: &pg_bus::Bus,
        channel_id: i64,
        thread_id: i64,
    ) -> Result<Outcome, AppError> {
        let Some(thread) = self.find_thread(thread_id, Some(channel_id)).await? else {
            return Ok(Outcome::NotFound);
        };
        let channel = self.thread_channel(&thread).await?;
        if !channel.threading_enabled {
            return Ok(Outcome::NotFound);
        }
        if !self.can_preview(&channel).await? {
            return Ok(Outcome::Forbidden);
        }
        let user_id = i64::from(self.guardian.user_id().unwrap_or(0));
        let now = crate::clock::now_naive();
        self.add_thread_member(bus, thread.id, user_id, NORMAL, now)
            .await?;
        sqlx::query(
            "UPDATE user_chat_thread_memberships SET thread_title_prompt_seen = TRUE, updated_at = $3 \
             WHERE thread_id = $1 AND user_id = $2 AND NOT thread_title_prompt_seen",
        )
        .bind(thread.id)
        .bind(user_id)
        .bind(now)
        .execute(&mut *self.conn)
        .await?;
        let membership = self
            .thread_membership(thread.id, user_id)
            .await?
            .ok_or(Unsupported("a thread membership gone while updating it"))?;
        Ok(Outcome::Done(
            json!({ "membership": membership_json(&membership) }),
        ))
    }
}

/// The job turning threading on enqueues.
pub const MARK_ALL_CHANNEL_THREADS_READ_JOB: &str = "Jobs::Chat::MarkAllChannelThreadsRead";

/// Jobs::Chat::MarkAllChannelThreadsRead: every membership of the live
/// channel's threads read up to its thread's last message
/// (`Chat::Channel#mark_all_threads_as_read`), while threading is on.
pub async fn mark_all_channel_threads_read(
    pool: &sqlx::PgPool,
    args: &Value,
) -> Result<(), AppError> {
    let Some(channel_id) = args["channel_id"].as_i64() else {
        return Ok(());
    };
    sqlx::query(
        "UPDATE user_chat_thread_memberships SET last_read_message_id = chat_threads.last_message_id \
         FROM chat_threads JOIN chat_channels ON chat_channels.id = chat_threads.channel_id \
         WHERE user_chat_thread_memberships.thread_id = chat_threads.id \
           AND chat_threads.channel_id = $1 \
           AND chat_channels.deleted_at IS NULL AND chat_channels.threading_enabled \
           AND (user_chat_thread_memberships.last_read_message_id < chat_threads.last_message_id \
             OR user_chat_thread_memberships.last_read_message_id IS NULL)",
    )
    .bind(channel_id)
    .execute(pool)
    .await?;
    Ok(())
}
