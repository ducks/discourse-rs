//! A channel's messages as GET /chat/api/channels/:id/messages serves
//! them: Chat::ListChannelMessages over Chat::MessagesQuery, serialized
//! by Chat::MessagesSerializer and Chat::MessageSerializer.
//!
//! Refused as Unsupported until their slices port them: threads (slice
//! 5), uploads, webhook events and blocks (slice 6), and user status.
//! The two writes Rails defers (Scheduler::Defer) to after the response,
//! the membership's last_viewed_at and the user's last chat channel, are
//! made before it.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};

use super::channels::{ChannelRow, Context, Found};
use crate::{AppError, Unsupported};

/// `Chat::MessagesQuery::PAST_MESSAGE_LIMIT`, and FUTURE_MESSAGE_LIMIT.
const AROUND_LIMIT: i64 = 25;
/// `Chat::MessagesQuery::MAX_PAGE_SIZE`
const MAX_PAGE_SIZE: i64 = 500;
/// `Chat::Message::EXCERPT_LENGTH`
const EXCERPT_LENGTH: usize = 150;
/// `Chat::LAST_CHAT_CHANNEL_ID`
pub const LAST_CHAT_CHANNEL_ID: &str = "last_chat_channel_id";
/// Chat::Message's polymorphic name, for bookmarks and upload references.
const POLYMORPHIC_NAME: &str = "ChatMessage";
/// `Discourse::SYSTEM_USER_ID`
const SYSTEM_USER_ID: i32 = -1;

/// Chat::ListChannelMessages' params, past the contract (cast, and
/// page_size within 1..=max_page_size).
pub struct ListParams {
    pub page_size: i64,
    pub target_message_id: Option<i64>,
    /// "past" or "future".
    pub direction: Option<String>,
    pub fetch_from_last_read: bool,
    pub target_date: Option<String>,
}

#[derive(sqlx::FromRow, Clone)]
struct MessageRow {
    id: i64,
    user_id: Option<i32>,
    created_at: NaiveDateTime,
    deleted_at: Option<NaiveDateTime>,
    deleted_by_id: Option<i32>,
    in_reply_to_id: Option<i64>,
    message: Option<String>,
    cooked: Option<String>,
    thread_id: Option<i64>,
    streaming: bool,
    excerpt: Option<String>,
    blocks: Option<Value>,
    chat_channel_id: i64,
}

const MESSAGE_COLUMNS: &str = "chat_messages.id, chat_messages.user_id, chat_messages.created_at, \
     chat_messages.deleted_at, chat_messages.deleted_by_id, chat_messages.in_reply_to_id, \
     chat_messages.message, chat_messages.cooked, chat_messages.thread_id, chat_messages.streaming, \
     chat_messages.excerpt, chat_messages.blocks, chat_messages.chat_channel_id";

#[derive(sqlx::FromRow, Clone)]
struct UserRow {
    id: i32,
    username: String,
    name: Option<String>,
    uploaded_avatar_id: Option<i32>,
    admin: bool,
    moderator: bool,
    trust_level: i32,
    primary_group_id: Option<i32>,
    flair_group_id: Option<i32>,
    chat_enabled: bool,
}

/// The viewer's bookmark of a message.
#[derive(sqlx::FromRow)]
struct BookmarkRow {
    id: i64,
    reminder_at: Option<NaiveDateTime>,
    name: Option<String>,
    auto_delete_preference: i32,
    bookmarkable_id: i64,
    bookmarkable_type: String,
}

/// `MessagesQuery.call`'s result: the messages in order and the meta.
struct Page {
    messages: Vec<MessageRow>,
    target_message_id: Option<i64>,
    can_load_more_future: Option<bool>,
    can_load_more_past: Option<bool>,
}

/// What the serializers read once per request.
struct Shared {
    viewer_id: Option<i32>,
    enable_names: bool,
    pinned_setting: bool,
    max_mentions: i64,
    logo_small_url: Option<String>,
    deleted_username: String,
    custom_emoji: HashSet<String>,
    /// The flag types' name keys, by position.
    flag_types: Vec<String>,
    /// can_flag_chat_messages? and can_flag_in_chat_channel?
    can_flag_here: bool,
    /// The viewer may send personal messages (notify_user).
    can_message: bool,
    allow_flagging_staff: bool,
    /// The viewer can chat: Chat::BasicUserSerializer's can_chat with a
    /// scope.
    viewer_can_chat: bool,
    threading_enabled: bool,
    users: HashMap<i32, UserRow>,
    groups: HashMap<i32, crate::groups::Group>,
}

pub enum Listed {
    Messages(Value),
    NotFound,
    Forbidden,
}

/// `Time#iso8601`: whole seconds.
fn iso8601(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// `Chat::Message#action?`: a /me message.
fn is_action(message: Option<&str>) -> bool {
    let Some(rest) = message.and_then(|m| m.strip_prefix("/me")) else {
        return false;
    };
    let body = rest.trim_start_matches([' ', '\t']);
    body.len() < rest.len() && !body.is_empty() && !body.contains(['\r', '\n'])
}

/// `s.gsub(%r{^[^:]+://}, "")`: at each line start, everything up to the
/// first colon when that colon starts "://" (which may span lines).
fn strip_schemes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let at_line_start = i == 0 || s.as_bytes()[i - 1] == b'\n';
        if at_line_start
            && let Some(colon) = s[i..].find(':')
            && colon > 0
            && s[i + colon..].starts_with("://")
        {
            i += colon + 3;
            continue;
        }
        let next = s[i..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&s[i..i + next]);
        i += next;
    }
    out
}

/// `Chat::Message#build_excerpt`
fn build_excerpt(m: &MessageRow) -> Result<String, AppError> {
    let cooked = m.cooked.as_deref().unwrap_or_default();
    // Just the URL if the whole message is URLs: oneboxes don't excerpt.
    let urls = crate::posting::links::extract(cooked)?;
    if !urls.is_empty() {
        let mut clean: Vec<String> = urls.iter().map(|u| strip_schemes(u)).collect();
        let stripped = strip_schemes(m.message.as_deref().unwrap_or_default());
        let mut words: Vec<&str> = stripped.split_whitespace().collect();
        clean.sort();
        words.sort();
        if words == clean.iter().map(String::as_str).collect::<Vec<_>>() {
            return Ok(crate::excerpt::excerpt(
                &urls.join(" "),
                EXCERPT_LENGTH,
                &crate::excerpt::Options::default(),
            ));
        }
    }
    Ok(crate::excerpt::excerpt(
        cooked,
        EXCERPT_LENGTH,
        &crate::excerpt::Options {
            strip_links: true,
            ..Default::default()
        },
    ))
}

/// `Chat::Message#build_excerpt` for a message not yet saved.
pub(crate) fn build_excerpt_for(message: &str, cooked: &str) -> Result<String, AppError> {
    build_excerpt(&MessageRow {
        id: 0,
        user_id: None,
        created_at: NaiveDateTime::default(),
        deleted_at: None,
        deleted_by_id: None,
        in_reply_to_id: None,
        message: Some(message.to_string()),
        cooked: Some(cooked.to_string()),
        thread_id: None,
        streaming: false,
        excerpt: None,
        blocks: None,
        chat_channel_id: 0,
    })
}

/// `Chat::Message#excerpt_for_display`: the stored excerpt, else one
/// built from the cooked message.
fn excerpt_for_display(m: &MessageRow) -> Result<String, AppError> {
    match &m.excerpt {
        Some(e) => Ok(e.clone()),
        None => build_excerpt(m),
    }
}

impl Context<'_> {
    /// `can_moderate_chat?(channel.chatable)`: staff, or a category group
    /// moderator of the channel's category.
    pub(crate) async fn can_moderate(&mut self, channel: &ChannelRow) -> Result<bool, AppError> {
        let g = self.guardian;
        if g.is_staff() {
            return Ok(true);
        }
        if !self
            .settings
            .get("enable_category_group_moderation")?
            .truthy()
        {
            return Ok(false);
        }
        let Some(uid) = g.user_id() else {
            return Ok(false);
        };
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM category_moderation_groups cmg \
             JOIN group_users gu ON gu.group_id = cmg.group_id \
             WHERE cmg.category_id = $1 AND gu.user_id = $2)",
        )
        .bind(channel.chatable_id as i32)
        .bind(uid)
        .fetch_one(&mut *self.conn)
        .await?)
    }

    /// Chat::LastMessageSerializer for a channel's last message, None when
    /// it is deleted (the association's default scope hides it, leaving
    /// the NullMessage).
    pub(crate) async fn last_message(&mut self, id: i64) -> Result<Option<Value>, AppError> {
        let sql = format!(
            "SELECT {MESSAGE_COLUMNS} FROM chat_messages WHERE id = $1 AND deleted_at IS NULL"
        );
        let m: Option<MessageRow> = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(&mut *self.conn)
            .await?;
        let Some(m) = m else {
            return Ok(None);
        };
        let uploads: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM upload_references WHERE target_type = $1 AND target_id = $2)",
        )
        .bind(POLYMORPHIC_NAME)
        .bind(m.id)
        .fetch_one(&mut *self.conn)
        .await?;
        if uploads {
            return Err(Unsupported("chat message uploads").into());
        }
        Ok(Some(json!({
            "id": m.id,
            "message": m.message,
            "cooked": m.cooked,
            "created_at": iso8601(m.created_at),
            "excerpt": excerpt_for_display(&m)?,
            "deleted_at": null,
            "deleted_by_id": m.deleted_by_id,
            "thread_id": m.thread_id,
            "chat_channel_id": m.chat_channel_id,
            "streaming": m.streaming,
        })))
    }

    /// Chat::MessageSerializer for these messages of the channel, by id, as
    /// this context's guardian sees them (the publisher's anonymous one
    /// for the bus).
    pub(crate) async fn serialize_messages(
        &mut self,
        channel: &ChannelRow,
        ids: &[i64],
    ) -> Result<Vec<Value>, AppError> {
        let sql = format!(
            "SELECT {MESSAGE_COLUMNS} FROM chat_messages WHERE chat_messages.id = ANY($1) ORDER BY chat_messages.id"
        );
        let rows: Vec<MessageRow> = sqlx::query_as(&sql)
            .bind(ids)
            .fetch_all(&mut *self.conn)
            .await?;
        let shared = self.shared(channel, &rows).await?;
        let mut out = Vec::new();
        for m in &rows {
            out.push(self.message(&shared, m).await?);
        }
        Ok(out)
    }

    /// GET /chat/api/channels/:channel_id/messages
    pub async fn list_messages(
        &mut self,
        channel_id: i64,
        params: &ListParams,
    ) -> Result<Listed, AppError> {
        // model :channel, policy :can_view_channel
        let channel = match self.find_joinable(&channel_id.to_string()).await? {
            Found::Channel(channel) => channel,
            Found::NotFound => return Ok(Listed::NotFound),
            Found::Forbidden => return Ok(Listed::Forbidden),
        };
        let membership = self
            .memberships()
            .await?
            .into_iter()
            .find(|m| m.chat_channel_id == channel.id);
        let mut target = if params.fetch_from_last_read {
            membership.as_ref().and_then(|m| m.last_read_message_id)
        } else {
            params.target_message_id
        };
        // Chat::Channel::Policy::MessageExistence: a deleted target is
        // dropped unless it is the viewer's or they are staff.
        if let Some(id) = target {
            let found: Option<(Option<i32>, bool)> = sqlx::query_as(
                "SELECT user_id, deleted_at IS NOT NULL FROM chat_messages \
                 WHERE id = $1 AND chat_channel_id = $2",
            )
            .bind(id)
            .bind(channel.id)
            .fetch_optional(&mut *self.conn)
            .await?;
            match found {
                None => return Ok(Listed::NotFound),
                Some((user_id, true))
                    if !(user_id.is_some() && user_id == self.guardian.user_id()
                        || self.guardian.is_staff()) =>
                {
                    target = None
                }
                Some(_) => {}
            }
        }
        // With no threads, the thread clauses (include_thread_messages or
        // not) keep every message.
        self.refuse_threads().await?;
        let with_deleted = self.can_moderate(&channel).await?;
        let page = self
            .messages_query(&channel, with_deleted, target, params)
            .await?;

        let shared = self.shared(&channel, &page.messages).await?;
        let mut messages = Vec::new();
        for m in &page.messages {
            messages.push(self.message(&shared, m).await?);
        }

        // update_membership_last_viewed_at, update_user_last_channel
        let now = crate::clock::now_naive();
        if let Some(uid) = self.guardian.user_id() {
            if membership.is_some() {
                sqlx::query(
                    "UPDATE user_chat_channel_memberships SET last_viewed_at = $1, updated_at = $1 \
                     WHERE user_id = $2 AND chat_channel_id = $3",
                )
                .bind(now)
                .bind(uid)
                .bind(channel.id)
                .execute(&mut *self.conn)
                .await?;
            }
            let last: Option<Option<String>> = sqlx::query_scalar(
                "SELECT value FROM user_custom_fields WHERE user_id = $1 AND name = $2 ORDER BY id LIMIT 1",
            )
            .bind(uid)
            .bind(LAST_CHAT_CHANNEL_ID)
            .fetch_optional(&mut *self.conn)
            .await?;
            // The field is an integer one: compared as its to_i.
            let same = last
                .flatten()
                .is_some_and(|v| crate::ruby::to_i(&v) == channel.id);
            if !same {
                // upsert_custom_fields
                let updated = sqlx::query(
                    "UPDATE user_custom_fields SET value = $1, updated_at = $2 WHERE user_id = $3 AND name = $4",
                )
                .bind(channel.id.to_string())
                .bind(now)
                .bind(uid)
                .bind(LAST_CHAT_CHANNEL_ID)
                .execute(&mut *self.conn)
                .await?;
                if updated.rows_affected() == 0 {
                    sqlx::query(
                        "INSERT INTO user_custom_fields (user_id, name, value, created_at, updated_at) \
                         VALUES ($1, $2, $3, $4, $4)",
                    )
                    .bind(uid)
                    .bind(LAST_CHAT_CHANNEL_ID)
                    .bind(channel.id.to_string())
                    .bind(now)
                    .execute(&mut *self.conn)
                    .await?;
                }
            }
        }

        // Chat::TrackingStateReportQuery for no channels and the messages'
        // threads (include_threads): none without threads, else the
        // member's threads.
        let thread_tracking = if page.messages.iter().any(|m| m.thread_id.is_some()) {
            self.thread_tracking().await?
        } else {
            serde_json::Map::new()
        };
        Ok(Listed::Messages(json!({
            "messages": messages,
            "tracking": {"channel_tracking": {}, "thread_tracking": thread_tracking},
            "meta": {
                "target_message_id": page.target_message_id,
                "can_load_more_future": page.can_load_more_future,
                "can_load_more_past": page.can_load_more_past,
            },
        })))
    }

    /// `Chat::MessagesQuery.call`: around a target, around a date, or a
    /// page in a direction (the latest without one).
    async fn messages_query(
        &mut self,
        channel: &ChannelRow,
        with_deleted: bool,
        target: Option<i64>,
        params: &ListParams,
    ) -> Result<Page, AppError> {
        let base = format!(
            "SELECT {MESSAGE_COLUMNS} FROM chat_messages WHERE chat_messages.chat_channel_id = $1{}",
            if with_deleted {
                ""
            } else {
                " AND chat_messages.deleted_at IS NULL"
            }
        );
        if let (Some(target), None) = (target, params.direction.as_deref()) {
            // query_around_target: the target found with deleted ones.
            let sql = format!(
                "SELECT {MESSAGE_COLUMNS} FROM chat_messages WHERE chat_messages.chat_channel_id = $1 \
                 AND chat_messages.id = $2"
            );
            let target: MessageRow = sqlx::query_as(&sql)
                .bind(channel.id)
                .bind(target)
                .fetch_one(&mut *self.conn)
                .await?;
            let (mut past, future) = self
                .around(&base, channel.id, target.created_at, "<", ">")
                .await?;
            let more_past = past.len() as i64 == AROUND_LIMIT;
            let more_future = future.len() as i64 == AROUND_LIMIT;
            past.reverse();
            let target_id = target.id;
            past.push(target);
            past.extend(future);
            return Ok(Page {
                messages: past,
                target_message_id: Some(target_id),
                can_load_more_future: Some(more_future),
                can_load_more_past: Some(more_past),
            });
        }
        if let Some(date) = params
            .target_date
            .as_deref()
            .filter(|d| !d.trim().is_empty())
        {
            // query_by_date: target_date.to_time.utc, the server's zone UTC.
            let at = parse_date(date)?;
            let (mut past, future) = self.around(&base, channel.id, at, "<=", ">").await?;
            let more_past = past.len() as i64 == AROUND_LIMIT;
            let more_future = future.len() as i64 == AROUND_LIMIT;
            past.reverse();
            past.extend(future);
            return Ok(Page {
                messages: past,
                target_message_id: None,
                can_load_more_future: Some(more_future),
                can_load_more_past: Some(more_past),
            });
        }
        // query_paginated_messages
        let page_size = params.page_size.min(MAX_PAGE_SIZE);
        let direction = params.direction.as_deref();
        let mut sql = base;
        if let Some(target) = target {
            let op = if direction == Some("past") { "<" } else { ">" };
            sql.push_str(&format!(" AND chat_messages.id {op} {target}"));
        }
        let order = if direction == Some("future") {
            "ASC"
        } else {
            "DESC"
        };
        sql.push_str(&format!(
            " ORDER BY chat_messages.created_at {order}, chat_messages.id {order} LIMIT {page_size}"
        ));
        let mut messages: Vec<MessageRow> = sqlx::query_as(&sql)
            .bind(channel.id)
            .fetch_all(&mut *self.conn)
            .await?;
        let full = messages.len() as i64 == page_size;
        let (more_future, more_past) = match direction {
            Some("future") => (Some(full), None),
            Some(_) => (None, Some(full)),
            None => (Some(false), Some(full)),
        };
        if direction != Some("future") {
            messages.reverse();
        }
        Ok(Page {
            messages,
            target_message_id: None,
            can_load_more_future: more_future,
            can_load_more_past: more_past,
        })
    }

    /// The messages before (newest first) and after a time, 25 each.
    async fn around(
        &mut self,
        base: &str,
        channel_id: i64,
        at: NaiveDateTime,
        before: &str,
        after: &str,
    ) -> Result<(Vec<MessageRow>, Vec<MessageRow>), AppError> {
        let past: Vec<MessageRow> = sqlx::query_as(&format!(
            "{base} AND chat_messages.created_at {before} $2 ORDER BY chat_messages.created_at DESC LIMIT {AROUND_LIMIT}"
        ))
        .bind(channel_id)
        .bind(at)
        .fetch_all(&mut *self.conn)
        .await?;
        let future: Vec<MessageRow> = sqlx::query_as(&format!(
            "{base} AND chat_messages.created_at {after} $2 ORDER BY chat_messages.created_at ASC LIMIT {AROUND_LIMIT}"
        ))
        .bind(channel_id)
        .bind(at)
        .fetch_all(&mut *self.conn)
        .await?;
        Ok((past, future))
    }

    /// What every message's serializer reads: the settings, the viewer's
    /// flag permissions, and the users and groups involved.
    async fn shared(
        &mut self,
        channel: &ChannelRow,
        messages: &[MessageRow],
    ) -> Result<Shared, AppError> {
        let settings = self.settings;
        let g = self.guardian;
        let ids: Vec<i64> = messages.iter().map(|m| m.id).collect();
        // Not ported yet: refused rather than served without them.
        let uploads: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM upload_references WHERE target_type = $1 AND target_id = ANY($2))",
        )
        .bind(POLYMORPHIC_NAME)
        .bind(&ids)
        .fetch_one(&mut *self.conn)
        .await?;
        if uploads {
            return Err(Unsupported("chat message uploads").into());
        }
        let webhooks: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM chat_webhook_events WHERE chat_message_id = ANY($1) \
               OR chat_message_id IN (SELECT in_reply_to_id FROM chat_messages WHERE id = ANY($1)))",
        )
        .bind(&ids)
        .fetch_one(&mut *self.conn)
        .await?;
        if webhooks {
            return Err(Unsupported("chat webhook messages").into());
        }
        if messages.iter().any(|m| m.blocks.is_some()) {
            return Err(Unsupported("chat message blocks").into());
        }

        // The authors, mentioned users, reacting users and replied-to
        // authors.
        let user_ids: Vec<i32> = sqlx::query_scalar(
            "SELECT user_id FROM chat_messages WHERE id = ANY($1) AND user_id IS NOT NULL \
             UNION SELECT target_id FROM chat_mentions WHERE chat_message_id = ANY($1) \
               AND type = 'Chat::UserMention' AND target_id IS NOT NULL \
             UNION SELECT user_id FROM chat_message_reactions WHERE chat_message_id = ANY($1) AND user_id IS NOT NULL \
             UNION SELECT r.user_id FROM chat_messages m JOIN chat_messages r ON r.id = m.in_reply_to_id \
               WHERE m.id = ANY($1) AND r.user_id IS NOT NULL",
        )
        .bind(&ids)
        .fetch_all(&mut *self.conn)
        .await?;
        let users: Vec<UserRow> = sqlx::query_as(
            "SELECT users.id, users.username, users.name, users.uploaded_avatar_id, users.admin, \
                    users.moderator, users.trust_level, users.primary_group_id, users.flair_group_id, \
                    COALESCE(user_options.chat_enabled, TRUE) AS chat_enabled \
             FROM users LEFT JOIN user_options ON user_options.user_id = users.id \
             WHERE users.id = ANY($1)",
        )
        .bind(&user_ids)
        .fetch_all(&mut *self.conn)
        .await?;
        let user_status = settings.get("enable_user_status")?.truthy();
        if user_status {
            let any: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM user_statuses WHERE user_id = ANY($1) AND (ends_at IS NULL OR ends_at > now()))",
            )
            .bind(&user_ids)
            .fetch_one(&mut *self.conn)
            .await?;
            if any {
                return Err(Unsupported("user status").into());
            }
        }
        let group_ids: Vec<i32> = users
            .iter()
            .flat_map(|u| [u.primary_group_id, u.flair_group_id])
            .flatten()
            .collect();
        let groups = crate::groups::load(&mut *self.conn, &group_ids).await?;

        let can_chat = super::can_chat(&mut *self.conn, settings, g).await?;
        // can_flag_chat_messages? (unsilenced, in the allowed groups) and
        // can_flag_in_chat_channel? (a channel messages can change in,
        // and joinable).
        let modifiable = if g.is_staff() {
            channel.status == 0 || channel.status == 2
        } else {
            channel.status == 0
        };
        let can_flag_here = g.is_authenticated()
            && !g.is_silenced()
            && g.in_setting_groups(settings, "chat_message_flag_allowed_groups")?
            && modifiable
            && self.can_join(channel, None).await?;
        let flag_types: Vec<String> = sqlx::query_scalar(
            "SELECT name_key FROM flags WHERE NOT score_type AND id <> 2 ORDER BY position",
        )
        .fetch_all(&mut *self.conn)
        .await?;
        let custom_emoji: HashSet<String> = sqlx::query_scalar("SELECT name FROM custom_emojis")
            .fetch_all(&mut *self.conn)
            .await?
            .into_iter()
            .collect();
        Ok(Shared {
            viewer_id: g.user_id(),
            enable_names: settings.get("enable_names")?.truthy(),
            pinned_setting: settings.get("chat_pinned_messages")?.truthy(),
            max_mentions: settings.get("max_mentions_per_chat_message")?.to_i(),
            logo_small_url: crate::admin_users::logo_small_url(&mut *self.conn, settings).await?,
            deleted_username: self
                .i18n
                .t("chat.deleted_chat_username")
                .ok_or(Unsupported("chat.deleted_chat_username translation"))?
                .to_string(),
            custom_emoji,
            flag_types,
            can_flag_here,
            can_message: g.is_authenticated()
                && g.in_setting_groups(settings, "personal_message_enabled_groups")?,
            allow_flagging_staff: settings.get("allow_flagging_staff")?.truthy(),
            viewer_can_chat: can_chat,
            threading_enabled: channel.threading_enabled,
            users: users.into_iter().map(|u| (u.id, u)).collect(),
            groups,
        })
    }

    /// The avatar template of a user.
    fn avatar(&self, s: &Shared, u: &UserRow) -> Result<String, AppError> {
        let urls = crate::url::Urls {
            config: self.config,
            settings: self.settings,
        };
        Ok(crate::avatar::avatar_template(
            &urls,
            u.id,
            &u.username,
            u.uploaded_avatar_id,
            s.logo_small_url.as_deref(),
        )?)
    }

    /// Chat::BasicUserSerializer (BasicUserSerializer, as chat's
    /// serializers name it, resolves to it). Without a scope its can_chat
    /// and has_chat_enabled are nil.
    fn basic_user(
        &self,
        s: &Shared,
        u: &UserRow,
        scoped: bool,
    ) -> Result<Map<String, Value>, AppError> {
        let mut out = Map::new();
        out.insert("id".into(), json!(u.id));
        out.insert("username".into(), json!(u.username));
        if s.enable_names {
            out.insert("name".into(), json!(u.name));
        }
        out.insert("avatar_template".into(), json!(self.avatar(s, u)?));
        if scoped {
            out.insert("can_chat".into(), json!(s.viewer_can_chat));
            out.insert(
                "has_chat_enabled".into(),
                json!(s.viewer_can_chat && u.chat_enabled),
            );
        } else {
            out.insert("can_chat".into(), Value::Null);
            out.insert("has_chat_enabled".into(), Value::Null);
        }
        Ok(out)
    }

    /// Chat::NullUser, a deleted author, through Chat::BasicUserSerializer.
    fn null_user(&self, s: &Shared, scoped: bool) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("id".into(), Value::Null);
        out.insert("username".into(), json!(s.deleted_username));
        if s.enable_names {
            out.insert("name".into(), Value::Null);
        }
        out.insert(
            "avatar_template".into(),
            json!("/plugins/chat/images/deleted-chat-user-avatar.png"),
        );
        // A new UserOption's chat_enabled is true.
        out.insert(
            "can_chat".into(),
            if scoped {
                json!(s.viewer_can_chat)
            } else {
                Value::Null
            },
        );
        out.insert(
            "has_chat_enabled".into(),
            if scoped {
                json!(s.viewer_can_chat)
            } else {
                Value::Null
            },
        );
        out
    }

    /// Chat::MessageUserSerializer: the basic user, its flair and roles.
    fn message_user(&self, s: &Shared, user_id: Option<i32>) -> Result<Value, AppError> {
        let Some(u) = user_id.and_then(|id| s.users.get(&id)) else {
            let mut out = self.null_user(s, false);
            out.insert("moderator".into(), json!(false));
            out.insert("admin".into(), json!(false));
            out.insert("staff".into(), json!(false));
            // A new User's trust level is 0.
            out.insert("new_user".into(), json!(true));
            out.insert("primary_group_name".into(), Value::Null);
            return Ok(Value::Object(out));
        };
        let mut out = self.basic_user(s, u, false)?;
        // UserFlairMixin
        if let Some(flair) = u.flair_group_id.and_then(|id| s.groups.get(&id)) {
            out.insert("flair_name".into(), json!(flair.name));
            if let Some(url) = flair.flair_url()?.filter(|u| !u.is_empty()) {
                out.insert("flair_url".into(), json!(url));
            }
            if let Some(bg) = flair.flair_bg_color.as_deref().filter(|c| !c.is_empty()) {
                out.insert("flair_bg_color".into(), json!(bg));
            }
            if let Some(color) = flair.flair_color.as_deref().filter(|c| !c.is_empty()) {
                out.insert("flair_color".into(), json!(color));
            }
        }
        if let Some(id) = u.flair_group_id {
            out.insert("flair_group_id".into(), json!(id));
        }
        out.insert("moderator".into(), json!(u.moderator));
        out.insert("admin".into(), json!(u.admin));
        out.insert("staff".into(), json!(u.admin || u.moderator));
        out.insert("new_user".into(), json!(u.trust_level == 0));
        out.insert(
            "primary_group_name".into(),
            json!(
                u.primary_group_id
                    .and_then(|id| s.groups.get(&id))
                    .map(|g| g.name.clone())
            ),
        );
        Ok(Value::Object(out))
    }

    /// Chat::MessageSerializer
    async fn message(&mut self, s: &Shared, m: &MessageRow) -> Result<Value, AppError> {
        let author = m.user_id.and_then(|id| s.users.get(&id));
        let mut out = Map::new();
        out.insert("id".into(), json!(m.id));
        out.insert("message".into(), json!(m.message));
        out.insert("cooked".into(), json!(m.cooked));
        out.insert("created_at".into(), json!(iso8601(m.created_at)));
        out.insert("excerpt".into(), json!(excerpt_for_display(m)?));
        // A deleted author's message reads as deleted by the system, now.
        if author.is_none() {
            out.insert(
                "deleted_at".into(),
                json!(crate::topic_list::time_json(crate::clock::now_naive())),
            );
            out.insert("deleted_by_id".into(), json!(SYSTEM_USER_ID));
        } else if let Some(at) = m.deleted_at {
            out.insert("deleted_at".into(), json!(iso8601(at)));
            out.insert("deleted_by_id".into(), json!(m.deleted_by_id));
        }
        if s.threading_enabled {
            out.insert("thread_id".into(), json!(m.thread_id));
        }
        out.insert("chat_channel_id".into(), json!(m.chat_channel_id));
        out.insert("streaming".into(), json!(m.streaming));
        out.insert("user".into(), self.message_user(s, m.user_id)?);

        // mentioned_users: the first max_mentions_per_chat_message, by id.
        let mentioned: Vec<i32> = sqlx::query_scalar(
            "SELECT target_id FROM chat_mentions WHERE chat_message_id = $1 \
             AND type = 'Chat::UserMention' ORDER BY id LIMIT $2",
        )
        .bind(m.id)
        .bind(s.max_mentions)
        .fetch_all(&mut *self.conn)
        .await?;
        let mut mentioned: Vec<&UserRow> =
            mentioned.iter().filter_map(|id| s.users.get(id)).collect();
        mentioned.sort_by_key(|u| u.id);
        let mentioned: Vec<Value> = mentioned
            .into_iter()
            .map(|u| self.basic_user(s, u, false).map(Value::Object))
            .collect::<Result<_, _>>()?;
        out.insert("mentioned_users".into(), Value::Array(mentioned));

        // reactions, grouped by emoji in the order they were made; five
        // users each.
        let reactions: Vec<(Option<String>, Option<i32>)> = sqlx::query_as(
            "SELECT emoji, user_id FROM chat_message_reactions WHERE chat_message_id = $1 ORDER BY id",
        )
        .bind(m.id)
        .fetch_all(&mut *self.conn)
        .await?;
        if !reactions.is_empty() {
            let mut groups: Vec<(String, Vec<Option<i32>>)> = Vec::new();
            for (emoji, user_id) in reactions {
                let emoji = emoji.unwrap_or_default();
                match groups.iter_mut().find(|(e, _)| *e == emoji) {
                    Some((_, users)) => users.push(user_id),
                    None => groups.push((emoji, vec![user_id])),
                }
            }
            let mut list = Vec::new();
            for (emoji, user_ids) in groups {
                if !crate::plugins::reactions::emoji_exists(&emoji, &s.custom_emoji) {
                    continue;
                }
                let mut users = Vec::new();
                for id in user_ids.iter().take(5) {
                    let Some(u) = id.and_then(|id| s.users.get(&id)) else {
                        return Err(Unsupported("chat reactions of deleted users").into());
                    };
                    users.push(Value::Object(self.basic_user(s, u, false)?));
                }
                list.push(json!({
                    "emoji": emoji,
                    "count": user_ids.len(),
                    "users": users,
                    "reacted": s.viewer_id.is_some() && user_ids.contains(&s.viewer_id),
                }));
            }
            out.insert("reactions".into(), Value::Array(list));
        }

        // bookmark: the viewer's.
        if let Some(viewer) = s.viewer_id {
            let bookmark: Option<BookmarkRow> = sqlx::query_as(
                    "SELECT id, reminder_at, name, auto_delete_preference, bookmarkable_id, bookmarkable_type \
                     FROM bookmarks WHERE bookmarkable_type = $1 AND bookmarkable_id = $2 AND user_id = $3 \
                     ORDER BY id LIMIT 1",
                )
                .bind(POLYMORPHIC_NAME)
                .bind(m.id)
                .bind(viewer)
                .fetch_optional(&mut *self.conn)
                .await?;
            if let Some(b) = bookmark {
                out.insert(
                    "bookmark".into(),
                    json!({
                        "id": b.id,
                        "reminder_at": b.reminder_at.map(crate::topic_list::time_json),
                        "name": b.name,
                        "auto_delete_preference": b.auto_delete_preference,
                        "bookmarkable_id": b.bookmarkable_id,
                        "bookmarkable_type": b.bookmarkable_type,
                    }),
                );
            }
        }

        out.insert(
            "available_flags".into(),
            json!(self.available_flags(s, m, author)),
        );
        let edited: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM chat_message_revisions WHERE chat_message_id = $1)",
        )
        .bind(m.id)
        .fetch_one(&mut *self.conn)
        .await?;
        if edited {
            out.insert("edited".into(), json!(true));
        }
        out.insert("blocks".into(), json!([]));
        if s.pinned_setting {
            let pinned: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM chat_pinned_messages WHERE chat_message_id = $1)",
            )
            .bind(m.id)
            .fetch_one(&mut *self.conn)
            .await?;
            out.insert("pinned".into(), json!(pinned));
        }
        out.insert("is_action".into(), json!(is_action(m.message.as_deref())));
        out.insert("chat_webhook_event".into(), Value::Null);
        if let Some(parent_id) = m.in_reply_to_id {
            out.insert("in_reply_to".into(), self.in_reply_to(s, parent_id).await?);
        }
        out.insert("uploads".into(), json!([]));
        Ok(Value::Object(out))
    }

    /// Chat::InReplyToSerializer: the live message replied to (null when
    /// it was deleted), its user serialized with the viewer's scope.
    async fn in_reply_to(&mut self, s: &Shared, id: i64) -> Result<Value, AppError> {
        let sql = format!(
            "SELECT {MESSAGE_COLUMNS} FROM chat_messages WHERE id = $1 AND deleted_at IS NULL"
        );
        let parent: Option<MessageRow> = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(&mut *self.conn)
            .await?;
        let Some(parent) = parent else {
            return Ok(Value::Null);
        };
        let user = match parent.user_id.and_then(|id| s.users.get(&id)) {
            Some(u) => self.basic_user(s, u, true)?,
            None => self.null_user(s, true),
        };
        Ok(json!({
            "id": parent.id,
            "cooked": parent.cooked,
            "excerpt": excerpt_for_display(&parent)?,
            "user": user,
            "chat_webhook_event": null,
        }))
    }

    /// `available_flags`: the flag types (but notify_user to a bot, or by
    /// a viewer who can't send personal messages) when the viewer may
    /// flag the message. Rails also compares the viewer with the
    /// serialized user, which never matches.
    fn available_flags(&self, s: &Shared, m: &MessageRow, author: Option<&UserRow>) -> Vec<String> {
        // can_flag_chat_message?
        let Some(author) = author else {
            return Vec::new();
        };
        if m.deleted_at.is_some()
            || s.viewer_id.is_none()
            || ((author.admin || author.moderator) && !s.allow_flagging_staff)
            || Some(author.id) == s.viewer_id
            || !s.can_flag_here
        {
            return Vec::new();
        }
        s.flag_types
            .iter()
            .filter(|t| !(t.as_str() == "notify_user" && (author.id <= 0 || !s.can_message)))
            .cloned()
            .collect()
    }
}

/// `String#to_time` for the dates the client sends: an ISO 8601 date, or
/// date and time, read as UTC.
fn parse_date(s: &str) -> Result<NaiveDateTime, AppError> {
    let s = s.trim();
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(t.naive_utc());
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(t) = NaiveDateTime::parse_from_str(s, format) {
            return Ok(t);
        }
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d.and_hms_opt(0, 0, 0).unwrap_or_default());
    }
    Err(Unsupported("chat target dates other than ISO 8601").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_messages() {
        assert!(is_action(Some("/me waves")));
        assert!(is_action(Some("/me\twaves")));
        assert!(!is_action(Some("/me")));
        assert!(!is_action(Some("/me ")));
        assert!(!is_action(Some("/mewaves")));
        assert!(!is_action(Some("/me waves\nagain")));
        assert!(!is_action(None));
    }

    #[test]
    fn schemes_are_stripped_per_line() {
        assert_eq!(strip_schemes("https://a.com b"), "a.com b");
        assert_eq!(strip_schemes("x https://a.com"), "a.com");
        assert_eq!(strip_schemes("a\nb://c"), "c");
        assert_eq!(strip_schemes("a: https://b"), "a: https://b");
        assert_eq!(strip_schemes("https://a\nhttp://b"), "a\nb");
    }
}
