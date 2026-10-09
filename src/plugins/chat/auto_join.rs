//! Chat::AutoJoinChannels for one user, as chat runs it on the user's
//! first `user_seen` event (their first seen request): a following
//! membership (join mode automatic) in every channel set to auto join
//! that they may post in, then each channel's user count marked stale
//! for Jobs::Chat::UpdateChannelUserCount, and the channel published to
//! them on /chat/new-channel.
//!
//! The other triggers (a confirmed email, a group membership, a category
//! or channel change) run it too in Rails; they are ported with the
//! features that cause them.

use serde_json::json;
use sqlx::PgConnection;

use super::channels::{ChannelRow, Context, MembershipRow};
use crate::AppError;
use crate::guardian::auto_groups;
use crate::site_settings::SiteSettings;

/// The job that recounts a channel's members.
pub const UPDATE_USER_COUNT_JOB: &str = "Jobs::Chat::UpdateChannelUserCount";
/// `Chat::Publisher::NEW_CHANNEL_MESSAGE_BUS_CHANNEL`
const NEW_CHANNEL: &str = "/chat/new-channel";

/// `Chat::AutoJoinChannels.call(params: { user_id: })`
pub async fn user(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    state: &crate::AppState,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<(), AppError> {
    if !super::enabled(settings)? {
        return Ok(());
    }
    let group_ids = settings.group_ids("chat_allowed_groups")?;
    let everyone_allowed = group_ids.contains(&auto_groups::EVERYONE)
        || group_ids.contains(&auto_groups::LOGGED_IN_USERS);
    let max_users = settings.get("max_chat_auto_joined_users")?.to_i();
    let groups_sql = if everyone_allowed {
        String::new()
    } else {
        "AND EXISTS (SELECT 1 FROM group_users gu WHERE gu.user_id = u.id AND gu.group_id = ANY($4))"
            .to_string()
    };
    // CategoryGroup create_post (2) and full (1).
    let sql = format!(
        "WITH chat_users AS ( \
           SELECT u.id FROM users u \
           JOIN user_options uo ON uo.user_id = u.id AND uo.chat_enabled = TRUE \
           WHERE u.id > 0 AND u.active = TRUE AND u.staged = FALSE \
             AND (u.suspended_till IS NULL OR u.suspended_till <= $2) \
             AND (u.silenced_till IS NULL OR u.silenced_till <= $2) \
             AND NOT EXISTS (SELECT 1 FROM anonymous_users au WHERE au.user_id = u.id) \
             AND u.last_seen_at > $2 - interval '30 days' \
             AND u.id = $1 {groups_sql} \
           ORDER BY u.last_seen_at DESC LIMIT $3 \
         ), valid_chat_channels AS ( \
           SELECT cc.id, cc.chatable_id FROM chat_channels cc \
           WHERE cc.auto_join_users = TRUE AND cc.chatable_type = 'Category' \
             AND cc.deleted_at IS NULL AND cc.user_count < $3 \
         ), public AS ( \
           SELECT cu.id AS user_id, cc.id AS chat_channel_id FROM valid_chat_channels cc \
           CROSS JOIN chat_users cu \
           JOIN categories c ON c.id = cc.chatable_id AND c.read_restricted = FALSE \
         ), private AS ( \
           SELECT DISTINCT gu.user_id, cc.id AS chat_channel_id FROM valid_chat_channels cc \
           JOIN categories c ON c.id = cc.chatable_id AND c.read_restricted = TRUE \
           JOIN category_groups cg ON cg.category_id = c.id AND cg.permission_type IN (2, 1) \
           JOIN group_users gu ON gu.group_id = cg.group_id AND gu.user_id IN (SELECT id FROM chat_users) \
         ) \
         INSERT INTO user_chat_channel_memberships (user_id, chat_channel_id, following, join_mode, created_at, updated_at) \
         SELECT p.user_id, p.chat_channel_id, TRUE, 1, $2, $2 \
         FROM (SELECT * FROM public UNION ALL SELECT * FROM private) p \
         LEFT JOIN user_chat_channel_memberships uccm \
           ON uccm.user_id = p.user_id AND uccm.chat_channel_id = p.chat_channel_id \
         WHERE uccm.user_id IS NULL \
         ON CONFLICT DO NOTHING \
         RETURNING chat_channel_id"
    );
    let joined: Vec<i64> = sqlx::query_scalar(&sql)
        .bind(user_id)
        .bind(crate::clock::now_naive())
        .bind(max_users)
        .bind(&group_ids)
        .fetch_all(&mut *conn)
        .await?;
    if joined.is_empty() {
        return Ok(());
    }

    let Some(session_user) =
        crate::session::current::SessionUser::load(&mut *conn, user_id).await?
    else {
        return Ok(());
    };
    let guardian = crate::guardian::Guardian::for_user(&mut *conn, &session_user).await?;
    for channel_id in joined {
        recalculate_user_count(&mut *conn, channel_id).await?;
        // Chat::Publisher.publish_new_channel: the channel as the user sees it.
        let channel: Option<ChannelRow> = sqlx::query_as(&format!(
            "SELECT {} FROM chat_channels WHERE id = $1",
            super::channels::CHANNEL_COLUMNS
        ))
        .bind(channel_id)
        .fetch_optional(&mut *conn)
        .await?;
        let Some(channel) = channel else {
            continue;
        };
        let membership: Option<MembershipRow> = sqlx::query_as(
            "SELECT chat_channel_id, following, muted, notification_level, last_read_message_id, \
                    last_viewed_at, last_viewed_pins_at, starred \
             FROM user_chat_channel_memberships WHERE user_id = $1 AND chat_channel_id = $2",
        )
        .bind(user_id)
        .bind(channel_id)
        .fetch_optional(&mut *conn)
        .await?;
        let base_path = state.config.globals.relative_url_root();
        let serialized = Context {
            conn: &mut *conn,
            settings,
            i18n: &state.i18n,
            guardian: &guardian,
            base_path,
            config: &state.config,
        }
        .channel(&channel, membership.as_ref(), None, None)
        .await?;
        bus.publish(
            &mut *conn,
            NEW_CHANNEL,
            &json!({ "channel": serialized }),
            Some(&[crate::bus::user_tag(user_id)]),
        )
        .await?;
    }
    Ok(())
}

/// `ChannelMembershipManager#recalculate_user_count`: marked stale, and
/// recounted by the job in 3 seconds (once while stale).
async fn recalculate_user_count(conn: &mut PgConnection, channel_id: i64) -> Result<(), AppError> {
    let marked = sqlx::query(
        "UPDATE chat_channels SET user_count_stale = TRUE, updated_at = $2 WHERE id = $1 AND NOT user_count_stale",
    )
    .bind(channel_id)
    .bind(crate::clock::now_naive())
    .execute(&mut *conn)
    .await?;
    if marked.rows_affected() == 1 {
        crate::jobs::enqueue_in(
            &mut *conn,
            3,
            UPDATE_USER_COUNT_JOB,
            json!({ "chat_channel_id": channel_id }),
        )
        .await?;
    }
    Ok(())
}

/// `Jobs::Chat::UpdateChannelUserCount`: the channel's members counted
/// again (ChannelMembershipsQuery.count), and its metadata published.
pub async fn update_user_count(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    args: &serde_json::Value,
) -> Result<(), AppError> {
    let Some(channel_id) = args["chat_channel_id"].as_i64() else {
        return Ok(());
    };
    let row: Option<(bool, i64, String)> = sqlx::query_as(
        "SELECT user_count_stale, chatable_id, chatable_type FROM chat_channels \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(channel_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((true, chatable_id, chatable_type)) = row else {
        return Ok(());
    };
    if chatable_type != "Category" {
        return Err(crate::Unsupported("chat direct messages").into());
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM user_chat_channel_memberships m JOIN users ON users.id = m.user_id \
         WHERE m.chat_channel_id = $1 AND m.following AND users.id > 0 \
           AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = users.id) \
           AND users.active AND NOT users.staged \
           AND (users.suspended_till IS NULL OR users.suspended_till <= $2) \
           AND (users.silenced_till IS NULL OR users.silenced_till <= $2)",
    )
    .bind(channel_id)
    .bind(crate::clock::now_naive())
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE chat_channels SET user_count = $2, user_count_stale = FALSE, updated_at = $3 WHERE id = $1",
    )
    .bind(channel_id)
    .bind(count as i32)
    .bind(crate::clock::now_naive())
    .execute(&mut *conn)
    .await?;

    // Chat::Publisher.publish_chat_channel_metadata, to the channel's
    // audience (Chat::Publisher.permissions).
    let restricted: bool =
        sqlx::query_scalar("SELECT COALESCE(read_restricted, FALSE) FROM categories WHERE id = $1")
            .bind(chatable_id as i32)
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or(false);
    if restricted {
        return Err(crate::Unsupported("chat bus audiences of restricted categories").into());
    }
    let allowed = settings.group_ids("chat_allowed_groups")?;
    if settings.get("enable_public_channels")?.truthy()
        && allowed.contains(&auto_groups::ANONYMOUS_USERS)
    {
        return Err(crate::Unsupported("anonymous access to chat channels").into());
    }
    // Chat.message_bus_allowed_group_ids: everyone and logged in users as
    // trust level 0, anonymous users left out.
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
    bus.publish(
        &mut *conn,
        "/chat/channel-metadata",
        &json!({ "chat_channel_id": channel_id, "memberships_count": count }),
        Some(&tags),
    )
    .await?;
    Ok(())
}
