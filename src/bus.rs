//! Live updates over pg-bus, where Discourse uses MessageBus: the channels
//! Rails publishes to and who may hear them. MessageBus's `user_ids` and
//! `group_ids` become audience tags, `user:<id>` and `group:<id>`.
//!
//! Messages are written in the transaction that made the change, so they
//! go out when it commits (Rails publishes from after_commit) and never if
//! it rolls back.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::AppError;
use crate::site_settings::SiteSettings;

/// The schema holding the backlog, beside `discourse_rs`.
pub const SCHEMA: &str = "discourse_rs_bus";

pub fn config() -> pg_bus::Config {
    pg_bus::Config {
        schema: SCHEMA.to_string(),
        ..pg_bus::Config::default()
    }
}

pub fn user_tag(user_id: i32) -> String {
    format!("user:{user_id}")
}

pub fn group_tag(group_id: i32) -> String {
    format!("group:{group_id}")
}

/// The tags a viewer holds: their user and every group they are in,
/// automatic groups (staff, trust levels) included. Anonymous viewers hold
/// none and hear only messages without an audience.
pub async fn tags(conn: &mut PgConnection, user_id: Option<i32>) -> Result<Vec<String>, AppError> {
    let Some(user_id) = user_id else {
        return Ok(Vec::new());
    };
    let groups: Vec<i32> =
        sqlx::query_scalar("SELECT group_id FROM group_users WHERE user_id = $1 ORDER BY group_id")
            .bind(user_id)
            .fetch_all(&mut *conn)
            .await?;
    let mut tags = vec![user_tag(user_id)];
    tags.extend(groups.into_iter().map(group_tag));
    Ok(tags)
}

pub fn notification_channel(user_id: i32) -> String {
    format!("/notification/{user_id}")
}

/// `User#publish_notifications_state`: the counts the header and the user
/// menu show, to the user alone. Skipped for users not seen in 30 days
/// (`allow_live_notifications?`).
pub async fn publish_notifications_state(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<(), AppError> {
    let user: Option<(i64, bool, bool)> = sqlx::query_as(
        "SELECT u.seen_notification_id, \
                COALESCE(u.last_seen_at >= now() - interval '30 days', FALSE), \
                COALESCE(o.skip_new_user_tips, FALSE) \
         FROM users u LEFT JOIN user_options o ON o.user_id = u.id WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((seen, live, skip_new_user_tips)) = user else {
        return Ok(());
    };
    if !live {
        return Ok(());
    }
    let last = crate::notifications::last_visible(conn, settings, user_id).await?;
    let recent: Vec<(i64, bool)> = sqlx::query_as(
        "SELECT * FROM ( \
           SELECT n.id, n.read FROM notifications n LEFT JOIN topics t ON n.topic_id = t.id \
           WHERE t.deleted_at IS NULL AND n.high_priority AND n.user_id = $1 AND NOT read \
           ORDER BY n.id DESC LIMIT 20) AS x \
         UNION ALL \
         SELECT * FROM ( \
           SELECT n.id, n.read FROM notifications n LEFT JOIN topics t ON n.topic_id = t.id \
           WHERE t.deleted_at IS NULL AND (n.high_priority = FALSE OR read) AND n.user_id = $1 \
           ORDER BY n.id DESC LIMIT 20) AS y",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    // unread_notifications and all_unread_notifications_count stop at
    // User::MAX_UNREAD_NOTIFICATIONS, grouped_unread_notifications reads at
    // most MAX_UNREAD_BACKLOG.
    let (unread, unread_high, all_unread, new_pms): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
           (SELECT COUNT(*) FROM (SELECT 1 FROM notifications n LEFT JOIN topics t ON t.id = n.topic_id \
              WHERE t.deleted_at IS NULL AND n.high_priority = FALSE AND n.user_id = $1 \
              AND n.id > $2 AND NOT read LIMIT 99) x), \
           (SELECT COUNT(*) FROM notifications n LEFT JOIN topics t ON t.id = n.topic_id \
              WHERE t.deleted_at IS NULL AND n.high_priority = TRUE AND n.user_id = $1 AND NOT read), \
           (SELECT COUNT(*) FROM (SELECT 1 FROM notifications n LEFT JOIN topics t ON t.id = n.topic_id \
              WHERE t.deleted_at IS NULL AND n.user_id = $1 AND n.id > $2 AND NOT read LIMIT 99) x), \
           (SELECT COUNT(*) FROM notifications WHERE user_id = $1 AND id > $2 AND NOT read \
              AND notification_type = 6)",
    )
    .bind(user_id)
    .bind(seen)
    .fetch_one(&mut *conn)
    .await?;
    let grouped: Vec<(i32, i64)> = sqlx::query_as(
        "SELECT x.notification_type, COUNT(*) FROM (SELECT n.notification_type FROM notifications n \
         LEFT JOIN topics t ON t.id = n.topic_id WHERE t.deleted_at IS NULL AND n.user_id = $1 \
         AND NOT n.read LIMIT 400) x GROUP BY x.notification_type ORDER BY x.notification_type",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    let grouped: Map<String, Value> = grouped
        .into_iter()
        .map(|(t, c)| (t.to_string(), json!(c)))
        .collect();
    let payload = json!({
        "unread_notifications": unread,
        "unread_high_priority_notifications": unread_high,
        "read_first_notification": seen != 0 || skip_new_user_tips,
        // as_json with the serializer's root.
        "last_notification": last.map(|n| json!({ "notification": n })),
        "recent": recent.into_iter().map(|(id, read)| json!([id, read])).collect::<Vec<_>>(),
        "seen_notification_id": seen,
        "all_unread_notifications_count": all_unread,
        "grouped_unread_notifications": grouped,
        "new_personal_messages_notifications_count": new_pms,
    });
    bus.publish(
        conn,
        &notification_channel(user_id),
        &payload,
        Some(&[user_tag(user_id)]),
    )
    .await?;
    Ok(())
}
