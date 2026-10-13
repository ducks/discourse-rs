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
/// in users as trust level 0, anonymous users left out); for a restricted
/// category, its allowed groups as chat's restricted category ids.
pub async fn audience(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    chatable_type: &str,
    chatable_id: i64,
) -> Result<Vec<String>, AppError> {
    // A direct message's: its participants (allowed_user_ids).
    if chatable_type == "DirectMessage" {
        let ids: Vec<i32> = sqlx::query_scalar(
            "SELECT user_id FROM direct_message_users WHERE direct_message_channel_id = $1 ORDER BY id",
        )
        .bind(chatable_id)
        .fetch_all(&mut *conn)
        .await?;
        return Ok(ids.into_iter().map(crate::bus::user_tag).collect());
    }
    if chatable_type != "Category" {
        return Err(Unsupported("chat channels of other chatables").into());
    }
    let restricted: bool =
        sqlx::query_scalar("SELECT COALESCE(read_restricted, FALSE) FROM categories WHERE id = $1")
            .bind(chatable_id as i32)
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or(false);
    if restricted {
        // CategoryChannel#allowed_group_ids: staff and the category's
        // groups, each as chat's restricted category audience id.
        let mut groups = vec![1, 2, 3];
        let secure: Vec<i32> = sqlx::query_scalar(
            "SELECT group_id FROM category_groups WHERE category_id = $1 ORDER BY id",
        )
        .bind(chatable_id as i32)
        .fetch_all(&mut *conn)
        .await?;
        for id in secure {
            if !groups.contains(&id) {
                groups.push(id);
            }
        }
        return Ok(groups
            .into_iter()
            .map(crate::bus::chat_restricted_category_tag)
            .collect());
    }
    let allowed = settings.group_ids("chat_allowed_groups")?;
    if settings.get("enable_public_channels")?.truthy()
        && allowed.contains(&auto_groups::ANONYMOUS_USERS)
    {
        return Err(Unsupported("anonymous access to chat channels").into());
    }
    Ok(super::message_bus_allowed_group_ids(settings)?
        .into_iter()
        .map(crate::bus::group_tag)
        .collect())
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
    let tags = audience(
        &mut *cx.conn,
        cx.settings,
        &channel.chatable_type,
        channel.chatable_id,
    )
    .await?;
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
    let tags = audience(
        &mut *cx.conn,
        cx.settings,
        &channel.chatable_type,
        channel.chatable_id,
    )
    .await?;
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
    let tags = audience(
        &mut *cx.conn,
        cx.settings,
        &channel.chatable_type,
        channel.chatable_id,
    )
    .await?;
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

/// BasicUserSerializer without a scope, as the publisher's payloads carry
/// a user.
async fn basic_user(cx: &mut Context<'_>, user_id: i32) -> Result<Value, AppError> {
    let row: Option<(String, Option<String>, Option<i32>)> =
        sqlx::query_as("SELECT username, name, uploaded_avatar_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&mut *cx.conn)
            .await?;
    let Some((username, name, avatar)) = row else {
        return Ok(Value::Null);
    };
    let logo = crate::admin_users::logo_small_url(&mut *cx.conn, cx.settings).await?;
    let urls = crate::url::Urls {
        config: cx.config,
        settings: cx.settings,
    };
    let mut out = serde_json::Map::new();
    out.insert("id".into(), json!(user_id));
    out.insert("username".into(), json!(username));
    if cx.settings.get("enable_names")?.truthy() {
        out.insert("name".into(), json!(name));
    }
    out.insert(
        "avatar_template".into(),
        json!(crate::avatar::avatar_template(
            &urls,
            user_id,
            &username,
            avatar,
            logo.as_deref()
        )?),
    );
    Ok(Value::Object(out))
}

/// `Chat::Publisher.publish_reaction!`
pub async fn publish_reaction(
    cx: &mut Context<'_>,
    bus: &pg_bus::Bus,
    channel: &ChannelRow,
    message_id: i64,
    action: &str,
    user_id: i32,
    emoji: &str,
) -> Result<(), AppError> {
    let mut user = basic_user(cx, user_id).await?;
    // Chat::BasicUserSerializer, unscoped.
    if let Value::Object(u) = &mut user {
        u.insert("can_chat".into(), Value::Null);
        u.insert("has_chat_enabled".into(), Value::Null);
    }
    let payload = json!({
        "action": action,
        "user": user,
        "emoji": emoji,
        "type": "reaction",
        "chat_message_id": message_id,
    });
    publish_to_channel(cx, bus, channel, &payload).await
}

/// `Chat::Publisher.publish_delete!` for a channel message: the latest
/// live message before it goes along for the channel lists.
pub async fn publish_delete(
    cx: &mut Context<'_>,
    bus: &pg_bus::Bus,
    channel: &ChannelRow,
    message_id: i64,
) -> Result<(), AppError> {
    let (deleted_at, deleted_by_id): (Option<chrono::NaiveDateTime>, Option<i32>) =
        sqlx::query_as("SELECT deleted_at, deleted_by_id FROM chat_messages WHERE id = $1")
            .bind(message_id)
            .fetch_one(&mut *cx.conn)
            .await?;
    let latest =
        super::modify::latest_not_deleted_message_id(&mut *cx.conn, channel.id, Some(message_id))
            .await?;
    let payload = json!({
        "type": "delete",
        "deleted_id": message_id,
        "deleted_at": deleted_at.map(super::channels::time_json),
        "deleted_by_id": deleted_by_id,
        "latest_not_deleted_message_id": latest,
    });
    publish_to_channel(cx, bus, channel, &payload).await
}

/// `Chat::Publisher.publish_unpin!`
pub async fn publish_unpin(
    cx: &mut Context<'_>,
    bus: &pg_bus::Bus,
    channel: &ChannelRow,
    message_id: i64,
    unpinned_by_id: i32,
) -> Result<(), AppError> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM chat_pinned_messages WHERE chat_channel_id = $1")
            .bind(channel.id)
            .fetch_one(&mut *cx.conn)
            .await?;
    let payload = json!({
        "type": "unpin",
        "chat_message_id": message_id,
        "unpinned_by_id": unpinned_by_id,
        "pinned_message_count": count,
    });
    publish_to_channel(cx, bus, channel, &payload).await
}

/// `Chat::Publisher.publish_thread_original_message_metadata!`: the
/// thread's preview (Chat::ThreadPreviewSerializer, its participants from
/// Chat::ThreadParticipantQuery) on the channel. The reply count is the
/// thread's live replies, what Rails' Redis count tracks.
pub async fn publish_thread_metadata(
    cx: &mut Context<'_>,
    bus: &pg_bus::Bus,
    channel: &ChannelRow,
    thread_id: i64,
) -> Result<(), AppError> {
    let (original_message_id, last_message_id): (i64, Option<i64>) = sqlx::query_as(
        "SELECT original_message_id, last_message_id FROM chat_threads WHERE id = $1",
    )
    .bind(thread_id)
    .fetch_one(&mut *cx.conn)
    .await?;
    #[derive(sqlx::FromRow)]
    struct Last {
        id: i64,
        created_at: chrono::NaiveDateTime,
        user_id: Option<i32>,
        excerpt: Option<String>,
        message: Option<String>,
        cooked: Option<String>,
    }
    let last: Option<Last> = sqlx::query_as(
        "SELECT id, created_at, user_id, excerpt, message, cooked FROM chat_messages \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(last_message_id)
    .fetch_optional(&mut *cx.conn)
    .await?;
    let Some(last) = last else {
        return Err(Unsupported("chat threads without a live last message").into());
    };
    let reply_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM chat_messages WHERE thread_id = $1 AND deleted_at IS NULL AND id <> $2",
    )
    .bind(thread_id)
    .bind(original_message_id)
    .fetch_one(&mut *cx.conn)
    .await?;
    let excerpt = match last.excerpt {
        Some(e) => e,
        None => super::messages::build_excerpt_for(
            last.message.as_deref().unwrap_or_default(),
            last.cooked.as_deref().unwrap_or_default(),
        )?,
    };
    let last_user = match last.user_id {
        Some(id) => basic_user(cx, id).await?,
        None => Value::Null,
    };
    // Chat::ThreadParticipantQuery: the members who wrote in it, the most
    // prolific first, the most recent last, ten at most.
    let stats: Vec<(i32,)> = sqlx::query_as(
        "SELECT chat_messages.user_id FROM chat_messages \
         INNER JOIN user_chat_thread_memberships m ON m.thread_id = chat_messages.thread_id \
           AND m.user_id = chat_messages.user_id \
         WHERE chat_messages.thread_id = $1 AND chat_messages.deleted_at IS NULL \
         GROUP BY chat_messages.user_id ORDER BY COUNT(*) DESC, chat_messages.user_id ASC",
    )
    .bind(thread_id)
    .fetch_all(&mut *cx.conn)
    .await?;
    let recent: Option<i32> = sqlx::query_scalar(
        "SELECT chat_messages.user_id FROM chat_messages \
         INNER JOIN user_chat_thread_memberships m ON m.thread_id = chat_messages.thread_id \
           AND m.user_id = chat_messages.user_id \
         WHERE chat_messages.thread_id = $1 AND chat_messages.deleted_at IS NULL \
         ORDER BY chat_messages.created_at DESC LIMIT 1",
    )
    .bind(thread_id)
    .fetch_optional(&mut *cx.conn)
    .await?
    .flatten();
    let mut preview = json!({
        "last_reply_created_at": last.created_at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "last_reply_excerpt": excerpt,
        "last_reply_id": last.id,
        "reply_count": reply_count,
        "last_reply_user": last_user,
    });
    if !stats.is_empty() {
        let mut users = Vec::new();
        for (id,) in &stats {
            if users.len() < 9 && Some(*id) != recent {
                users.push(basic_user(cx, *id).await?);
            }
        }
        if let Some(id) = recent {
            users.push(basic_user(cx, id).await?);
        }
        preview["participant_count"] = json!(stats.len());
        preview["participant_users"] = json!(users);
    }
    let payload = json!({
        "type": "update_thread_original_message",
        "original_message_id": original_message_id,
        "thread_id": thread_id,
        "channel_id": channel.id,
        "preview": preview,
    });
    publish_to_channel(cx, bus, channel, &payload).await
}

/// `Chat::Publisher.publish_chat_channel_metadata`: the channel's member
/// count, to its audience.
pub async fn publish_channel_metadata(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    channel_id: i64,
    chatable_type: &str,
    chatable_id: i64,
    memberships_count: i64,
) -> Result<(), AppError> {
    let tags = audience(&mut *conn, settings, chatable_type, chatable_id).await?;
    bus.publish(
        &mut *conn,
        "/chat/channel-metadata",
        &json!({ "chat_channel_id": channel_id, "memberships_count": memberships_count }),
        Some(&tags),
    )
    .await?;
    Ok(())
}

/// `Chat::Publisher.publish_kick_users`: the removed members told to leave
/// the channel (Jobs::Chat::KickUsersFromChannel).
pub async fn publish_kick_users(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    channel_id: i64,
    user_ids: &[i32],
) -> Result<(), AppError> {
    let tags: Vec<String> = user_ids
        .iter()
        .map(|id| crate::bus::user_tag(*id))
        .collect();
    bus.publish(
        &mut *conn,
        &format!("/chat/{channel_id}/kick"),
        &json!({ "channel_id": channel_id }),
        Some(&tags),
    )
    .await?;
    Ok(())
}
