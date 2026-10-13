//! Who is in a direct message: `Chat::AddUsersToChannel`,
//! `Chat::RemoveUserFromChannel`, a group DM's `leave`, and
//! `Chat::UpdateChannel` for a direct message channel.

use serde_json::{Value, json};

use super::channels::{CHANNEL_COLUMNS, ChannelRow, Context};
use crate::user_comm_screener::{Actor, UserCommScreener};
use crate::{AppError, Unsupported};

/// How the services end, as their controllers render them.
pub enum Outcome {
    /// The JSON body, a 200.
    Done(Value),
    NotFound,
    /// `failed_json`, a 422.
    Failed,
    /// The contract's errors, a 400 `failed_json` with them.
    Invalid(Vec<String>),
    /// render_json_error, a 422 (or the status given).
    Error(String, u16),
    /// `{error: ...}`, a 400.
    Refused(String),
    /// Discourse::InvalidAccess
    Forbidden,
}

/// A user the services add.
#[derive(sqlx::FromRow, Clone)]
struct Target {
    id: i32,
    username: String,
}

impl Context<'_> {
    async fn live_channel(&mut self, id: i64) -> Result<Option<ChannelRow>, AppError> {
        Ok(sqlx::query_as(&format!(
            "SELECT {CHANNEL_COLUMNS} FROM chat_channels WHERE chat_channels.id = $1 AND chat_channels.deleted_at IS NULL"
        ))
        .bind(id)
        .fetch_optional(&mut *self.conn)
        .await?)
    }

    /// `ChannelMembershipsQuery.count` written back, not stale.
    async fn recompute_user_count(&mut self, channel: &ChannelRow) -> Result<(), AppError> {
        let count: i64 = sqlx::query_scalar(
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
        .await?;
        sqlx::query(
            "UPDATE chat_channels SET user_count = $2, user_count_stale = FALSE, updated_at = $3 \
             WHERE id = $1 AND (user_count <> $2 OR user_count_stale)",
        )
        .bind(channel.id)
        .bind(count as i32)
        .bind(crate::clock::now_naive())
        .execute(&mut *self.conn)
        .await?;
        Ok(())
    }

    /// `DirectMessageChannel#leave(user)` for a group DM: the membership
    /// and the participant row go. Callers handle a 1:1 (an unfollow).
    pub(crate) async fn leave_group_dm(
        &mut self,
        channel: &ChannelRow,
        user_id: i32,
    ) -> Result<(), AppError> {
        sqlx::query(
            "DELETE FROM user_chat_channel_memberships WHERE user_id = $1 AND chat_channel_id = $2",
        )
        .bind(user_id)
        .bind(channel.id)
        .execute(&mut *self.conn)
        .await?;
        sqlx::query(
            "DELETE FROM direct_message_users WHERE direct_message_channel_id = $1 AND user_id = $2",
        )
        .bind(channel.chatable_id)
        .bind(user_id)
        .execute(&mut *self.conn)
        .await?;
        Ok(())
    }

    /// `Chat::AddUsersToChannel`
    pub async fn add_users_to_channel(
        &mut self,
        host: &crate::pretty_text::Host,
        bus: &pg_bus::Bus,
        channel_id: i64,
        usernames: &[String],
        groups: &[String],
    ) -> Result<Outcome, AppError> {
        let s = self.settings;
        let g = self.guardian;
        let Some(actor) = g.user().cloned() else {
            return Ok(Outcome::Forbidden);
        };
        let t = |key: &str, args: &[(&str, &str)]| {
            self.i18n
                .t_with(key, args)
                .unwrap_or_else(|| key.to_string())
        };
        // The contract (the length limit read from the setting).
        let max = s.get("chat_max_direct_message_users")?.to_i();
        let mut errors = Vec::new();
        if usernames.is_empty() && groups.is_empty() {
            errors.push("Usernames can't be blank".to_string());
        }
        if usernames.len() as i64 > max {
            errors.push(format!(
                "Usernames is too long (maximum is {max} character{})",
                if max == 1 { "" } else { "s" }
            ));
        }
        if groups.is_empty() && usernames.is_empty() {
            errors.push("Groups can't be blank".to_string());
        }
        if !errors.is_empty() {
            return Ok(Outcome::Invalid(errors));
        }
        let Some(channel) = self.live_channel(channel_id).await? else {
            return Ok(Outcome::Failed);
        };
        // can_add_users_to_channel
        let joined: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM user_chat_channel_memberships WHERE user_id = $1 \
               AND chat_channel_id = $2 AND following)",
        )
        .bind(actor.id)
        .bind(channel.id)
        .fetch_one(&mut *self.conn)
        .await?;
        let messages_count: i32 =
            sqlx::query_scalar("SELECT messages_count FROM chat_channels WHERE id = $1")
                .bind(channel.id)
                .fetch_one(&mut *self.conn)
                .await?;
        let allowed = (actor.admin || joined)
            && channel.chatable_type == "DirectMessage"
            && (self.direct_message(channel.chatable_id).await?.group || messages_count == 0);
        if !allowed {
            return Ok(Outcome::Error(
                t("chat.errors.users_cant_be_added_to_channel", &[]),
                422,
            ));
        }
        // target_users: the new users (chat on, accepting PMs, not yet in
        // the DM), then the DM's other participants.
        let visible_groups: Vec<String> = if groups.is_empty() {
            Vec::new()
        } else {
            sqlx::query_scalar(&format!(
                "SELECT groups.name FROM groups WHERE groups.name = ANY($1) AND {} AND {} ORDER BY groups.id",
                crate::groups::visible_groups_where(g, "groups"),
                crate::groups::members_visible_groups_where(g, "groups"),
            ))
            .bind(groups)
            .fetch_all(&mut *self.conn)
            .await?
        };
        let mut normalized = Vec::new();
        for username in usernames {
            normalized.push(crate::accounts::normalize_username(username)?);
        }
        let mut targets: Vec<Target> = sqlx::query_as(
            "SELECT DISTINCT users.id, users.username FROM users \
             INNER JOIN user_options ON user_options.user_id = users.id \
             LEFT OUTER JOIN group_users ON group_users.user_id = users.id \
             LEFT OUTER JOIN groups ON groups.id = group_users.group_id \
             WHERE user_options.chat_enabled = TRUE AND user_options.allow_private_messages = TRUE \
               AND (users.username_lower = ANY($1) OR (groups.name = ANY($2) AND group_users.user_id IS NOT NULL)) \
               AND users.id NOT IN (SELECT user_id FROM direct_message_users WHERE direct_message_channel_id = $3) \
             ORDER BY users.id",
        )
        .bind(&normalized)
        .bind(&visible_groups)
        .bind(channel.chatable_id)
        .fetch_all(&mut *self.conn)
        .await?;
        let existing: Vec<Target> = sqlx::query_as(
            "SELECT users.id, users.username FROM users \
             INNER JOIN direct_message_users ON direct_message_users.user_id = users.id \
             WHERE direct_message_users.direct_message_channel_id = $1 AND users.id <> $2 \
             ORDER BY direct_message_users.id",
        )
        .bind(channel.chatable_id)
        .bind(actor.id)
        .fetch_all(&mut *self.conn)
        .await?;
        targets.extend(existing);
        let others: Vec<Target> = targets
            .iter()
            .filter(|u| u.id != actor.id)
            .cloned()
            .collect();
        let ids: Vec<i32> = targets.iter().map(|u| u.id).collect();
        let screener = UserCommScreener::new(
            &mut *self.conn,
            Actor {
                id: actor.id,
                staff: g.is_staff(),
            },
            &ids,
        )
        .await?;
        // actor_allows_dms
        if screener.actor_disallowing_all_pms() {
            return Ok(Outcome::Error(
                t("chat.errors.actor_disallowed_dms", &[]),
                422,
            ));
        }
        // CanCommunicateAllParties
        if let Some(first) = screener.preventing_actor_communication().first() {
            let username = others
                .iter()
                .find(|u| u.id == *first)
                .map(|u| u.username.clone())
                .unwrap_or_default();
            return Ok(Outcome::Error(
                t("chat.errors.not_accepting_dms", &[("username", &username)]),
                422,
            ));
        }
        for (check, key) in [
            (0, "chat.errors.actor_preventing_target_user_from_dm"),
            (1, "chat.errors.actor_ignoring_target_user"),
            (2, "chat.errors.actor_muting_target_user"),
        ] {
            for user in &others {
                let hit = match check {
                    0 => screener.actor_disallowing_pms(user.id)?,
                    1 => screener.actor_ignoring(user.id)?,
                    _ => screener.actor_muting(user.id)?,
                };
                if hit {
                    return Ok(Outcome::Error(t(key, &[("username", &user.username)]), 422));
                }
            }
        }
        // MaxUsersExcess
        if !g.is_staff() && others.len() as i64 > max {
            return Ok(Outcome::Refused(if max == 0 {
                t(
                    "chat.errors.over_chat_max_direct_message_users_allow_self",
                    &[],
                )
            } else {
                let form = if max == 1 { "one" } else { "other" };
                t(
                    &format!("chat.errors.over_chat_max_direct_message_users.{form}"),
                    &[("count", &max.to_string())],
                )
            }));
        }

        // create_memberships: those inserted are the added ones.
        let now = crate::clock::now_naive();
        let mut added: Vec<i32> = Vec::new();
        for user in &targets {
            let inserted: Option<i32> = sqlx::query_scalar(
                "INSERT INTO user_chat_channel_memberships \
                   (user_id, chat_channel_id, muted, following, notification_level, created_at, updated_at) \
                 VALUES ($1, $2, FALSE, TRUE, 2, $3, $3) \
                 ON CONFLICT (user_id, chat_channel_id) DO NOTHING RETURNING user_id",
            )
            .bind(user.id)
            .bind(channel.id)
            .bind(now)
            .fetch_optional(&mut *self.conn)
            .await?;
            added.extend(inserted);
        }
        for user_id in &added {
            sqlx::query(
                "INSERT INTO direct_message_users (direct_message_channel_id, user_id, created_at, updated_at) \
                 VALUES ($1, $2, $3, $3) ON CONFLICT (direct_message_channel_id, user_id) DO NOTHING",
            )
            .bind(channel.chatable_id)
            .bind(user_id)
            .bind(now)
            .execute(&mut *self.conn)
            .await?;
        }
        if added.is_empty() {
            return Ok(Outcome::Done(json!({ "success": "OK" })));
        }
        self.recompute_user_count(&channel).await?;

        // notice_channel: the system user tells the channel, through the
        // whole of CreateMessage.
        let invited: Vec<String> = targets
            .iter()
            .filter(|u| added.contains(&u.id))
            .map(|u| format!("@{}", u.username))
            .collect();
        let count = invited.len().to_string();
        let form = if invited.len() == 1 { "one" } else { "other" };
        let message = t(
            &format!("chat.channel.users_invited_to_channel.{form}"),
            &[
                ("invited_users", &invited.join(", ")),
                ("inviting_user", &format!("@{}", actor.username)),
                ("count", &count),
            ],
        );
        let system = crate::session::current::SessionUser::load(&mut *self.conn, -1)
            .await?
            .ok_or(Unsupported("a site without its system user"))?;
        let system_guardian = crate::guardian::Guardian::for_user(&mut *self.conn, &system).await?;
        let outcome = Context {
            conn: &mut *self.conn,
            settings: self.settings,
            i18n: self.i18n,
            guardian: &system_guardian,
            base_path: self.base_path,
            config: self.config,
        }
        .create_message(
            host,
            bus,
            super::create::Params {
                chat_channel_id: channel.id.to_string(),
                message: Some(message),
                ..Default::default()
            },
        )
        .await?;
        if !matches!(outcome, super::create::Outcome::Created(_)) {
            // fail!(failure: "Failed to notice the channel")
            return Ok(Outcome::Failed);
        }
        Ok(Outcome::Done(json!({ "success": "OK" })))
    }

    /// `Chat::RemoveUserFromChannel`
    pub async fn remove_user_from_channel(
        &mut self,
        channel_id: i64,
        user_id: i64,
    ) -> Result<Outcome, AppError> {
        let Some(channel) = self.live_channel(channel_id).await? else {
            return Ok(Outcome::NotFound);
        };
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
            .bind(user_id)
            .fetch_one(&mut *self.conn)
            .await?;
        if !exists {
            return Ok(Outcome::NotFound);
        }
        let user_id = user_id as i32;
        // can_remove_members?: admins, on a category channel or a group DM.
        let group_dm = channel.chatable_type == "DirectMessage"
            && self.direct_message(channel.chatable_id).await?.group;
        if !(self.guardian.is_admin() && (channel.chatable_type == "Category" || group_dm)) {
            return Ok(Outcome::Error(
                self.i18n
                    .t("chat.errors.user_cant_be_removed_from_channel")
                    .unwrap_or_default()
                    .to_string(),
                403,
            ));
        }
        if group_dm {
            self.leave_group_dm(&channel, user_id).await?;
        } else {
            // ChannelMembershipManager#unfollow for the target.
            let unfollowed = sqlx::query(
                "UPDATE user_chat_channel_memberships SET following = FALSE, starred = FALSE, updated_at = $3 \
                 WHERE user_id = $1 AND chat_channel_id = $2 AND following",
            )
            .bind(user_id)
            .bind(channel.id)
            .bind(crate::clock::now_naive())
            .execute(&mut *self.conn)
            .await?
            .rows_affected();
            if unfollowed > 0 {
                self.mark_user_count_stale(channel.id).await?;
            }
        }
        self.recompute_user_count(&channel).await?;
        Ok(Outcome::Done(json!({ "success": "OK" })))
    }

    /// `Chat::UpdateChannel` for a direct message channel: the editable
    /// fields (blank ones nil), the hidden chat_threads_enabled setting
    /// saved, `/chat/channel-edits` to the participants, and the channel
    /// with the actor's membership back.
    pub async fn update_dm_channel(
        &mut self,
        bus: &pg_bus::Bus,
        defs: &crate::site_settings::Definitions,
        channel_id: i64,
        edits: &serde_json::Map<String, Value>,
    ) -> Result<Outcome, AppError> {
        // channel_from_params: Chat::Channel.find, ensure_can_join_chat_channel!
        let Some(channel) = self.live_channel(channel_id).await? else {
            return Ok(Outcome::NotFound);
        };
        if !self.can_join(&channel, None).await? {
            return Ok(Outcome::Forbidden);
        }
        if channel.chatable_type != "DirectMessage" {
            return Err(Unsupported("editing category chat channels").into());
        }
        // check_channel_permission: can preview, and can_edit_chat_channel?
        let dm = self.direct_message(channel.chatable_id).await?;
        if !(self.can_preview(&channel).await?
            && (self.guardian.is_staff() || dm.user_can_access(self.guardian)))
        {
            return Ok(Outcome::Forbidden);
        }
        let text = |key: &str| -> Option<Option<String>> {
            edits
                .get(key)
                .map(|v| crate::params::scalar(v).filter(|s| !s.trim().is_empty()))
        };
        if edits.contains_key("slug") {
            return Err(Unsupported("changing a chat channel's slug").into());
        }
        let name = text("name").unwrap_or(channel.name.clone());
        let description = text("description").unwrap_or(channel.description.clone());
        let emoji = text("emoji").unwrap_or(channel.emoji.clone());
        let threading = match edits
            .get("threading_enabled")
            .and_then(crate::params::scalar)
        {
            Some(v) => crate::ruby::cast_boolean(Some(&v)).unwrap_or(false),
            None => channel.threading_enabled,
        };
        let max_title = self.settings.get("max_topic_title_length")?.to_i();
        if name
            .as_deref()
            .is_some_and(|n| n.chars().count() as i64 > max_title)
            || description
                .as_deref()
                .is_some_and(|d| d.chars().count() > 500)
            || emoji.as_deref().is_some_and(|e| e.chars().count() > 100)
        {
            return Err(Unsupported("an invalid chat channel edit (record_invalid)").into());
        }
        let changed = name != channel.name
            || description != channel.description
            || emoji != channel.emoji
            || threading != channel.threading_enabled;
        if changed {
            sqlx::query(
                "UPDATE chat_channels SET name = $2, description = $3, emoji = $4, threading_enabled = $5, \
                   updated_at = $6 WHERE id = $1",
            )
            .bind(channel.id)
            .bind(&name)
            .bind(&description)
            .bind(&emoji)
            .bind(threading)
            .bind(crate::clock::now_naive())
            .execute(&mut *self.conn)
            .await?;
        }
        // update_site_settings_if_needed: always saved.
        let any_threading: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM chat_channels WHERE threading_enabled AND deleted_at IS NULL)",
        )
        .fetch_one(&mut *self.conn)
        .await?;
        let def = defs
            .get("chat_threads_enabled")
            .ok_or(Unsupported("the chat_threads_enabled setting"))?;
        let previous = self.settings.get("chat_threads_enabled")?.clone();
        crate::site_setting_update::save_setting(
            &mut *self.conn,
            bus,
            def,
            &previous,
            any_threading,
        )
        .await?;
        // publish_chat_channel_edit
        let channel = self
            .live_channel(channel.id)
            .await?
            .ok_or(Unsupported("a chat channel gone while editing"))?;
        let title = match &channel.name {
            Some(name) if dm.group => json!(name),
            _ => self.dm_title(&channel, &dm)?,
        };
        let audience = super::publisher::audience(
            &mut *self.conn,
            self.settings,
            &channel.chatable_type,
            channel.chatable_id,
        )
        .await?;
        bus.publish(
            &mut *self.conn,
            "/chat/channel-edits",
            &json!({
                "chat_channel_id": channel.id,
                "name": title,
                "description": channel.description,
                "slug": channel.slug,
            }),
            Some(&audience),
        )
        .await?;
        let membership: Option<super::channels::MembershipRow> = sqlx::query_as(
            "SELECT chat_channel_id, following, muted, notification_level, last_read_message_id, \
                    last_viewed_at, last_viewed_pins_at, starred \
             FROM user_chat_channel_memberships WHERE user_id = $1 AND chat_channel_id = $2",
        )
        .bind(self.guardian.user_id())
        .bind(channel.id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let body = self
            .channel(&channel, membership.as_ref(), None, None)
            .await?;
        Ok(Outcome::Done(json!({ "channel": body })))
    }
}
