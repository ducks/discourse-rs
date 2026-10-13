//! Messages to groups: which groups a user may message (`Group.messageable`),
//! the groups a new message is allowed to (TopicCreator#add_groups), their
//! members' default levels on it, and the groups' read counts as replies
//! come in (`TopicGroup.new_message_update`); the groups' inbox summary
//! notifications (`PostAlerter#notify_group_summary`) and a group's
//! archived message moved back to its inbox.

use serde_json::json;
use sqlx::PgConnection;

use super::{Ctx, TopicUserAttr, change_topic_user, notification_levels, notification_reasons};
use crate::guardian::Guardian;
use crate::{AppError, Unsupported};

/// `Group.alias_levels(user)`
fn alias_levels(guardian: &Guardian) -> Vec<i32> {
    if guardian.is_admin() {
        vec![99, 1, 2, 3, 4]
    } else if guardian.is_moderator() {
        vec![99, 2, 3, 4]
    } else {
        vec![99]
    }
}

/// `Group.messageable(user)` as a condition on `groups`, its levels as $1
/// and the user as $2.
const MESSAGEABLE: &str = "(groups.messageable_level = ANY($1) OR \
     (groups.messageable_level = 3 AND groups.id IN (SELECT group_id FROM group_users WHERE user_id = $2)) OR \
     (groups.messageable_level = 4 AND groups.id IN (SELECT group_id FROM group_users WHERE user_id = $2 AND owner IS TRUE)))";

/// `Group.messageable(current_user).where("lower(name) in (?)", names)
/// .pluck("lower(name)")`: the recipients that are groups.
pub async fn messageable_names(
    conn: &mut PgConnection,
    guardian: &Guardian,
    names: &[String],
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(&format!(
        "SELECT lower(name) FROM groups WHERE {MESSAGEABLE} AND lower(name) = ANY($3) ORDER BY id"
    ))
    .bind(alias_levels(guardian))
    .bind(guardian.user_id())
    .bind(names)
    .fetch_all(conn)
    .await
}

/// A group a new message is allowed to.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AllowedGroup {
    pub id: i32,
    pub name: String,
    pub user_count: i32,
    pub has_messages: bool,
}

/// `Group.where("lower(name) in (?)", names)`, in id order.
pub async fn find(
    conn: &mut PgConnection,
    names: &[String],
) -> Result<Vec<AllowedGroup>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, name, user_count, has_messages FROM groups WHERE lower(name) = ANY($1) ORDER BY id",
    )
    .bind(names)
    .fetch_all(conn)
    .await
}

/// `Guardian#can_send_private_message?(group)`: messaging allowed (or the
/// group messageable by the user), the group messageable unless from the
/// system or a moderators notice, and a silenced user only to the staff
/// groups.
pub async fn can_send_to(
    conn: &mut PgConnection,
    guardian: &Guardian,
    can_send_private_messages: bool,
    notify_moderators: bool,
    group: &AllowedGroup,
) -> Result<bool, sqlx::Error> {
    let messageable: bool = sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM groups WHERE {MESSAGEABLE} AND groups.id = $3)"
    ))
    .bind(alias_levels(guardian))
    .bind(guardian.user_id())
    .bind(group.id)
    .fetch_one(conn)
    .await?;
    let from_system = guardian.user_id() == Some(-1);
    // Group#staff?
    let staff_group = matches!(group.name.as_str(), "admins" | "moderators" | "staff");
    Ok((can_send_private_messages || messageable)
        && (from_system || messageable || notify_moderators)
        && (!guardian.is_silenced() || staff_group))
}

/// `Group#set_message_default_notification_levels!(topic, ignore_existing:)`
/// once the group fits group_pm_user_limit: each member but the system
/// user and the author (and, ignoring existing, those with a level on it
/// already) gets their group level on the message (TopicNotifier, so the
/// reason is user_changed); watching first post tracks.
pub async fn apply_default_levels(
    conn: &mut PgConnection,
    group_id: i32,
    topic_id: i32,
    author: Option<i32>,
    ignore_existing: bool,
) -> Result<(), AppError> {
    let members: Vec<(i32, i32)> = sqlx::query_as(
        "SELECT user_id, notification_level FROM group_users WHERE group_id = $1 ORDER BY id",
    )
    .bind(group_id)
    .fetch_all(&mut *conn)
    .await?;
    for (user_id, level) in members {
        if user_id == -1 || Some(user_id) == author {
            continue;
        }
        if ignore_existing {
            let existing: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM topic_users WHERE user_id = $1 AND topic_id = $2)",
            )
            .bind(user_id)
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
            if existing {
                continue;
            }
        }
        let level = match level {
            notification_levels::TRACKING
            | notification_levels::REGULAR
            | notification_levels::MUTED
            | notification_levels::WATCHING => level,
            _ => notification_levels::TRACKING,
        };
        change_topic_user(
            conn,
            user_id,
            topic_id,
            &[TopicUserAttr::NotificationLevel(
                level,
                notification_reasons::USER_CHANGED,
            )],
        )
        .await?;
    }
    Ok(())
}

/// `TopicGroup.new_message_update(user, topic_id, post_number)`: the
/// poster's groups' read counts. The update is Rails' own, which touches
/// every topic_groups row of the message once the poster is in any of
/// its groups; then rows for the poster's groups not yet updated, and the
/// read indicator for groups that publish their read state.
pub async fn new_message_update(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    user_id: i32,
    topic_id: i32,
    post_number: i32,
) -> Result<(), AppError> {
    let now = crate::clock::now_naive();
    let updated: Vec<i32> = sqlx::query_scalar(
        "UPDATE topic_groups tg \
         SET last_read_post_number = GREATEST($3, tg.last_read_post_number), updated_at = $4 \
         FROM topic_allowed_groups tag \
         INNER JOIN group_users gu ON gu.group_id = tag.group_id \
         WHERE gu.user_id = $1 AND tag.topic_id = $2 AND tg.topic_id = $2 \
         RETURNING tg.group_id",
    )
    .bind(user_id)
    .bind(topic_id)
    .bind(post_number)
    .bind(now)
    .fetch_all(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO topic_groups (topic_id, group_id, last_read_post_number, created_at, updated_at) \
         SELECT tag.topic_id, tag.group_id, $3, $4, $4 \
           FROM topic_allowed_groups tag \
           INNER JOIN group_users gu ON gu.group_id = tag.group_id \
           WHERE gu.user_id = $1 AND tag.topic_id = $2 \
           AND (cardinality($5::int[]) = 0 OR NOT(tag.group_id = ANY($5::int[]))) \
         ON CONFLICT(topic_id, group_id) \
         DO UPDATE SET last_read_post_number = $3, created_at = $4, updated_at = $4",
    )
    .bind(user_id)
    .bind(topic_id)
    .bind(post_number)
    .bind(now)
    .bind(&updated)
    .execute(&mut *conn)
    .await?;
    publish_read_indicator_on_write(bus, conn, topic_id, user_id).await
}

/// `TopicTrackingState.publish_read_indicator_on_write`: to the members of
/// the message's groups that publish their read state, for each group the
/// writer is not in, that the message has news.
async fn publish_read_indicator_on_write(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    topic_id: i32,
    user_id: i32,
) -> Result<(), AppError> {
    let groups: Vec<Vec<i32>> = sqlx::query_scalar(
        "SELECT ARRAY_AGG(group_users.user_id) AS members FROM groups \
         INNER JOIN topic_allowed_groups ON groups.id = topic_allowed_groups.group_id \
         INNER JOIN group_users ON group_users.group_id = groups.id \
         WHERE topic_allowed_groups.topic_id = $1 AND groups.publish_read_state = TRUE \
         GROUP BY groups.id",
    )
    .bind(topic_id)
    .fetch_all(&mut *conn)
    .await?;
    let members: Vec<i32> = groups
        .into_iter()
        .filter(|members| !members.contains(&user_id))
        .flatten()
        .collect();
    if members.is_empty() {
        return Ok(());
    }
    let tags: Vec<String> = members.into_iter().map(crate::bus::user_tag).collect();
    bus.publish(
        conn,
        &format!("/private-messages/unread-indicator/{topic_id}"),
        &json!({"topic_id": topic_id, "show_indicator": true}),
        Some(&tags),
    )
    .await?;
    Ok(())
}

/// `Notification.types[:group_message_summary]`
const GROUP_MESSAGE_SUMMARY: i32 = 16;

/// One of `PostAlerter#group_stats`: an allowed group of the message and
/// how many messages are in its inbox.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GroupStat {
    group_id: i32,
    group_name: String,
    inbox_count: i64,
}

/// `PostAlerter#group_stats(topic)`: for each of the message's groups, the
/// messages to it neither deleted nor archived by it.
pub async fn group_stats(
    conn: &mut PgConnection,
    topic_id: i32,
) -> Result<Vec<GroupStat>, sqlx::Error> {
    sqlx::query_as(
        "SELECT groups.id AS group_id, groups.name AS group_name, \
                (SELECT COUNT(*) FROM topics t \
                 JOIN topic_allowed_groups g ON g.group_id = groups.id AND g.topic_id = t.id \
                 LEFT JOIN group_archived_messages a ON a.topic_id = t.id AND a.group_id = g.group_id \
                 WHERE a.id IS NULL AND t.deleted_at IS NULL AND t.archetype = 'private_message') AS inbox_count \
         FROM groups INNER JOIN topic_allowed_groups ON groups.id = topic_allowed_groups.group_id \
         WHERE topic_allowed_groups.topic_id = $1 ORDER BY topic_allowed_groups.id",
    )
    .bind(topic_id)
    .fetch_all(conn)
    .await
}

/// `PostAlerter#notify_group_summary(user, topic, acting_user_id:)`: the
/// user's summary of the inbox of the first of the message's groups they
/// are in, replacing their earlier one for that group (read when they did
/// it themselves), or removed once the inbox is empty. False when they are
/// in none of its groups (Rails' nil).
pub async fn notify_group_summary(
    ctx: &Ctx<'_>,
    conn: &mut PgConnection,
    stats: &[GroupStat],
    user_id: i32,
    topic_id: i32,
    acting_user_id: Option<i32>,
) -> Result<bool, AppError> {
    let group_id: Option<i32> = sqlx::query_scalar(
        "SELECT group_id FROM topic_allowed_groups WHERE topic_id = $1 \
         AND group_id IN (SELECT group_id FROM group_users WHERE user_id = $2) \
         ORDER BY id LIMIT 1",
    )
    .bind(topic_id)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(stat) = stats.iter().find(|s| Some(s.group_id) == group_id) else {
        return Ok(false);
    };
    if stat.inbox_count == 0 {
        // delete_all: no callbacks.
        sqlx::query(
            "DELETE FROM notifications WHERE user_id = $1 AND notification_type = $2 \
             AND data::json ->> 'group_id' = $3",
        )
        .bind(user_id)
        .bind(GROUP_MESSAGE_SUMMARY)
        .bind(stat.group_id.to_string())
        .execute(&mut *conn)
        .await?;
        return Ok(true);
    }
    let username: String = sqlx::query_scalar("SELECT username_lower FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    // Consolidation's DeletePreviousNotifications on group_id, then the save.
    sqlx::query(
        "DELETE FROM notifications WHERE user_id = $1 AND notification_type = $2 \
         AND data::json ->> 'group_id' = $3",
    )
    .bind(user_id)
    .bind(GROUP_MESSAGE_SUMMARY)
    .bind(stat.group_id.to_string())
    .execute(&mut *conn)
    .await?;
    let data = json!({
        "group_id": stat.group_id,
        "group_name": stat.group_name,
        "inbox_count": stat.inbox_count,
        "username": username,
    });
    sqlx::query(
        "INSERT INTO notifications (notification_type, user_id, data, read, high_priority, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, FALSE, clock_timestamp(), clock_timestamp())",
    )
    .bind(GROUP_MESSAGE_SUMMARY)
    .bind(user_id)
    .bind(data.to_string())
    .bind(acting_user_id == Some(user_id))
    .execute(&mut *conn)
    .await?;
    // after_commit refresh_notification_count, then send_email: no email
    // for this type, but a user in do not disturb gets it shelved.
    crate::bus::publish_notifications_state(ctx.bus, &mut *conn, ctx.settings, user_id).await?;
    let dnd: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM do_not_disturb_timings WHERE user_id = $1 \
         AND starts_at <= now() AND ends_at > now())",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if dnd {
        return Err(Unsupported("do not disturb (shelved notifications)").into());
    }
    Ok(true)
}

/// `GroupArchivedMessage.move_to_inbox!(group_id, topic, acting_user_id:)`:
/// the group's archived message back in its inbox, the group told, and its
/// members' summaries brought up to date by a job.
pub async fn move_to_inbox(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    group_id: i32,
    topic_id: i32,
    acting_user_id: i32,
) -> Result<(), AppError> {
    let allowed: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topic_allowed_groups WHERE topic_id = $1 AND group_id = $2) \
         AND EXISTS (SELECT 1 FROM topics WHERE id = $1 AND archetype = 'private_message')",
    )
    .bind(topic_id)
    .bind(group_id)
    .fetch_one(&mut *conn)
    .await?;
    if !allowed {
        return Ok(());
    }
    sqlx::query("DELETE FROM group_archived_messages WHERE group_id = $1 AND topic_id = $2")
        .bind(group_id)
        .bind(topic_id)
        .execute(&mut *conn)
        .await?;
    let tags = [crate::bus::group_tag(group_id)];
    bus.publish(
        conn,
        &format!("/topic/{topic_id}"),
        &json!({"type": "move_to_inbox"}),
        Some(&tags),
    )
    .await?;
    crate::pm_tracking_state::publish_group_archived(
        bus,
        conn,
        topic_id,
        group_id,
        Some(acting_user_id),
    )
    .await?;
    crate::jobs::enqueue(
        conn,
        "group_pm_update_summary",
        json!({"group_id": group_id, "topic_id": topic_id, "acting_user_id": acting_user_id}),
    )
    .await?;
    Ok(())
}
