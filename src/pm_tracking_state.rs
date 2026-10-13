//! `PrivateMessageTopicTrackingState`'s publishing: what keeps the
//! messages' inbox counts live, on
//! `/private-message-topic-tracking-state/user/<id>` and `.../group/<id>`.
//! `tracked_topic?` is always true (no plugin narrows it).

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::AppError;
use crate::bus::{group_tag, user_tag};
use crate::posting::post_types;
use crate::site_settings::SiteSettings;

const CHANNEL_PREFIX: &str = "/private-message-topic-tracking-state";
/// `Group::AUTO_GROUPS[:staff]`
const STAFF: i32 = 3;

fn user_channel(user_id: i32) -> String {
    format!("{CHANNEL_PREFIX}/user/{user_id}")
}

fn group_channel(group_id: i32) -> String {
    format!("{CHANNEL_PREFIX}/group/{group_id}")
}

/// `topic.allowed_groups.pluck(:id)`
async fn allowed_groups(conn: &mut PgConnection, topic_id: i32) -> Result<Vec<i32>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT groups.id FROM groups \
         INNER JOIN topic_allowed_groups ON groups.id = topic_allowed_groups.group_id \
         WHERE topic_allowed_groups.topic_id = $1",
    )
    .bind(topic_id)
    .fetch_all(conn)
    .await
}

/// `publish_new(topic)`: a new message, to each allowed user but its
/// author, then each allowed group.
pub async fn publish_new(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    topic_id: i32,
) -> Result<(), AppError> {
    let topic: Option<(String, Option<i32>)> =
        sqlx::query_as("SELECT archetype, user_id FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((archetype, author)) = topic else {
        return Ok(());
    };
    if archetype != "private_message" {
        return Ok(());
    }
    let groups = allowed_groups(conn, topic_id).await?;
    let message = json!({
        "message_type": "new_topic",
        "topic_id": topic_id,
        "payload": {
            "last_read_post_number": Value::Null,
            "highest_post_number": 1,
            "group_ids": groups,
            "created_by_user_id": author,
        },
    });
    // topic.allowed_users.pluck(:id)
    let users: Vec<i32> = sqlx::query_scalar(
        "SELECT users.id FROM users \
         INNER JOIN topic_allowed_users ON users.id = topic_allowed_users.user_id \
         WHERE topic_allowed_users.topic_id = $1 ORDER BY topic_allowed_users.id",
    )
    .bind(topic_id)
    .fetch_all(&mut *conn)
    .await?;
    for user_id in users {
        if Some(user_id) == author {
            continue;
        }
        bus.publish(
            conn,
            &user_channel(user_id),
            &message,
            Some(&[user_tag(user_id)]),
        )
        .await?;
    }
    for group_id in groups {
        bus.publish(
            conn,
            &group_channel(group_id),
            &message,
            Some(&[group_tag(group_id)]),
        )
        .await?;
    }
    Ok(())
}

/// `publish_unread(post)`: a reply, to each user tracking or watching the
/// message but its author, each with their own read state. With allowed
/// groups (or staff, for a whisper) only their members, once per
/// membership as Rails' join gives them; a user who never read a message
/// older than their new-topic start date is left out.
pub async fn publish_unread(
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    conn: &mut PgConnection,
    post_id: i32,
) -> Result<(), AppError> {
    let post: Option<(i32, i32, Option<i32>, i32, String, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT p.topic_id, p.post_number, p.user_id, p.post_type, t.archetype, t.created_at \
         FROM posts p JOIN topics t ON t.id = p.topic_id WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((topic_id, post_number, author, post_type, archetype, topic_created_at)) = post else {
        return Ok(());
    };
    if archetype != "private_message" || post_type == post_types::SMALL_ACTION {
        return Ok(());
    }
    let groups = if post_type == post_types::WHISPER {
        vec![STAFF]
    } else {
        allowed_groups(conn, topic_id).await?
    };
    // TopicUser.tracking(topic_id), joined to the groups' members.
    let rows: Vec<(i32, Option<i32>, i32)> = sqlx::query_as(
        "SELECT topic_users.user_id, topic_users.last_read_post_number, topic_users.notification_level \
         FROM topic_users \
         LEFT JOIN group_users gu ON gu.user_id = topic_users.user_id AND cardinality($2::int[]) > 0 \
         WHERE topic_users.topic_id = $1 AND COALESCE(topic_users.notification_level, 1) >= 2 \
           AND (cardinality($2::int[]) = 0 OR gu.group_id = ANY($2::int[]))",
    )
    .bind(topic_id)
    .bind(&groups)
    .fetch_all(&mut *conn)
    .await?;
    for (user_id, last_read, level) in rows {
        if Some(user_id) == author {
            continue;
        }
        if last_read.is_none() {
            let start =
                crate::guardian::treat_as_new_topic_start_date_for(conn, settings, user_id).await?;
            if start.is_some_and(|start| topic_created_at < start) {
                continue;
            }
        }
        let message = json!({
            "topic_id": topic_id,
            "message_type": "unread",
            "payload": {
                "last_read_post_number": last_read,
                "highest_post_number": post_number,
                "notification_level": level,
                "group_ids": groups,
                "created_by_user_id": author,
            },
        });
        bus.publish(
            conn,
            &user_channel(user_id),
            &message,
            Some(&[user_tag(user_id)]),
        )
        .await?;
    }
    Ok(())
}

/// `publish_read(topic_id, last_read_post_number, user, level)`: the
/// user's read position in a message, on their own channel.
pub async fn publish_read(
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    conn: &mut PgConnection,
    topic_id: i32,
    last_read_post_number: Option<i32>,
    user_id: i32,
    notification_level: Option<i32>,
) -> Result<(), AppError> {
    let highest =
        crate::topic_tracking_state::highest_for_reader(settings, conn, topic_id, user_id).await?;
    let message = json!({
        "message_type": "read",
        "topic_id": topic_id,
        "payload": {
            "last_read_post_number": last_read_post_number,
            "notification_level": notification_level,
            "highest_post_number": highest,
        },
    });
    bus.publish(
        conn,
        &user_channel(user_id),
        &message,
        Some(&[user_tag(user_id)]),
    )
    .await?;
    Ok(())
}
