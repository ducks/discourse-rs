//! Chat channels as the API serves them: Chat::ChannelFetcher's channel
//! lists, Chat::ChannelUnreadsQuery's tracking state, and
//! Chat::ChannelSerializer with its membership and last message, for
//! GET /chat/api/me/channels (Chat::ListUserChannels with
//! Chat::ChannelIndexSerializer).
//!
//! Category channels only, for now: a viewer with direct message
//! channels, or a site with threads, is refused until those are ported.
//! Message bus ids are 0, as the topic view's are (rs's bus has no
//! per-channel counters), and presence is not ported, so the global
//! presence channel is always empty.

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::allowed_channel_ids_sql;
use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `ChannelFetcher::MAX_PUBLIC_CHANNEL_RESULTS`
const MAX_PUBLIC_CHANNEL_RESULTS: i64 = 100;

/// A row of chat_channels.
#[derive(sqlx::FromRow, Clone)]
pub struct ChannelRow {
    pub id: i64,
    pub chatable_id: i64,
    pub chatable_type: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub emoji: Option<String>,
    pub status: i32,
    pub user_count: i32,
    pub auto_join_users: bool,
    pub allow_channel_wide_mentions: bool,
    pub threading_enabled: bool,
    pub slug: Option<String>,
    pub last_message_id: Option<i64>,
}

const CHANNEL_COLUMNS: &str = "chat_channels.id, chat_channels.chatable_id, chat_channels.chatable_type, \
     chat_channels.name, chat_channels.description, chat_channels.emoji, chat_channels.status, \
     chat_channels.user_count, chat_channels.auto_join_users, chat_channels.allow_channel_wide_mentions, \
     chat_channels.threading_enabled, chat_channels.slug, chat_channels.last_message_id";

/// `Chat::Channel` statuses.
pub const STATUSES: [&str; 4] = ["open", "read_only", "closed", "archived"];
const OPEN: i32 = 0;
const CLOSED: i32 = 2;

/// secured_public_channel_search's options.
#[derive(Default)]
pub struct Search {
    pub filter: Option<String>,
    pub status: Option<i32>,
    /// chatable_type and chatable_id, both given.
    pub chatable: Option<(String, i64)>,
    pub include_subcategories: bool,
    /// Only channels the viewer follows (starred ones, if starred).
    pub following: bool,
    pub starred: bool,
    pub limit: Option<i64>,
    pub offset: i64,
}

/// `ActiveRecord::Base.sanitize_sql_like`
fn sanitize_sql_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `ChannelsMembershipsController::INDEX_LIMIT`
pub const MEMBERSHIPS_LIMIT: i64 = 50;

/// `Group::AUTO_GROUPS` admins, moderators, staff: `Group::STAFF_GROUPS`.
const STAFF_GROUP_IDS: [i32; 3] = [1, 2, 3];

/// A row of user_chat_channel_memberships.
#[derive(sqlx::FromRow, Clone)]
pub struct MembershipRow {
    pub chat_channel_id: i64,
    pub following: bool,
    pub muted: bool,
    pub notification_level: i32,
    pub last_read_message_id: Option<i64>,
    pub last_viewed_at: NaiveDateTime,
    pub last_viewed_pins_at: Option<NaiveDateTime>,
    pub starred: bool,
}

/// `UserChatChannelMembership::NOTIFICATION_LEVELS`
const NOTIFICATION_LEVELS: [&str; 3] = ["never", "mention", "always"];

fn time_json(t: NaiveDateTime) -> String {
    crate::topic_list::time_json(t)
}

/// What the serializers need besides the channel.
pub struct Context<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub i18n: &'a I18n,
    pub guardian: &'a Guardian,
    pub base_path: &'a str,
    pub config: &'a crate::config::Config,
}

impl Context<'_> {
    /// `ChannelFetcher.secured_public_channels`, through
    /// secured_public_channel_search with filter_on_category_name: the
    /// category channels the viewer may see, filtered, by name.
    pub async fn public_channels(&mut self, search: &Search) -> Result<Vec<ChannelRow>, AppError> {
        if !self.settings.get("enable_public_channels")?.truthy() {
            return Ok(Vec::new());
        }
        let allowed = allowed_channel_ids_sql(self.settings, self.guardian, true)?;
        let mut sql = format!(
            "SELECT {CHANNEL_COLUMNS} FROM chat_channels \
             LEFT JOIN categories ON categories.id = chat_channels.chatable_id \
               AND chat_channels.chatable_type = 'Category' \
             WHERE chat_channels.deleted_at IS NULL AND chat_channels.chatable_type = 'Category' \
               AND chat_channels.id IN ({allowed})"
        );
        if let Some(status) = search.status {
            sql.push_str(&format!(" AND chat_channels.status = {status}"));
        }
        // A chatable the viewer can't see filters nothing.
        if let Some((chatable_type, chatable_id)) = &search.chatable {
            if chatable_type != "Category" {
                return Err(Unsupported("chat channels of chatables other than categories").into());
            }
            if self.can_see_category(*chatable_id).await? {
                let ids: Vec<i64> = if search.include_subcategories {
                    let nesting = self.settings.get("max_category_nesting")?.to_i();
                    sqlx::query_scalar(
                        "WITH RECURSIVE subcategories AS ( \
                             SELECT $1::int AS id, 1 AS depth \
                             UNION \
                             SELECT categories.id, subcategories.depth + 1 \
                             FROM categories JOIN subcategories ON subcategories.id = categories.parent_category_id \
                             WHERE subcategories.depth < $2) \
                         SELECT id::bigint FROM subcategories",
                    )
                    .bind(*chatable_id as i32)
                    .bind(nesting as i32)
                    .fetch_all(&mut *self.conn)
                    .await?
                } else {
                    vec![*chatable_id]
                };
                let ids: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
                sql.push_str(&format!(
                    " AND chat_channels.chatable_id IN ({})",
                    ids.join(", ")
                ));
            }
        }
        let filter = search
            .filter
            .as_deref()
            .filter(|f| !f.trim().is_empty())
            .map(str::to_lowercase);
        let order = if filter.is_some() {
            sql.push_str(
                " AND (LOWER(chat_channels.name) = $1 OR LOWER(chat_channels.name) LIKE $2 \
                   OR LOWER(chat_channels.name) LIKE $3 OR LOWER(chat_channels.slug) = $1 \
                   OR LOWER(chat_channels.slug) LIKE $2 OR LOWER(chat_channels.slug) LIKE $3 \
                   OR categories.name ILIKE $3)",
            );
            // MATCH_QUALITY_EXACT, _PREFIX, _PARTIAL
            "CASE WHEN LOWER(chat_channels.name) = $1 OR LOWER(chat_channels.slug) = $1 THEN 1 \
               WHEN LOWER(chat_channels.name) LIKE $2 OR LOWER(chat_channels.slug) LIKE $2 THEN 2 \
               ELSE 3 END ASC, chat_channels.name ASC, categories.name ASC"
        } else {
            "LOWER(chat_channels.name) ASC"
        };
        if search.following {
            sql.push_str(
                " AND EXISTS (SELECT 1 FROM user_chat_channel_memberships m \
                   WHERE m.chat_channel_id = chat_channels.id AND m.user_id = $4 AND m.following \
                   AND ($5 = FALSE OR m.starred))",
            );
        }
        let limit = search
            .limit
            .unwrap_or(MAX_PUBLIC_CHANNEL_RESULTS)
            .clamp(1, MAX_PUBLIC_CHANNEL_RESULTS);
        sql.push_str(&format!(
            " ORDER BY {order} LIMIT {limit} OFFSET {}",
            search.offset.max(0)
        ));
        let term = filter.unwrap_or_default();
        let like = sanitize_sql_like(&term);
        Ok(sqlx::query_as(&sql)
            .bind(&term)
            .bind(format!("{like}%"))
            .bind(format!("%{like}%"))
            .bind(self.guardian.user_id().unwrap_or(0))
            .bind(search.starred)
            .fetch_all(&mut *self.conn)
            .await?)
    }

    /// `secured_public_channels(guardian, status: :open, following: true)`:
    /// the viewer's followed open category channels.
    pub async fn followed_public_channels(
        &mut self,
        starred: bool,
    ) -> Result<Vec<ChannelRow>, AppError> {
        if self.guardian.user_id().is_none() {
            return Ok(Vec::new());
        }
        self.public_channels(&Search {
            status: Some(OPEN),
            following: true,
            starred,
            ..Search::default()
        })
        .await
    }

    /// `Chat::ChannelMembershipManager.all_for_user`
    pub async fn memberships(&mut self) -> Result<Vec<MembershipRow>, AppError> {
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(Vec::new());
        };
        Ok(sqlx::query_as(
            "SELECT chat_channel_id, following, muted, notification_level, last_read_message_id, \
                    last_viewed_at, last_viewed_pins_at, starred \
             FROM user_chat_channel_memberships WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_all(&mut *self.conn)
        .await?)
    }

    /// `Chat::ChannelUnreadsQuery` with include_missing_memberships and
    /// include_read: each channel's unread, mention and watched thread
    /// counts.
    pub async fn channel_tracking(
        &mut self,
        channel_ids: &[i64],
    ) -> Result<Map<String, Value>, AppError> {
        let mut out = Map::new();
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(out);
        };
        if channel_ids.is_empty() {
            return Ok(out);
        }
        let rows: Vec<(i64, i64, i64, i64)> = sqlx::query_as(
            "WITH limited_channels AS ( \
               SELECT memberships.chat_channel_id, memberships.last_read_message_id, memberships.muted, \
                      chat_channels.threading_enabled \
               FROM user_chat_channel_memberships AS memberships \
               INNER JOIN chat_channels ON chat_channels.id = memberships.chat_channel_id \
               WHERE memberships.user_id = $1 AND memberships.chat_channel_id = ANY($2) LIMIT 1000) \
             SELECT lc.chat_channel_id AS channel_id, \
                    CASE WHEN lc.muted THEN 0 ELSE COALESCE(unread_calc.cnt, 0) END AS unread_count, \
                    COALESCE(mention_calc.cnt, 0) AS mention_count, \
                    COALESCE(watched_calc.cnt, 0) AS watched_threads_unread_count \
             FROM limited_channels lc \
             LEFT JOIN LATERAL ( \
               SELECT (SELECT COUNT(*) FROM chat_messages cm WHERE cm.chat_channel_id = lc.chat_channel_id \
                         AND cm.thread_id IS NULL AND cm.id > COALESCE(lc.last_read_message_id, 0) \
                         AND cm.deleted_at IS NULL) \
                    + (SELECT COUNT(*) FROM chat_threads ct INNER JOIN chat_messages cm ON cm.id = ct.original_message_id \
                         WHERE ct.channel_id = lc.chat_channel_id \
                           AND ct.original_message_id > COALESCE(lc.last_read_message_id, 0) \
                           AND cm.deleted_at IS NULL) \
                    + (SELECT COUNT(*) FROM chat_threads ct INNER JOIN chat_messages cm \
                         ON cm.thread_id = ct.id AND cm.deleted_at IS NULL \
                         WHERE ct.channel_id = lc.chat_channel_id AND NOT lc.threading_enabled AND NOT ct.force \
                           AND cm.id != ct.original_message_id \
                           AND cm.id > COALESCE(lc.last_read_message_id, 0) AND cm.user_id != $1) AS cnt \
             ) unread_calc ON true \
             LEFT JOIN LATERAL ( \
               SELECT COUNT(*) AS cnt FROM notifications n \
               INNER JOIN chat_messages cm ON cm.id = (n.data::json->>'chat_message_id')::bigint \
               LEFT JOIN chat_threads ct ON ct.id = cm.thread_id \
               LEFT JOIN user_chat_thread_memberships uctm ON uctm.thread_id = cm.thread_id AND uctm.user_id = $1 \
               WHERE n.user_id = $1 AND n.notification_type = 29 AND NOT n.read \
                 AND (n.data::json->>'chat_channel_id')::bigint = lc.chat_channel_id \
                 AND (((cm.thread_id IS NULL OR cm.id = ct.original_message_id \
                         OR (NOT lc.threading_enabled AND NOT ct.force)) \
                       AND cm.id > COALESCE(lc.last_read_message_id, 0)) \
                      OR (cm.thread_id IS NOT NULL AND uctm.id IS NOT NULL \
                          AND cm.id > COALESCE(uctm.last_read_message_id, 0))) \
             ) mention_calc ON true \
             LEFT JOIN LATERAL ( \
               SELECT COUNT(*) AS cnt FROM chat_threads ct \
               INNER JOIN user_chat_thread_memberships uctm ON uctm.thread_id = ct.id \
                 AND uctm.user_id = $1 AND uctm.notification_level = 3 \
               INNER JOIN chat_messages cm ON cm.thread_id = ct.id AND cm.chat_channel_id = lc.chat_channel_id \
               WHERE ct.channel_id = lc.chat_channel_id AND (lc.threading_enabled OR ct.force) \
                 AND cm.id != ct.original_message_id AND cm.user_id != $1 AND cm.deleted_at IS NULL \
                 AND cm.id > COALESCE(uctm.last_read_message_id, 0) \
             ) watched_calc ON true \
             UNION ALL \
             SELECT chat_channels.id, 0, 0, 0 FROM chat_channels \
             LEFT JOIN user_chat_channel_memberships ON user_chat_channel_memberships.chat_channel_id = chat_channels.id \
               AND user_chat_channel_memberships.user_id = $1 \
             WHERE chat_channels.id = ANY($2) AND user_chat_channel_memberships.id IS NULL \
             GROUP BY chat_channels.id",
        )
        .bind(user_id)
        .bind(channel_ids)
        .fetch_all(&mut *self.conn)
        .await?;
        for (id, unread, mention, watched) in rows {
            out.insert(
                id.to_string(),
                json!({
                    "mention_count": mention,
                    "unread_count": unread,
                    "watched_threads_unread_count": watched,
                }),
            );
        }
        Ok(out)
    }

    /// `Chat::Thread.viewable_by_user(user).exists?`, refused while threads
    /// aren't ported: false only when the site has none.
    async fn refuse_threads(&mut self) -> Result<(), AppError> {
        let any: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM chat_threads)")
            .fetch_one(&mut *self.conn)
            .await?;
        if any {
            return Err(Unsupported("chat threads").into());
        }
        Ok(())
    }

    /// `Category.post_create_allowed(guardian).where(id: ids).pluck(:id)`
    async fn post_allowed_category_ids(&mut self, ids: &[i64]) -> Result<Vec<i64>, AppError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT categories.id::bigint FROM categories WHERE categories.id = ANY($1) AND {}",
            super::categories_scoped_to(self.guardian, "1, 2")
        );
        Ok(sqlx::query_scalar(&sql)
            .bind(ids)
            .fetch_all(&mut *self.conn)
            .await?)
    }

    /// GET /chat/api/me/channels: Chat::ListUserChannels with
    /// Chat::ChannelIndexSerializer.
    pub async fn me_channels(&mut self) -> Result<Value, AppError> {
        let Some(user_id) = self.guardian.user_id() else {
            return Err(Unsupported("anonymous access to chat channels").into());
        };
        // ChannelFetcher.structured
        let memberships = self.memberships().await?;
        let mut public = self.followed_public_channels(false).await?;
        let dms: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM direct_message_users WHERE user_id = $1)",
        )
        .bind(user_id)
        .fetch_one(&mut *self.conn)
        .await?;
        if dms {
            return Err(Unsupported("chat direct messages").into());
        }
        if public.len() as i64 == MAX_PUBLIC_CHANNEL_RESULTS {
            for starred in self.followed_public_channels(true).await? {
                if !public.iter().any(|c| c.id == starred.id) {
                    public.push(starred);
                }
            }
        }
        let ids: Vec<i64> = public.iter().map(|c| c.id).collect();
        let channel_tracking = self.channel_tracking(&ids).await?;
        // inject_unread_thread_overview, inject_has_threads
        self.refuse_threads().await?;
        let category_ids: Vec<i64> = public.iter().map(|c| c.chatable_id).collect();
        let post_allowed = self.post_allowed_category_ids(&category_ids).await?;

        let mut public_json = Vec::new();
        for channel in &public {
            let membership = memberships.iter().find(|m| m.chat_channel_id == channel.id);
            public_json.push(
                self.channel(channel, membership, Some(true), Some(&post_allowed))
                    .await?,
            );
        }
        let mut out = Map::new();
        out.insert("public_channels".into(), Value::Array(public_json));
        out.insert("direct_message_channels".into(), json!([]));
        out.insert(
            "tracking".into(),
            json!({"channel_tracking": channel_tracking, "thread_tracking": {}}),
        );
        out.insert(
            "meta".into(),
            json!({"message_bus_last_ids": {
                "channel_metadata": 0,
                "channel_edits": 0,
                "channel_status": 0,
                "new_channel": 0,
                "archive_status": 0,
                "user_tracking_state": 0,
                "user_has_threads": 0,
            }}),
        );
        out.insert("unread_thread_overview".into(), json!({}));
        out.insert("has_threads".into(), json!(false));
        // PresenceChannel("/chat/online").state: presence isn't ported.
        out.insert(
            "global_presence_channel_state".into(),
            json!({"count": 0, "last_message_id": null, "users": []}),
        );
        Ok(Value::Object(out))
    }

    /// GET /chat/api/channels: Chat::Api::ChannelsController#index, the
    /// channels serialized with the viewer's memberships.
    pub async fn index(&mut self, search: &Search) -> Result<Vec<Value>, AppError> {
        let memberships = self.memberships().await?;
        let channels = self.public_channels(search).await?;
        let mut out = Vec::new();
        for channel in &channels {
            let membership = memberships.iter().find(|m| m.chat_channel_id == channel.id);
            out.push(self.channel(channel, membership, None, None).await?);
        }
        Ok(out)
    }

    /// `Chat::Channel.find` (live channels) and
    /// `ensure_can_join_chat_channel!`, as the channel actions find
    /// theirs.
    pub async fn find_joinable(&mut self, id: &str) -> Result<Found, AppError> {
        // An id casts as ActiveModel's integer type casts: digits first,
        // else no id at all.
        let numeric = id
            .trim_start()
            .trim_start_matches(['+', '-'])
            .starts_with(|c: char| c.is_ascii_digit());
        if !numeric {
            return Ok(Found::NotFound);
        }
        let sql = format!(
            "SELECT {CHANNEL_COLUMNS} FROM chat_channels WHERE id = $1 AND deleted_at IS NULL"
        );
        let channel: Option<ChannelRow> = sqlx::query_as(&sql)
            .bind(crate::ruby::to_i(id))
            .fetch_optional(&mut *self.conn)
            .await?;
        let Some(channel) = channel else {
            return Ok(Found::NotFound);
        };
        if channel.chatable_type != "Category" {
            return Err(Unsupported("chat direct messages").into());
        }
        if !self.can_join(&channel, None).await? {
            return Ok(Found::Forbidden);
        }
        Ok(Found::Channel(channel))
    }

    /// GET /chat/api/channels/:id: the channel with the viewer's
    /// membership (`membership_for`, followed or not).
    pub async fn show(&mut self, channel: &ChannelRow) -> Result<Value, AppError> {
        let membership = self
            .memberships()
            .await?
            .into_iter()
            .find(|m| m.chat_channel_id == channel.id);
        Ok(json!({"channel": self.channel(channel, membership.as_ref(), None, None).await?}))
    }

    /// GET /chat/api/channels/:id/memberships:
    /// Chat::ChannelMembershipsQuery with
    /// Chat::MemberListChannelMembershipSerializer, and the next page's url.
    pub async fn members(
        &mut self,
        channel: &ChannelRow,
        offset: i64,
        limit: i64,
        username: Option<&str>,
    ) -> Result<Value, AppError> {
        let settings = self.settings;
        let by_username = settings.get("prioritize_username_in_ux")?.truthy()
            || !settings.get("enable_names")?.truthy();
        // Real, active, unstaged, unsuspended, unsilenced users following
        // the channel.
        let mut sql = String::from(
            "SELECT users.id, users.username, users.name, users.uploaded_avatar_id, \
                    COALESCE(user_options.chat_enabled, FALSE) AS chat_enabled \
             FROM user_chat_channel_memberships m \
             JOIN users ON users.id = m.user_id \
             LEFT JOIN user_options ON user_options.user_id = users.id \
             WHERE m.chat_channel_id = $1 AND m.following \
               AND users.id > 0 \
               AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = users.id) \
               AND users.active AND NOT users.staged \
               AND (users.suspended_till IS NULL OR users.suspended_till <= $2) \
               AND (users.silenced_till IS NULL OR users.silenced_till <= $2)",
        );
        // A read restricted category's channel: only members of the
        // groups that can see it, and staff.
        let restricted: Option<bool> =
            sqlx::query_scalar("SELECT read_restricted FROM categories WHERE id = $1")
                .bind(channel.chatable_id)
                .fetch_optional(&mut *self.conn)
                .await?;
        if restricted == Some(true) {
            let mut groups: Vec<i32> =
                sqlx::query_scalar("SELECT group_id FROM category_groups WHERE category_id = $1")
                    .bind(channel.chatable_id as i32)
                    .fetch_all(&mut *self.conn)
                    .await?;
            groups.extend(STAFF_GROUP_IDS);
            let groups: Vec<String> = groups.iter().map(|g| g.to_string()).collect();
            sql.push_str(&format!(
                " AND m.user_id IN (SELECT user_id FROM group_users WHERE group_id IN ({}))",
                groups.join(", ")
            ));
        }
        let username = username.filter(|u| !u.trim().is_empty());
        if username.is_some() {
            sql.push_str(if by_username {
                " AND users.username_lower ILIKE $3"
            } else {
                " AND (LOWER(users.name) ILIKE $3 OR users.username_lower ILIKE $3)"
            });
        }
        sql.push_str(if by_username {
            " ORDER BY users.username_lower ASC"
        } else {
            " ORDER BY users.name ASC, users.username_lower ASC"
        });
        sql.push_str(" OFFSET $4 LIMIT $5");
        let rows: Vec<MemberRow> = sqlx::query_as(&sql)
            .bind(channel.id)
            .bind(crate::clock::now_naive())
            .bind(format!("%{}%", username.unwrap_or_default()))
            .bind(offset)
            .bind(limit)
            .fetch_all(&mut *self.conn)
            .await?;

        // Chat::BasicUserSerializer: can_chat is the viewer's.
        let can_chat = super::enabled(settings)?
            && super::can_chat(&mut *self.conn, settings, self.guardian).await?;
        let enable_names = settings.get("enable_names")?.truthy();
        let status = settings.get("enable_user_status")?.truthy();
        let logo = crate::admin_users::logo_small_url(&mut *self.conn, settings).await?;
        let urls = crate::url::Urls {
            config: self.config,
            settings,
        };
        let mut memberships = Vec::new();
        for MemberRow {
            id,
            username,
            name,
            uploaded_avatar_id: avatar,
            chat_enabled,
        } in rows
        {
            if status {
                let has_status: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM user_statuses WHERE user_id = $1 AND (ends_at IS NULL OR ends_at > now()))",
                )
                .bind(id)
                .fetch_one(&mut *self.conn)
                .await?;
                if has_status {
                    return Err(Unsupported("user status").into());
                }
            }
            let mut user = Map::new();
            user.insert("id".into(), json!(id));
            user.insert("username".into(), json!(username));
            if enable_names {
                user.insert("name".into(), json!(name));
            }
            user.insert(
                "avatar_template".into(),
                json!(crate::avatar::avatar_template(
                    &urls,
                    id,
                    &username,
                    avatar,
                    logo.as_deref()
                )?),
            );
            user.insert("can_chat".into(), json!(can_chat));
            user.insert("has_chat_enabled".into(), json!(can_chat && chat_enabled));
            memberships.push(json!({"user": user}));
        }
        Ok(json!({
            "memberships": memberships,
            "meta": {
                "total_rows": channel.user_count,
                "load_more_url": format!(
                    "/chat/api/channels/{}/memberships?offset={}&limit={limit}&username={}",
                    channel.id,
                    offset + limit,
                    username.unwrap_or_default()
                ),
            },
        }))
    }

    /// The category a channel belongs to, as Category::find gives it.
    async fn category(
        &mut self,
        id: i64,
    ) -> Result<Option<crate::categories::CategoryRow>, AppError> {
        let rows = crate::categories::Categories::load_all(&mut *self.conn).await?;
        Ok(rows.into_iter().find(|r| i64::from(r.id) == id))
    }

    /// `Chat::ChannelSerializer`. `can_join` is the serializer's
    /// can_join_chat_channel option when given.
    pub async fn channel(
        &mut self,
        channel: &ChannelRow,
        membership: Option<&MembershipRow>,
        can_join: Option<bool>,
        post_allowed_category_ids: Option<&[i64]>,
    ) -> Result<Value, AppError> {
        if channel.chatable_type != "Category" {
            return Err(Unsupported("chat direct messages").into());
        }
        let g = self.guardian;
        let staff = g.is_staff();
        let Some(category) = self.category(channel.chatable_id).await? else {
            return Err(Unsupported("chat channels of deleted categories").into());
        };
        let pinned_setting = self.settings.get("chat_pinned_messages")?.truthy();
        let mut out = Map::new();
        out.insert("id".into(), json!(channel.id));
        // can_edit_chat_channel? for a category channel: staff.
        if staff {
            out.insert("auto_join_users".into(), json!(channel.auto_join_users));
        }
        out.insert(
            "allow_channel_wide_mentions".into(),
            json!(channel.allow_channel_wide_mentions),
        );
        // BasicCategorySerializer, without a scope (no can_edit); plugin
        // custom fields aren't ported.
        let mut cats = crate::categories::Categories {
            conn: &mut *self.conn,
            settings: self.settings,
            i18n: self.i18n,
            guardian: g,
            base_path: self.base_path,
            topic_url_via_slug: false,
        };
        let mut chatable = cats.basic_fields(&category).await?;
        cats.uploads(&mut chatable, &category).await?;
        out.insert("chatable".into(), Value::Object(chatable));
        out.insert("chatable_id".into(), json!(channel.chatable_id));
        out.insert("chatable_type".into(), json!(channel.chatable_type));
        // Category#url
        let full_slug = crate::category::Category::find(&mut *self.conn, category.id)
            .await?
            .ok_or(Unsupported("chat channels of deleted categories"))?
            .full_slug(&mut *self.conn)
            .await?;
        out.insert(
            "chatable_url".into(),
            json!(format!("{}/c/{full_slug}", self.base_path)),
        );
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
        // CategoryChannel#title: the channel's name, else its category's.
        let title = channel
            .name
            .clone()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| category.name.clone());
        out.insert("title".into(), json!(title));
        out.insert(
            "unicode_title".into(),
            json!(crate::emoji::gsub_emoji_to_unicode(&title)),
        );
        out.insert("slug".into(), json!(channel.slug));
        out.insert(
            "status".into(),
            json!(
                STATUSES
                    .get(channel.status as usize)
                    .copied()
                    .unwrap_or("open")
            ),
        );
        // The archive's progress, for staff: refused until archiving is
        // ported.
        if staff {
            let archived: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM chat_channel_archives WHERE chat_channel_id = $1)",
            )
            .bind(channel.id)
            .fetch_one(&mut *self.conn)
            .await?;
            if archived {
                return Err(Unsupported("archived chat channels").into());
            }
        }
        out.insert("memberships_count".into(), json!(channel.user_count));
        if let Some(m) = membership {
            out.insert(
                "current_user_membership".into(),
                self.membership(channel, m, pinned_setting).await?,
            );
        }
        out.insert(
            "meta".into(),
            self.meta(channel, can_join, post_allowed_category_ids)
                .await?,
        );
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
        // include_last_message?: can_preview_chat_channel?, seeing the
        // category. Without one, the NullMessage, dated now.
        if self.can_see_category(channel.chatable_id).await? {
            if channel.last_message_id.is_some() {
                return Err(Unsupported("chat channels' last messages").into());
            }
            out.insert(
                "last_message".into(),
                json!({
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
                }),
            );
        }
        Ok(Value::Object(out))
    }

    /// `can_see_category?`
    async fn can_see_category(&mut self, category_id: i64) -> Result<bool, AppError> {
        let read_restricted: Option<bool> =
            sqlx::query_scalar("SELECT read_restricted FROM categories WHERE id = $1")
                .bind(category_id)
                .fetch_optional(&mut *self.conn)
                .await?;
        match read_restricted {
            None => Ok(false),
            Some(false) => Ok(true),
            Some(true) => {
                let secure = self
                    .guardian
                    .secure_category_ids(&mut *self.conn, self.settings)
                    .await?;
                Ok(secure.contains(&(category_id as i32)))
            }
        }
    }

    /// `Chat::BaseChannelMembershipSerializer`
    async fn membership(
        &mut self,
        channel: &ChannelRow,
        m: &MembershipRow,
        pinned_setting: bool,
    ) -> Result<Value, AppError> {
        let authenticated = self.guardian.is_authenticated();
        let mut out = Map::new();
        out.insert("following".into(), json!(m.following));
        out.insert("muted".into(), json!(m.muted));
        out.insert(
            "notification_level".into(),
            json!(
                NOTIFICATION_LEVELS
                    .get(m.notification_level as usize)
                    .copied()
            ),
        );
        out.insert("chat_channel_id".into(), json!(m.chat_channel_id));
        out.insert("last_read_message_id".into(), json!(m.last_read_message_id));
        out.insert("last_viewed_at".into(), json!(time_json(m.last_viewed_at)));
        if pinned_setting && authenticated {
            out.insert(
                "last_viewed_pins_at".into(),
                json!(m.last_viewed_pins_at.map(time_json)),
            );
        }
        if authenticated {
            out.insert("starred".into(), json!(m.starred));
        }
        if pinned_setting && authenticated {
            // has_unseen_pins?: a pin by someone else since the member last
            // looked at the pins.
            let unseen: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM chat_pinned_messages WHERE chat_channel_id = $1 \
                   AND pinned_by_id <> $2 AND ($3::timestamp IS NULL OR created_at > $3))",
            )
            .bind(channel.id)
            .bind(i64::from(self.guardian.user_id().unwrap_or(0)))
            .bind(m.last_viewed_pins_at)
            .fetch_one(&mut *self.conn)
            .await?;
            out.insert("has_unseen_pins".into(), json!(unseen));
        }
        Ok(Value::Object(out))
    }

    /// `can_join_chat_channel?`: a member who can chat, see the channel's
    /// category and post in it (can_post_in_chatable?: the post-allowed
    /// ids when given, else can_post_in_category?).
    pub async fn can_join(
        &mut self,
        channel: &ChannelRow,
        post_allowed_category_ids: Option<&[i64]>,
    ) -> Result<bool, AppError> {
        let g = self.guardian;
        if !g.is_authenticated() || !super::can_chat(&mut *self.conn, self.settings, g).await? {
            return Ok(false);
        }
        if !self.can_see_category(channel.chatable_id).await? {
            return Ok(false);
        }
        Ok(match post_allowed_category_ids {
            Some(ids) => g.is_admin() || ids.contains(&channel.chatable_id),
            None => !self
                .post_allowed_category_ids(&[channel.chatable_id])
                .await?
                .is_empty(),
        })
    }

    /// The serializer's `meta`: the message bus ids and the viewer's
    /// permissions on the channel.
    async fn meta(
        &mut self,
        channel: &ChannelRow,
        can_join: Option<bool>,
        post_allowed_category_ids: Option<&[i64]>,
    ) -> Result<Value, AppError> {
        let g = self.guardian;
        let settings = self.settings;
        let mut ids = Map::new();
        ids.insert("channel_message_bus_last_id".into(), json!(0));
        if g.is_authenticated() {
            ids.insert("new_messages".into(), json!(0));
            ids.insert("new_mentions".into(), json!(0));
            ids.insert("kick".into(), json!(0));
        }
        let can_chat = super::can_chat(&mut *self.conn, settings, g).await?;
        let can_preview = self.can_see_category(channel.chatable_id).await?;
        let joinable = self.can_join(channel, post_allowed_category_ids).await?;
        let mut meta = Map::new();
        meta.insert("message_bus_last_ids".into(), Value::Object(ids));
        meta.insert(
            "can_join_chat_channel".into(),
            json!(can_join.unwrap_or(joinable)),
        );
        // can_flag_in_chat_channel?: can_modify_channel_message? and
        // can_join_chat_channel?.
        let modifiable = if g.is_staff() {
            channel.status == OPEN || channel.status == CLOSED
        } else {
            channel.status == OPEN
        };
        meta.insert("can_flag".into(), json!(modifiable && joinable));
        // !can_create_chat_message?: SpamRule::AutoSilence.prevent_posting?
        let silenced = match g.user() {
            None => true,
            Some(u) => {
                g.is_silenced()
                    || (u.trust_level < 1
                        && !u.admin
                        && !u.moderator
                        && crate::current_user::autosilence_pending(
                            &mut *self.conn,
                            settings,
                            u.id,
                        )
                        .await?)
            }
        };
        meta.insert("user_silenced".into(), json!(silenced));
        // can_moderate_chat?: staff, or the category's group moderators.
        let can_moderate = g.is_staff() || {
            settings.get("enable_category_group_moderation")?.truthy()
                && match g.user_id() {
                    None => false,
                    Some(uid) => {
                        sqlx::query_scalar::<_, bool>(
                            "SELECT EXISTS (SELECT 1 FROM category_moderation_groups cmg \
                         JOIN group_users gu ON gu.group_id = cmg.group_id \
                         WHERE cmg.category_id = $1 AND gu.user_id = $2)",
                        )
                        .bind(channel.chatable_id as i32)
                        .bind(uid)
                        .fetch_one(&mut *self.conn)
                        .await?
                    }
                }
        };
        meta.insert("can_moderate".into(), json!(can_moderate));
        meta.insert(
            "can_delete_self".into(),
            json!(settings.get("max_post_deletions_per_day")?.to_i() >= 1),
        );
        meta.insert("can_delete_others".into(), json!(can_moderate));
        meta.insert("can_remove_members".into(), json!(g.is_admin()));
        // can_manage_chat_channel_pins?
        let pins = can_chat
            && can_preview
            && g.in_setting_groups(settings, "chat_pinning_messages_allowed_groups")?;
        meta.insert("can_manage_pins".into(), json!(pins));
        Ok(Value::Object(meta))
    }
}

/// A channel member, as the members list reads them.
#[derive(sqlx::FromRow)]
struct MemberRow {
    id: i32,
    username: String,
    name: Option<String>,
    uploaded_avatar_id: Option<i32>,
    chat_enabled: bool,
}

/// A channel action's channel.
pub enum Found {
    Channel(ChannelRow),
    NotFound,
    Forbidden,
}
