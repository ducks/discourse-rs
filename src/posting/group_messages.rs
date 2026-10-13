//! Messages to groups: which groups a user may message (`Group.messageable`),
//! the groups a new message is allowed to (TopicCreator#add_groups), their
//! members' default levels on it, and the groups' read counts as replies
//! come in (`TopicGroup.new_message_update`).

use serde_json::json;
use sqlx::PgConnection;

use super::{TopicUserAttr, change_topic_user, notification_levels, notification_reasons};
use crate::AppError;
use crate::guardian::Guardian;

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

/// `Group#set_message_default_notification_levels!(topic)` once the group
/// fits group_pm_user_limit: each member but the system user and the
/// author gets their group level on the message (TopicNotifier, so the
/// reason is user_changed); watching first post tracks.
pub async fn apply_default_levels(
    conn: &mut PgConnection,
    group_id: i32,
    topic_id: i32,
    author: i32,
) -> Result<(), AppError> {
    let members: Vec<(i32, i32)> = sqlx::query_as(
        "SELECT user_id, notification_level FROM group_users WHERE group_id = $1 ORDER BY id",
    )
    .bind(group_id)
    .fetch_all(&mut *conn)
    .await?;
    for (user_id, level) in members {
        if user_id == -1 || user_id == author {
            continue;
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
