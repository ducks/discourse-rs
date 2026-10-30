//! What a channel's info pages change besides its name and description
//! (dm_groups.rs's UpdateChannel): the member's notification settings
//! (ChannelsCurrentUserNotificationsSettingsController),
//! `Chat::UpdateChannelStatus`, `Chat::TrashChannel` with
//! Jobs::Chat::ChannelDelete, and `Chat::InviteUsersToChannel`.

use serde_json::{Map, Value, json};

use super::channels::{CHANNEL_COLUMNS, ChannelRow, Context};
use super::dm_groups::Outcome;
use crate::{AppError, AppState, Unsupported};

/// `Jobs::Chat::ChannelDelete`
pub const CHANNEL_DELETE_JOB: &str = "Jobs::Chat::ChannelDelete";

/// `Notification.types[:chat_invitation]`
const CHAT_INVITATION: i32 = 31;

/// `UserChatChannelMembership::NOTIFICATION_LEVELS`
const NOTIFICATION_LEVELS: [&str; 3] = ["never", "mention", "always"];

/// `Chat::Channel.editable_statuses`
const EDITABLE_STATUSES: [&str; 2] = ["open", "closed"];

impl Context<'_> {
    async fn find_live_channel(&mut self, id: i64) -> Result<Option<ChannelRow>, AppError> {
        Ok(sqlx::query_as(&format!(
            "SELECT {CHANNEL_COLUMNS} FROM chat_channels WHERE id = $1 AND deleted_at IS NULL"
        ))
        .bind(id)
        .fetch_optional(&mut *self.conn)
        .await?)
    }

    /// PUT /chat/api/channels/:id/notifications-settings/me: the member's
    /// muted and notification_level (`membership.update!`), the membership
    /// back (Chat::UserChannelMembershipSerializer). An enum value the
    /// membership doesn't know raises ArgumentError in Rails: refused.
    pub async fn update_notifications_settings(
        &mut self,
        channel_id: &str,
        edits: &Map<String, Value>,
    ) -> Result<super::membership::Outcome, AppError> {
        use super::membership::Outcome as M;
        let channel = match self.find_joinable(channel_id).await? {
            super::channels::Found::Channel(c) => c,
            super::channels::Found::NotFound => return Ok(M::NotFound),
            super::channels::Found::Forbidden => return Ok(M::Forbidden),
        };
        let Some(m) = self.own_membership(channel.id).await? else {
            return Ok(M::NotFound);
        };
        let mut muted = m.muted;
        let mut level = m.notification_level;
        if let Some(v) = edits.get("muted") {
            muted = crate::ruby::cast_boolean(crate::params::scalar(v).as_deref())
                .ok_or(Unsupported("a chat membership muted as null (NOT NULL)"))?;
        }
        if let Some(v) = edits.get("notification_level") {
            let raw = crate::params::scalar(v).unwrap_or_default();
            level = match NOTIFICATION_LEVELS.iter().position(|l| *l == raw) {
                Some(i) => i as i32,
                None => match raw.parse::<i32>() {
                    Ok(n) if (0..3).contains(&n) && n.to_string() == raw => n,
                    _ => {
                        return Err(Unsupported(
                            "a chat notification level that isn't one (ArgumentError)",
                        )
                        .into());
                    }
                },
            };
        }
        if muted != m.muted || level != m.notification_level {
            sqlx::query(
                "UPDATE user_chat_channel_memberships SET muted = $3, notification_level = $4, updated_at = $5 \
                 WHERE user_id = $1 AND chat_channel_id = $2",
            )
            .bind(self.guardian.user_id())
            .bind(channel.id)
            .bind(muted)
            .bind(level)
            .bind(crate::clock::now_naive())
            .execute(&mut *self.conn)
            .await?;
        }
        let m = self
            .own_membership(channel.id)
            .await?
            .ok_or(Unsupported("a membership gone while updating it"))?;
        Ok(M::Done(
            json!({ "membership": self.membership_json(&channel, &m).await? }),
        ))
    }

    /// PUT /chat/api/channels/:id/status: `Chat::UpdateChannelStatus`.
    /// Staff open a closed channel or close an open one
    /// (can_change_channel_status?); `Channel#change_status` logs it,
    /// publishes /chat/channel-status, and the channel comes back without
    /// the actor's membership.
    pub async fn update_status(
        &mut self,
        bus: &pg_bus::Bus,
        channel_id: Option<i64>,
        status: Option<&str>,
    ) -> Result<Outcome, AppError> {
        let mut errors = Vec::new();
        if channel_id.is_none() {
            errors.push("Channel can't be blank".to_string());
        }
        if !status.is_some_and(|s| EDITABLE_STATUSES.contains(&s)) {
            errors.push("Status is not included in the list".to_string());
        }
        if !errors.is_empty() {
            return Ok(Outcome::Invalid(errors));
        }
        let status = status.unwrap_or_default();
        let Some(channel) = self.find_live_channel(channel_id.unwrap_or(0)).await? else {
            return Ok(Outcome::NotFound);
        };
        let target = super::channels::STATUSES
            .iter()
            .position(|s| *s == status)
            .unwrap_or(0) as i32;
        if !(self.can_preview(&channel).await?
            && can_change_status(self.guardian, &channel, target))
        {
            return Ok(Outcome::Forbidden);
        }
        let user_id = self.guardian.user_id().unwrap_or(0);
        self.change_status(bus, &channel, target, user_id).await?;
        let channel = self
            .find_live_channel(channel.id)
            .await?
            .ok_or(Unsupported("a chat channel gone while changing its status"))?;
        Ok(Outcome::Done(
            json!({ "channel": self.channel(&channel, None, None, None).await? }),
        ))
    }

    /// `Chat::Channel#change_status`: the status saved, the staff action
    /// log, and `Chat::Publisher.publish_channel_status`.
    pub(crate) async fn change_status(
        &mut self,
        bus: &pg_bus::Bus,
        channel: &ChannelRow,
        target: i32,
        acting_user_id: i32,
    ) -> Result<(), AppError> {
        sqlx::query("UPDATE chat_channels SET status = $2, updated_at = $3 WHERE id = $1")
            .bind(channel.id)
            .bind(target)
            .bind(crate::clock::now_naive())
            .execute(&mut *self.conn)
            .await?;
        let status_name = |s: i32| super::channels::STATUSES[s as usize].to_string();
        crate::staff_action_logs::log_custom(
            &mut *self.conn,
            acting_user_id,
            "chat_channel_status_change",
            &[
                ("chat_channel_id", channel.id.to_string()),
                (
                    "chat_channel_name",
                    channel.name.clone().unwrap_or_default(),
                ),
                (
                    "category_id",
                    if channel.chatable_type == "Category" {
                        channel.chatable_id.to_string()
                    } else {
                        String::new()
                    },
                ),
            ],
            Some(&status_name(channel.status)),
            Some(&status_name(target)),
        )
        .await?;
        let audience = super::publisher::audience(
            &mut *self.conn,
            self.settings,
            &channel.chatable_type,
            channel.chatable_id,
        )
        .await?;
        bus.publish(
            &mut *self.conn,
            "/chat/channel-status",
            &json!({ "chat_channel_id": channel.id, "status": status_name(target) }),
            Some(&audience),
        )
        .await?;
        Ok(())
    }

    /// DELETE /chat/api/channels/:id: `Chat::TrashChannel`. Staff only;
    /// the slug is moved aside (`<time>-<slug>-deleted`, so a new
    /// channel may take it, its hashtags remapped), the channel trashed
    /// and logged, and Jobs::Chat::ChannelDelete clears what it held.
    pub async fn trash_channel(&mut self, channel_id: Option<i64>) -> Result<Outcome, AppError> {
        let Some(channel_id) = channel_id else {
            return Ok(Outcome::Invalid(vec!["Channel can't be blank".into()]));
        };
        let Some(channel) = self.find_live_channel(channel_id).await? else {
            return Ok(Outcome::NotFound);
        };
        if !(self.can_preview(&channel).await? && self.guardian.is_staff()) {
            return Ok(Outcome::Forbidden);
        }
        let user_id = self.guardian.user_id().unwrap_or(0);
        let now = crate::clock::now_naive();
        // prevents_slug_collision: truncated to max_topic_title_length,
        // then ensure_slug_ok's encoded slug.
        let max = self.settings.get("max_topic_title_length")?.to_i().max(0) as usize;
        let moved: String = format!(
            "{}-{}-deleted",
            now.format("%Y%m%d-%H%M"),
            channel.slug.as_deref().unwrap_or("")
        )
        .chars()
        .take(max)
        .collect();
        let slug = crate::category_writes::encoded_slug(&moved);
        if slug.is_empty() {
            return Err(Unsupported("a deleted chat channel's slug that comes out blank").into());
        }
        sqlx::query("UPDATE chat_channels SET slug = $2, updated_at = $3 WHERE id = $1")
            .bind(channel.id)
            .bind(&slug)
            .bind(now)
            .execute(&mut *self.conn)
            .await?;
        if let Some(old) = channel.slug.as_deref() {
            channel_hashtag_remap(&mut *self.conn, channel.id, old, &slug).await?;
        }
        // channel.trash!(user)
        sqlx::query(
            "UPDATE chat_channels SET deleted_at = $2, deleted_by_id = $3, updated_at = $2 WHERE id = $1",
        )
        .bind(channel.id)
        .bind(crate::clock::now_naive())
        .bind(user_id)
        .execute(&mut *self.conn)
        .await?;
        let title = self.channel_title(&channel).await?;
        crate::staff_action_logs::log_custom(
            &mut *self.conn,
            user_id,
            "chat_channel_delete",
            &[
                ("chat_channel_id", channel.id.to_string()),
                ("chat_channel_name", title),
                (
                    "category_id",
                    if channel.chatable_type == "Category" {
                        channel.chatable_id.to_string()
                    } else {
                        String::new()
                    },
                ),
            ],
            None,
            None,
        )
        .await?;
        crate::jobs::enqueue(
            &mut *self.conn,
            CHANNEL_DELETE_JOB,
            json!({ "chat_channel_id": channel.id }),
        )
        .await?;
        Ok(Outcome::Done(json!({ "success": "OK" })))
    }

    /// POST /chat/api/channels/:id/invites: `Chat::InviteUsersToChannel`.
    /// Each of the (first 50) users with chat on and not suspended who may
    /// join the channel gets a chat_invitation notification.
    pub async fn invite_users(
        &mut self,
        state: &AppState,
        channel_id: Option<i64>,
        user_ids: &[String],
        message_id: Option<i64>,
    ) -> Result<Outcome, AppError> {
        let mut errors = Vec::new();
        if user_ids.is_empty() {
            errors.push("User ids can't be blank".to_string());
        }
        if channel_id.is_none() {
            errors.push("Channel can't be blank".to_string());
        }
        if !errors.is_empty() {
            return Ok(Outcome::Invalid(errors));
        }
        let Some(channel) = self.find_live_channel(channel_id.unwrap_or(0)).await? else {
            return Ok(Outcome::NotFound);
        };
        if !self.can_join(&channel, None).await? {
            return Ok(Outcome::Forbidden);
        }
        let ids: Vec<i32> = user_ids
            .iter()
            .filter_map(|id| id.trim().parse::<i32>().ok())
            .collect();
        let users: Vec<i32> = sqlx::query_scalar(
            "SELECT users.id FROM users JOIN user_options ON user_options.user_id = users.id \
             WHERE user_options.chat_enabled AND (users.suspended_till IS NULL OR users.suspended_till <= $2) \
               AND users.id = ANY($1) ORDER BY users.id LIMIT 50",
        )
        .bind(&ids)
        .bind(crate::clock::now_naive())
        .fetch_all(&mut *self.conn)
        .await?;
        let inviter = self
            .guardian
            .user()
            .map(|u| u.username.clone())
            .unwrap_or_default();
        for user_id in users {
            let Some(user) =
                crate::session::current::SessionUser::load(&mut *self.conn, user_id).await?
            else {
                continue;
            };
            let guardian = crate::guardian::Guardian::for_user(&mut *self.conn, &user).await?;
            let mut theirs = Context {
                conn: &mut *self.conn,
                settings: self.settings,
                i18n: self.i18n,
                guardian: &guardian,
                base_path: self.base_path,
                config: self.config,
            };
            if !theirs.can_join(&channel, None).await? {
                continue;
            }
            let title = theirs.channel_title(&channel).await?;
            let mut data = Map::new();
            data.insert("message".into(), json!("chat.invitation_notification"));
            data.insert("chat_channel_id".into(), json!(channel.id));
            data.insert("chat_channel_title".into(), json!(title));
            if let Some(slug) = &channel.slug {
                data.insert("chat_channel_slug".into(), json!(slug));
            }
            data.insert("invited_by_username".into(), json!(inviter));
            if let Some(id) = message_id {
                data.insert("chat_message_id".into(), json!(id));
            }
            crate::notification_create::create(
                &mut *self.conn,
                &state.bus,
                self.settings,
                &crate::notification_create::NewNotification::new(
                    CHAT_INVITATION,
                    user_id,
                    Value::Object(data),
                )
                .high_priority(true),
            )
            .await?;
        }
        Ok(Outcome::Done(json!({ "success": "OK" })))
    }
}

/// `can_change_channel_status?`: staff, from open to closed or back (and
/// to read only or archived from where those go).
pub(crate) fn can_change_status(
    guardian: &crate::guardian::Guardian,
    channel: &ChannelRow,
    target: i32,
) -> bool {
    if channel.status == target || !guardian.is_staff() {
        return false;
    }
    let (open, read_only, closed) = (0, 1, 2);
    match target {
        2 => channel.status == open,
        0 => channel.status == closed,
        3 => channel.status == read_only,
        1 => channel.status == closed || channel.status == open,
        _ => false,
    }
}

/// `Slug.for(title, "")` by slug_generation_method, as
/// CategoryChannel#generate_auto_slug takes it.
pub(crate) fn auto_slug(
    settings: &crate::site_settings::SiteSettings,
    title: &str,
) -> Result<String, AppError> {
    Ok(
        match settings.get("slug_generation_method")?.to_s().as_str() {
            "ascii" => crate::posting::text::slug_with_default(
                title,
                &settings.get("default_locale")?.to_s(),
                "",
            )?,
            "encoded" => crate::category_writes::encoded_slug(title),
            _ => String::new(),
        },
    )
}

/// `Chat::Channel#duplicate_slug?`: another live channel has it.
pub(crate) async fn duplicate_slug(
    conn: &mut sqlx::PgConnection,
    slug: &str,
    channel_id: i64,
) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM chat_channels WHERE slug = $1 AND id IS DISTINCT FROM $2 AND deleted_at IS NULL)",
    )
    .bind(slug)
    .bind(channel_id)
    .fetch_one(conn)
    .await?)
}

/// `Chat::Channel#ensure_slug_ok` for a slug given: its encoded form, or
/// the full message of what is wrong with it.
pub(crate) async fn checked_slug(
    conn: &mut sqlx::PgConnection,
    settings: &crate::site_settings::SiteSettings,
    i18n: &crate::i18n::I18n,
    given: &str,
    channel_id: i64,
) -> Result<Result<String, String>, AppError> {
    let method = settings.get("slug_generation_method")?.to_s();
    let unescaped = if method == "encoded" {
        crate::category_writes::cgi_unescape(given)
    } else {
        given.to_string()
    };
    let slug = crate::category_writes::encoded_slug(&unescaped);
    let t = |key: &str| i18n.t(key).unwrap_or(key).to_string();
    Ok(if slug.is_empty() {
        Err("Slug is invalid".into())
    } else if method == "ascii" && !crate::category_writes::cgi_unescape(&slug).is_ascii() {
        Err(format!(
            "Slug {}",
            t("chat.category_channel.errors.slug_contains_non_ascii_chars")
        ))
    } else if duplicate_slug(conn, &slug, channel_id).await? {
        Err(format!(
            "Slug {}",
            t("chat.category_channel.errors.is_already_in_use")
        ))
    } else {
        Ok(slug)
    })
}

/// `enqueue_channel_hashtag_remap` through HashtagRemapper.enqueue: a
/// remap_hashtag job for the old slug, unless only its case changed.
pub(crate) async fn channel_hashtag_remap(
    conn: &mut sqlx::PgConnection,
    channel_id: i64,
    old: &str,
    new: &str,
) -> Result<(), AppError> {
    if old.is_empty() || old.to_lowercase() == new.to_lowercase() {
        return Ok(());
    }
    crate::jobs::enqueue(
        conn,
        "remap_hashtag",
        json!({ "remaps": [{ "type": "channel", "id": channel_id, "old_ref": old }] }),
    )
    .await?;
    Ok(())
}

/// Jobs::Chat::ChannelDelete: a trashed channel's incoming webhooks and
/// their events, drafts and memberships deleted; its messages' mentions,
/// revisions, reactions, upload references and hotlinked media deleted,
/// and the messages trashed by whoever trashed the channel.
pub async fn channel_delete(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(channel_id) = args["chat_channel_id"].as_i64().or_else(|| {
        args["chat_channel_id"]
            .as_str()
            .and_then(|s| s.parse().ok())
    }) else {
        return Ok(());
    };
    let mut tx = state.pool.begin().await?;
    let deleted_by: Option<Option<i32>> =
        sqlx::query_scalar("SELECT deleted_by_id FROM chat_channels WHERE id = $1")
            .bind(channel_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(deleted_by) = deleted_by else {
        tracing::warn!("Chat channel {channel_id} could not be found, aborting delete job.");
        return Ok(());
    };
    for sql in [
        "DELETE FROM chat_webhook_events WHERE incoming_chat_webhook_id IN \
           (SELECT id FROM incoming_chat_webhooks WHERE chat_channel_id = $1)",
        "DELETE FROM incoming_chat_webhooks WHERE chat_channel_id = $1",
        "DELETE FROM chat_drafts WHERE chat_channel_id = $1",
        "DELETE FROM user_chat_channel_memberships WHERE chat_channel_id = $1",
    ] {
        sqlx::query(sql).bind(channel_id).execute(&mut *tx).await?;
    }
    // Chat::Message's default scope: the live ones.
    let message_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM chat_messages WHERE chat_channel_id = $1 AND deleted_at IS NULL ORDER BY id",
    )
    .bind(channel_id)
    .fetch_all(&mut *tx)
    .await?;
    if !message_ids.is_empty() {
        for sql in [
            "DELETE FROM chat_mentions WHERE chat_message_id = ANY($1)",
            "DELETE FROM chat_message_revisions WHERE chat_message_id = ANY($1)",
            "DELETE FROM chat_message_reactions WHERE chat_message_id = ANY($1)",
            "DELETE FROM upload_references WHERE target_type = 'ChatMessage' AND target_id = ANY($1)",
            "DELETE FROM chat_message_hotlinked_media WHERE chat_message_id = ANY($1)",
        ] {
            sqlx::query(sql)
                .bind(&message_ids)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query(
            "UPDATE chat_messages SET deleted_by_id = $2, deleted_at = $3 WHERE id = ANY($1)",
        )
        .bind(&message_ids)
        .bind(deleted_by)
        .bind(crate::clock::now_naive())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}
