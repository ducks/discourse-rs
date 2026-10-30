//! Pinned chat messages: Chat::ListChannelPins (GET
//! /chat/api/channels/:channel_id/pins), Chat::MarkPinsAsRead (PUT
//! .../pins/read), Chat::PinMessage and Chat::UnpinMessage (POST and
//! DELETE .../messages/:message_id/pin), with Chat::ChannelPinsSerializer
//! and the pin and unpin publishes.

use chrono::NaiveDateTime;
use serde_json::{Value, json};

use super::channels::{ChannelRow, Context};
use crate::{AppError, Unsupported};

/// `Chat::PinnedMessage::MAX_PINS_PER_CHANNEL`
pub const MAX_PINS_PER_CHANNEL: i64 = 20;

pub enum PinOutcome {
    Done(Value),
    NotFound,
    /// `Discourse::InvalidAccess`
    Forbidden,
    /// The contract's errors (400).
    Invalid(Vec<String>),
    /// A policy's error (422, `failed_json` with its `error`).
    Failed(String),
}

#[derive(sqlx::FromRow)]
struct PinRow {
    id: i64,
    chat_message_id: i64,
    pinned_by_id: i64,
    created_at: NaiveDateTime,
    message: Option<String>,
    cooked: Option<String>,
}

fn t(i18n: &crate::i18n::I18n, key: &str) -> String {
    i18n.t(key).unwrap_or(key).to_string()
}

impl Context<'_> {
    /// `Chat::Channel.find_by(id:)`: live channels.
    async fn pins_channel(&mut self, channel_id: i64) -> Result<Option<ChannelRow>, AppError> {
        let sql = format!(
            "SELECT {} FROM chat_channels WHERE id = $1 AND deleted_at IS NULL",
            super::channels::CHANNEL_COLUMNS
        );
        Ok(sqlx::query_as(&sql)
            .bind(channel_id)
            .fetch_optional(&mut *self.conn)
            .await?)
    }

    /// `can_manage_chat_channel_pins?`: a member who can chat and see the
    /// channel; in a category channel, of chat_pinning_messages_allowed_groups.
    pub(crate) async fn can_manage_pins(&mut self, channel: &ChannelRow) -> Result<bool, AppError> {
        let g = self.guardian;
        if !super::can_chat(&mut *self.conn, self.settings, g).await? {
            return Ok(false);
        }
        if !self.can_preview(channel).await? {
            return Ok(false);
        }
        Ok(channel.chatable_type == "DirectMessage"
            || g.in_setting_groups(self.settings, "chat_pinning_messages_allowed_groups")?)
    }

    /// Chat::ListChannelPins with Chat::ChannelPinsSerializer: the pins in
    /// timeline order and the viewer's membership.
    pub async fn list_pins(&mut self, channel_id: Option<i64>) -> Result<PinOutcome, AppError> {
        let Some(channel_id) = channel_id else {
            return Ok(PinOutcome::Invalid(vec!["Channel can't be blank".into()]));
        };
        let Some(channel) = self.pins_channel(channel_id).await? else {
            return Ok(PinOutcome::NotFound);
        };
        // policy :can_view_channel; the anonymous preview can't apply
        // behind the controller's guard.
        if !self.can_join(&channel, None).await? {
            return Ok(PinOutcome::Forbidden);
        }
        let membership = self.own_membership(channel.id).await?;
        let pins: Vec<PinRow> = sqlx::query_as(
            "SELECT p.id, p.chat_message_id, p.pinned_by_id, p.created_at, m.message, m.cooked \
             FROM chat_pinned_messages p JOIN chat_messages m ON m.id = p.chat_message_id \
             WHERE p.chat_channel_id = $1 AND m.deleted_at IS NULL ORDER BY p.chat_message_id",
        )
        .bind(channel.id)
        .fetch_all(&mut *self.conn)
        .await?;
        let plugins = crate::excerpt::Plugins::from_settings(self.settings)?;
        let mut list = Vec::new();
        for pin in &pins {
            let excerpt = super::messages::build_excerpt_keeping_links(
                pin.message.as_deref().unwrap_or_default(),
                pin.cooked.as_deref().unwrap_or_default(),
                plugins,
            )?;
            let pinned_by_id = i32::try_from(pin.pinned_by_id)
                .map_err(|_| Unsupported("chat pins by users past int4"))?;
            let pinned_by = super::publisher::basic_user(self, pinned_by_id).await?;
            let message = self
                .serialize_messages(&channel, &[pin.chat_message_id])
                .await?
                .into_iter()
                .next()
                .unwrap_or(Value::Null);
            list.push(json!({
                "id": pin.id,
                "chat_message_id": pin.chat_message_id,
                "pinned_at": crate::topic_list::time_json(pin.created_at),
                "excerpt": excerpt,
                "pinned_by": pinned_by,
                "message": message,
            }));
        }
        let membership = match membership {
            Some(m) => self.membership_json(&channel, &m).await?,
            None => Value::Null,
        };
        Ok(PinOutcome::Done(
            json!({"pinned_messages": list, "membership": membership}),
        ))
    }

    /// Chat::MarkPinsAsRead: the member's last_viewed_pins_at, now.
    pub async fn mark_pins_read(
        &mut self,
        channel_id: Option<i64>,
    ) -> Result<PinOutcome, AppError> {
        let Some(channel_id) = channel_id else {
            return Ok(PinOutcome::Invalid(vec!["Channel can't be blank".into()]));
        };
        let Some(channel) = self.pins_channel(channel_id).await? else {
            return Ok(PinOutcome::NotFound);
        };
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(PinOutcome::NotFound);
        };
        if self.own_membership(channel.id).await?.is_none() {
            return Ok(PinOutcome::NotFound);
        }
        if !self.can_preview(&channel).await? {
            return Ok(PinOutcome::Forbidden);
        }
        let now = crate::clock::now_naive();
        sqlx::query(
            "UPDATE user_chat_channel_memberships SET last_viewed_pins_at = $3, updated_at = $3 \
             WHERE user_id = $1 AND chat_channel_id = $2",
        )
        .bind(user_id)
        .bind(channel.id)
        .bind(now)
        .execute(&mut *self.conn)
        .await?;
        Ok(PinOutcome::Done(json!({"success": "OK"})))
    }

    /// The pin and unpin contract, then `model :message` (a live message
    /// of the channel) and its channel.
    async fn pin_message_model(
        &mut self,
        channel_id: Option<i64>,
        message_id: Option<i64>,
    ) -> Result<Result<(ChannelRow, i64), PinOutcome>, AppError> {
        let mut errors = Vec::new();
        if message_id.is_none() {
            errors.push("Message can't be blank".to_string());
        }
        if channel_id.is_none() {
            errors.push("Channel can't be blank".to_string());
        }
        let (Some(channel_id), Some(message_id)) = (channel_id, message_id) else {
            return Ok(Err(PinOutcome::Invalid(errors)));
        };
        let found: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM chat_messages WHERE id = $1 AND chat_channel_id = $2 AND deleted_at IS NULL",
        )
        .bind(message_id)
        .bind(channel_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        if found.is_none() {
            return Ok(Err(PinOutcome::NotFound));
        }
        let Some(channel) = self.pins_channel(channel_id).await? else {
            return Err(Unsupported("pins in deleted chat channels").into());
        };
        Ok(Ok((channel, message_id)))
    }

    /// Chat::PinMessage, by the guardian's user, and publish_pin!.
    pub async fn pin_message(
        &mut self,
        bus: &pg_bus::Bus,
        channel_id: Option<i64>,
        message_id: Option<i64>,
    ) -> Result<PinOutcome, AppError> {
        let (channel, message_id) = match self.pin_message_model(channel_id, message_id).await? {
            Ok(found) => found,
            Err(refused) => return Ok(refused),
        };
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(PinOutcome::Forbidden);
        };
        if !self.can_manage_pins(&channel).await? {
            return Ok(PinOutcome::Forbidden);
        }
        let count = pin_count(&mut *self.conn, channel.id).await?;
        if count >= MAX_PINS_PER_CHANNEL {
            return Ok(PinOutcome::Failed(
                t(self.i18n, "chat.errors.pin_limit_reached")
                    .replace("%{limit}", &MAX_PINS_PER_CHANNEL.to_string()),
            ));
        }
        let pinned: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM chat_pinned_messages WHERE chat_message_id = $1)",
        )
        .bind(message_id)
        .fetch_one(&mut *self.conn)
        .await?;
        if pinned {
            return Ok(PinOutcome::Failed(t(
                self.i18n,
                "chat.errors.message_already_pinned",
            )));
        }
        let now = crate::clock::now_naive();
        sqlx::query(
            "INSERT INTO chat_pinned_messages (chat_message_id, chat_channel_id, pinned_by_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $4)",
        )
        .bind(message_id)
        .bind(channel.id)
        .bind(i64::from(user_id))
        .bind(now)
        .execute(&mut *self.conn)
        .await?;
        let payload = json!({
            "type": "pin",
            "chat_message_id": message_id,
            "pinned_at": crate::topic_list::time_json(now),
            "pinned_by_id": user_id,
            "pinned_message_count": count + 1,
        });
        super::publisher::publish_to_channel(self, bus, &channel, &payload).await?;
        Ok(PinOutcome::Done(json!({"success": "OK"})))
    }

    /// Chat::UnpinMessage, by the guardian's user, and publish_unpin!.
    pub async fn unpin_message(
        &mut self,
        bus: &pg_bus::Bus,
        channel_id: Option<i64>,
        message_id: Option<i64>,
    ) -> Result<PinOutcome, AppError> {
        let (channel, message_id) = match self.pin_message_model(channel_id, message_id).await? {
            Ok(found) => found,
            Err(refused) => return Ok(refused),
        };
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(PinOutcome::Forbidden);
        };
        if !self.can_manage_pins(&channel).await? {
            return Ok(PinOutcome::Forbidden);
        }
        let destroyed = sqlx::query("DELETE FROM chat_pinned_messages WHERE chat_message_id = $1")
            .bind(message_id)
            .execute(&mut *self.conn)
            .await?
            .rows_affected();
        if destroyed == 0 {
            return Ok(PinOutcome::Failed(t(
                self.i18n,
                "chat.errors.message_not_pinned",
            )));
        }
        super::publisher::publish_unpin(self, bus, &channel, message_id, user_id).await?;
        Ok(PinOutcome::Done(json!({"success": "OK"})))
    }
}

async fn pin_count(conn: &mut sqlx::PgConnection, channel_id: i64) -> Result<i64, AppError> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM chat_pinned_messages WHERE chat_channel_id = $1")
            .bind(channel_id)
            .fetch_one(conn)
            .await?,
    )
}
