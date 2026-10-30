//! Searching chat messages: Chat::SearchMessage (GET /chat/api/search)
//! with Chat::Action::SearchForMessages, ProcessSearchQuery and
//! ApplyFiltersAndSorting, over the chat_message_search_data the indexer
//! writes (posting/search_index.rs).
//!
//! Not ported, and refused: CJK segmentation and search_ignore_accents,
//! as the post search refuses them.

use serde_json::{Map, Value, json};

use super::channels::{ChannelRow, Context};
use crate::{AppError, Unsupported};

/// `ProcessSearchQuery::MAX_FILTERS`
const MAX_FILTERS: usize = 10;

/// The params, as the contract casts them (None where a param casts to
/// nil; a missing limit or offset is already its default).
pub struct SearchParams {
    pub query: Option<String>,
    pub channel_id: Option<i64>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub exclude_threads: bool,
    pub sort: Option<String>,
}

pub enum Searched {
    Found(Value),
    /// The contract's errors (400).
    Invalid(Vec<String>),
    /// No such channel, or one the viewer can't see.
    NotFound,
}

enum Filter {
    Username(String),
    Channel(String),
}

impl Context<'_> {
    /// Chat::SearchMessage, as the guardian searches.
    pub async fn search_messages(&mut self, p: SearchParams) -> Result<Searched, AppError> {
        let settings = self.settings;
        // The contract.
        let mut errors = Vec::new();
        let query = p.query.unwrap_or_default();
        if query.trim().is_empty() {
            errors.push("Query can't be blank".to_string());
        }
        match p.limit {
            None => errors.push("Limit is not a number".to_string()),
            Some(n) if !(1..=40).contains(&n) => errors.push("Limit must be in 1..40".to_string()),
            _ => {}
        }
        match p.offset {
            None => errors.push("Offset is not a number".to_string()),
            Some(n) if n < 0 => {
                errors.push("Offset must be greater than or equal to 0".to_string())
            }
            _ => {}
        }
        let sort = p.sort.unwrap_or_default();
        if sort != "relevance" && sort != "latest" {
            errors.push("Sort is not included in the list".to_string());
        }
        if !errors.is_empty() {
            return Ok(Searched::Invalid(errors));
        }
        let (limit, offset) = (p.limit.unwrap_or(20), p.offset.unwrap_or(0));
        let query = crate::search::clean_term(&query);

        // only_if(:provided_channel_id): model :channel, policy
        // :can_view_channel (can_preview_anonymous_public_chat_channel?
        // can't apply: the controller wants a member who can chat).
        let channel = match p.channel_id {
            None => None,
            Some(id) => {
                let sql = format!(
                    "SELECT {} FROM chat_channels WHERE id = $1 AND deleted_at IS NULL",
                    super::channels::CHANNEL_COLUMNS
                );
                let channel: Option<ChannelRow> = sqlx::query_as(&sql)
                    .bind(id)
                    .fetch_optional(&mut *self.conn)
                    .await?;
                let Some(channel) = channel else {
                    return Ok(Searched::NotFound);
                };
                if !self.can_join(&channel, None).await? {
                    return Ok(Searched::NotFound);
                }
                Some(channel)
            }
        };

        // ProcessSearchQuery: @username and #channel words are filters
        // (the first ten), the rest the query.
        let mut filters = Vec::new();
        let mut words = Vec::new();
        // Ruby's /\s+/: ASCII whitespace.
        let split = query
            .split(|c: char| c.is_ascii_whitespace() || c == '\x0b')
            .filter(|w| !w.is_empty());
        for word in split {
            match (word.strip_prefix('@'), word.strip_prefix('#')) {
                (Some(name), _) if !name.is_empty() => {
                    filters.push(Filter::Username(name.to_string()))
                }
                (_, Some(slug)) if !slug.is_empty() => {
                    filters.push(Filter::Channel(slug.to_string()))
                }
                _ => words.push(word),
            }
        }
        filters.truncate(MAX_FILTERS);
        let processed = words.join(" ");
        if processed.is_empty() && filters.is_empty() {
            return Ok(Searched::Found(json!({
                "messages": [],
                "meta": {"has_more": false, "limit": limit, "offset": offset},
            })));
        }

        let scope = match &channel {
            Some(channel) => vec![channel.id],
            None => {
                super::bookmarks::all_secured_channel_ids(&mut *self.conn, settings, self.guardian)
                    .await?
            }
        };
        let mut sql = String::from(
            "SELECT chat_messages.id, chat_messages.chat_channel_id FROM chat_messages \
             INNER JOIN chat_channels ON chat_channels.id = chat_messages.chat_channel_id \
               AND chat_channels.deleted_at IS NULL",
        );
        if !processed.is_empty() {
            sql.push_str(
                " INNER JOIN chat_message_search_data \
                   ON chat_message_search_data.chat_message_id = chat_messages.id",
            );
        }
        sql.push_str(" WHERE chat_messages.deleted_at IS NULL AND chat_channels.id = ANY($1)");
        for filter in &filters {
            match filter {
                Filter::Username(name) => {
                    let mut name = crate::avatar::normalize_username(name);
                    if name == "me" {
                        name = self
                            .guardian
                            .user()
                            .map(|u| u.username.to_lowercase())
                            .unwrap_or_default();
                    }
                    let id: Option<i32> = sqlx::query_scalar(
                        "SELECT id FROM users WHERE username_lower = $1 AND NOT staged",
                    )
                    .bind(&name)
                    .fetch_optional(&mut *self.conn)
                    .await?;
                    match id {
                        Some(id) => sql.push_str(&format!(" AND chat_messages.user_id = {id}")),
                        None => sql.push_str(" AND 1 = 0"),
                    }
                }
                Filter::Channel(slug) => {
                    let sql_channel = format!(
                        "SELECT {} FROM chat_channels WHERE slug = $1 AND deleted_at IS NULL LIMIT 1",
                        super::channels::CHANNEL_COLUMNS
                    );
                    let found: Option<ChannelRow> = sqlx::query_as(&sql_channel)
                        .bind(slug.to_lowercase())
                        .fetch_optional(&mut *self.conn)
                        .await?;
                    match found {
                        Some(c) if self.can_preview(&c).await? => {
                            sql.push_str(&format!(" AND chat_channels.id = {}", c.id))
                        }
                        _ => sql.push_str(" AND 1 = 0"),
                    }
                }
            }
        }
        let ts_value = if processed.is_empty() {
            String::new()
        } else {
            if settings.get("search_ignore_accents")?.truthy() {
                return Err(Unsupported("search_ignore_accents (unaccent)").into());
            }
            let locale = settings.get("default_locale")?.to_s();
            if settings.get("search_tokenize_chinese")?.truthy()
                || settings.get("search_tokenize_japanese")?.truthy()
                || locale.starts_with("zh")
                || locale.starts_with("ja")
            {
                return Err(Unsupported("CJK segmentation in chat search").into());
            }
            sql.push_str(&format!(
                " AND chat_message_search_data.search_data @@ {}",
                crate::search::ts_query_sql(crate::search::ts_config(&locale), 2)
            ));
            // Search.prepare_data: URLs lose their query.
            crate::search::ts_query_value(
                &crate::search::uri::strip_url_queries(&processed),
                "",
                true,
            )
        };
        if ts_value.is_empty() {
            sql.push_str(" AND $2::text IS NOT NULL");
        }
        // ApplyFiltersAndSorting
        if p.exclude_threads {
            sql.push_str(
                " AND (chat_messages.thread_id IS NULL OR chat_messages.id IN ( \
                   SELECT ct.original_message_id FROM chat_threads ct WHERE ct.id = chat_messages.thread_id))",
            );
        }
        // "relevance" orders nothing, as Rails doesn't.
        if sort == "latest" {
            sql.push_str(" ORDER BY chat_messages.created_at DESC");
        }
        sql.push_str(" OFFSET $3 LIMIT $4");
        let rows: Vec<(i64, i64)> = sqlx::query_as(&sql)
            .bind(&scope)
            .bind(&ts_value)
            .bind(offset)
            .bind(limit + 1)
            .fetch_all(&mut *self.conn)
            .await?;
        let has_more = rows.len() as i64 > limit;

        // Chat::MessageSerializer with include_channel.
        let mut channels: Vec<ChannelRow> = Vec::new();
        let mut messages = Vec::new();
        for (id, channel_id) in rows.into_iter().take(limit as usize) {
            if !channels.iter().any(|c| c.id == channel_id) {
                let sql = format!(
                    "SELECT {} FROM chat_channels WHERE id = $1",
                    super::channels::CHANNEL_COLUMNS
                );
                channels.push(
                    sqlx::query_as(&sql)
                        .bind(channel_id)
                        .fetch_one(&mut *self.conn)
                        .await?,
                );
            }
            let channel = channels
                .iter()
                .find(|c| c.id == channel_id)
                .cloned()
                .ok_or(Unsupported("chat search hits without a channel"))?;
            let serialized = self.serialize_messages(&channel, &[id]).await?;
            let channel_json = self.channel(&channel, None, None, None).await?;
            for m in serialized {
                messages.push(with_channel(m, &channel_json));
            }
        }
        Ok(Searched::Found(json!({
            "messages": messages,
            "meta": {"has_more": has_more, "limit": limit, "offset": offset},
        })))
    }
}

/// The serialized message with its `channel` where the serializer's
/// attributes put it, after `blocks`.
pub(crate) fn with_channel(message: Value, channel: &Value) -> Value {
    let Value::Object(map) = message else {
        return message;
    };
    let mut out = Map::new();
    for (k, v) in map {
        let blocks = k == "blocks";
        out.insert(k, v);
        if blocks {
            out.insert("channel".into(), channel.clone());
        }
    }
    Value::Object(out)
}
