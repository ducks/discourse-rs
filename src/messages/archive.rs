//! `UserArchivedMessage` and `GroupArchivedMessage`: a message archived
//! out of a user's or a group's inbox, and moved back to it.

use serde_json::json;
use sqlx::PgConnection;

use crate::AppError;
use crate::bus::{group_tag, user_tag};

/// `UserArchivedMessage.archive!(user_id, topic)`: a fresh row, and the
/// user told.
pub async fn user_archive(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    user_id: i32,
    topic_id: i32,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM user_archived_messages WHERE user_id = $1 AND topic_id = $2")
        .bind(user_id)
        .bind(topic_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO user_archived_messages (user_id, topic_id, created_at, updated_at) \
         VALUES ($1, $2, clock_timestamp(), clock_timestamp())",
    )
    .bind(user_id)
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    bus.publish(
        conn,
        &format!("/topic/{topic_id}"),
        &json!({"type": "archived"}),
        Some(&[user_tag(user_id)]),
    )
    .await?;
    Ok(())
}

/// `UserArchivedMessage.move_to_inbox!(user_id, topic)`: back in the
/// user's inbox, unless they muted the message. True when it acted
/// (and `:move_to_inbox` fired).
pub async fn user_move_to_inbox(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    user_id: i32,
    topic_id: i32,
) -> Result<bool, AppError> {
    let muted: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topic_users WHERE user_id = $1 AND topic_id = $2 \
           AND notification_level = 0)",
    )
    .bind(user_id)
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    if muted {
        return Ok(false);
    }
    sqlx::query("DELETE FROM user_archived_messages WHERE user_id = $1 AND topic_id = $2")
        .bind(user_id)
        .bind(topic_id)
        .execute(&mut *conn)
        .await?;
    bus.publish(
        conn,
        &format!("/topic/{topic_id}"),
        &json!({"type": "move_to_inbox"}),
        Some(&[user_tag(user_id)]),
    )
    .await?;
    Ok(true)
}

/// Whether the group may archive the message: it is a message and the
/// group one of its allowed groups.
async fn group_allowed(
    conn: &mut PgConnection,
    group_id: i32,
    topic_id: i32,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topic_allowed_groups WHERE topic_id = $1 AND group_id = $2) \
         AND EXISTS (SELECT 1 FROM topics WHERE id = $1 AND archetype = 'private_message')",
    )
    .bind(topic_id)
    .bind(group_id)
    .fetch_one(conn)
    .await
}

/// The rest of both group changes: the group told on the topic and its
/// tracking channel, and its members' summaries brought up to date by a
/// job.
async fn group_changed(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    group_id: i32,
    topic_id: i32,
    kind: &str,
    acting_user_id: Option<i32>,
) -> Result<(), AppError> {
    bus.publish(
        conn,
        &format!("/topic/{topic_id}"),
        &json!({"type": kind}),
        Some(&[group_tag(group_id)]),
    )
    .await?;
    crate::pm_tracking_state::publish_group_archived(bus, conn, topic_id, group_id, acting_user_id)
        .await?;
    crate::jobs::enqueue(
        conn,
        "group_pm_update_summary",
        json!({"group_id": group_id, "topic_id": topic_id, "acting_user_id": acting_user_id}),
    )
    .await?;
    Ok(())
}

/// `GroupArchivedMessage.archive!(group_id, topic, acting_user_id:)`: true
/// when it acted (and `:archive_message` fired).
pub async fn group_archive(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    group_id: i32,
    topic_id: i32,
    acting_user_id: Option<i32>,
) -> Result<bool, AppError> {
    if !group_allowed(conn, group_id, topic_id).await? {
        return Ok(false);
    }
    sqlx::query("DELETE FROM group_archived_messages WHERE group_id = $1 AND topic_id = $2")
        .bind(group_id)
        .bind(topic_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO group_archived_messages (group_id, topic_id, created_at, updated_at) \
         VALUES ($1, $2, clock_timestamp(), clock_timestamp())",
    )
    .bind(group_id)
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    group_changed(bus, conn, group_id, topic_id, "archived", acting_user_id).await?;
    Ok(true)
}

/// `GroupArchivedMessage.move_to_inbox!(group_id, topic, acting_user_id:)`:
/// true when it acted (and `:move_to_inbox` fired).
pub async fn group_move_to_inbox(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    group_id: i32,
    topic_id: i32,
    acting_user_id: Option<i32>,
) -> Result<bool, AppError> {
    if !group_allowed(conn, group_id, topic_id).await? {
        return Ok(false);
    }
    sqlx::query("DELETE FROM group_archived_messages WHERE group_id = $1 AND topic_id = $2")
        .bind(group_id)
        .bind(topic_id)
        .execute(&mut *conn)
        .await?;
    group_changed(
        bus,
        conn,
        group_id,
        topic_id,
        "move_to_inbox",
        acting_user_id,
    )
    .await?;
    Ok(true)
}
