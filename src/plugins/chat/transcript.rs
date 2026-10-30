//! Chat::TranscriptService#generate_markdown as POST
//! /chat/:chat_channel_id/quote (ChatController#quote_messages) asks for
//! it: a `[chat]` block per run of one author's messages, its quote,
//! channel, multiQuote and chained attributes. At this reference the
//! block carries its attributes only (render leaves the messages out).
//!
//! Not ported, and refused: channels with threading on, and messages of
//! deleted users.

use super::channels::{ChannelRow, Context};
use crate::{AppError, Unsupported};

/// `TranscriptService::QUOTATION_MARKS`
const QUOTATION_MARKS: [&str; 9] = ["\"\"", "''", "«»", "“”", "””", "‘’", "„“", "‚’", "‹›"];

/// `serialize_attr`: the value in the first pair of marks it contains
/// neither of, else in double quotes without its own.
fn serialize_attr(name: &str, value: &str) -> String {
    for pair in QUOTATION_MARKS {
        let mut chars = pair.chars();
        let (open, close) = (chars.next().unwrap_or('"'), chars.next().unwrap_or('"'));
        if value.contains(open) || value.contains(close) {
            continue;
        }
        return format!("{name}={open}{value}{close}");
    }
    format!("{name}=\"{}\"", value.replace('"', ""))
}

impl Context<'_> {
    /// The transcript of the channel's messages among `message_ids`, as the
    /// guardian's user quotes them.
    pub async fn transcript_markdown(
        &mut self,
        channel: &ChannelRow,
        message_ids: &[i64],
    ) -> Result<String, AppError> {
        if channel.threading_enabled {
            return Err(Unsupported("transcripts of chat channels with threads").into());
        }
        let messages: Vec<(i64, Option<i32>, chrono::NaiveDateTime, Option<String>)> =
            sqlx::query_as(
                "SELECT chat_messages.id, chat_messages.user_id, chat_messages.created_at, users.username \
                 FROM chat_messages LEFT JOIN users ON users.id = chat_messages.user_id \
                 WHERE chat_messages.id = ANY($1) AND chat_messages.chat_channel_id = $2 \
                   AND chat_messages.deleted_at IS NULL \
                 ORDER BY chat_messages.created_at",
            )
            .bind(message_ids)
            .bind(channel.id)
            .fetch_all(&mut *self.conn)
            .await?;
        if messages.is_empty() {
            return Err(Unsupported("a transcript of no chat messages").into());
        }
        if messages.iter().any(|m| m.3.is_none()) {
            return Err(Unsupported("transcripts of deleted users' chat messages").into());
        }
        let first_user = messages[0].1;
        let all_same_user = messages.iter().all(|m| m.1 == first_user);
        let title = self.channel(channel, None, None, None).await?["title"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        // The first block names the channel and whether it quotes several;
        // a new one starts at each change of author.
        let mut blocks: Vec<String> = Vec::new();
        let mut first = true;
        let mut previous: Option<Option<i32>> = None;
        for (id, user_id, created_at, username) in &messages {
            if previous.is_some_and(|p| p != *user_id) || previous.is_none() {
                let mut attrs = vec![format!(
                    "quote=\"{};{id};{}\"",
                    username.as_deref().unwrap_or_default(),
                    created_at.format("%Y-%m-%dT%H:%M:%SZ")
                )];
                if first {
                    attrs.push(serialize_attr("channel", &title));
                    attrs.push(format!("channelId=\"{}\"", channel.id));
                    if messages.len() > 1 {
                        attrs.push("multiQuote=\"true\"".into());
                    }
                }
                if !all_same_user {
                    attrs.push("chained=\"true\"".into());
                }
                blocks.push(format!("[chat {}]\n[/chat]\n", attrs.join(" ")));
                first = false;
            }
            previous = Some(*user_id);
        }
        Ok(blocks.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::serialize_attr;

    #[test]
    fn serialize_attr_picks_marks_the_value_lacks() {
        assert_eq!(serialize_attr("channel", "General"), "channel=\"General\"");
        assert_eq!(
            serialize_attr("channel", "say \"hi\""),
            "channel='say \"hi\"'"
        );
    }
}
