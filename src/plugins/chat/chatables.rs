//! `Chat::SearchChatable` (GET /chat/api/chatables, the new message
//! modal's search) with `Chat::ChatablesSerializer`: users, groups, the
//! viewer's direct message channels and category channels matching a
//! term, each with its match quality.

use serde_json::{Value, json};

use super::channels::{CHANNEL_COLUMNS, ChannelRow, Context, MembershipRow, Search};
use super::direct_messages::DmUser;
use crate::AppError;

/// `SearchChatable::SEARCH_RESULT_LIMIT`
const SEARCH_RESULT_LIMIT: i64 = 10;
/// `ChannelFetcher::MAX_DM_CHANNEL_RESULTS`
const MAX_DM_CHANNEL_RESULTS: i64 = 75;
/// `ChannelFetcher::MATCH_QUALITY_*`
const EXACT: i32 = 1;
const PREFIX: i32 = 2;
const PARTIAL: i32 = 3;

/// A user the search found.
#[derive(sqlx::FromRow)]
struct FoundUser {
    id: i32,
    username: String,
    name: Option<String>,
    uploaded_avatar_id: Option<i32>,
    chat_enabled: Option<bool>,
    username_lower: String,
}

/// The service's params.
pub struct Params {
    pub term: String,
    pub include_users: bool,
    pub include_groups: bool,
    pub include_category_channels: bool,
    pub include_direct_message_channels: bool,
    pub excluded_memberships_channel_id: Option<i64>,
}

/// `ActiveRecord::Base.sanitize_sql_like`
fn like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The match quality of `name` against the term (exact, prefix, partial).
fn quality(name: &str, term: &str) -> i32 {
    if name == term {
        EXACT
    } else if name.starts_with(term) {
        PREFIX
    } else {
        PARTIAL
    }
}

impl Context<'_> {
    /// `Chat::SearchChatable` and ChatablesSerializer.
    pub async fn search_chatables(&mut self, p: &Params) -> Result<Value, AppError> {
        let s = self.settings;
        let g = self.guardian;
        // after_validation: downcased, stripped, leading @ and # gone.
        let term = p.term.to_lowercase();
        let term = term.trim().trim_start_matches(['@', '#']).to_string();
        let can_create_dm = g.is_staff() || super::can_direct_message(s, g)?;

        let users = if p.include_users && can_create_dm {
            self.search_users(&term, p.excluded_memberships_channel_id)
                .await?
        } else {
            Vec::new()
        };
        let groups = if p.include_groups && can_create_dm {
            self.search_groups(&term).await?
        } else {
            Vec::new()
        };
        let category_channels =
            if p.include_category_channels && s.get("enable_public_channels")?.truthy() {
                self.public_channels(&Search {
                    filter: Some(term.clone()),
                    status: Some(0),
                    limit: Some(SEARCH_RESULT_LIMIT),
                    skip_category_name: true,
                    ..Search::default()
                })
                .await?
            } else {
                Vec::new()
            };
        let dm_channels = if p.include_direct_message_channels && can_create_dm {
            self.search_dm_channels(&term, p.include_users && !users.is_empty())
                .await?
        } else {
            Vec::new()
        };

        let mut out_users = Vec::new();
        for (user, q) in &users {
            out_users.push(json!({
                "identifier": format!("u-{}", user.id),
                "model": self.chatable_user(Some(user)).await?,
                "type": "user",
                "match_quality": q,
            }));
        }
        let mut out_groups = Vec::new();
        for (group_id, q) in &groups {
            out_groups.push(json!({
                "identifier": format!("g-{group_id}"),
                "model": self.chatable_group(*group_id).await?,
                "type": "group",
                "match_quality": q,
            }));
        }
        let mut out_dms = Vec::new();
        for (channel, q) in &dm_channels {
            out_dms.push(self.chatable_channel(channel, *q).await?);
        }
        let mut out_categories = Vec::new();
        for channel in &category_channels {
            // The fetcher's match quality: by name or slug when filtered,
            // partial otherwise.
            let q = if term.is_empty() {
                PARTIAL
            } else {
                let name = channel.name.as_deref().unwrap_or_default().to_lowercase();
                let slug = channel.slug.as_deref().unwrap_or_default().to_lowercase();
                quality(&name, &term).min(quality(&slug, &term))
            };
            out_categories.push(self.chatable_channel(channel, q).await?);
        }
        Ok(json!({
            "users": out_users,
            "groups": out_groups,
            "direct_message_channels": out_dms,
            "category_channels": out_categories,
        }))
    }

    /// A channel with the viewer's membership (among all of theirs).
    async fn chatable_channel(&mut self, channel: &ChannelRow, q: i32) -> Result<Value, AppError> {
        let membership: Option<MembershipRow> = sqlx::query_as(
            "SELECT chat_channel_id, following, muted, notification_level, last_read_message_id, \
                    last_viewed_at, last_viewed_pins_at, starred \
             FROM user_chat_channel_memberships WHERE user_id = $1 AND chat_channel_id = $2",
        )
        .bind(self.guardian.user_id())
        .bind(channel.id)
        .fetch_optional(&mut *self.conn)
        .await?;
        Ok(json!({
            "identifier": format!("c-{}", channel.id),
            "type": "channel",
            "model": self.channel(channel, membership.as_ref(), None, None).await?,
            "match_quality": q,
        }))
    }

    /// `search_users`: UserSearch's scoped users (a blank term) or its
    /// search, real users only, outside the excluded channel's members
    /// when the viewer may preview it, by match quality then username.
    async fn search_users(
        &mut self,
        term: &str,
        excluded_channel: Option<i64>,
    ) -> Result<Vec<(DmUser, i32)>, AppError> {
        let settings = self.settings;
        let guardian = self.guardian;
        let mut search = match crate::user_search::UserSearch::new(
            &mut *self.conn,
            settings,
            guardian,
            term,
            crate::user_search::Options {
                limit: Some(SEARCH_RESULT_LIMIT),
                ..Default::default()
            },
        )
        .await?
        {
            Ok(search) => search,
            Err(_) => return Ok(Vec::new()),
        };
        let (joins, condition, ids) = if term.is_empty() {
            let (joins, condition) = search.scoped_users_sql()?;
            (joins, condition, None)
        } else {
            let ids = search.search_ids().await?;
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            (String::new(), "TRUE".to_string(), Some(ids))
        };
        drop(search);
        let mut extra = String::new();
        if let Some(channel_id) = excluded_channel {
            let channel: Option<ChannelRow> = sqlx::query_as(&format!(
                "SELECT {CHANNEL_COLUMNS} FROM chat_channels WHERE chat_channels.id = $1 AND chat_channels.deleted_at IS NULL"
            ))
            .bind(channel_id)
            .fetch_optional(&mut *self.conn)
            .await?;
            if let Some(channel) = channel
                && self.can_preview(&channel).await?
            {
                extra = format!(
                    " AND NOT EXISTS (SELECT 1 FROM user_chat_channel_memberships \
                       WHERE user_id = users.id AND chat_channel_id = {channel_id})"
                );
            }
        }
        let rows: Vec<FoundUser> = sqlx::query_as(&format!(
                "SELECT users.id, users.username, users.name, users.uploaded_avatar_id, \
                        (SELECT chat_enabled FROM user_options WHERE user_id = users.id) AS chat_enabled, users.username_lower \
                 FROM users {joins} \
                 WHERE {condition} AND ($1::int[] IS NULL OR users.id = ANY($1)) \
                   AND users.id > 0 \
                   AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = users.id){extra}"
            ))
            .bind(&ids)
            .fetch_all(&mut *self.conn)
            .await?;
        let mut users: Vec<(DmUser, i32, String)> = rows
            .into_iter()
            .map(|r| {
                let q = quality(&r.username_lower, term);
                (
                    DmUser {
                        id: r.id,
                        username: r.username,
                        name: r.name,
                        uploaded_avatar_id: r.uploaded_avatar_id,
                        chat_enabled: r.chat_enabled,
                    },
                    q,
                    r.username_lower,
                )
            })
            .collect();
        users.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2)));
        Ok(users.into_iter().map(|(u, q, _)| (u, q)).collect())
    }

    /// `search_groups`: visible groups whose members are visible, by name
    /// or full name, best matches first.
    async fn search_groups(&mut self, term: &str) -> Result<Vec<(i32, i32)>, AppError> {
        let g = self.guardian;
        let l = like(term);
        Ok(sqlx::query_as(&format!(
            "SELECT groups.id, CASE WHEN LOWER(groups.name) = $1 THEN {EXACT} \
                    WHEN LOWER(groups.name) LIKE $2 THEN {PREFIX} ELSE {PARTIAL} END AS match_quality \
             FROM groups WHERE {} AND {} \
               AND (LOWER(groups.name) = $1 OR LOWER(groups.name) LIKE $2 OR LOWER(groups.name) LIKE $3 \
                    OR LOWER(groups.full_name) LIKE $3) \
             ORDER BY match_quality ASC, groups.name ASC LIMIT {SEARCH_RESULT_LIMIT}",
            crate::groups::visible_groups_where(g, "groups"),
            crate::groups::members_visible_groups_where(g, "groups"),
        ))
        .bind(term)
        .bind(format!("{l}%"))
        .bind(format!("%{l}%"))
        .fetch_all(&mut *self.conn)
        .await?)
    }

    /// Chat::ChatableGroupSerializer: BasicGroupSerializer, then whether
    /// chat is on, how many of its human members have it on, and whether
    /// a DM with all of them (and the viewer) fits the limit.
    async fn chatable_group(&mut self, group_id: i32) -> Result<Value, AppError> {
        let basic: crate::groups::BasicGroup = sqlx::query_as(&format!(
            "SELECT {} FROM groups g LEFT JOIN uploads u ON u.id = g.flair_upload_id WHERE g.id = $1",
            crate::groups::BASIC_GROUP_COLUMNS
        ))
        .bind(group_id)
        .fetch_one(&mut *self.conn)
        .await?;
        let owner: Option<bool> = sqlx::query_scalar(
            "SELECT owner FROM group_users WHERE group_id = $1 AND user_id = $2",
        )
        .bind(group_id)
        .bind(self.guardian.user_id())
        .fetch_optional(&mut *self.conn)
        .await?;
        let mut out = basic.json(
            self.i18n,
            self.guardian,
            self.settings,
            owner.map(|o| (true, o)),
        )?;
        let enabled = super::enabled(self.settings)?;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users INNER JOIN group_users ON group_users.user_id = users.id \
             INNER JOIN user_options ON user_options.user_id = users.id \
             WHERE group_users.group_id = $1 AND users.id > 0 AND user_options.chat_enabled",
        )
        .bind(group_id)
        .fetch_one(&mut *self.conn)
        .await?;
        let max = self.settings.get("chat_max_direct_message_users")?.to_i();
        if let Value::Object(map) = &mut out {
            map.insert("chat_enabled".into(), json!(enabled));
            map.insert("chat_enabled_user_count".into(), json!(count));
            map.insert("can_chat".into(), json!(enabled && count < max));
        }
        Ok(out)
    }

    /// `secured_direct_message_channels_search(filter: term)`: the
    /// viewer's DM channels with a member matching (the blank term too, as
    /// Rails' `if options[:filter]` takes ""), best match then latest
    /// message first; 1:1s dropped when users were found.
    async fn search_dm_channels(
        &mut self,
        term: &str,
        users_found: bool,
    ) -> Result<Vec<(ChannelRow, i32)>, AppError> {
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(Vec::new());
        };
        let l = like(term);
        let rows: Vec<(i64, i32)> = sqlx::query_as(&format!(
            "SELECT chat_channels.id, MIN(CASE WHEN users.username_lower = $2 THEN {EXACT} \
                    WHEN users.username_lower LIKE $3 THEN {PREFIX} ELSE {PARTIAL} END) AS match_quality \
             FROM chat_channels \
             LEFT JOIN chat_messages last_message ON last_message.id = chat_channels.last_message_id \
             INNER JOIN user_chat_channel_memberships m ON m.chat_channel_id = chat_channels.id \
             INNER JOIN users ON users.id = m.user_id \
             WHERE chat_channels.deleted_at IS NULL AND chat_channels.id IN ( \
                 SELECT chat_channels.id FROM chat_channels \
                 INNER JOIN direct_message_channels ON direct_message_channels.id = chat_channels.chatable_id \
                   AND chat_channels.chatable_type = 'DirectMessage' \
                 INNER JOIN direct_message_users ON direct_message_users.direct_message_channel_id = direct_message_channels.id \
                 WHERE direct_message_users.user_id = $1) \
               AND (users.username_lower = $2 OR users.username_lower LIKE $3 OR users.username_lower LIKE $4 \
                    OR LOWER(chat_channels.name) LIKE $4 OR LOWER(chat_channels.slug) LIKE $4) \
             GROUP BY chat_channels.id, last_message.id \
             ORDER BY match_quality ASC, last_message.created_at DESC NULLS LAST \
             LIMIT {MAX_DM_CHANNEL_RESULTS}"
        ))
        .bind(user_id)
        .bind(term)
        .bind(format!("{l}%"))
        .bind(format!("%{l}%"))
        .fetch_all(&mut *self.conn)
        .await?;
        let mut out = Vec::new();
        for (id, q) in rows {
            let channel: ChannelRow = sqlx::query_as(&format!(
                "SELECT {CHANNEL_COLUMNS} FROM chat_channels WHERE chat_channels.id = $1"
            ))
            .bind(id)
            .fetch_one(&mut *self.conn)
            .await?;
            if users_found {
                // allowed_user_ids less the viewer: a 1:1 has one at most.
                let others: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM direct_message_users WHERE direct_message_channel_id = $1 AND user_id <> $2",
                )
                .bind(channel.chatable_id)
                .bind(user_id)
                .fetch_one(&mut *self.conn)
                .await?;
                if others <= 1 {
                    continue;
                }
            }
            out.push((channel, q));
        }
        Ok(out)
    }
}
