//! Moving chat messages to another channel: Chat::Api::
//! ChannelsMessagesMovesController#create (POST
//! /chat/api/channels/:channel_id/messages/moves) with Chat::MessageMover:
//! the messages copied into the destination (their reactions, uploads,
//! mentions, revisions, webhook events and hotlinked media following
//! them), the originals trashed and published as a bulk delete, replies
//! to them cut, and the system user's placeholder in the source.
//!
//! Not ported, and refused: moving threads.

use serde_json::{Value, json};

use super::channels::{ChannelRow, Context, Found};
use crate::{AppError, Unsupported};

pub enum Moved {
    Done(Value),
    NotFound,
    Forbidden,
    /// `ActionController::ParameterMissing`
    ParamMissing(&'static str),
    /// NoMessagesFound and InvalidChannel (render_json_error, 422).
    Failed(String),
}

fn t(i18n: &crate::i18n::I18n, key: &str) -> String {
    i18n.t(key).unwrap_or(key).to_string()
}

impl Context<'_> {
    /// The move, by the guardian's user. `message_ids` and
    /// `destination_channel_id` are the `move` params (None when absent).
    pub async fn move_messages(
        &mut self,
        host: &crate::pretty_text::Host,
        bus: &pg_bus::Bus,
        channel_id: &str,
        message_ids: Option<&[String]>,
        destination_channel_id: Option<&str>,
    ) -> Result<Moved, AppError> {
        // channel_from_params, before the action's requires.
        let source = match self.find_joinable(channel_id).await? {
            Found::Channel(channel) => channel,
            Found::NotFound => return Ok(Moved::NotFound),
            Found::Forbidden => return Ok(Moved::Forbidden),
        };
        let Some(message_ids) = message_ids.filter(|ids| !ids.is_empty()) else {
            return Ok(Moved::ParamMissing("message_ids"));
        };
        let Some(destination_id) = destination_channel_id.filter(|d| !d.trim().is_empty()) else {
            return Ok(Moved::ParamMissing("destination_channel_id"));
        };
        let Some(user) = self.guardian.user().cloned() else {
            return Ok(Moved::Forbidden);
        };
        // can_move_chat_messages?
        if !self.can_moderate(&source).await? {
            return Ok(Moved::Forbidden);
        }
        let destination = match self.find_joinable(destination_id).await? {
            Found::Channel(channel) => channel,
            Found::NotFound => return Ok(Moved::NotFound),
            Found::Forbidden => return Ok(Moved::Forbidden),
        };
        let ids: Vec<i64> = message_ids.iter().map(|id| crate::ruby::to_i(id)).collect();

        // MessageMover.new: the source's live messages among the ids, in
        // the order they were made.
        let found: Vec<(i64, Option<i64>)> = sqlx::query_as(
            "SELECT id, thread_id FROM chat_messages WHERE chat_channel_id = $1 AND deleted_at IS NULL \
               AND (id = ANY($2) OR thread_id IN (SELECT thread_id FROM chat_messages WHERE id = ANY($2))) \
             ORDER BY created_at ASC, id ASC",
        )
        .bind(source.id)
        .bind(&ids)
        .fetch_all(&mut *self.conn)
        .await?;
        let ordered: Vec<i64> = found.iter().map(|(id, _)| *id).collect();
        // message_ids & found: in the order asked, once each.
        let mut source_ids: Vec<i64> = Vec::new();
        for id in &ids {
            if ordered.contains(id) && !source_ids.contains(id) {
                source_ids.push(*id);
            }
        }

        // move_to_channel
        if source.chatable_type != "Category" || destination.chatable_type != "Category" {
            return Ok(Moved::Failed(t(
                self.i18n,
                "chat.errors.message_move_invalid_channel",
            )));
        }
        if ordered.is_empty() {
            return Ok(Moved::Failed(t(
                self.i18n,
                "chat.errors.message_move_no_messages_found",
            )));
        }
        if found.iter().any(|(_, thread)| thread.is_some()) {
            return Err(Unsupported("moving chat threads").into());
        }

        // create_destination_messages_in_channel, without in_reply_to_id.
        let mut moved: Vec<(i64, i64)> = Vec::new();
        for old in &ordered {
            let new: i64 = sqlx::query_scalar(
                "INSERT INTO chat_messages (chat_channel_id, user_id, last_editor_id, message, cooked, \
                                            cooked_version, thread_id, created_at, updated_at) \
                 SELECT $1, user_id, last_editor_id, message, cooked, cooked_version, NULL, \
                        CLOCK_TIMESTAMP(), CLOCK_TIMESTAMP() \
                 FROM chat_messages WHERE id = $2 RETURNING id",
            )
            .bind(destination.id)
            .bind(old)
            .fetch_one(&mut *self.conn)
            .await?;
            moved.push((*old, new));
        }
        // update_message_references
        let (olds, news): (Vec<i64>, Vec<i64>) = moved.iter().copied().unzip();
        for (table, column, extra) in [
            ("chat_message_reactions", "chat_message_id", ""),
            (
                "upload_references",
                "target_id",
                " AND t.target_type = 'ChatMessage'",
            ),
            ("chat_mentions", "chat_message_id", ""),
            ("chat_message_revisions", "chat_message_id", ""),
            ("chat_webhook_events", "chat_message_id", ""),
            ("chat_message_hotlinked_media", "chat_message_id", ""),
        ] {
            sqlx::query(&format!(
                "UPDATE {table} t SET {column} = mm.new_id \
                 FROM UNNEST($1::bigint[], $2::bigint[]) AS mm(old_id, new_id) \
                 WHERE t.{column} = mm.old_id{extra}"
            ))
            .bind(&olds)
            .bind(&news)
            .execute(&mut *self.conn)
            .await?;
        }
        // delete_source_messages
        sqlx::query(
            "UPDATE chat_messages SET deleted_at = NOW(), deleted_by_id = $2 WHERE id = ANY($1)",
        )
        .bind(&source_ids)
        .bind(user.id)
        .execute(&mut *self.conn)
        .await?;
        let published: Vec<Value> = source_ids.iter().map(|id| json!(id)).collect();
        super::publisher::publish_bulk_delete(self, bus, &source, &published).await?;
        // update_reply_references
        sqlx::query(
            "UPDATE chat_messages SET in_reply_to_id = NULL WHERE in_reply_to_id = ANY($1)",
        )
        .bind(&source_ids)
        .execute(&mut *self.conn)
        .await?;
        // update_tracking_state: ResetUserLastReadChannelMessage.
        sqlx::query(
            "WITH cte AS (SELECT chat_channels.id AS chat_channel_id, last_message_id FROM chat_channels \
               WHERE chat_channels.id = $2) \
             UPDATE user_chat_channel_memberships SET last_read_message_id = cte.last_message_id FROM cte \
             WHERE user_chat_channel_memberships.last_read_message_id = ANY($1) \
               AND cte.chat_channel_id = user_chat_channel_memberships.chat_channel_id",
        )
        .bind(&source_ids)
        .bind(source.id)
        .execute(&mut *self.conn)
        .await?;

        let first_moved = news[0];
        let destination_title = self.title_for_viewer(&destination).await?;
        self.add_moved_placeholder(
            host,
            bus,
            &source,
            &destination_title,
            first_moved,
            &user.username,
            &destination,
            source_ids.len(),
        )
        .await?;
        Ok(Moved::Done(json!({
            "success": "OK",
            "destination_channel_id": destination.id,
            "destination_channel_title": destination_title,
            "first_moved_message_id": first_moved,
        })))
    }

    /// `channel.title(user)`: the serializer's title.
    async fn title_for_viewer(&mut self, channel: &ChannelRow) -> Result<String, AppError> {
        let json = self.channel(channel, None, None, None).await?;
        Ok(json["title"].as_str().unwrap_or_default().to_string())
    }

    /// `add_moved_placeholder`: the system user, a member of the source,
    /// says where the messages went (Chat::CreateMessage).
    #[allow(clippy::too_many_arguments)]
    async fn add_moved_placeholder(
        &mut self,
        host: &crate::pretty_text::Host,
        bus: &pg_bus::Bus,
        source: &ChannelRow,
        destination_title: &str,
        first_moved: i64,
        acting_username: &str,
        destination: &ChannelRow,
        count: usize,
    ) -> Result<(), AppError> {
        let key = if count == 1 {
            "chat.channel.messages_moved.one"
        } else {
            "chat.channel.messages_moved.other"
        };
        let url = format!(
            "{}/chat/c/-/{}/{first_moved}",
            self.base_path, destination.id
        );
        let text = t(self.i18n, key)
            .replace("%{count}", &count.to_string())
            .replace("%{acting_username}", acting_username)
            .replace("%{channel_name}", destination_title)
            .replace("%{first_moved_message_url}", &url);
        let system = crate::session::current::SessionUser::load(&mut *self.conn, -1)
            .await?
            .ok_or(Unsupported("a site without its system user"))?;
        let guardian = crate::guardian::Guardian::for_user(&mut *self.conn, &system).await?;
        let mut cx = Context {
            conn: &mut *self.conn,
            settings: self.settings,
            i18n: self.i18n,
            guardian: &guardian,
            base_path: self.base_path,
            config: self.config,
        };
        let params = super::create::Params {
            chat_channel_id: source.id.to_string(),
            message: Some(text),
            in_reply_to_id: None,
            staged_id: None,
            thread_id: None,
            upload_ids: Vec::new(),
            blocks: false,
            client_created_at: None,
            enforce_membership: true,
        };
        match Box::pin(cx.create_message(host, bus, params)).await? {
            super::create::Outcome::Created(_) => Ok(()),
            _ => Err(Unsupported("a moved messages placeholder the system user can't post").into()),
        }
    }
}
