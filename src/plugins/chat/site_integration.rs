//! What chat registers on core (plugin.rb): its markdown options on the
//! site (Site.markdown_additional_options["chat"]), its hashtag data
//! source and type priorities (Chat::ChannelHashtagDataSource,
//! register_hashtag_type_priority_for_context), the chat_notifications
//! key of /notifications/totals, the users a post quoting their messages
//! notifies (Chat::PostNotificationHandler), and the chat_summary email
//! unsubscriber.

use serde_json::{Value, json};
use sqlx::PgConnection;

use super::channels::Context;
use crate::AppError;
use crate::site_settings::{SettingError, SiteSettings};

/// `Chat::Message::MARKDOWN_FEATURES`
pub const MARKDOWN_FEATURES: [&str; 22] = [
    "anchor",
    "bbcode-block",
    "bbcode-inline",
    "code",
    "category-hashtag",
    "censored",
    "chat-transcript",
    "discourse-local-dates",
    "emoji",
    "inlineEmoji",
    "html-img",
    "hashtag-autocomplete",
    "mentions",
    "unicodeUsernames",
    "onebox",
    "quotes",
    "spoiler-alert",
    "table",
    "text-post-process",
    "upload-protocol",
    "watched-words",
    "chat-html-inline",
];

/// `Chat::Message::MARKDOWN_IT_RULES`
pub const MARKDOWN_IT_RULES: [&str; 16] = [
    "autolink",
    "list",
    "backticks",
    "newline",
    "code",
    "fence",
    "image",
    "table",
    "linkify",
    "link",
    "strikethrough",
    "blockquote",
    "emphasis",
    "replacements",
    "escape",
    "entity",
];

/// Chat's `register_hashtag_type_priority_for_context` calls, in order:
/// (type, context, priority).
pub const HASHTAG_TYPE_PRIORITIES: [(&str, &str, i64); 4] = [
    ("channel", "chat-composer", 200),
    ("category", "chat-composer", 100),
    ("tag", "chat-composer", 50),
    ("channel", "topic-composer", 10),
];

/// `Chat::ChannelHashtagDataSource.type` and `.icon`.
pub const HASHTAG_TYPE: &str = "channel";
pub const HASHTAG_ICON: &str = "comment";

/// `Chat::ChannelHashtagDataSource.enabled?`
pub fn hashtag_source_enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("enable_public_channels")?.truthy())
}

/// `Site.markdown_additional_options["chat"]`. Rails sets it once in
/// after_initialize, which runs whether or not chat is enabled, so the
/// key is always there; its hashtag configurations are the ones core
/// computes (`contexts_with_ordered_types`), here as of the request.
pub fn markdown_additional_options(hashtag_configurations: Value) -> Value {
    json!({
        "limited_pretty_text_features": MARKDOWN_FEATURES,
        "limited_pretty_text_markdown_rules": MARKDOWN_IT_RULES,
        "hashtag_configurations": hashtag_configurations,
    })
}

impl Context<'_> {
    /// `Chat::ChannelFetcher.unreads_total`: the mentions in the viewer's
    /// followed open category channels plus the unread messages of their
    /// followed DMs.
    pub async fn unreads_total(&mut self) -> Result<i64, AppError> {
        let public: Vec<i64> = self
            .followed_public_channels(false)
            .await?
            .iter()
            .map(|c| c.id)
            .collect();
        let dms: Vec<i64> = self
            .followed_dm_channels(false)
            .await?
            .iter()
            .map(|c| c.id)
            .collect();
        let count = |tracking: &serde_json::Map<String, Value>, key: &str| -> i64 {
            tracking
                .values()
                .map(|t| t[key].as_i64().unwrap_or(0))
                .sum()
        };
        let publics = self.channel_tracking(&public).await?;
        let directs = self.channel_tracking(&dms).await?;
        Ok(count(&publics, "mention_count") + count(&directs, "unread_count"))
    }
}

/// `Chat::PostNotificationHandler#extract_quoted_users`, less those
/// already notified: the authors named by the post's `[chat quote="..."]`
/// tags, the post's author left out. None of them for a whisper, a post
/// without its topic or one in a message (`handle`'s early returns).
pub async fn quoted_user_ids(
    conn: &mut PgConnection,
    raw: &str,
    author_id: Option<i32>,
    notified: &[i32],
) -> Result<Vec<i32>, AppError> {
    // raw.scan(/\[chat quote=\"([^;]+);.+\"\]/): the name runs to the
    // first ";" (newlines included), then `.+\"\]` wants at least one
    // character and a `"]` on that line, the greedy `.+` ending the match
    // at its last one, where the scan goes on.
    const OPEN: &str = "[chat quote=\"";
    let mut usernames: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(start) = raw[from..].find(OPEN).map(|i| from + i) {
        let name_at = start + OPEN.len();
        let Some(semi) = raw[name_at..].find(';').map(|i| name_at + i) else {
            break;
        };
        let line_end = raw[semi + 1..]
            .find('\n')
            .map_or(raw.len(), |i| semi + 1 + i);
        let line = &raw[semi + 1..line_end];
        let close = line
            .char_indices()
            .nth(1)
            .and_then(|(second, _)| line[second..].rfind("\"]").map(|i| second + i));
        match close {
            Some(close) if semi > name_at => {
                let username = raw[name_at..semi].trim().to_lowercase();
                if !usernames.contains(&username) {
                    usernames.push(username);
                }
                from = semi + 1 + close + 2;
            }
            _ => from = start + 1,
        }
    }
    if usernames.is_empty() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_scalar(
        "SELECT id FROM users WHERE username_lower = ANY($1) \
           AND id IS DISTINCT FROM $2 AND NOT (id = ANY($3)) ORDER BY id",
    )
    .bind(&usernames)
    .bind(author_id)
    .bind(notified)
    .fetch_all(conn)
    .await?)
}

/// `UserOption.chat_email_frequencies`, in enum order.
pub const EMAIL_FREQUENCIES: [&str; 2] = ["never", "when_away"];
