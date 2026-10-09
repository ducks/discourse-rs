//! Chat::Publisher's audiences and the channel payloads the writes send:
//! a category channel's messages go to the groups that can chat
//! (`permissions`), a user's own state to that user alone.

use serde_json::{Value, json};
use sqlx::PgConnection;

use super::channels::{ChannelRow, Context};
use crate::guardian::{Guardian, auto_groups};
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `Chat::Publisher.permissions(channel)` for a category channel, as bus
/// audience tags: Chat.message_bus_allowed_group_ids (everyone and logged
/// in users as trust level 0, anonymous users left out). A restricted
/// category's own audiences are refused until ported.
pub async fn audience(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    chatable_id: i64,
) -> Result<Vec<String>, AppError> {
    let restricted: bool =
        sqlx::query_scalar("SELECT COALESCE(read_restricted, FALSE) FROM categories WHERE id = $1")
            .bind(chatable_id as i32)
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or(false);
    if restricted {
        return Err(Unsupported("chat bus audiences of restricted categories").into());
    }
    let allowed = settings.group_ids("chat_allowed_groups")?;
    if settings.get("enable_public_channels")?.truthy()
        && allowed.contains(&auto_groups::ANONYMOUS_USERS)
    {
        return Err(Unsupported("anonymous access to chat channels").into());
    }
    let mut tags: Vec<String> = Vec::new();
    for id in allowed {
        if id == auto_groups::ANONYMOUS_USERS {
            continue;
        }
        let id = if id == auto_groups::EVERYONE || id == auto_groups::LOGGED_IN_USERS {
            auto_groups::TRUST_LEVEL_0
        } else {
            id
        };
        let tag = crate::bus::group_tag(id as i32);
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    Ok(tags)
}

/// `serialize_message_with_type`: the message as the anonymous guardian
/// sees it, under `chat_message`, with its type.
async fn message_payload(
    cx: &mut Context<'_>,
    channel: &ChannelRow,
    message_id: i64,
    kind: &str,
) -> Result<(Value, Value), AppError> {
    let anonymous = Guardian::anonymous();
    let mut public = Context {
        conn: &mut *cx.conn,
        settings: cx.settings,
        i18n: cx.i18n,
        guardian: &anonymous,
        base_path: cx.base_path,
        config: cx.config,
    };
    let message = public
        .serialize_messages(channel, &[message_id])
        .await?
        .into_iter()
        .next()
        .unwrap_or(Value::Null);
    Ok((json!({ "chat_message": message, "type": kind }), message))
}

/// `Chat::Publisher.publish_new!` for a channel message (no threads):
/// the message on /chat/:id with its staged id, and on
/// /chat/:id/new-messages for the channel lists.
pub async fn publish_new(
    cx: &mut Context<'_>,
    bus: &pg_bus::Bus,
    channel: &ChannelRow,
    message_id: i64,
    staged_id: Option<&str>,
) -> Result<(), AppError> {
    let tags = audience(&mut *cx.conn, cx.settings, channel.chatable_id).await?;
    let (mut payload, message) = message_payload(cx, channel, message_id, "sent").await?;
    payload["staged_id"] = json!(staged_id);
    bus.publish(
        &mut *cx.conn,
        &format!("/chat/{}", channel.id),
        &payload,
        Some(&tags),
    )
    .await?;
    let thread_id = message["thread_id"].clone();
    bus.publish(
        &mut *cx.conn,
        &format!("/chat/{}/new-messages", channel.id),
        &json!({
            "type": "channel",
            "channel_id": channel.id,
            "thread_id": thread_id,
            "message": message,
        }),
        Some(&tags),
    )
    .await?;
    Ok(())
}

/// `Chat::Publisher.publish_processed!`, `publish_edit!` and the like: the
/// message again on /chat/:id with the given type.
pub async fn publish_message(
    cx: &mut Context<'_>,
    bus: &pg_bus::Bus,
    channel: &ChannelRow,
    message_id: i64,
    kind: &str,
) -> Result<(), AppError> {
    let tags = audience(&mut *cx.conn, cx.settings, channel.chatable_id).await?;
    let (payload, _) = message_payload(cx, channel, message_id, kind).await?;
    bus.publish(
        &mut *cx.conn,
        &format!("/chat/{}", channel.id),
        &payload,
        Some(&tags),
    )
    .await?;
    Ok(())
}

/// `Chat::Publisher.publish_to_channel!`: a payload on /chat/:id.
pub async fn publish_to_channel(
    cx: &mut Context<'_>,
    bus: &pg_bus::Bus,
    channel: &ChannelRow,
    payload: &Value,
) -> Result<(), AppError> {
    let tags = audience(&mut *cx.conn, cx.settings, channel.chatable_id).await?;
    bus.publish(
        &mut *cx.conn,
        &format!("/chat/{}", channel.id),
        payload,
        Some(&tags),
    )
    .await?;
    Ok(())
}

/// `Chat::Publisher.publish_user_tracking_state!` for a channel message.
pub async fn publish_user_tracking_state(
    cx: &mut Context<'_>,
    bus: &pg_bus::Bus,
    channel_id: i64,
    message_id: i64,
    thread_id: Option<i64>,
) -> Result<(), AppError> {
    let user_id = cx.guardian.user_id().unwrap_or(0);
    let tracking = cx.channel_tracking(&[channel_id]).await?;
    let mut data = json!({
        "channel_id": channel_id,
        "last_read_message_id": message_id,
        "thread_id": thread_id,
    });
    if let (Some(d), Some(Value::Object(t))) =
        (data.as_object_mut(), tracking.get(&channel_id.to_string()))
    {
        for (k, v) in t {
            d.insert(k.clone(), v.clone());
        }
    }
    bus.publish(
        &mut *cx.conn,
        &format!("/chat/user-tracking-state/{user_id}"),
        &data,
        Some(&[crate::bus::user_tag(user_id)]),
    )
    .await?;
    Ok(())
}
