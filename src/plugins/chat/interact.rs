//! Smaller message actions: the reaction users popover (GET
//! /chat/:chat_channel_id/:message_id/reactions-users,
//! Chat::MessageReactionUsersQuery), stopping a streaming message (DELETE
//! .../messages/:message_id/streaming, Chat::StopMessageStreaming) and
//! the composer's group mention hints (GET /chat/api/mentions/groups,
//! HintsController#check_group_mentions).

use serde_json::{Map, Value, json};

use super::channels::{ChannelRow, Context};
use crate::{AppError, Unsupported};

/// A reacting user: id, username, name, uploaded avatar, the reaction.
type ReactionUserRow = (i32, String, Option<String>, Option<i32>, Option<String>);
/// A message with its blocks: id, author, text, blocks.
type BlockMessage = (i64, Option<i32>, Option<String>, Option<Value>);

pub enum Acted {
    Done(Value),
    NotFound,
    Forbidden,
    Invalid(Vec<String>),
}

impl Context<'_> {
    /// ChatController#message_reactions_users: the message's reactions in
    /// the order they were made, a page of them (the emoji asked for is
    /// not applied, as the query leaves it out), each user with the
    /// reaction (Chat::ReactionUserSerializer).
    pub async fn reaction_users(
        &mut self,
        channel: &ChannelRow,
        message_id: i64,
        page: i64,
        limit: i64,
    ) -> Result<Acted, AppError> {
        let message: Option<(i64, Option<i32>, bool)> = sqlx::query_as(
            "SELECT id, user_id, deleted_at IS NOT NULL FROM chat_messages WHERE id = $1 AND chat_channel_id = $2",
        )
        .bind(message_id)
        .bind(channel.id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some((id, author, trashed)) = message else {
            return Ok(Acted::NotFound);
        };
        // can_see_chat_message?
        let visible = self.can_preview(channel).await?
            && (!trashed || author == self.guardian.user_id() || self.guardian.is_staff());
        if !visible {
            return Ok(Acted::NotFound);
        }
        let rows: Vec<ReactionUserRow> = sqlx::query_as(
            "SELECT u.id, u.username, u.name, u.uploaded_avatar_id, cmr.emoji AS reaction \
             FROM chat_message_reactions cmr INNER JOIN users u ON u.id = cmr.user_id \
             WHERE cmr.chat_message_id = $1 ORDER BY cmr.created_at ASC, cmr.id ASC LIMIT $2 OFFSET $3",
        )
        .bind(id)
        .bind(limit)
        .bind(page * limit)
        .fetch_all(&mut *self.conn)
        .await?;
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM chat_message_reactions cmr WHERE cmr.chat_message_id = $1",
        )
        .bind(id)
        .fetch_one(&mut *self.conn)
        .await?;
        let names = self.settings.get("enable_names")?.truthy();
        let urls = crate::url::Urls {
            config: self.config,
            settings: self.settings,
        };
        let logo = crate::admin_users::logo_small_url(&mut *self.conn, self.settings).await?;
        let mut users = Vec::new();
        for (uid, username, name, avatar, reaction) in rows {
            let mut u = Map::new();
            u.insert("id".into(), json!(uid));
            u.insert("username".into(), json!(username));
            if names {
                u.insert("name".into(), json!(name));
            }
            u.insert(
                "avatar_template".into(),
                json!(crate::avatar::avatar_template(
                    &urls,
                    uid,
                    &username,
                    avatar,
                    logo.as_deref()
                )?),
            );
            u.insert("reaction".into(), json!(reaction));
            users.push(Value::Object(u));
        }
        Ok(Acted::Done(json!({ "users": users, "total_rows": total })))
    }

    /// Chat::StopMessageStreaming, from the channel the route names.
    pub async fn stop_streaming(
        &mut self,
        bus: &pg_bus::Bus,
        message_id: Option<i64>,
    ) -> Result<Acted, AppError> {
        let Some(message_id) = message_id else {
            return Ok(Acted::Invalid(vec!["Message can't be blank".into()]));
        };
        // model :message: any channel's live message.
        let message: Option<(i64, Option<i32>, i64, Option<i64>)> = sqlx::query_as(
            "SELECT id, user_id, chat_channel_id, in_reply_to_id FROM chat_messages \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(message_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some((id, author, channel_id, in_reply_to_id)) = message else {
            return Ok(Acted::NotFound);
        };
        let Some(user_id) = self.guardian.user_id() else {
            return Ok(Acted::Forbidden);
        };
        if user_id <= 0 {
            return Err(Unsupported("bots stopping chat streams").into());
        }
        // model :membership
        let member: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM user_chat_channel_memberships WHERE user_id = $1 AND chat_channel_id = $2)",
        )
        .bind(user_id)
        .bind(channel_id)
        .fetch_one(&mut *self.conn)
        .await?;
        if !member {
            return Ok(Acted::NotFound);
        }
        // policy :can_stop_streaming
        let replied_to: Option<Option<i32>> = match in_reply_to_id {
            Some(parent) => {
                sqlx::query_scalar(
                    "SELECT user_id FROM chat_messages WHERE id = $1 AND deleted_at IS NULL",
                )
                .bind(parent)
                .fetch_optional(&mut *self.conn)
                .await?
            }
            None => None,
        };
        let allowed = self.guardian.is_admin()
            || author == Some(user_id)
            || replied_to.flatten() == Some(user_id);
        if !allowed {
            return Ok(Acted::Forbidden);
        }
        sqlx::query(
            "UPDATE chat_messages SET streaming = FALSE, updated_at = clock_timestamp() \
             WHERE id = $1 AND streaming",
        )
        .bind(id)
        .execute(&mut *self.conn)
        .await?;
        let sql = format!(
            "SELECT {} FROM chat_channels WHERE id = $1",
            super::channels::CHANNEL_COLUMNS
        );
        let channel: ChannelRow = sqlx::query_as(&sql)
            .bind(channel_id)
            .fetch_one(&mut *self.conn)
            .await?;
        super::publisher::publish_message(self, bus, &channel, id, "edit").await?;
        Ok(Acted::Done(json!({ "success": "OK" })))
    }
}

impl Context<'_> {
    /// Chat::CreateMessageInteraction: the member pressed one of the
    /// message's block buttons; the interaction is saved and
    /// :chat_message_interaction has no listener here (discourse-workflows
    /// resumes on it, 202). Chat::MessageInteractionSerializer, under
    /// `interaction`.
    pub async fn create_interaction(
        &mut self,
        channel_id: Option<i64>,
        message_id: Option<i64>,
        action_id: Option<&str>,
    ) -> Result<Acted, AppError> {
        let mut errors = Vec::new();
        let action_id = action_id.filter(|a| !a.trim().is_empty());
        if action_id.is_none() {
            errors.push("Action can't be blank".to_string());
        }
        if message_id.is_none() {
            errors.push("Message can't be blank".to_string());
        }
        if channel_id.is_none() {
            errors.push("Channel can't be blank".to_string());
        }
        let (Some(action_id), Some(message_id), Some(channel_id)) =
            (action_id, message_id, channel_id)
        else {
            return Ok(Acted::Invalid(errors));
        };
        let message: Option<BlockMessage> = sqlx::query_as(
            "SELECT id, user_id, message, blocks FROM chat_messages \
             WHERE id = $1 AND chat_channel_id = $2 AND deleted_at IS NULL",
        )
        .bind(message_id)
        .bind(channel_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some((id, author, text, blocks)) = message else {
            return Ok(Acted::NotFound);
        };
        let sql = format!(
            "SELECT {} FROM chat_channels WHERE id = $1 AND deleted_at IS NULL",
            super::channels::CHANNEL_COLUMNS
        );
        let channel: ChannelRow = sqlx::query_as(&sql)
            .bind(channel_id)
            .fetch_optional(&mut *self.conn)
            .await?
            .ok_or(Unsupported("interactions in deleted chat channels"))?;
        // policy :can_interact_with_message
        if !self.can_join(&channel, None).await? {
            return Ok(Acted::NotFound);
        }
        // model :action: the first block element with that action_id.
        let action = blocks
            .as_ref()
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find_map(|block| {
                block["elements"]
                    .as_array()?
                    .iter()
                    .find(|e| e["action_id"].as_str() == Some(action_id))
                    .cloned()
            });
        let Some(action) = action else {
            return Ok(Acted::NotFound);
        };
        let Some(user) = self.guardian.user().cloned() else {
            return Ok(Acted::Forbidden);
        };
        sqlx::query(
            "INSERT INTO chat_message_interactions (user_id, chat_message_id, action, created_at, updated_at) \
             VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
        )
        .bind(user.id)
        .bind(id)
        .bind(&action)
        .execute(&mut *self.conn)
        .await?;
        let title = self.channel(&channel, None, None, None).await?["title"].clone();
        let mut shown = action.as_object().cloned().unwrap_or_default();
        shown.remove("value");
        Ok(Acted::Done(json!({
            "interaction": {
                "user": { "id": user.id, "username": user.username },
                "channel": { "id": channel.id, "title": title },
                "message": { "id": id, "text": text, "user_id": author },
                "action": shown,
            }
        })))
    }
}

/// HintsController#check_group_mentions for `names` as given: the visible
/// groups among them (matched by their lowercased name) the member can't
/// mention, those over max_users_notified_per_group_mention, and the
/// rest of the names.
pub async fn group_mention_hints(
    conn: &mut sqlx::PgConnection,
    settings: &crate::site_settings::SiteSettings,
    guardian: &crate::guardian::Guardian,
    names: &[String],
) -> Result<Value, AppError> {
    let Some(user) = guardian.user() else {
        return Ok(json!({}));
    };
    let visible: Vec<String> = sqlx::query_scalar(&format!(
        "SELECT groups.name FROM groups WHERE LOWER(groups.name) = ANY($1) AND {}",
        crate::groups::visible_groups_where(guardian, "groups")
    ))
    .bind(names)
    .fetch_all(&mut *conn)
    .await?;
    let mentionable: Vec<(String, i32)> = if visible.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as(&format!(
            "SELECT groups.name, groups.user_count FROM groups WHERE groups.name = ANY($1) AND ({})",
            crate::user_search::mentionable_where_with(guardian, user.id, false)
        ))
        .bind(&visible)
        .fetch_all(&mut *conn)
        .await?
    };
    let max = settings.get("max_users_notified_per_group_mention")?.to_i();
    let unreachable: Vec<&String> = visible
        .iter()
        .filter(|v| !mentionable.iter().any(|(n, _)| n == *v))
        .collect();
    let over: Vec<&String> = mentionable
        .iter()
        .filter(|(_, count)| i64::from(*count) > max)
        .map(|(n, _)| n)
        .collect();
    let invalid: Vec<&String> = names
        .iter()
        .filter(|n| !unreachable.contains(n) && !over.contains(n))
        .collect();
    Ok(json!({
        "unreachable": unreachable,
        "over_members_limit": over,
        "invalid": invalid,
    }))
}
