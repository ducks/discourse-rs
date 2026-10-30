//! The member's own membership writes: joining (POST memberships/me),
//! unfollowing (DELETE memberships/me/follows), leaving (DELETE
//! memberships/me), starring (PUT memberships/me) and marking read (PUT
//! read, one channel or all), with Chat::UserChannelMembershipSerializer
//! and the user tracking state they publish.

use serde_json::{Map, Value, json};

use super::channels::{ChannelRow, Context, MembershipRow};
use crate::{AppError, Unsupported};

/// `Notification.types[:chat_mention]`
const CHAT_MENTION: i32 = 29;

/// The rows Chat::UserExtension's associations take with a destroyed user
/// (`dependent: :destroy`, no callbacks of their own): channel and thread
/// memberships and reactions. The user's messages stay unless their posts
/// go too (`DELETE_USER_MESSAGES_JOB`).
pub async fn user_destroying(
    conn: &mut sqlx::PgConnection,
    user_id: i32,
) -> Result<(), sqlx::Error> {
    for table in [
        "user_chat_channel_memberships",
        "user_chat_thread_memberships",
        "chat_message_reactions",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE user_id = $1"))
            .bind(user_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// `Jobs::Chat::DeleteUserMessages`, which chat's
/// user_destroyer_on_content_deletion_callback enqueues.
pub const DELETE_USER_MESSAGES_JOB: &str = "Jobs::Chat::DeleteUserMessages";

pub enum Outcome {
    Done(Value),
    NotFound,
    Forbidden,
    /// A service contract's errors (400).
    Invalid(Vec<String>),
    /// Discourse::InvalidParameters for this parameter (400).
    InvalidParameter(&'static str),
    /// A failed service without its own answer (422, `failed_json`).
    Unprocessable,
}

const MEMBERSHIP_COLUMNS: &str = "chat_channel_id, following, muted, notification_level, \
     last_read_message_id, last_viewed_at, last_viewed_pins_at, starred";

impl Context<'_> {
    /// `Chat::Channel.find_by(id:)`, live channels.
    async fn find_channel(&mut self, id: i64) -> Result<Option<ChannelRow>, AppError> {
        let sql = format!(
            "SELECT {} FROM chat_channels WHERE id = $1 AND deleted_at IS NULL",
            super::channels::CHANNEL_COLUMNS
        );
        let channel: Option<ChannelRow> = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(&mut *self.conn)
            .await?;
        if channel
            .as_ref()
            .is_some_and(|c| c.chatable_type != "Category" && c.chatable_type != "DirectMessage")
        {
            return Err(Unsupported("chat channels of other chatables").into());
        }
        Ok(channel)
    }

    /// The viewer's membership of a channel (`find_for_user`).
    pub(crate) async fn own_membership(
        &mut self,
        channel_id: i64,
    ) -> Result<Option<MembershipRow>, AppError> {
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(None);
        };
        let sql = format!(
            "SELECT {MEMBERSHIP_COLUMNS} FROM user_chat_channel_memberships \
             WHERE user_id = $1 AND chat_channel_id = $2"
        );
        Ok(sqlx::query_as(&sql)
            .bind(user_id)
            .bind(channel_id)
            .fetch_optional(&mut *self.conn)
            .await?)
    }

    /// Chat::UserChannelMembershipSerializer: the base membership and the
    /// member, through Chat::BasicUserSerializer with the viewer's scope.
    pub(crate) async fn membership_json(
        &mut self,
        channel: &ChannelRow,
        m: &MembershipRow,
    ) -> Result<Value, AppError> {
        let pinned = self.settings.get("chat_pinned_messages")?.truthy();
        let mut out = match self.membership(channel, m, pinned).await? {
            Value::Object(o) => o,
            _ => Map::new(),
        };
        let user_id = self.guardian.user_id().unwrap_or(0);
        out.insert("user".into(), self.scoped_basic_user(user_id).await?);
        Ok(Value::Object(out))
    }

    /// Chat::BasicUserSerializer for a user, with the viewer's scope.
    async fn scoped_basic_user(&mut self, user_id: i32) -> Result<Value, AppError> {
        let row: Option<(String, Option<String>, Option<i32>, bool)> = sqlx::query_as(
            "SELECT users.username, users.name, users.uploaded_avatar_id, COALESCE(uo.chat_enabled, TRUE) \
             FROM users LEFT JOIN user_options uo ON uo.user_id = users.id WHERE users.id = $1",
        )
        .bind(user_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some((username, name, avatar, chat_enabled)) = row else {
            return Ok(Value::Null);
        };
        if self.settings.get("enable_user_status")?.truthy() {
            let status: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM user_statuses WHERE user_id = $1 AND (ends_at IS NULL OR ends_at > now()))",
            )
            .bind(user_id)
            .fetch_one(&mut *self.conn)
            .await?;
            if status {
                return Err(Unsupported("user status").into());
            }
        }
        let can_chat = super::enabled(self.settings)?
            && super::can_chat(&mut *self.conn, self.settings, self.guardian).await?;
        let logo = crate::admin_users::logo_small_url(&mut *self.conn, self.settings).await?;
        let urls = crate::url::Urls {
            config: self.config,
            settings: self.settings,
        };
        let mut out = Map::new();
        out.insert("id".into(), json!(user_id));
        out.insert("username".into(), json!(username));
        if self.settings.get("enable_names")?.truthy() {
            out.insert("name".into(), json!(name));
        }
        out.insert(
            "avatar_template".into(),
            json!(crate::avatar::avatar_template(
                &urls,
                user_id,
                &username,
                avatar,
                logo.as_deref()
            )?),
        );
        out.insert("can_chat".into(), json!(can_chat));
        out.insert("has_chat_enabled".into(), json!(can_chat && chat_enabled));
        Ok(Value::Object(out))
    }

    /// `ChannelMembershipManager#recalculate_user_count`: stale, recounted by
    /// the job in 3 seconds.
    pub(crate) async fn mark_user_count_stale(&mut self, channel_id: i64) -> Result<(), AppError> {
        let marked = sqlx::query(
            "UPDATE chat_channels SET user_count_stale = TRUE, updated_at = $2 WHERE id = $1 AND NOT user_count_stale",
        )
        .bind(channel_id)
        .bind(crate::clock::now_naive())
        .execute(&mut *self.conn)
        .await?;
        if marked.rows_affected() == 1 {
            crate::jobs::enqueue_in(
                &mut *self.conn,
                3,
                super::auto_join::UPDATE_USER_COUNT_JOB,
                json!({ "chat_channel_id": channel_id }),
            )
            .await?;
        }
        Ok(())
    }

    /// POST /chat/api/channels/:id/memberships/me: `channel.add(user)`
    /// after ensure_can_join_chat_channel!.
    pub async fn join(&mut self, channel_id: i64) -> Result<Outcome, AppError> {
        let channel = match self.find_joinable(&channel_id.to_string()).await? {
            super::channels::Found::Channel(c) => c,
            super::channels::Found::NotFound => return Ok(Outcome::NotFound),
            super::channels::Found::Forbidden => return Ok(Outcome::Forbidden),
        };
        let user_id = self.guardian.user_id().unwrap_or(0);
        let now = crate::clock::now_naive();
        match self.own_membership(channel.id).await? {
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
                self.mark_user_count_stale(channel.id).await?;
            }
            Some(m) if !m.following => {
                sqlx::query(
                    "UPDATE user_chat_channel_memberships SET following = TRUE, updated_at = $3 \
                     WHERE user_id = $1 AND chat_channel_id = $2",
                )
                .bind(user_id)
                .bind(channel.id)
                .bind(now)
                .execute(&mut *self.conn)
                .await?;
                self.mark_user_count_stale(channel.id).await?;
            }
            Some(_) => {}
        }
        let m = self
            .own_membership(channel.id)
            .await?
            .ok_or(Unsupported("a membership gone while joining"))?;
        Ok(Outcome::Done(
            json!({ "membership": self.membership_json(&channel, &m).await? }),
        ))
    }

    /// `ChannelMembershipManager#unfollow`: following and starred off.
    async fn unfollow_membership(
        &mut self,
        channel: &ChannelRow,
    ) -> Result<Option<MembershipRow>, AppError> {
        let Some(m) = self.own_membership(channel.id).await? else {
            return Ok(None);
        };
        if m.following {
            sqlx::query(
                "UPDATE user_chat_channel_memberships SET following = FALSE, starred = FALSE, updated_at = $3 \
                 WHERE user_id = $1 AND chat_channel_id = $2",
            )
            .bind(self.guardian.user_id())
            .bind(channel.id)
            .bind(crate::clock::now_naive())
            .execute(&mut *self.conn)
            .await?;
            self.mark_user_count_stale(channel.id).await?;
        }
        self.own_membership(channel.id).await
    }

    /// DELETE /chat/api/channels/:id/memberships/me/follows:
    /// Chat::UnfollowChannel, the membership (or null) back.
    pub async fn unfollow(&mut self, channel_id: i64) -> Result<Outcome, AppError> {
        let Some(channel) = self.find_channel(channel_id).await? else {
            return Ok(Outcome::NotFound);
        };
        let membership = match self.unfollow_membership(&channel).await? {
            Some(m) => self.membership_json(&channel, &m).await?,
            None => Value::Null,
        };
        Ok(Outcome::Done(json!({ "membership": membership })))
    }

    /// DELETE /chat/api/channels/:id/memberships/me: Chat::LeaveChannel,
    /// an unfollow for a category channel, then the user count recounted.
    pub async fn leave(&mut self, channel_id: i64) -> Result<Outcome, AppError> {
        let Some(channel) = self.find_channel(channel_id).await? else {
            return Ok(Outcome::NotFound);
        };
        // DirectMessageChannel#leave: a group's member leaves it outright
        // (membership and participant gone); a one-to-one is unfollowed as
        // a category channel is.
        if channel.chatable_type == "DirectMessage"
            && self.direct_message(channel.chatable_id).await?.group
        {
            let user_id = self.guardian.user_id().unwrap_or(0);
            self.leave_group_dm(&channel, user_id).await?;
        } else {
            self.unfollow_membership(&channel).await?;
        }
        let count = self.members_count(&channel).await?;
        sqlx::query(
            "UPDATE chat_channels SET user_count = $2, user_count_stale = FALSE, updated_at = $3 WHERE id = $1",
        )
        .bind(channel.id)
        .bind(count as i32)
        .bind(crate::clock::now_naive())
        .execute(&mut *self.conn)
        .await?;
        Ok(Outcome::Done(json!({ "success": "OK" })))
    }

    /// `ChannelMembershipsQuery.count`: followers of a category channel,
    /// every member of a DM.
    async fn members_count(&mut self, channel: &ChannelRow) -> Result<i64, AppError> {
        Ok(sqlx::query_scalar(
            "SELECT COUNT(*) FROM user_chat_channel_memberships m JOIN users ON users.id = m.user_id \
             WHERE m.chat_channel_id = $1 AND (m.following OR $3 = 'DirectMessage') AND users.id > 0 \
               AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = users.id) \
               AND users.active AND NOT users.staged \
               AND (users.suspended_till IS NULL OR users.suspended_till <= $2) \
               AND (users.silenced_till IS NULL OR users.silenced_till <= $2)",
        )
        .bind(channel.id)
        .bind(crate::clock::now_naive())
        .bind(&channel.chatable_type)
        .fetch_one(&mut *self.conn)
        .await?)
    }

    /// PUT /chat/api/channels/:id/memberships/me:
    /// Chat::UpdateUserChannelMembership, `starred` cast as a boolean.
    pub async fn star(
        &mut self,
        channel_id: i64,
        starred: Option<bool>,
    ) -> Result<Outcome, AppError> {
        let Some(starred) = starred else {
            return Ok(Outcome::Invalid(vec![
                "Starred is not included in the list".into(),
            ]));
        };
        let Some(channel) = self.find_channel(channel_id).await? else {
            return Ok(Outcome::NotFound);
        };
        if self.own_membership(channel.id).await?.is_none() {
            return Ok(Outcome::NotFound);
        }
        // can_preview_chat_channel?
        if !self.can_preview(&channel).await? {
            return Ok(Outcome::Forbidden);
        }
        sqlx::query(
            "UPDATE user_chat_channel_memberships SET starred = $3, updated_at = $4 \
             WHERE user_id = $1 AND chat_channel_id = $2",
        )
        .bind(self.guardian.user_id())
        .bind(channel.id)
        .bind(starred)
        .bind(crate::clock::now_naive())
        .execute(&mut *self.conn)
        .await?;
        let m = self
            .own_membership(channel.id)
            .await?
            .ok_or(Unsupported("a membership gone while starring"))?;
        Ok(Outcome::Done(
            json!({ "membership": self.membership_json(&channel, &m).await? }),
        ))
    }

    /// PUT /chat/api/channels/:id/read: Chat::UpdateUserChannelLastRead.
    pub async fn mark_read(
        &mut self,
        bus: &pg_bus::Bus,
        channel_id: i64,
        message_id: Option<i64>,
    ) -> Result<Outcome, AppError> {
        let Some(message_id) = message_id else {
            return Ok(Outcome::Invalid(vec!["Message can't be blank".into()]));
        };
        let Some(channel) = self.find_channel(channel_id).await? else {
            return Ok(Outcome::NotFound);
        };
        let Some(m) = self
            .own_membership(channel.id)
            .await?
            .filter(|m| m.following)
        else {
            return Ok(Outcome::NotFound);
        };
        if !self.can_join(&channel, None).await? {
            return Ok(Outcome::Forbidden);
        }
        let message: Option<Option<i64>> = sqlx::query_scalar(
            "SELECT thread_id FROM chat_messages WHERE chat_channel_id = $1 AND id = $2",
        )
        .bind(channel.id)
        .bind(message_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some(_) = message else {
            return Ok(Outcome::NotFound);
        };
        // ensure_message_id_recency
        if m.last_read_message_id.is_some_and(|last| message_id < last) {
            return Ok(Outcome::InvalidParameter("message_id"));
        }
        let user_id = self.guardian.user_id().unwrap_or(0);
        sqlx::query(
            "UPDATE user_chat_channel_memberships SET last_read_message_id = $3, last_viewed_at = $4, updated_at = $4 \
             WHERE user_id = $1 AND chat_channel_id = $2",
        )
        .bind(user_id)
        .bind(channel.id)
        .bind(message_id)
        .bind(crate::clock::now_naive())
        .execute(&mut *self.conn)
        .await?;
        self.mark_mentions_read(&[channel.id], Some(message_id))
            .await?;
        super::publisher::publish_user_tracking_state(self, bus, &channel, message_id).await?;
        Ok(Outcome::Done(json!({ "success": "OK" })))
    }

    /// `Chat::Action::MarkMentionsRead`: the member's unread chat mention
    /// notifications in the channels, up to a message when given.
    async fn mark_mentions_read(
        &mut self,
        channel_ids: &[i64],
        message_id: Option<i64>,
    ) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE notifications SET read = TRUE WHERE id IN ( \
               SELECT notifications.id FROM notifications \
               JOIN chat_mention_notifications ON chat_mention_notifications.notification_id = notifications.id \
               JOIN chat_mentions ON chat_mentions.id = chat_mention_notifications.chat_mention_id \
               JOIN chat_messages ON chat_mentions.chat_message_id = chat_messages.id \
               WHERE notifications.notification_type = $1 AND notifications.user_id = $2 AND NOT notifications.read \
                 AND chat_messages.chat_channel_id = ANY($3) \
                 AND ($4::bigint IS NULL OR chat_messages.id <= $4))",
        )
        .bind(CHAT_MENTION)
        .bind(self.guardian.user_id())
        .bind(channel_ids)
        .bind(message_id)
        .execute(&mut *self.conn)
        .await?;
        Ok(())
    }

    /// PUT /chat/api/channels/read: Chat::MarkAllUserChannelsRead.
    pub async fn mark_all_read(&mut self, bus: &pg_bus::Bus) -> Result<Outcome, AppError> {
        let user_id = self.guardian.user_id().unwrap_or(0);
        let updated: Vec<(i64, i64, Option<i64>)> = sqlx::query_as(
            "UPDATE user_chat_channel_memberships \
             SET last_read_message_id = chat_channels.last_message_id \
             FROM chat_channels \
             WHERE user_chat_channel_memberships.chat_channel_id = chat_channels.id \
               AND chat_channels.last_message_id > COALESCE(user_chat_channel_memberships.last_read_message_id, 0) \
               AND user_chat_channel_memberships.user_id = $1 \
               AND user_chat_channel_memberships.following \
             RETURNING user_chat_channel_memberships.id::bigint, user_chat_channel_memberships.chat_channel_id, \
                       user_chat_channel_memberships.last_read_message_id",
        )
        .bind(user_id)
        .fetch_all(&mut *self.conn)
        .await?;
        let channel_ids: Vec<i64> = updated.iter().map(|(_, c, _)| *c).collect();
        if !updated.is_empty() {
            self.mark_mentions_read(&channel_ids, None).await?;
        }
        // publish_bulk_user_tracking_state!
        let tracking = self.channel_tracking(&channel_ids).await?;
        let mut data = Map::new();
        for (membership_id, channel_id, last_read) in &updated {
            let mut entry = json!({
                "last_read_message_id": last_read,
                "membership_id": membership_id,
            });
            if let (Some(e), Some(Value::Object(t))) =
                (entry.as_object_mut(), tracking.get(&channel_id.to_string()))
            {
                for (k, v) in t {
                    e.insert(k.clone(), v.clone());
                }
            }
            data.insert(channel_id.to_string(), entry);
        }
        bus.publish(
            &mut *self.conn,
            &format!("/chat/bulk-user-tracking-state/{user_id}"),
            &Value::Object(data),
            Some(&[crate::bus::user_tag(user_id)]),
        )
        .await?;
        let memberships: Vec<Value> = updated
            .iter()
            .map(|(membership_id, channel_id, last_read)| {
                json!({
                    "membership_id": membership_id,
                    "channel_id": channel_id,
                    "last_read_message_id": last_read,
                })
            })
            .collect();
        Ok(Outcome::Done(
            json!({ "success": "OK", "updated_memberships": memberships }),
        ))
    }
}

impl Context<'_> {
    /// POST /chat/api/channels/:id/drafts and
    /// .../threads/:thread_id/drafts: Chat::UpsertDraft. The contract
    /// types `data` as a string, so the client's draft object is stored as
    /// Ruby writes the parameter hash (`{message: "..."}`), as Rails
    /// stores it; a blank one removes the draft. A thread not in the
    /// channel fails the service: 404 from the thread's route
    /// (`thread_route`), 422 from the channel's.
    pub async fn upsert_draft(
        &mut self,
        channel_id: i64,
        data: Option<&Value>,
        thread_id: Option<&str>,
        thread_route: bool,
    ) -> Result<Outcome, AppError> {
        let data = match data {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(v) => crate::ruby::inspect_value(v),
        };
        let max = self.settings.get("max_chat_draft_length")?.to_i();
        if data.chars().count() as i64 > max {
            return Ok(Outcome::Invalid(vec![format!(
                "Data is too long (maximum is {max} characters)"
            )]));
        }
        let Some(channel) = self.find_channel(channel_id).await? else {
            return Ok(Outcome::NotFound);
        };
        // policy :can_upsert_draft, failing as the service fails (422).
        if !self.can_join(&channel, None).await? {
            return Ok(Outcome::Unprocessable);
        }
        // check_thread_exists
        let thread_id = thread_id
            .filter(|t| !t.trim().is_empty())
            .map(crate::ruby::to_i);
        if let Some(thread_id) = thread_id {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM chat_threads WHERE id = $1 AND channel_id = $2)",
            )
            .bind(thread_id)
            .bind(channel.id)
            .fetch_one(&mut *self.conn)
            .await?;
            if !exists {
                return Ok(if thread_route {
                    Outcome::NotFound
                } else {
                    Outcome::Unprocessable
                });
            }
        }
        let user_id = self.guardian.user_id().unwrap_or(0);
        let now = crate::clock::now_naive();
        if crate::ruby::is_blank(&data) {
            sqlx::query(
                "DELETE FROM chat_drafts WHERE user_id = $1 AND chat_channel_id = $2 \
                   AND thread_id IS NOT DISTINCT FROM $3",
            )
            .bind(user_id)
            .bind(channel.id)
            .bind(thread_id)
            .execute(&mut *self.conn)
            .await?;
        } else {
            let updated = sqlx::query(
                "UPDATE chat_drafts SET data = $3, updated_at = $4 \
                 WHERE user_id = $1 AND chat_channel_id = $2 AND thread_id IS NOT DISTINCT FROM $5",
            )
            .bind(user_id)
            .bind(channel.id)
            .bind(&data)
            .bind(now)
            .bind(thread_id)
            .execute(&mut *self.conn)
            .await?;
            if updated.rows_affected() == 0 {
                sqlx::query(
                    "INSERT INTO chat_drafts (user_id, chat_channel_id, data, thread_id, created_at, updated_at) \
                     VALUES ($1, $2, $3, $4, $5, $5)",
                )
                .bind(user_id)
                .bind(channel.id)
                .bind(&data)
                .bind(thread_id)
                .bind(now)
                .execute(&mut *self.conn)
                .await?;
            }
        }
        Ok(Outcome::Done(json!({ "success": "OK" })))
    }
}
