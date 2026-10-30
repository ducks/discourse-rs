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

/// Where a message's changes go (`calculate_publish_targets`): its
/// thread's original message to the channel and the thread, a reply to
/// the thread, anything else (or a hidden thread's) to the channel.
pub(crate) struct Targets {
    pub channels: Vec<String>,
    pub thread: Option<super::threads::ThreadRow>,
    /// `thread_reply?` in a thread that shows (allow_publish_to_thread?).
    pub thread_reply: bool,
}

pub(crate) async fn targets(
    cx: &mut Context<'_>,
    channel: &ChannelRow,
    message_id: i64,
) -> Result<Targets, AppError> {
    let root = format!("/chat/{}", channel.id);
    let thread_id: Option<i64> =
        sqlx::query_scalar("SELECT thread_id FROM chat_messages WHERE id = $1")
            .bind(message_id)
            .fetch_optional(&mut *cx.conn)
            .await?
            .flatten();
    let thread = match thread_id {
        Some(id) => cx.find_thread(id, None).await?,
        None => None,
    };
    let Some(thread) = thread.filter(|t| super::threads::shows(channel, t)) else {
        return Ok(Targets {
            channels: vec![root],
            thread: None,
            thread_reply: false,
        });
    };
    let thread_channel = format!("/chat/{}/thread/{}", channel.id, thread.id);
    let om = thread.original_message_id == message_id;
    Ok(Targets {
        channels: if om {
            vec![root, thread_channel]
        } else {
            vec![thread_channel]
        },
        thread_reply: !om,
        thread: Some(thread),
    })
}

/// `publish_to_targets!`: a payload on each target, to the channel's
/// audience.
async fn publish_to_targets(
    cx: &mut Context<'_>,
    bus: &pg_bus::Bus,
    channel: &ChannelRow,
    targets: &[String],
    payload: &Value,
) -> Result<(), AppError> {
    let tags = audience(
        &mut *cx.conn,
        cx.settings,
        &channel.chatable_type,
        channel.chatable_id,
    )
    .await?;
    for target in targets {
        bus.publish(&mut *cx.conn, target, payload, Some(&tags))
            .await?;
    }
    Ok(())
}

/// `Chat::Publisher.publish_new!`: the message on its targets with its
/// staged id, and on /chat/:id/new-messages for the channel lists (as a
/// thread's when it is a reply in a thread that shows, with the thread's
/// preview after).
pub async fn publish_new(
    cx: &mut Context<'_>,
    bus: &pg_bus::Bus,
    channel: &ChannelRow,
    message_id: i64,
    staged_id: Option<&str>,
) -> Result<(), AppError> {
    let targets = targets(cx, channel, message_id).await?;
    let (mut payload, message) = message_payload(cx, channel, message_id, "sent").await?;
    payload["staged_id"] = json!(staged_id);
    publish_to_targets(cx, bus, channel, &targets.channels, &payload).await?;
    let tags = audience(
        &mut *cx.conn,
        cx.settings,
        &channel.chatable_type,
        channel.chatable_id,
    )
    .await?;
    let thread_id = message["thread_id"].clone();
    match (&targets.thread, targets.thread_reply) {
        (Some(thread), true) => {
            bus.publish(
                &mut *cx.conn,
                &format!("/chat/{}/new-messages", channel.id),
                &json!({
                    "type": "thread",
                    "channel_id": channel.id,
                    "thread_id": thread.id,
                    "force_thread": thread.force,
                    "message": message,
                }),
                Some(&tags),
            )
            .await?;
            publish_thread_metadata(cx, bus, channel, thread.id).await?;
        }
        _ => {
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
        }
    }
    Ok(())
}

/// `Chat::Publisher.publish_processed!`, `publish_edit!` and the like: the
/// message again on its targets with the given type.
pub async fn publish_message(
    cx: &mut Context<'_>,
    bus: &pg_bus::Bus,
    channel: &ChannelRow,
    message_id: i64,
    kind: &str,
) -> Result<(), AppError> {
    let targets = targets(cx, channel, message_id).await?;
    let (payload, _) = message_payload(cx, channel, message_id, kind).await?;
    publish_to_targets(cx, bus, channel, &targets.channels, &payload).await
}

/// `Chat::Publisher.publish_thread_created!`: the original message that
/// started a thread, to the channel.
pub async fn publish_thread_created(
    cx: &mut Context<'_>,
    bus: &pg_bus::Bus,
    channel: &ChannelRow,
    message_id: i64,
    thread_id: i64,
) -> Result<(), AppError> {
    let (mut payload, _) = message_payload(cx, channel, message_id, "thread_created").await?;
    payload["thread_id"] = json!(thread_id);
    publish_to_channel(cx, bus, channel, &payload).await
}

/// `Chat::Publisher.publish_to_channel!`: a payload on /chat/:id.
pub async fn publish_to_channel(
    cx: &mut Context<'_>,
    bus: &pg_bus::Bus,
    channel: &ChannelRow,
    payload: &Value,
) -> Result<(), AppError> {
    let root = [format!("/chat/{}", channel.id)];
    publish_to_targets(cx, bus, channel, &root, payload).await
}

/// `Chat::Publisher.publish_user_tracking_state!`: the member's last read
/// message in the channel with its tracking, and, for a reply in a
/// thread of a channel with threading, the unread threads' overview and
/// the thread's tracking.
pub async fn publish_user_tracking_state(
    cx: &mut Context<'_>,
    bus: &pg_bus::Bus,
    channel: &ChannelRow,
    message_id: i64,
) -> Result<(), AppError> {
    let user_id = cx.guardian.user_id().unwrap_or(0);
    let thread_id: Option<i64> =
        sqlx::query_scalar("SELECT thread_id FROM chat_messages WHERE id = $1")
            .bind(message_id)
            .fetch_optional(&mut *cx.conn)
            .await?
            .flatten();
    let tracking = cx.channel_tracking(&[channel.id]).await?;
    let mut data = json!({
        "channel_id": channel.id,
        "last_read_message_id": message_id,
        "thread_id": thread_id,
    });
    if let (Some(d), Some(Value::Object(t))) =
        (data.as_object_mut(), tracking.get(&channel.id.to_string()))
    {
        for (k, v) in t {
            d.insert(k.clone(), v.clone());
        }
    }
    let targets = targets(cx, channel, message_id).await?;
    if channel.threading_enabled
        && targets.thread_reply
        && let Some(thread) = &targets.thread
    {
        let overview = cx.unread_thread_overview(&[channel.id]).await?;
        data["unread_thread_overview"] = overview
            .get(&channel.id.to_string())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let thread_tracking = cx.thread_tracking_for(&[thread.id], true).await?;
        // TrackingStateInfo#to_h: zeros without a row.
        let mut info = json!({
            "unread_count": 0,
            "mention_count": 0,
            "watched_threads_unread_count": 0,
            "last_reply_created_at": null,
        });
        if let Some(Value::Object(t)) = thread_tracking.get(&thread.id.to_string()) {
            for key in [
                "unread_count",
                "mention_count",
                "watched_threads_unread_count",
            ] {
                info[key] = t.get(key).cloned().unwrap_or(json!(0));
            }
        }
        data["thread_tracking"] = info;
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
pub(crate) async fn basic_user(cx: &mut Context<'_>, user_id: i32) -> Result<Value, AppError> {
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
    let targets = targets(cx, channel, message_id).await?;
    publish_to_targets(cx, bus, channel, &targets.channels, &payload).await
}

/// `Chat::Publisher.publish_delete!`: the latest live message before it
/// (in its thread, for a reply in a channel with threading) goes along
/// for the channel lists.
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
    let targets = targets(cx, channel, message_id).await?;
    let latest = match &targets.thread {
        Some(thread) if targets.thread_reply && channel.threading_enabled => {
            sqlx::query_scalar(
                "SELECT id FROM chat_messages WHERE chat_channel_id = $1 AND thread_id = $2 \
                   AND deleted_at IS NULL AND id < $3 ORDER BY created_at DESC, id DESC LIMIT 1",
            )
            .bind(channel.id)
            .bind(thread.id)
            .bind(message_id)
            .fetch_optional(&mut *cx.conn)
            .await?
        }
        _ => {
            super::modify::latest_not_deleted_message_id(
                &mut *cx.conn,
                channel.id,
                Some(message_id),
            )
            .await?
        }
    };
    let payload = json!({
        "type": "delete",
        "deleted_id": message_id,
        "deleted_at": deleted_at.map(super::channels::time_json),
        "deleted_by_id": deleted_by_id,
        "latest_not_deleted_message_id": latest,
    });
    publish_to_targets(cx, bus, channel, &targets.channels, &payload).await
}

/// `Chat::Publisher.publish_bulk_delete!` for channel messages (threads
/// are refused before it): the ids as the request gave them.
pub async fn publish_bulk_delete(
    cx: &mut Context<'_>,
    bus: &pg_bus::Bus,
    channel: &ChannelRow,
    deleted_ids: &[Value],
) -> Result<(), AppError> {
    let payload = json!({
        "type": "bulk_delete",
        "deleted_ids": deleted_ids,
        "deleted_at": super::channels::time_json(crate::clock::now_naive()),
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
/// Chat::ThreadParticipantQuery) on the channel.
pub async fn publish_thread_metadata(
    cx: &mut Context<'_>,
    bus: &pg_bus::Bus,
    channel: &ChannelRow,
    thread_id: i64,
) -> Result<(), AppError> {
    let Some(thread) = cx.find_thread(thread_id, None).await? else {
        return Ok(());
    };
    let participants = cx.thread_participants(&[thread.id]).await?;
    let preview = cx
        .thread_preview(&thread, participants.get(&thread.id))
        .await?;
    let payload = json!({
        "type": "update_thread_original_message",
        "original_message_id": thread.original_message_id,
        "thread_id": thread.id,
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
