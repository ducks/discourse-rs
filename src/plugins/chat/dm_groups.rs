//! Who is in a direct message: `Chat::AddUsersToChannel`,
//! `Chat::RemoveUserFromChannel`, a group DM's `leave`, and
//! `Chat::UpdateChannel`.

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
    /// RateLimiter::LimitExceeded
    RateLimited(crate::rate_limiter::LimitExceeded),
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
            // Discourse.system_user.guardian on nil, inside the transaction
            .ok_or(std::io::Error::other(
                "NoMethodError: guardian of a missing system user",
            ))?;
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

    /// The slug an edit saves: the one given (the channel's when none, nil
    /// when blank); a category channel without one gets its title's
    /// (`generate_auto_slug`, blank when taken). A changed one goes
    /// through `ensure_slug_ok`: encoded, and refused as the model's
    /// record_invalid when it comes out blank, non-ASCII under the ascii
    /// method, taken by another live channel or over 100 characters.
    async fn channel_slug(
        &mut self,
        channel: &ChannelRow,
        given: Option<Option<String>>,
        name: Option<&str>,
    ) -> Result<Option<String>, AppError> {
        let invalid = || Unsupported("an invalid chat channel edit (record_invalid)");
        let method = self.settings.get("slug_generation_method")?.to_s();
        let mut slug = given.unwrap_or_else(|| channel.slug.clone());
        if channel.chatable_type == "Category" && slug.is_none() {
            // CategoryChannel#title: its name, else its category's.
            let title = match name {
                Some(n) => n.to_string(),
                None => self
                    .category(channel.chatable_id)
                    .await?
                    .map(|c| c.name)
                    .unwrap_or_default(),
            };
            let title = crate::ruby::strip(&title).to_string();
            let generated = match method.as_str() {
                "ascii" => crate::posting::text::slug_with_default(
                    &title,
                    &self.settings.get("default_locale")?.to_s(),
                    "",
                )?,
                "encoded" => crate::category_writes::encoded_slug(&title),
                _ => String::new(),
            };
            let taken = !generated.is_empty() && self.slug_taken(channel.id, &generated).await?;
            slug = Some(if taken { String::new() } else { generated });
        }
        if slug == channel.slug {
            return Ok(slug);
        }
        if let Some(given) = slug.clone().filter(|s| !s.is_empty()) {
            let unescaped = if method == "encoded" {
                crate::category_writes::cgi_unescape(&given)
            } else {
                given
            };
            let encoded = crate::category_writes::encoded_slug(&unescaped);
            if encoded.is_empty()
                || (method == "ascii" && !crate::category_writes::cgi_unescape(&encoded).is_ascii())
                || self.slug_taken(channel.id, &encoded).await?
            {
                return Err(invalid().into());
            }
            slug = Some(encoded);
        }
        if slug.as_deref().is_some_and(|s| s.chars().count() > 100) {
            return Err(invalid().into());
        }
        Ok(slug)
    }

    /// `duplicate_slug?`: another live channel has it.
    async fn slug_taken(&mut self, channel_id: i64, slug: &str) -> Result<bool, AppError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM chat_channels WHERE slug = $1 AND id <> $2 AND deleted_at IS NULL)",
        )
        .bind(slug)
        .bind(channel_id)
        .fetch_one(&mut *self.conn)
        .await?)
    }

    /// `Chat::UpdateChannel` (with the controller's channel_from_params
    /// and auto join limiter): the editable fields (blank text nil; a
    /// category channel's auto_join_users and allow_channel_wide_mentions
    /// too), the threads marked read when a category channel turns
    /// threading on, the hidden chat_threads_enabled setting saved,
    /// `/chat/channel-edits` to the channel's audience, auto join when on,
    /// and the channel with the actor's membership back.
    pub async fn update_channel(
        &mut self,
        state: &crate::AppState,
        channel_id: i64,
        edits: &serde_json::Map<String, Value>,
    ) -> Result<Outcome, AppError> {
        let bus = &state.bus;
        // channel_from_params: Chat::Channel.find, ensure_can_join_chat_channel!
        let Some(channel) = self.live_channel(channel_id).await? else {
            return Ok(Outcome::NotFound);
        };
        if !self.can_join(&channel, None).await? {
            return Ok(Outcome::Forbidden);
        }
        let category = channel.chatable_type == "Category";
        let flag = |key: &str| -> Option<bool> {
            edits.get(key).map(|v| {
                crate::ruby::cast_boolean(crate::params::scalar(v).as_deref()).unwrap_or(false)
            })
        };
        let auto_join = if category {
            flag("auto_join_users").unwrap_or(channel.auto_join_users)
        } else {
            channel.auto_join_users
        };
        // auto_join_limiter, staff included, when auto join is asked for.
        if category && flag("auto_join_users") == Some(true) {
            let user = self
                .guardian
                .user()
                .ok_or(Unsupported("editing a chat channel anonymously"))?;
            let limiter = crate::rate_limiter::RateLimiter::new(
                Some(crate::rate_limiter::LimitUser {
                    id: user.id,
                    staff: self.guardian.is_staff(),
                }),
                &format!("auto_join_users_channel_{}", channel.id),
                1,
                180,
            )
            .apply_limit_to_staff();
            if let Err(refused) = limiter
                .performed(&state.pool, state.config.rate_limits)
                .await?
            {
                return Ok(Outcome::RateLimited(refused));
            }
        }
        // check_channel_permission: can preview, and can_edit_chat_channel?
        // (a DM's participants or staff; staff for a category channel).
        let dm = if category {
            None
        } else {
            Some(self.direct_message(channel.chatable_id).await?)
        };
        let can_edit = self.guardian.is_staff()
            || dm
                .as_ref()
                .is_some_and(|dm| dm.user_can_access(self.guardian));
        if !(self.can_preview(&channel).await? && can_edit) {
            return Ok(Outcome::Forbidden);
        }
        let text = |key: &str| -> Option<Option<String>> {
            edits
                .get(key)
                .map(|v| crate::params::scalar(v).filter(|s| !s.trim().is_empty()))
        };
        let name = text("name").unwrap_or(channel.name.clone());
        let slug = self
            .channel_slug(&channel, text("slug"), name.as_deref())
            .await?;
        let description = text("description").unwrap_or(channel.description.clone());
        let emoji = text("emoji").unwrap_or(channel.emoji.clone());
        let threading = flag("threading_enabled").unwrap_or(channel.threading_enabled);
        let wide_mentions = if category {
            flag("allow_channel_wide_mentions").unwrap_or(channel.allow_channel_wide_mentions)
        } else {
            channel.allow_channel_wide_mentions
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
            || threading != channel.threading_enabled
            || auto_join != channel.auto_join_users
            || wide_mentions != channel.allow_channel_wide_mentions
            || slug != channel.slug;
        if changed {
            sqlx::query(
                "UPDATE chat_channels SET name = $2, description = $3, emoji = $4, threading_enabled = $5, \
                   auto_join_users = $6, allow_channel_wide_mentions = $7, updated_at = $8, slug = $9 WHERE id = $1",
            )
            .bind(channel.id)
            .bind(&name)
            .bind(&description)
            .bind(&emoji)
            .bind(threading)
            .bind(auto_join)
            .bind(wide_mentions)
            .bind(crate::clock::now_naive())
            .bind(&slug)
            .execute(&mut *self.conn)
            .await?;
        }
        // enqueue_channel_hashtag_remap through HashtagRemapper.enqueue:
        // the old slug, unless only its case changed.
        if let Some(old) = channel.slug.as_deref().filter(|s| !s.is_empty())
            && slug.as_deref().unwrap_or_default().to_lowercase() != old.to_lowercase()
        {
            crate::jobs::enqueue(
                &mut *self.conn,
                "remap_hashtag",
                json!({ "remaps": [{ "type": "channel", "id": channel.id, "old_ref": old }] }),
            )
            .await?;
        }
        // mark_all_threads_as_read_if_needed
        if category && threading && !channel.threading_enabled {
            crate::jobs::enqueue(
                &mut *self.conn,
                "Jobs::Chat::MarkAllChannelThreadsRead",
                json!({ "channel_id": channel.id }),
            )
            .await?;
        }
        // update_site_settings_if_needed: always saved.
        let any_threading: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM chat_channels WHERE threading_enabled AND deleted_at IS NULL)",
        )
        .fetch_one(&mut *self.conn)
        .await?;
        let def = state
            .site_setting_defs
            .get("chat_threads_enabled")
            .ok_or_else(|| std::io::Error::other("chat_threads_enabled has no definition"))?;
        let previous = self.settings.get("chat_threads_enabled")?.clone();
        crate::site_setting_update::save_setting(
            &mut *self.conn,
            bus,
            def,
            &previous,
            any_threading,
        )
        .await?;
        // publish_chat_channel_edit: the channel's title for the actor.
        let channel = self
            .live_channel(channel.id)
            .await?
            .ok_or(Unsupported("a chat channel gone while editing"))?;
        let title = match (&dm, &channel.name) {
            (Some(dm), Some(name)) if dm.group => json!(name),
            (Some(dm), _) => json!(self.dm_title(&channel, dm)?),
            (None, Some(name)) => json!(name),
            (None, None) => json!(
                self.category(channel.chatable_id)
                    .await?
                    .map(|c| c.name)
                    .unwrap_or_default()
            ),
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
        // auto_join_users_if_needed
        if category && channel.auto_join_users {
            super::auto_join::create_memberships(
                &mut *self.conn,
                bus,
                &state.i18n,
                &state.config,
                self.settings,
                None,
                Some(channel.id),
            )
            .await?;
        }
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
