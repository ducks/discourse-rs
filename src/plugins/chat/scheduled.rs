//! Chat's scheduled jobs: PeriodicalUpdates (the channels' and threads'
//! counts), AutoJoinUsers (auto join and auto leave for everyone) and
//! DeleteOldMessages (messages past the retention days), and the
//! KickUsersFromChannel job auto leave queues.

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

pub const PERIODICAL_UPDATES: &str = "Jobs::Chat::PeriodicalUpdates";
pub const AUTO_JOIN_USERS: &str = "Jobs::Chat::AutoJoinUsers";
pub const DELETE_OLD_MESSAGES: &str = "Jobs::Chat::DeleteOldMessages";
pub const KICK_USERS_FROM_CHANNEL: &str = "Jobs::Chat::KickUsersFromChannel";

/// `Discourse.system_user`
const SYSTEM_USER_ID: i32 = -1;
/// `UserHistory.actions[:custom_staff]`
const CUSTOM_STAFF: i32 = 23;

async fn settings(state: &AppState, conn: &mut PgConnection) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// `Jobs::Chat::PeriodicalUpdates`: Chat::Channel.ensure_consistency!
/// and Chat::Thread.ensure_consistency!.
pub async fn periodical_updates(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    if !super::enabled(&s)? {
        return Ok(());
    }
    // update_message_counts
    sqlx::query(
        "UPDATE chat_channels channels SET messages_count = subquery.messages_count \
         FROM (SELECT COUNT(*) AS messages_count, chat_channel_id FROM chat_messages \
               WHERE chat_messages.deleted_at IS NULL GROUP BY chat_channel_id) subquery \
         WHERE channels.id = subquery.chat_channel_id AND channels.deleted_at IS NULL \
           AND subquery.messages_count != channels.messages_count",
    )
    .execute(&mut *conn)
    .await?;
    // update_user_counts, for open and closed channels; the ones changed
    // publish their metadata (find_each, by id).
    let mut updated: Vec<(i64, i64, i32)> = sqlx::query_as(
        "UPDATE chat_channels channels SET user_count = subquery.user_count, user_count_stale = false \
         FROM (SELECT COUNT(DISTINCT user_chat_channel_memberships.id) AS user_count, \
                      user_chat_channel_memberships.chat_channel_id \
               FROM user_chat_channel_memberships \
               INNER JOIN users ON users.id = user_chat_channel_memberships.user_id \
               WHERE users.active \
                 AND (users.suspended_till IS NULL OR users.suspended_till <= CURRENT_TIMESTAMP) \
                 AND NOT users.staged AND user_chat_channel_memberships.following and users.id > 0 \
               GROUP BY user_chat_channel_memberships.chat_channel_id) subquery \
         WHERE channels.id = subquery.chat_channel_id AND channels.deleted_at IS NULL \
           AND subquery.user_count != channels.user_count AND channels.status IN (0, 1) \
         RETURNING channels.id, channels.chatable_id, channels.user_count",
    )
    .fetch_all(&mut *conn)
    .await?;
    updated.sort();
    for (channel_id, chatable_id, user_count) in updated {
        let kind: String =
            sqlx::query_scalar("SELECT chatable_type FROM chat_channels WHERE id = $1")
                .bind(channel_id)
                .fetch_one(&mut *conn)
                .await?;
        if kind != "Category" {
            return Err(crate::Unsupported("chat direct messages").into());
        }
        super::publisher::publish_channel_metadata(
            &mut conn,
            &state.bus,
            &s,
            channel_id,
            chatable_id,
            i64::from(user_count),
        )
        .await?;
    }
    // Chat::Thread.update_counts: the replies, less the original message.
    // Its cache (clear_caches!) is Redis the port doesn't keep.
    sqlx::query(
        "UPDATE chat_threads ct SET replies_count = GREATEST(COALESCE(subquery.new_count, 0), 0) \
         FROM (SELECT cm.thread_id, COUNT(cm.*) - 1 AS new_count FROM chat_threads \
               LEFT JOIN chat_messages cm ON cm.thread_id = chat_threads.id AND cm.deleted_at IS NULL \
               GROUP BY cm.thread_id) AS subquery \
         WHERE ct.id = subquery.thread_id \
           AND ct.replies_count IS DISTINCT FROM GREATEST(COALESCE(subquery.new_count, 0), 0)",
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `Jobs::Chat::AutoJoinUsers`: Chat::AutoJoinChannels for everyone, then
/// Chat::AutoLeaveChannels (event hourly_job, or the args' event).
pub async fn auto_join_users(state: &AppState, args: &Value) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let s = settings(state, &mut tx).await?;
    if !super::enabled(&s)? {
        return Ok(());
    }
    super::auto_join::create_memberships(&mut tx, &state.bus, state, &s, None).await?;
    let event = args["event"].as_str().unwrap_or("hourly_job").to_string();
    auto_leave_channels(&mut tx, &s, &event).await?;
    tx.commit().await?;
    Ok(())
}

/// `Chat::AutoLeaveChannels#remove_memberships` for everyone: without a
/// chat-allowed group (unless everyone may chat), out of every channel;
/// in a restricted category's channel without a group that may post
/// there (staff aside), out of it. Then PublishAutoRemovedUser.
async fn auto_leave_channels(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    event: &str,
) -> Result<(), AppError> {
    let group_ids = settings.group_ids("chat_allowed_groups")?;
    let everyone_allowed = group_ids.contains(&crate::guardian::auto_groups::EVERYONE)
        || group_ids.contains(&crate::guardian::auto_groups::LOGGED_IN_USERS);
    let mut removed: Vec<(i64, i32)> = Vec::new();
    if !everyone_allowed {
        let rows: Vec<(i64, i32)> = sqlx::query_as(
            "DELETE FROM user_chat_channel_memberships uccm WHERE NOT EXISTS ( \
               SELECT 1 FROM group_users gu WHERE gu.user_id = uccm.user_id AND gu.group_id = ANY($1)) \
             RETURNING chat_channel_id, user_id",
        )
        .bind(&group_ids)
        .fetch_all(&mut *conn)
        .await?;
        removed.extend(rows);
    }
    // CategoryGroup create_post (2) and full (1).
    let rows: Vec<(i64, i32)> = sqlx::query_as(
        "WITH valid_permissions AS ( \
           SELECT gu.user_id, cg.category_id FROM group_users gu \
           JOIN category_groups cg ON cg.group_id = gu.group_id AND cg.permission_type IN (2, 1)) \
         DELETE FROM user_chat_channel_memberships WHERE (user_id, chat_channel_id) IN ( \
           SELECT uccm.user_id, uccm.chat_channel_id FROM user_chat_channel_memberships uccm \
           JOIN users u ON u.id = uccm.user_id AND u.id > 0 AND u.moderator = FALSE AND u.admin = FALSE \
           JOIN chat_channels cc ON cc.id = uccm.chat_channel_id AND cc.chatable_type = 'Category' \
           JOIN categories c ON c.id = cc.chatable_id AND c.read_restricted = TRUE \
           LEFT JOIN valid_permissions vp ON vp.user_id = uccm.user_id AND vp.category_id = c.id \
           WHERE vp.user_id IS NULL) \
         RETURNING chat_channel_id, user_id",
    )
    .fetch_all(&mut *conn)
    .await?;
    removed.extend(rows);
    publish_auto_removed_users(conn, event, &removed).await
}

/// `Chat::Action::PublishAutoRemovedUser`: per channel (in the order the
/// removals came), kick jobs for its removed members, a thousand a job,
/// five seconds apart (JobTimeSpacer), and a custom staff action logged
/// by the system user.
async fn publish_auto_removed_users(
    conn: &mut PgConnection,
    event: &str,
    removed: &[(i64, i32)],
) -> Result<(), AppError> {
    let mut channels: Vec<i64> = Vec::new();
    for (channel, _) in removed {
        if !channels.contains(channel) {
            channels.push(*channel);
        }
    }
    for channel_id in channels {
        let user_ids: Vec<i32> = removed
            .iter()
            .filter(|(c, _)| *c == channel_id)
            .map(|(_, u)| *u)
            .collect();
        for (i, batch) in user_ids.chunks(1000).enumerate() {
            crate::jobs::enqueue_at(
                &mut *conn,
                crate::clock::now_naive() + chrono::Duration::seconds(5 * (i as i64 + 1)),
                KICK_USERS_FROM_CHANNEL,
                json!({ "channel_id": channel_id, "user_ids": batch }),
            )
            .await?;
        }
        // StaffActionLogger#log_custom: the details as "key: value" lines,
        // admin only (UserHistory.admin_only_action_ids has custom_staff).
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, custom_type, details, admin_only, \
                                         created_at, updated_at) \
             VALUES ($1, $2, 'chat_auto_remove_membership', $3, TRUE, clock_timestamp(), clock_timestamp())",
        )
        .bind(CUSTOM_STAFF)
        .bind(SYSTEM_USER_ID)
        .bind(format!(
            "users_removed: {}\nchannel_id: {channel_id}\nevent: {event}",
            user_ids.len()
        ))
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `Jobs::Chat::KickUsersFromChannel`
pub async fn kick_users_from_channel(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(channel_id) = args["channel_id"].as_i64() else {
        return Ok(());
    };
    let user_ids: Vec<i32> = args["user_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_i64().map(|id| id as i32))
        .collect();
    let mut conn = state.pool.acquire().await?;
    // Chat::Channel.exists?: trashed channels too are out of its scope.
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM chat_channels WHERE id = $1 AND deleted_at IS NULL)",
    )
    .bind(channel_id)
    .fetch_one(&mut *conn)
    .await?;
    if !exists || user_ids.is_empty() {
        return Ok(());
    }
    super::publisher::publish_kick_users(&mut conn, &state.bus, channel_id, &user_ids).await
}

/// `Jobs::Chat::DeleteOldMessages`: the messages (trashed ones too, pinned
/// ones never) of public channels past chat_channel_retention_days, then
/// of direct message channels past chat_dm_retention_days, each when set.
pub async fn delete_old_messages(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    if !super::enabled(&s)? {
        return Ok(());
    }
    for (setting, kinds) in [
        ("chat_channel_retention_days", "'Category'"),
        ("chat_dm_retention_days", "'DirectMessage'"),
    ] {
        let days = s.get(setting)?.to_i();
        if days <= 0 {
            continue;
        }
        loop {
            // in_batches(of: 200), by id.
            let ids: Vec<i64> = sqlx::query_scalar(&format!(
                "SELECT m.id FROM chat_messages m \
                 JOIN chat_channels c ON c.id = m.chat_channel_id AND c.chatable_type IN ({kinds}) \
                 WHERE m.created_at < now() - make_interval(days => $1) \
                   AND m.id NOT IN (SELECT chat_message_id FROM chat_pinned_messages) \
                 ORDER BY m.id LIMIT 200"
            ))
            .bind(days as i32)
            .fetch_all(&mut *conn)
            .await?;
            if ids.is_empty() {
                break;
            }
            let mut tx = state.pool.begin().await?;
            destroy_batch(&mut tx, &state.bus, &s, &ids).await?;
            tx.commit().await?;
        }
    }
    Ok(())
}

/// `Chat::MessageDestroyer#destroy_in_batches` for one batch: the
/// messages destroyed with what depends on them, the channels' last
/// message and their members' last read reset past them, their flags
/// gone.
async fn destroy_batch(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    ids: &[i64],
) -> Result<(), AppError> {
    let channel_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT chat_channel_id FROM chat_messages WHERE id = ANY($1) ORDER BY 1",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    // dependent: :nullify
    sqlx::query("UPDATE chat_messages SET in_reply_to_id = NULL WHERE in_reply_to_id = ANY($1)")
        .bind(ids)
        .execute(&mut *conn)
        .await?;
    // A mention's notifications go with it (through the join model).
    let notified: Vec<i32> = sqlx::query_scalar(
        "DELETE FROM notifications WHERE id IN ( \
           SELECT cmn.notification_id FROM chat_mention_notifications cmn \
           JOIN chat_mentions cm ON cm.id = cmn.chat_mention_id WHERE cm.chat_message_id = ANY($1)) \
         RETURNING user_id",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    for statement in [
        "DELETE FROM chat_mention_notifications WHERE chat_mention_id IN \
           (SELECT id FROM chat_mentions WHERE chat_message_id = ANY($1))",
        "DELETE FROM chat_mentions WHERE chat_message_id = ANY($1)",
        "DELETE FROM chat_message_interactions WHERE chat_message_id = ANY($1)",
        "DELETE FROM chat_message_revisions WHERE chat_message_id = ANY($1)",
        "DELETE FROM chat_message_reactions WHERE chat_message_id = ANY($1)",
        "DELETE FROM bookmarks WHERE bookmarkable_type = 'ChatMessage' AND bookmarkable_id = ANY($1)",
        "DELETE FROM upload_references WHERE target_type = 'ChatMessage' AND target_id = ANY($1)",
        "DELETE FROM chat_message_hotlinked_media WHERE chat_message_id = ANY($1)",
        "DELETE FROM chat_webhook_events WHERE chat_message_id = ANY($1)",
        "DELETE FROM chat_pinned_messages WHERE chat_message_id = ANY($1)",
        "DELETE FROM chat_message_search_data WHERE chat_message_id = ANY($1)",
        "DELETE FROM chat_messages WHERE id = ANY($1)",
    ] {
        sqlx::query(statement).bind(ids).execute(&mut *conn).await?;
    }
    let mut users = notified;
    users.sort();
    users.dedup();
    for user_id in users {
        // after_commit refresh_notification_count
        crate::bus::publish_notifications_state(bus, &mut *conn, settings, user_id).await?;
    }
    // ResetChannelsLastMessageIds: update_last_message_id! for the channels
    // whose last message went.
    let stale: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM chat_channels WHERE id = ANY($1) AND last_message_id = ANY($2) ORDER BY id",
    )
    .bind(&channel_ids)
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    for channel_id in stale {
        let latest =
            super::modify::latest_not_deleted_message_id(&mut *conn, channel_id, None).await?;
        sqlx::query("UPDATE chat_channels SET last_message_id = $2, updated_at = clock_timestamp() WHERE id = $1")
            .bind(channel_id)
            .bind(latest)
            .execute(&mut *conn)
            .await?;
    }
    // ResetUserLastReadChannelMessage
    sqlx::query(
        "WITH cte AS (SELECT chat_channels.id AS chat_channel_id, last_message_id FROM chat_channels \
                      WHERE chat_channels.id = ANY($2)) \
         UPDATE user_chat_channel_memberships SET last_read_message_id = cte.last_message_id FROM cte \
         WHERE user_chat_channel_memberships.last_read_message_id = ANY($1) \
           AND cte.chat_channel_id = user_chat_channel_memberships.chat_channel_id",
    )
    .bind(ids)
    .bind(&channel_ids)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE user_chat_channel_memberships SET last_read_message_id = NULL \
         WHERE last_read_message_id = ANY($1)",
    )
    .bind(ids)
    .execute(&mut *conn)
    .await?;
    // delete_flags: Chat::ReviewableMessage.where(target_id:).destroy_all
    for statement in [
        "DELETE FROM reviewable_histories WHERE reviewable_id IN \
           (SELECT id FROM reviewables WHERE type = 'Chat::ReviewableMessage' AND target_id = ANY($1))",
        "DELETE FROM reviewable_scores WHERE reviewable_id IN \
           (SELECT id FROM reviewables WHERE type = 'Chat::ReviewableMessage' AND target_id = ANY($1))",
        "DELETE FROM reviewable_notes WHERE reviewable_id IN \
           (SELECT id FROM reviewables WHERE type = 'Chat::ReviewableMessage' AND target_id = ANY($1))",
        "DELETE FROM reviewables WHERE type = 'Chat::ReviewableMessage' AND target_id = ANY($1)",
    ] {
        sqlx::query(statement).bind(ids).execute(&mut *conn).await?;
    }
    Ok(())
}
