//! Direct message channels, read side: Chat::DirectMessage (the
//! participants, who may access it, its title for a viewer),
//! Chat::DirectMessageSerializer with Chat::ChatableUserSerializer, and
//! ChannelFetcher's secured_direct_message_channels.

use serde_json::{Map, Value, json};

use super::channels::{CHANNEL_COLUMNS, ChannelRow, Context};
use crate::guardian::Guardian;
use crate::session::current::SessionUser;
use crate::{AppError, Unsupported};

/// `ChannelFetcher::MAX_DM_CHANNEL_RESULTS`
pub const MAX_DM_CHANNEL_RESULTS: i64 = 75;

/// A participant's user; None in its place is a deleted user
/// (Chat::NullUser).
#[derive(sqlx::FromRow, Clone)]
pub struct DmUser {
    pub id: i32,
    pub username: String,
    pub name: Option<String>,
    pub uploaded_avatar_id: Option<i32>,
    pub chat_enabled: Option<bool>,
}

/// A participant row, its user's columns NULL when the user is gone.
#[derive(sqlx::FromRow)]
struct DmUserRow {
    id: Option<i32>,
    username: Option<String>,
    name: Option<String>,
    uploaded_avatar_id: Option<i32>,
    chat_enabled: Option<bool>,
}

/// A direct message: whether it's a group, and its users in the
/// association's order.
pub struct DirectMessage {
    pub group: bool,
    pub users: Vec<Option<DmUser>>,
}

impl DirectMessage {
    /// `user_can_access?`: a participant, or an admin on a group.
    pub fn user_can_access(&self, guardian: &Guardian) -> bool {
        let Some(id) = guardian.user_id() else {
            return false;
        };
        self.users.iter().flatten().any(|u| u.id == id) || (self.group && guardian.is_admin())
    }
}

impl Context<'_> {
    /// The channel's direct message (`chatable`).
    pub async fn direct_message(&mut self, dm_id: i64) -> Result<DirectMessage, AppError> {
        let group: Option<bool> =
            sqlx::query_scalar("SELECT \"group\" FROM direct_message_channels WHERE id = $1")
                .bind(dm_id)
                .fetch_optional(&mut *self.conn)
                .await?;
        let Some(group) = group else {
            return Err(Unsupported("direct message channels without their direct message").into());
        };
        let rows: Vec<DmUserRow> = sqlx::query_as(
            "SELECT u.id, u.username, u.name, u.uploaded_avatar_id, uo.chat_enabled \
             FROM direct_message_users dmu \
             LEFT JOIN users u ON u.id = dmu.user_id \
             LEFT JOIN user_options uo ON uo.user_id = u.id \
             WHERE dmu.direct_message_channel_id = $1 ORDER BY dmu.id",
        )
        .bind(dm_id)
        .fetch_all(&mut *self.conn)
        .await?;
        let users = rows
            .into_iter()
            .map(|r| {
                Some(DmUser {
                    id: r.id?,
                    username: r.username?,
                    name: r.name,
                    uploaded_avatar_id: r.uploaded_avatar_id,
                    chat_enabled: r.chat_enabled,
                })
            })
            .collect();
        Ok(DirectMessage { group, users })
    }

    /// `chat_channel_title_for_user`: the group's name, else the other
    /// participants (system users left out), sorted, joined and truncated
    /// past seven; the viewer's own name for a DM to themselves.
    pub fn dm_title(&self, channel: &ChannelRow, dm: &DirectMessage) -> Result<Value, AppError> {
        if dm.group
            && let Some(name) = channel.name.as_deref().filter(|n| !n.trim().is_empty())
        {
            return Ok(json!(name));
        }
        let s = self.settings;
        let viewer = self.guardian.user();
        let viewer_id = viewer.map(|u| u.id);
        let deleted = self
            .i18n
            .t("chat.deleted_chat_username")
            .unwrap_or("deleted");
        // (direct_message_users.map(&:user) - [acting_user]), NullUsers
        // kept, system users rejected.
        let others: Vec<(String, Option<String>)> = dm
            .users
            .iter()
            .filter(|u| u.as_ref().map(|u| u.id) != viewer_id)
            .filter(|u| u.as_ref().is_none_or(|u| u.id != -1))
            .map(|u| match u {
                Some(u) => (u.username.clone(), u.name.clone()),
                None => (deleted.to_string(), None),
            })
            .collect();
        let prefers_name = s.get("enable_names")?.truthy()
            && s.get("display_name_on_posts")?.truthy()
            && !s.get("prioritize_username_in_ux")?.truthy();
        let display = |username: &str, name: Option<&str>| -> String {
            if prefers_name {
                name.filter(|n| !n.trim().is_empty())
                    .unwrap_or(username)
                    .to_string()
            } else {
                username.to_string()
            }
        };
        if others.is_empty() {
            let Some(viewer) = viewer else {
                return Err(Unsupported("a direct message title without a viewer").into());
            };
            let username = display(&viewer.username, viewer.name.as_deref());
            return Ok(json!(
                self.i18n
                    .t_with(
                        "chat.channel.dm_title.single_user",
                        &[("username", &username)]
                    )
                    .unwrap_or(username)
            ));
        }
        let mut names: Vec<String> = others
            .iter()
            .map(|(username, name)| display(username, name.as_deref()))
            .collect();
        if prefers_name {
            // sort_by { |name| name[1..-1] }: from the second character.
            names.sort_by(|a, b| {
                let tail = |s: &String| s.chars().skip(1).collect::<String>();
                tail(a).cmp(&tail(b))
            });
        } else {
            names.sort();
        }
        let comma = self.i18n.t("word_connector.comma").unwrap_or(", ");
        if names.len() > 7 {
            let shown = names[..6].join(comma);
            let count = (names.len() - 6) as i64;
            let n = count.to_string();
            return Ok(json!(
                self.i18n
                    .t_count(
                        "chat.channel.dm_title.multi_user_truncated",
                        count,
                        &[("comma_separated_usernames", &shown), ("count", &n)],
                    )
                    .unwrap_or(shown)
            ));
        }
        let joined = names.join(comma);
        Ok(json!(
            self.i18n
                .t_with(
                    "chat.channel.dm_title.multi_user",
                    &[("comma_separated_usernames", &joined)],
                )
                .unwrap_or(joined)
        ))
    }

    /// Chat::DirectMessageSerializer: the group flag and the participants
    /// (the viewer left out unless alone).
    pub async fn dm_chatable(&mut self, dm: &DirectMessage) -> Result<Value, AppError> {
        let viewer_id = self.guardian.user_id();
        let mut users: Vec<&Option<DmUser>> = dm.users.iter().collect();
        if users.len() > 1 {
            users.retain(|u| u.as_ref().is_none() || u.as_ref().map(|u| u.id) != viewer_id);
        }
        let mut out = Vec::new();
        for u in users {
            out.push(self.chatable_user(u.as_ref()).await?);
        }
        Ok(json!({"group": dm.group, "users": out}))
    }

    /// Chat::ChatableUserSerializer (UserWithCustomFieldsSerializer with
    /// can_chat and has_chat_enabled); None is Chat::NullUser.
    pub(crate) async fn chatable_user(&mut self, u: Option<&DmUser>) -> Result<Value, AppError> {
        let s = self.settings;
        let g = self.guardian;
        let mut out = Map::new();
        // can_create_direct_message? is the viewer's.
        let viewer_creates = g.is_staff() || super::can_direct_message(s, g)?;
        let enabled = super::enabled(s)?;
        match u {
            Some(u) => {
                out.insert("id".into(), json!(u.id));
                out.insert("username".into(), json!(u.username));
                if s.get("enable_names")?.truthy() {
                    out.insert("name".into(), json!(u.name));
                }
                let urls = crate::url::Urls {
                    config: self.config,
                    settings: s,
                };
                let logo = crate::admin_users::logo_small_url(&mut *self.conn, s).await?;
                out.insert(
                    "avatar_template".into(),
                    json!(crate::avatar::avatar_template(
                        &urls,
                        u.id,
                        &u.username,
                        u.uploaded_avatar_id,
                        logo.as_deref()
                    )?),
                );
                if s.get("enable_user_status")?.truthy() {
                    let has_status: bool = sqlx::query_scalar(
                        "SELECT EXISTS (SELECT 1 FROM user_statuses WHERE user_id = $1 AND (ends_at IS NULL OR ends_at > now()))",
                    )
                    .bind(u.id)
                    .fetch_one(&mut *self.conn)
                    .await?;
                    if has_status {
                        return Err(Unsupported("user status").into());
                    }
                }
                out.insert("custom_fields".into(), self.custom_fields(u.id).await?);
                // object.guardian.can_chat?
                let target_can_chat = match SessionUser::load(&mut *self.conn, u.id).await? {
                    Some(target) => {
                        let tg = Guardian::for_user(&mut *self.conn, &target).await?;
                        super::can_chat(&mut *self.conn, s, &tg).await?
                    }
                    None => false,
                };
                let can_chat = enabled && target_can_chat && viewer_creates;
                out.insert("can_chat".into(), json!(can_chat));
                out.insert(
                    "has_chat_enabled".into(),
                    json!(can_chat && u.chat_enabled.unwrap_or(false)),
                );
            }
            None => {
                out.insert("id".into(), Value::Null);
                out.insert(
                    "username".into(),
                    json!(
                        self.i18n
                            .t("chat.deleted_chat_username")
                            .unwrap_or("deleted")
                    ),
                );
                if s.get("enable_names")?.truthy() {
                    out.insert("name".into(), Value::Null);
                }
                out.insert(
                    "avatar_template".into(),
                    json!(format!(
                        "{}/plugins/chat/images/deleted-chat-user-avatar.png",
                        self.base_path
                    )),
                );
                out.insert("custom_fields".into(), json!({}));
                // A NullUser is in no groups: only the logged-in (or, not
                // granular, everyone) shortcut lets it chat.
                let allowed = s.group_ids("chat_allowed_groups")?;
                let granular = s
                    .get("granular_anonymous_and_logged_in_groups_permissions")?
                    .truthy();
                let null_can_chat = allowed.contains(&5) || (!granular && allowed.contains(&0));
                let can_chat = enabled && null_can_chat && viewer_creates;
                out.insert("can_chat".into(), json!(can_chat));
                // UserOption.new: chat_enabled defaults on.
                out.insert("has_chat_enabled".into(), json!(can_chat));
            }
        }
        Ok(Value::Object(out))
    }

    /// `User.allowed_user_custom_fields(guardian)`
    fn allowed_custom_fields(&self) -> Result<Vec<String>, AppError> {
        Ok(crate::review_list::allowed_user_custom_fields(
            self.settings,
            self.guardian,
        )?)
    }

    /// `custom_fields`: the allowed fields with a value. The serializer's
    /// users are not the ones the fetcher preloaded, so this is
    /// User.custom_fields_for_ids even in the channel list.
    async fn custom_fields(&mut self, user_id: i32) -> Result<Value, AppError> {
        let fields = self.allowed_custom_fields()?;
        if fields.is_empty() {
            return Ok(json!({}));
        }
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT name, value FROM user_custom_fields WHERE user_id = $1 AND name = ANY($2) ORDER BY id",
        )
        .bind(user_id)
        .bind(&fields)
        .fetch_all(&mut *self.conn)
        .await?;
        let mut values = Map::new();
        for (name, value) in rows {
            if values.contains_key(&name) {
                return Err(Unsupported("multi-valued user custom fields").into());
            }
            values.insert(name, json!(value));
        }
        Ok(Value::Object(values))
    }

    /// `secured_direct_message_channels_search(following: true[, starred])`:
    /// the viewer's followed DM channels, latest message first.
    pub async fn followed_dm_channels(
        &mut self,
        starred: bool,
    ) -> Result<Vec<ChannelRow>, AppError> {
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(Vec::new());
        };
        let sql = format!(
            "SELECT {CHANNEL_COLUMNS} FROM chat_channels \
             JOIN user_chat_channel_memberships m ON m.chat_channel_id = chat_channels.id \
             LEFT JOIN chat_messages last_message ON last_message.id = chat_channels.last_message_id \
             WHERE chat_channels.deleted_at IS NULL \
               AND chat_channels.id IN ( \
                 SELECT c.id FROM chat_channels c \
                 JOIN direct_message_channels dmc ON dmc.id = c.chatable_id AND c.chatable_type = 'DirectMessage' \
                 JOIN direct_message_users dmu ON dmu.direct_message_channel_id = dmc.id \
                 WHERE dmu.user_id = $1) \
               AND m.user_id = $1 AND m.following AND ($2 = FALSE OR m.starred) \
             GROUP BY chat_channels.id, last_message.id \
             ORDER BY last_message.created_at DESC NULLS LAST \
             LIMIT {MAX_DM_CHANNEL_RESULTS}"
        );
        Ok(sqlx::query_as(&sql)
            .bind(user_id)
            .bind(starred)
            .fetch_all(&mut *self.conn)
            .await?)
    }

    /// Chat::ChannelSerializer for a direct message channel: no
    /// auto_join_users, archive or kick id; the chatable's participants and
    /// the title for the viewer.
    pub async fn dm_channel(
        &mut self,
        channel: &ChannelRow,
        membership: Option<&super::channels::MembershipRow>,
    ) -> Result<Value, AppError> {
        let dm = self.direct_message(channel.chatable_id).await?;
        let pinned_setting = self.settings.get("chat_pinned_messages")?.truthy();
        let mut out = Map::new();
        out.insert("id".into(), json!(channel.id));
        out.insert(
            "allow_channel_wide_mentions".into(),
            json!(channel.allow_channel_wide_mentions),
        );
        out.insert("chatable".into(), self.dm_chatable(&dm).await?);
        out.insert("chatable_id".into(), json!(channel.chatable_id));
        out.insert("chatable_type".into(), json!(channel.chatable_type));
        out.insert("chatable_url".into(), Value::Null);
        if let Some(description) = channel
            .description
            .as_deref()
            .filter(|d| !d.trim().is_empty())
        {
            out.insert("description".into(), json!(description));
        }
        if let Some(emoji) = channel.emoji.as_deref().filter(|e| !e.trim().is_empty()) {
            out.insert("emoji".into(), json!(emoji));
        }
        // object.name || object.title(scope.user)
        let title = match &channel.name {
            Some(name) => json!(name),
            None => self.dm_title(channel, &dm)?,
        };
        let Some(title_str) = title.as_str().map(str::to_string) else {
            return Err(Unsupported("a direct message whose participants are all deleted").into());
        };
        out.insert("title".into(), title);
        out.insert(
            "unicode_title".into(),
            json!(self.settings.emojis().gsub_to_unicode(&title_str)),
        );
        out.insert("slug".into(), json!(channel.slug));
        out.insert(
            "status".into(),
            json!(
                super::channels::STATUSES
                    .get(channel.status as usize)
                    .copied()
                    .unwrap_or("open")
            ),
        );
        out.insert("memberships_count".into(), json!(channel.user_count));
        if let Some(m) = membership {
            out.insert(
                "current_user_membership".into(),
                self.membership(channel, m, pinned_setting).await?,
            );
        }
        out.insert("meta".into(), self.meta(channel, None, None).await?);
        out.insert("threading_enabled".into(), json!(channel.threading_enabled));
        if pinned_setting {
            let pins: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM chat_pinned_messages WHERE chat_channel_id = $1",
            )
            .bind(channel.id)
            .fetch_one(&mut *self.conn)
            .await?;
            out.insert("pinned_messages_count".into(), json!(pins));
        }
        if dm.user_can_access(self.guardian) {
            let last = match channel.last_message_id {
                Some(id) => self.last_message(id).await?,
                None => None,
            };
            out.insert(
                "last_message".into(),
                last.unwrap_or_else(|| json!({
                    "id": null,
                    "message": null,
                    "cooked": null,
                    "created_at": crate::clock::now_naive().format("%Y-%m-%dT%H:%M:%S+00:00").to_string(),
                    "excerpt": null,
                    "deleted_at": null,
                    "deleted_by_id": null,
                    "thread_id": null,
                    "chat_channel_id": null,
                    "streaming": false,
                })),
            );
        }
        Ok(Value::Object(out))
    }
}
