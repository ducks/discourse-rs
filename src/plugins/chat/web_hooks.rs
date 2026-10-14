//! The chat plugin's outgoing web hook events (plugin.rb):
//! chat_message_created, chat_message_edited, chat_message_trashed and
//! chat_message_restored, each with the message (Chat::MessageSerializer)
//! and its channel (Chat::ChannelSerializer with the user's membership),
//! both as the event's user sees them.

use serde_json::{Map, Value, json};

use super::channels::{ChannelRow, Context, MembershipRow};
use crate::{AppError, Unsupported};

impl Context<'_> {
    /// `WebHook.enqueue_chat_message_hooks(event, payload.to_json,
    /// category_id:)` for a message of the channel, the event's user being
    /// this context's guardian.
    pub(crate) async fn enqueue_message_hooks(
        &mut self,
        event: &str,
        channel_id: i64,
        message_id: i64,
    ) -> Result<(), AppError> {
        if !crate::web_hooks::active(&mut *self.conn, event).await? {
            return Ok(());
        }
        let sql = format!(
            "SELECT {} FROM chat_channels WHERE id = $1",
            super::channels::CHANNEL_COLUMNS
        );
        let channel: ChannelRow = sqlx::query_as(&sql)
            .bind(channel_id)
            .fetch_one(&mut *self.conn)
            .await?;
        let message = self
            .serialize_messages(&channel, &[message_id])
            .await?
            .pop()
            .ok_or(Unsupported("a chat message web hook without its message"))?;
        // channel.membership_for(user)
        let membership: Option<MembershipRow> = match self.guardian.user_id() {
            Some(user_id) => {
                sqlx::query_as(
                    "SELECT chat_channel_id, following, muted, notification_level, \
                            last_read_message_id, last_viewed_at, last_viewed_pins_at, starred \
                     FROM user_chat_channel_memberships WHERE user_id = $1 AND chat_channel_id = $2",
                )
                .bind(user_id)
                .bind(channel.id)
                .fetch_optional(&mut *self.conn)
                .await?
            }
            None => None,
        };
        let mut channel_json = self
            .channel(&channel, membership.as_ref(), None, None)
            .await?;
        // meta's message_bus_last_ids: MessageBus.last_id of each channel,
        // the messages published to it so far.
        if let Some(Value::Object(ids)) = channel_json
            .get_mut("meta")
            .and_then(|m| m.get_mut("message_bus_last_ids"))
        {
            let keys: Vec<String> = ids.keys().cloned().collect();
            for key in keys {
                let bus_channel = match key.as_str() {
                    "channel_message_bus_last_id" => format!("/chat/{}", channel.id),
                    "new_messages" => format!("/chat/{}/new-messages", channel.id),
                    "new_mentions" => format!("/chat/{}/new-mentions", channel.id),
                    "kick" => format!("/chat/{}/kick", channel.id),
                    _ => continue,
                };
                let count: i64 = sqlx::query_scalar(&format!(
                    "SELECT count(*) FROM {}.messages WHERE channel = $1",
                    crate::bus::SCHEMA
                ))
                .bind(&bus_channel)
                .fetch_one(&mut *self.conn)
                .await?;
                ids.insert(key, json!(count));
            }
        }
        let payload = json!({ "message": message, "channel": channel_json });
        let category_id = (channel.chatable_type == "Category").then_some(channel.chatable_id);
        let mut opts = Map::new();
        opts.insert("payload".into(), json!(crate::web_hooks::to_json(&payload)));
        opts.insert("category_id".into(), json!(category_id));
        crate::web_hooks::enqueue_hooks(&mut *self.conn, "chat_message", event, opts).await
    }
}
