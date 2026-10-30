//! A channel's messages as GET /chat/api/channels/:id/messages serves
//! them: Chat::ListChannelMessages over Chat::MessagesQuery, serialized
//! by Chat::MessagesSerializer and Chat::MessageSerializer; a thread's
//! (Chat::ListChannelThreadMessages) over the same query.
//!
//! Refused as Unsupported until their slices port them: optimized videos
//! of uploads, and user status.
//! The two writes Rails defers (Scheduler::Defer) to after the response,
//! the membership's last_viewed_at and the user's last chat channel, are
//! made before it.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, NaiveDateTime};
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

/// `MessagesQuery.call`'s thread options.
struct Query {
    /// Only this thread's messages.
    thread_id: Option<i64>,
    /// Thread replies kept but a forced thread's; else every thread's
    /// replies left out.
    include_thread_messages: bool,
    /// A paged query's target included.
    include_target_message_id: bool,
}

/// Chat::ListChannelThreadMessages' params, past the contract.
pub struct ThreadListParams {
    pub page_size: i64,
    pub target_message_id: Option<i64>,
    pub direction: Option<String>,
    pub fetch_from_last_read: bool,
    pub fetch_from_last_message: bool,
    pub fetch_from_first_message: bool,
    pub target_date: Option<String>,
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
    /// A direct message channel: no notify flags.
    direct_message: bool,
    users: HashMap<i32, UserRow>,
    groups: HashMap<i32, crate::groups::Group>,
    /// The messages' uploads, by message.
    uploads: HashMap<i64, Vec<MessageUpload>>,
    /// The messages' threads, by id.
    threads: HashMap<i64, super::threads::ThreadRow>,
    /// The serializer's thread options: the original message in an
    /// original message's thread (unless asked not to), the participants
    /// and the viewer's memberships when given.
    thread_original_message: bool,
    thread_participants: Option<HashMap<i64, super::threads::Participants>>,
    thread_memberships: Option<HashMap<i64, super::threads::ThreadMembership>>,
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

/// Chat::BlockSerializer over a message's blocks: each block's type and
/// its elements, a button (Chat::Blocks::Elements::ButtonSerializer, its
/// text through TextSerializer) or a category (CategorySerializer, its
/// optional keys only when present). Rails raises on any other element
/// type and on a block or button missing what it reads.
fn serialize_blocks(blocks: &Value) -> Result<Value, AppError> {
    let malformed = || Unsupported("chat message blocks Rails can't serialize");
    let present = |v: &Value| match v {
        Value::Null => false,
        Value::String(s) => !s.trim().is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        _ => true,
    };
    let mut out = Vec::new();
    for block in blocks.as_array().ok_or_else(malformed)? {
        let mut elements = Vec::new();
        for e in block["elements"].as_array().ok_or_else(malformed)? {
            let element = match e["type"].as_str() {
                Some("button") => {
                    let text = &e["text"];
                    if !text.is_object() {
                        return Err(malformed().into());
                    }
                    json!({
                        "action_id": e["action_id"],
                        "type": e["type"],
                        "text": { "text": text["text"], "type": text["type"] },
                        "style": e["style"],
                        "icon": e["icon"],
                    })
                }
                Some("category") => {
                    let mut c = Map::new();
                    c.insert("type".into(), e["type"].clone());
                    c.insert("title".into(), e["title"].clone());
                    c.insert("color".into(), e["color"].clone());
                    for key in ["description", "url", "parent_name", "parent_color"] {
                        if present(&e[key]) {
                            c.insert(key.into(), e[key].clone());
                        }
                    }
                    Value::Object(c)
                }
                _ => return Err(malformed().into()),
            };
            elements.push(element);
        }
        out.push(json!({ "type": block["type"], "elements": elements }));
    }
    Ok(Value::Array(out))
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
fn build_excerpt(m: &MessageRow, plugins: crate::excerpt::Plugins) -> Result<String, AppError> {
    let cooked = m.cooked.as_deref().unwrap_or_default();
    // Just the URL if the whole message is URLs: oneboxes don't excerpt.
    let urls = crate::posting::links::extract_with_quotes(cooked)?;
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
                plugins,
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
        plugins,
    ))
}

/// `Chat::Message#build_excerpt` for a message not yet saved.
pub(crate) fn build_excerpt_for(
    message: &str,
    cooked: &str,
    plugins: crate::excerpt::Plugins,
) -> Result<String, AppError> {
    build_excerpt(
        &MessageRow {
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
        },
        plugins,
    )
}

/// `Chat::Message#build_excerpt` of a message with uploads: one whose
/// cooked is blank is its first upload's name.
pub(crate) fn build_excerpt_with_uploads(
    message: &str,
    cooked: &str,
    first_upload: Option<&str>,
    plugins: crate::excerpt::Plugins,
) -> Result<String, AppError> {
    match first_upload {
        // A blank cooked has no urls, so the urls-only case can't come
        // first.
        Some(name) if cooked.trim().is_empty() => Ok(upload_filename_excerpt(name)),
        _ => build_excerpt_for(message, cooked, plugins),
    }
}

/// `Chat::Message#upload_filename_excerpt`: the first upload's name, cut
/// to EXCERPT_LENGTH and escaped.
pub(crate) fn upload_filename_excerpt(filename: &str) -> String {
    crate::category_badge::html_escape(&filename.chars().take(EXCERPT_LENGTH).collect::<String>())
}

/// `Chat::Message#cooked_for_excerpt`: a message with a blank cooked and
/// uploads reads as its first upload's name.
pub(crate) fn cooked_for_excerpt(cooked: &str, first_upload: Option<&str>) -> String {
    match first_upload {
        Some(name) if cooked.trim().is_empty() => {
            format!("<p>{}</p>", upload_filename_excerpt(name))
        }
        _ => cooked.to_string(),
    }
}

/// The name of a message's first upload, in the order they were attached.
pub(crate) async fn first_upload_filename(
    conn: &mut sqlx::PgConnection,
    message_id: i64,
) -> Result<Option<String>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT u.original_filename FROM upload_references ur JOIN uploads u ON u.id = ur.upload_id \
         WHERE ur.target_type = $1 AND ur.target_id = $2 ORDER BY ur.id LIMIT 1",
    )
    .bind(POLYMORPHIC_NAME)
    .bind(message_id)
    .fetch_optional(conn)
    .await?)
}

/// An upload of a message: its name, and UploadSerializer's json.
pub(crate) struct MessageUpload {
    pub original_filename: String,
    pub json: Value,
}

/// `Chat::Message#excerpt_for_display`: an upload-only message's first
/// upload's name, else the stored excerpt, else one built from the cooked
/// message.
fn excerpt_for_display(
    m: &MessageRow,
    first_upload: Option<&str>,
    plugins: crate::excerpt::Plugins,
) -> Result<String, AppError> {
    // only_uploads?: a blank message with uploads.
    if let Some(name) = first_upload
        && m.message.as_deref().is_none_or(|t| t.trim().is_empty())
    {
        return Ok(upload_filename_excerpt(name));
    }
    match &m.excerpt {
        Some(e) => Ok(e.clone()),
        None => build_excerpt(m, plugins),
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
        // Category group moderators moderate category channels only.
        if channel.chatable_type != "Category" {
            return Ok(false);
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

    /// The name of a message's first upload (`uploads.first`), in the
    /// order they were attached.
    pub(crate) async fn first_upload_filename(
        &mut self,
        message_id: i64,
    ) -> Result<Option<String>, AppError> {
        first_upload_filename(&mut *self.conn, message_id).await
    }

    /// The messages' uploads (`has_many :uploads, through:
    /// :upload_references`) by message, each in the order it was attached,
    /// through UploadSerializer. An upload's optimized_video, which the
    /// serializer adds when video_conversion_enabled, is refused.
    pub(crate) async fn message_uploads(
        &mut self,
        ids: &[i64],
    ) -> Result<HashMap<i64, Vec<MessageUpload>>, AppError> {
        let refs: Vec<(i64, i64, String)> = sqlx::query_as(
            "SELECT ur.target_id, ur.upload_id, u.original_filename FROM upload_references ur \
             JOIN uploads u ON u.id = ur.upload_id \
             WHERE ur.target_type = $1 AND ur.target_id = ANY($2) ORDER BY ur.id",
        )
        .bind(POLYMORPHIC_NAME)
        .bind(ids)
        .fetch_all(&mut *self.conn)
        .await?;
        let mut out: HashMap<i64, Vec<MessageUpload>> = HashMap::new();
        if refs.is_empty() {
            return Ok(out);
        }
        if self.settings.get("video_conversion_enabled")?.truthy() {
            let upload_ids: Vec<i64> = refs.iter().map(|r| r.1).collect();
            let optimized: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM optimized_videos WHERE upload_id::int8 = ANY($1))",
            )
            .bind(&upload_ids)
            .fetch_one(&mut *self.conn)
            .await?;
            if optimized {
                return Err(Unsupported("optimized videos of chat message uploads").into());
            }
        }
        let urls = crate::url::Urls {
            config: self.config,
            settings: self.settings,
        };
        for (message_id, upload_id, original_filename) in refs {
            let json =
                crate::uploads::serialize(&mut *self.conn, self.settings, &urls, upload_id as i32)
                    .await?;
            out.entry(message_id).or_default().push(MessageUpload {
                original_filename,
                json,
            });
        }
        Ok(out)
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
        let first_upload = self.first_upload_filename(m.id).await?;
        Ok(Some(json!({
            "id": m.id,
            "message": m.message,
            "cooked": m.cooked,
            "created_at": iso8601(m.created_at),
            "excerpt": excerpt_for_display(&m, first_upload.as_deref(), crate::excerpt::Plugins::from_settings(self.settings)?)?,
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

    /// Chat::MessageSerializer for these messages as a page lists them:
    /// their threads with participants and the viewer's memberships.
    pub(crate) async fn serialize_page_messages(
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
        let mut shared = self.shared(channel, &rows).await?;
        let thread_ids: Vec<i64> = shared.threads.keys().copied().collect();
        if !thread_ids.is_empty() {
            shared.thread_participants = Some(self.thread_participants(&thread_ids).await?);
            shared.thread_memberships = Some(self.thread_memberships(&thread_ids).await?);
        }
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
        let with_deleted = self.can_moderate(&channel).await?;
        let query = Query {
            thread_id: None,
            include_thread_messages: !channel.threading_enabled,
            include_target_message_id: false,
        };
        let mut page = self
            .messages_query(&channel, with_deleted, target, params, &query)
            .await?;
        // fetch_messages: the target left out when it is a thread's reply.
        if let Some(target_id) = page.target_message_id {
            let target_thread = page
                .messages
                .iter()
                .find(|m| m.id == target_id)
                .and_then(|m| m.thread_id);
            if let Some(thread) = match target_thread {
                Some(id) => self.find_thread(id, None).await?,
                None => None,
            } && super::threads::shows(&channel, &thread)
                && thread.original_message_id != target_id
            {
                page.messages.retain(|m| m.id != target_id);
            }
        }

        let mut shared = self.shared(&channel, &page.messages).await?;
        // thread_ids, tracking, thread_participants, thread_memberships
        let thread_ids: Vec<i64> = {
            let mut ids: Vec<i64> = page.messages.iter().filter_map(|m| m.thread_id).collect();
            ids.sort_unstable();
            ids.dedup();
            ids
        };
        if !thread_ids.is_empty() {
            shared.thread_participants = Some(self.thread_participants(&thread_ids).await?);
            if self.guardian.is_authenticated() {
                shared.thread_memberships = Some(self.thread_memberships(&thread_ids).await?);
            }
        }
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

        // Chat::TrackingStateReportQuery for the messages' threads
        // (include_threads).
        let thread_tracking = self.thread_tracking_for(&thread_ids, false).await?;
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

    /// GET /chat/api/channels/:channel_id/threads/:thread_id/messages:
    /// Chat::ListChannelThreadMessages, serialized without the threads'
    /// original messages, participants, memberships or tracking.
    pub async fn list_thread_messages(
        &mut self,
        channel_id: i64,
        thread_id: i64,
        params: &ThreadListParams,
    ) -> Result<Listed, AppError> {
        // model :thread, policy :can_view_thread
        let Some(thread) = self.find_thread(thread_id, Some(channel_id)).await? else {
            return Ok(Listed::NotFound);
        };
        let channel = self.thread_channel(&thread).await?;
        if self.guardian.user_id() != Some(SYSTEM_USER_ID) && !self.can_join(&channel, None).await?
        {
            return Ok(Listed::Forbidden);
        }
        // policy :threading_enabled_for_channel, :original_message_not_deleted
        if !super::threads::shows(&channel, &thread) {
            return Ok(Listed::NotFound);
        }
        let om_deleted: bool =
            sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM chat_messages WHERE id = $1")
                .bind(thread.original_message_id)
                .fetch_one(&mut *self.conn)
                .await?;
        let with_deleted = self.can_moderate(&channel).await?;
        if om_deleted && !with_deleted {
            return Ok(Listed::NotFound);
        }
        // model :membership, model :target_message_id
        let membership = match self.guardian.user_id() {
            Some(_) => self
                .thread_memberships(&[thread.id])
                .await?
                .remove(&thread.id),
            None => None,
        };
        let target = if params.fetch_from_last_message {
            thread.last_message_id
        } else if params.fetch_from_first_message {
            Some(thread.original_message_id)
        } else if params.fetch_from_last_read || params.target_message_id.is_none() {
            membership.as_ref().and_then(|m| m.last_read_message_id)
        } else {
            params.target_message_id
        };
        // Chat::Thread::Policy::MessageExistence
        if let Some(id) = target {
            let found: Option<(Option<i32>, bool)> = sqlx::query_as(
                "SELECT user_id, deleted_at IS NOT NULL FROM chat_messages WHERE id = $1 AND thread_id = $2",
            )
            .bind(id)
            .bind(thread.id)
            .fetch_optional(&mut *self.conn)
            .await?;
            match found {
                None => return Ok(Listed::NotFound),
                Some((user_id, true))
                    if !(user_id.is_some() && user_id == self.guardian.user_id()
                        || self.guardian.is_staff()) =>
                {
                    return Ok(Listed::NotFound);
                }
                Some(_) => {}
            }
        }
        let list = ListParams {
            page_size: params.page_size,
            target_message_id: target,
            direction: params.direction.clone(),
            fetch_from_last_read: false,
            target_date: params.target_date.clone(),
        };
        let query = Query {
            thread_id: Some(thread.id),
            include_thread_messages: true,
            include_target_message_id: params.fetch_from_first_message
                || params.fetch_from_last_message,
        };
        let page = self
            .messages_query(&channel, with_deleted, target, &list, &query)
            .await?;
        let mut shared = self.shared(&channel, &page.messages).await?;
        shared.thread_original_message = false;
        let mut messages = Vec::new();
        for m in &page.messages {
            messages.push(self.message(&shared, m).await?);
        }
        Ok(Listed::Messages(json!({
            "messages": messages,
            "tracking": {},
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
        query: &Query,
    ) -> Result<Page, AppError> {
        let mut base = format!(
            "SELECT {MESSAGE_COLUMNS} FROM chat_messages{} WHERE chat_messages.chat_channel_id = $1{}",
            if query.thread_id.is_none() && query.include_thread_messages {
                " LEFT JOIN chat_threads ON chat_threads.id = chat_messages.thread_id"
            } else {
                ""
            },
            if with_deleted {
                ""
            } else {
                " AND chat_messages.deleted_at IS NULL"
            }
        );
        // A thread's messages; or the channel's, a forced thread's replies
        // left out (include_thread_messages), or every thread's replies.
        match query.thread_id {
            Some(thread_id) => {
                base.push_str(&format!(" AND chat_messages.thread_id = {thread_id}"));
            }
            None if query.include_thread_messages => base.push_str(
                " AND (chat_threads.id IS NULL OR chat_threads.force = false \
                   OR chat_messages.id = chat_threads.original_message_id)",
            ),
            None => base.push_str(
                " AND (chat_messages.thread_id IS NULL OR chat_messages.id IN ( \
                   SELECT original_message_id FROM chat_threads WHERE chat_threads.channel_id = $1))",
            ),
        }
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
            let op = match (direction == Some("past"), query.include_target_message_id) {
                (true, false) => "<",
                (true, true) => "<=",
                (false, false) => ">",
                (false, true) => ">=",
            };
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
        let uploads = self.message_uploads(&ids).await?;
        // A webhook event whose webhook is gone fails its delegation.
        let orphaned: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM chat_webhook_events e \
               LEFT JOIN incoming_chat_webhooks w ON w.id = e.incoming_chat_webhook_id \
               WHERE w.id IS NULL AND (e.chat_message_id = ANY($1) \
                 OR e.chat_message_id IN (SELECT in_reply_to_id FROM chat_messages WHERE id = ANY($1))))",
        )
        .bind(&ids)
        .fetch_one(&mut *self.conn)
        .await?;
        if orphaned {
            return Err(Unsupported("a chat webhook event without its webhook").into());
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
        let mut thread_ids: Vec<i64> = messages.iter().filter_map(|m| m.thread_id).collect();
        thread_ids.sort_unstable();
        thread_ids.dedup();
        let threads = self.threads_by_id(&thread_ids).await?;
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
            direct_message: channel.chatable_type == "DirectMessage",
            users: users.into_iter().map(|u| (u.id, u)).collect(),
            groups,
            uploads,
            threads,
            thread_original_message: true,
            thread_participants: None,
            thread_memberships: None,
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
        out.insert(
            "excerpt".into(),
            json!(excerpt_for_display(
                m,
                s.uploads
                    .get(&m.id)
                    .and_then(|u| u.first())
                    .map(|u| u.original_filename.as_str()),
                crate::excerpt::Plugins::from_settings(self.settings)?
            )?),
        );
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
        // include_thread_id?: threading on, or a forced thread.
        let thread = m.thread_id.and_then(|id| s.threads.get(&id));
        let include_thread_id = s.threading_enabled || thread.is_some_and(|t| t.force);
        if include_thread_id {
            out.insert("thread_id".into(), json!(m.thread_id));
        }
        let thread = thread.filter(|_| include_thread_id);
        let thread_om = thread.is_some_and(|t| t.original_message_id == m.id);
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
        if let Some(t) = thread.filter(|_| thread_om) {
            let opts = super::threads::ThreadOpts {
                preview: true,
                original_message: s.thread_original_message,
                channel: false,
                membership: s.thread_memberships.as_ref().and_then(|m| m.get(&t.id)),
                participants: s.thread_participants.as_ref().and_then(|p| p.get(&t.id)),
            };
            out.insert("thread".into(), Box::pin(self.thread_json(t, opts)).await?);
        }
        out.insert(
            "blocks".into(),
            match &m.blocks {
                Some(blocks) => serialize_blocks(blocks)?,
                None => json!([]),
            },
        );
        if let Some(t) = thread.filter(|_| !thread_om) {
            out.insert("thread_title".into(), json!(t.title));
        }
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
        out.insert("chat_webhook_event".into(), self.webhook_event(m.id).await?);
        if let Some(parent_id) = m.in_reply_to_id {
            out.insert("in_reply_to".into(), self.in_reply_to(s, parent_id).await?);
        }
        out.insert(
            "uploads".into(),
            Value::Array(
                s.uploads
                    .get(&m.id)
                    .into_iter()
                    .flatten()
                    .map(|u| u.json.clone())
                    .collect(),
            ),
        );
        Ok(Value::Object(out))
    }

    /// Chat::WebhookEventSerializer for the message's webhook event: its
    /// incoming webhook's username and emoji; null without one.
    async fn webhook_event(&mut self, message_id: i64) -> Result<Value, AppError> {
        let event: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT w.username, w.emoji FROM chat_webhook_events e \
             JOIN incoming_chat_webhooks w ON w.id = e.incoming_chat_webhook_id \
             WHERE e.chat_message_id = $1 ORDER BY e.id LIMIT 1",
        )
        .bind(message_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        Ok(match event {
            Some((username, emoji)) => json!({ "username": username, "emoji": emoji }),
            None => Value::Null,
        })
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
        let first_upload = self.first_upload_filename(parent.id).await?;
        Ok(json!({
            "id": parent.id,
            "cooked": parent.cooked,
            "excerpt": excerpt_for_display(&parent, first_upload.as_deref(), crate::excerpt::Plugins::from_settings(self.settings)?)?,
            "user": user,
            "chat_webhook_event": self.webhook_event(parent.id).await?,
        }))
    }

    /// `available_flags`: the flag types (no notify flags in a direct
    /// message, nor notify_user to a bot, or by
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
            .filter(|t| {
                !(s.direct_message
                    && (t.as_str() == "notify_moderators" || t.as_str() == "notify_user"))
            })
            .filter(|t| !(t.as_str() == "notify_user" && (author.id <= 0 || !s.can_message)))
            .cloned()
            .collect()
    }
}

/// `String#to_time` for the dates the client sends: ISO 8601 read as UTC,
/// other spellings through Date._parse.
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
    // Any other spelling as String#to_time reads it: Date._parse's
    // fields, the date's missing parts today's, the time's zero, in the
    // server's zone (UTC) unless it names an offset.
    if !s.is_ascii() {
        return Err(Unsupported("chat target dates that aren't ASCII").into());
    }
    let h = crate::ruby_date::parse(s);
    if h.year.is_none()
        && h.mon.is_none()
        && h.mday.is_none()
        && h.hour.is_none()
        && h.min.is_none()
        && h.sec.is_none()
        && h.zone.is_none()
    {
        // to_time is nil, and `.utc` on it raises: a 500.
        return Err(Unsupported("a chat target date String#to_time can't read").into());
    }
    if h.sec_fraction {
        return Err(Unsupported("chat target dates with fractions of a second").into());
    }
    let offset = match h.zone.as_deref() {
        None => 0,
        Some(z) => zone_offset(z).ok_or(Unsupported("chat target dates in a named time zone"))?,
    };
    let today = crate::clock::now_naive().date();
    let date = chrono::NaiveDate::from_ymd_opt(
        h.year.map_or(today.year(), |y| y as i32),
        h.mon.map_or(today.month(), |m| m as u32),
        h.mday.map_or(today.day(), |d| d as u32),
    )
    .ok_or(Unsupported("chat target dates Time.new rejects"))?;
    let time = date
        .and_hms_opt(
            h.hour.unwrap_or(0) as u32,
            h.min.unwrap_or(0) as u32,
            h.sec.unwrap_or(0) as u32,
        )
        .ok_or(Unsupported("chat target dates Time.new rejects"))?;
    Ok(time - chrono::Duration::seconds(offset))
}

/// The seconds east of UTC a numeric zone (or Z, UTC, GMT) names.
fn zone_offset(zone: &str) -> Option<i64> {
    let z = zone.trim();
    if ["z", "utc", "gmt", "ut"].contains(&z.to_ascii_lowercase().as_str()) {
        return Some(0);
    }
    let (sign, rest) = match z.as_bytes().first()? {
        b'+' => (1, &z[1..]),
        b'-' => (-1, &z[1..]),
        _ => return None,
    };
    let digits: String = rest.chars().filter(|c| *c != ':').collect();
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (hours, minutes) = match digits.len() {
        1 | 2 => (digits.parse::<i64>().ok()?, 0),
        3 | 4 => {
            let split = digits.len() - 2;
            (
                digits[..split].parse::<i64>().ok()?,
                digits[split..].parse::<i64>().ok()?,
            )
        }
        _ => return None,
    };
    Some(sign * (hours * 3600 + minutes * 60))
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
