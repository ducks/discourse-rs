//! Chat's incoming webhooks: POST /chat/hooks/:key and its Slack
//! compatible twin (Chat::IncomingWebhooksController with
//! Chat::SlackCompatibility), and the admin's webhooks
//! (Chat::Admin::IncomingWebhooksController: the list, new, edit, create,
//! update and destroy) with Chat::IncomingWebhookSerializer.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::channels::{CHANNEL_COLUMNS, ChannelRow, Context};
use crate::AppError;

/// `WEBHOOK_MESSAGES_PER_MINUTE_LIMIT`
pub const MESSAGES_PER_MINUTE: i64 = 10;

/// A row of incoming_chat_webhooks.
#[derive(sqlx::FromRow, Clone)]
pub struct Webhook {
    pub id: i64,
    pub name: String,
    pub key: String,
    pub chat_channel_id: i64,
    pub description: Option<String>,
    pub emoji: Option<String>,
    pub username: Option<String>,
    pub updated_at: chrono::NaiveDateTime,
}

const COLUMNS: &str = "id, name, key, chat_channel_id, description, emoji, username, updated_at";

pub async fn find_by_key(conn: &mut PgConnection, key: &str) -> Result<Option<Webhook>, AppError> {
    Ok(sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM incoming_chat_webhooks WHERE key = $1 ORDER BY id LIMIT 1"
    ))
    .bind(key)
    .fetch_optional(conn)
    .await?)
}

pub async fn find(conn: &mut PgConnection, id: i64) -> Result<Option<Webhook>, AppError> {
    Ok(sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM incoming_chat_webhooks WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(conn)
    .await?)
}

/// `Chat::IncomingWebhook.includes(:chat_channel).all`, by id.
pub async fn all(conn: &mut PgConnection) -> Result<Vec<Webhook>, AppError> {
    Ok(sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM incoming_chat_webhooks ORDER BY id"
    ))
    .fetch_all(conn)
    .await?)
}

/// `Chat::SlackCompatibility.process_text`: Slack's `<!here>` and
/// `<!all>` mentions, and its `<link>` and `<link|title>` links as
/// Markdown.
pub fn process_text(text: &str) -> String {
    static LINK: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(<[^\n<|>]+>|<[^\n<>]+>)").expect("the mrkdwn link pattern")
    });
    let mut text = text.replace("<!here>", "@here").replace("<!all>", "@all");
    let matches: Vec<String> = LINK
        .find_iter(&text.clone())
        .map(|m| m.as_str().to_string())
        .collect();
    for m in matches {
        let (link, title) = if m.contains('|') {
            let mut parts = m.split('|');
            (
                parts.next().unwrap_or_default().to_string(),
                parts.next().map(str::to_string),
            )
        } else {
            (m.clone(), None)
        };
        let strip = |s: &str| s.replace(['<', '>'], "");
        let link = strip(&link);
        text = match title {
            Some(title) => text.replace(&m, &format!("[{}]({link})", strip(&title))),
            None => text.replace(&m, &link),
        };
    }
    text
}

/// The payload's first legacy attachment's fallback, unescaped
/// (`process_legacy_attachments`); None where Rails raises NoMethodError.
pub fn legacy_attachment_text(attachments: &Value) -> Option<String> {
    let fallback = attachments.get(0)?.get("fallback")?.as_str()?;
    Some(process_text(&crate::category_writes::cgi_unescape(
        fallback,
    )))
}

impl Context<'_> {
    /// Chat::IncomingWebhookSerializer: its channel (ChannelSerializer, no
    /// membership) and attributes.
    pub async fn webhook_json(&mut self, w: &Webhook, base_url: &str) -> Result<Value, AppError> {
        let channel: Option<ChannelRow> = sqlx::query_as(&format!(
            "SELECT {CHANNEL_COLUMNS} FROM chat_channels WHERE id = $1 AND deleted_at IS NULL"
        ))
        .bind(w.chat_channel_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let channel = match channel {
            Some(c) => self.channel(&c, None, None, None).await?,
            None => Value::Null,
        };
        let mut out = Map::new();
        out.insert("id".into(), json!(w.id));
        out.insert("name".into(), json!(w.name));
        out.insert("description".into(), json!(w.description));
        out.insert("emoji".into(), json!(w.emoji));
        out.insert(
            "url".into(),
            json!(format!("{base_url}/chat/hooks/{}", w.key)),
        );
        out.insert("username".into(), json!(w.username));
        out.insert(
            "updated_at".into(),
            json!(super::channels::time_json(w.updated_at)),
        );
        out.insert("chat_channel".into(), channel);
        Ok(Value::Object(out))
    }

    /// `Chat::Channel.public_channels` through ChannelSerializer.
    pub async fn admin_public_channels(&mut self) -> Result<Vec<Value>, AppError> {
        let rows: Vec<ChannelRow> = sqlx::query_as(&format!(
            "SELECT {CHANNEL_COLUMNS} FROM chat_channels \
             JOIN categories ON categories.id = chat_channels.chatable_id \
             WHERE chat_channels.chatable_type = 'Category' AND chat_channels.deleted_at IS NULL \
             ORDER BY chat_channels.id"
        ))
        .fetch_all(&mut *self.conn)
        .await?;
        let mut out = Vec::new();
        for c in &rows {
            out.push(self.channel(c, None, None, None).await?);
        }
        Ok(out)
    }
}

/// A webhook's validations' full messages.
pub fn validate(
    name: &str,
    username: Option<&str>,
    description: Option<&str>,
    emoji: Option<&str>,
) -> Vec<String> {
    let mut errors = Vec::new();
    if name.trim().is_empty() {
        errors.push("Name can't be blank".to_string());
    }
    if name.chars().count() > 100 {
        errors.push("Name is too long (maximum is 100 characters)".to_string());
    }
    if username.is_some_and(|u| u.chars().count() > 100) {
        errors.push("Username is too long (maximum is 100 characters)".to_string());
    }
    if description.is_some_and(|d| d.chars().count() > 500) {
        errors.push("Description is too long (maximum is 500 characters)".to_string());
    }
    if emoji.is_some_and(|e| e.chars().count() > 100) {
        errors.push("Emoji is too long (maximum is 100 characters)".to_string());
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slack_text_becomes_markdown() {
        assert_eq!(process_text("hey <!here> and <!all>"), "hey @here and @all");
        assert_eq!(
            process_text("see <https://example.com|the site> or <https://other.com>"),
            "see [the site](https://example.com) or https://other.com"
        );
    }

    #[test]
    fn a_legacy_attachment_gives_its_fallback() {
        let attachments = json!([{ "fallback": "Alert%20%3Chttps%3A%2F%2Fx.com%7Clink%3E" }]);
        assert_eq!(
            legacy_attachment_text(&attachments).as_deref(),
            Some("Alert [link](https://x.com)")
        );
        assert_eq!(legacy_attachment_text(&json!([])), None);
    }
}
