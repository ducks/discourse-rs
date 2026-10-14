//! The one way a notification row comes to be: `Notification.create!`
//! (`create`) and `Notification.consolidate_or_create!`
//! (`consolidate_or_create`, the ConsolidationPlanner's plans), each with
//! the model's callbacks: `before_create` (high priority types) and the
//! `after_commit` ones on create, `refresh_notification_count` and
//! `send_email` (shelved while the user is in do not disturb, else
//! NotificationEmailer).
//!
//! The port runs the after_commit work in the caller's transaction, right
//! after the insert, as Rails does for a record saved outside one.

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::notification_emailer;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `Notification.types` the plans and callbacks name.
pub mod types {
    pub const LIKED: i32 = 5;
    pub const PRIVATE_MESSAGE: i32 = 6;
    pub const LINKED: i32 = 11;
    pub const GROUP_MESSAGE_SUMMARY: i32 = 16;
    pub const LIKED_CONSOLIDATED: i32 = 19;
    pub const BOOKMARK_REMINDER: i32 = 24;
    /// discourse-reactions' type.
    pub const REACTION: i32 = 25;
    pub const NEW_FEATURES: i32 = 37;
}

/// `Notification.high_priority_types`
const HIGH_PRIORITY_TYPES: [i32; 2] = [types::PRIVATE_MESSAGE, types::BOOKMARK_REMINDER];

/// The attributes a notification is created with.
#[derive(Clone)]
pub struct NewNotification {
    pub notification_type: i32,
    pub user_id: i32,
    pub topic_id: Option<i32>,
    pub post_number: Option<i32>,
    /// The data, in the key order it is serialized in.
    pub data: Map<String, Value>,
    pub read: bool,
    /// Set on create; the high priority types are regardless.
    pub high_priority: bool,
    pub post_action_id: Option<i32>,
    /// created_at and updated_at when given, else the insert's time.
    pub created_at: Option<NaiveDateTime>,
    /// `skip_send_email`: neither shelved nor emailed.
    pub skip_send_email: bool,
}

impl NewNotification {
    pub fn new(notification_type: i32, user_id: i32, data: Value) -> Self {
        Self {
            notification_type,
            user_id,
            topic_id: None,
            post_number: None,
            data: match data {
                Value::Object(map) => map,
                _ => Map::new(),
            },
            read: false,
            high_priority: false,
            post_action_id: None,
            created_at: None,
            skip_send_email: false,
        }
    }

    pub fn topic(mut self, topic_id: i32, post_number: Option<i32>) -> Self {
        self.topic_id = Some(topic_id);
        self.post_number = post_number;
        self
    }

    pub fn read(mut self, read: bool) -> Self {
        self.read = read;
        self
    }

    pub fn high_priority(mut self, high_priority: bool) -> Self {
        self.high_priority = high_priority;
        self
    }
}

/// The INSERT, with `before_create`'s high priority.
async fn insert(
    conn: &mut PgConnection,
    n: &NewNotification,
    data: &Map<String, Value>,
) -> Result<i64, AppError> {
    let high_priority = n.high_priority || HIGH_PRIORITY_TYPES.contains(&n.notification_type);
    Ok(sqlx::query_scalar(
        "INSERT INTO notifications (notification_type, user_id, topic_id, post_number, data, read, \
                                    high_priority, post_action_id, created_at, updated_at) \
         SELECT $1, $2, $3, $4, $5, $6, $7, $8, at, at FROM COALESCE($9, clock_timestamp()) AS at \
         RETURNING id",
    )
    .bind(n.notification_type)
    .bind(n.user_id)
    .bind(n.topic_id)
    .bind(n.post_number)
    .bind(Value::Object(data.clone()).to_string())
    .bind(n.read)
    .bind(high_priority)
    .bind(n.post_action_id)
    .bind(n.created_at)
    .fetch_one(&mut *conn)
    .await?)
}

/// `Notification.create!(attrs)`: the row and its after_commit work; the
/// new id.
pub async fn create(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    n: &NewNotification,
) -> Result<i64, AppError> {
    let id = insert(conn, n, &n.data).await?;
    after_create(conn, bus, settings, n, id, &n.data).await?;
    Ok(id)
}

/// The after_commit callbacks on create: `refresh_notification_count`,
/// then `send_email` with the record's in-memory `data_hash`.
async fn after_create(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    n: &NewNotification,
    id: i64,
    data_hash: &Map<String, Value>,
) -> Result<(), AppError> {
    crate::bus::publish_notifications_state(bus, &mut *conn, settings, n.user_id).await?;
    if n.skip_send_email {
        return Ok(());
    }
    send_email(
        conn,
        settings,
        &notification_emailer::Notification {
            id,
            notification_type: n.notification_type,
            user_id: n.user_id,
            topic_id: n.topic_id,
            post_number: n.post_number,
            data: data_hash,
        },
    )
    .await
}

/// `Notification#send_email`: shelved while the user is in do not
/// disturb, else NotificationEmailer.
async fn send_email(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    n: &notification_emailer::Notification<'_>,
) -> Result<(), AppError> {
    if do_not_disturb(conn, n.user_id).await? {
        sqlx::query("INSERT INTO shelved_notifications (notification_id) VALUES ($1)")
            .bind(n.id)
            .execute(&mut *conn)
            .await?;
        return Ok(());
    }
    notification_emailer::process_notification(conn, settings, n, false).await
}

/// `User#do_not_disturb?`: a timing covers now.
pub async fn do_not_disturb(conn: &mut PgConnection, user_id: i32) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM do_not_disturb_timings WHERE user_id = $1 \
           AND starts_at <= now() AND ends_at > now())",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?)
}

/// A notification's data as `data_hash` reads it.
fn data_hash(data: &str) -> Map<String, Value> {
    serde_json::from_str(data).unwrap_or_default()
}

/// `Notification.consolidate_or_create!(attrs)`: the first of the
/// ConsolidationPlanner's plans that takes the notification, or a plain
/// `save!`; the id of what was created or updated. Ported plans:
/// liked_by_two_users and liked, group_message_summary and
/// group_membership (refused when it would consolidate), and
/// discourse-reactions' reacted_by_two_users and consolidated_reactions.
/// A plan that updates a consolidated notification runs only the update's
/// callback (the count); one that saves runs the create's.
pub async fn consolidate_or_create(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    n: &NewNotification,
) -> Result<i64, AppError> {
    match n.notification_type {
        types::LIKED | types::REACTION => consolidate_likes(conn, bus, settings, n).await,
        types::LINKED => Err(Unsupported("linked notification consolidation").into()),
        types::NEW_FEATURES => Err(Unsupported("new features notification consolidation").into()),
        types::GROUP_MESSAGE_SUMMARY => {
            // DeletePreviousNotifications on the data's group_id.
            if let Some(group_id) = n.data.get("group_id").filter(|v| present(v)) {
                sqlx::query(
                    "DELETE FROM notifications WHERE user_id = $1 AND notification_type = $2 \
                     AND data::json ->> 'group_id' = $3",
                )
                .bind(n.user_id)
                .bind(types::GROUP_MESSAGE_SUMMARY)
                .bind(ruby_to_s(group_id))
                .execute(&mut *conn)
                .await?;
            }
            create(conn, bus, settings, n).await
        }
        types::PRIVATE_MESSAGE => {
            // group_membership's set_data_blk names the group the message
            // requests membership of in the in-memory data; with one the
            // plan applies.
            let original_post_id = n
                .data
                .get("original_post_id")
                .and_then(Value::as_i64)
                .map(|i| i as i32);
            let requested: Option<Option<String>> = sqlx::query_scalar(
                "SELECT value FROM post_custom_fields WHERE post_id = $1 AND name = 'requested_group_id' \
                 LIMIT 1",
            )
            .bind(original_post_id)
            .fetch_optional(&mut *conn)
            .await?;
            let group_name: Option<String> = match requested.flatten() {
                Some(group_id) if !group_id.trim().is_empty() => {
                    sqlx::query_scalar("SELECT name FROM groups WHERE id = $1")
                        .bind(crate::ruby::to_i(&group_id) as i32)
                        .fetch_optional(&mut *conn)
                        .await?
                }
                _ => None,
            };
            if group_name.is_some_and(|name| !name.trim().is_empty()) {
                return Err(Unsupported("group membership request messages").into());
            }
            let mut email_data = n.data.clone();
            email_data.insert("group_name".into(), Value::Null);
            let id = insert(conn, n, &n.data).await?;
            after_create(conn, bus, settings, n, id, &email_data).await?;
            Ok(id)
        }
        _ => create(conn, bus, settings, n).await,
    }
}

/// Ruby's `present?` on a data value.
fn present(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => false,
        Value::String(s) => !s.trim().is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        _ => true,
    }
}

/// Ruby's `to_s` on a data value.
fn ruby_to_s(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The like plans (liked_by_two_users, liked) and discourse-reactions'
/// (reacted_by_two_users, consolidated_reactions).
async fn consolidate_likes(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    n: &NewNotification,
) -> Result<i64, AppError> {
    let reaction = n.notification_type == types::REACTION;
    let (topic_id, post_number) = (n.topic_id, n.post_number);
    // like_notification_frequency: always 0.
    let like_frequency: Option<i32> = sqlx::query_scalar(
        "SELECT like_notification_frequency FROM user_options WHERE user_id = $1",
    )
    .bind(n.user_id)
    .fetch_optional(&mut *conn)
    .await?;
    // DeletePreviousNotifications (liked_by_two_users,
    // reacted_by_two_users): for users notified of every like, the
    // newest one of the last day on the post is replaced by one naming
    // both.
    if like_frequency == Some(0) {
        let previous: Option<(i64, String)> = sqlx::query_as(
            "SELECT id, data FROM notifications WHERE user_id = $1 AND topic_id IS NOT DISTINCT FROM $2 \
             AND post_number IS NOT DISTINCT FROM $3 \
             AND notification_type = $4 AND created_at > now() - interval '1 day' ORDER BY id DESC LIMIT 1",
        )
        .bind(n.user_id)
        .bind(topic_id)
        .bind(post_number)
        .bind(n.notification_type)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some((previous_id, previous_data)) = previous {
            let same = data_hash(&previous_data);
            let count = same
                .get("count")
                .filter(|c| !c.is_null())
                .map(|c| match c {
                    Value::String(s) => crate::ruby::to_i(s),
                    other => other.as_i64().unwrap_or(0),
                })
                .unwrap_or(1)
                + 1;
            let mut data = n.data.clone();
            data.insert("previous_notification_id".into(), json!(previous_id));
            data.insert(
                "username2".into(),
                same.get("display_username").cloned().unwrap_or(Value::Null),
            );
            if reaction {
                data.insert(
                    "name2".into(),
                    same.get("display_name").cloned().unwrap_or(Value::Null),
                );
            }
            data.insert("count".into(), json!(count));
            sqlx::query("DELETE FROM notifications WHERE user_id = $1 AND notification_type = $2 AND id = $3")
                .bind(n.user_id)
                .bind(n.notification_type)
                .bind(previous_id)
                .execute(&mut *conn)
                .await?;
            // notification.data = data.to_json; save!
            let id = insert(conn, n, &data).await?;
            after_create(conn, bus, settings, n, id, &n.data).await?;
            return Ok(id);
        }
    }

    // ConsolidateNotifications (liked, consolidated_reactions): one
    // user's likes or reactions within the window roll into one.
    let threshold = settings.get("notification_consolidation_threshold")?.to_i();
    if threshold == 0 {
        return create(conn, bus, settings, n).await;
    }
    let window = settings
        .get("likes_notification_consolidation_window_mins")?
        .to_i() as i32;
    let to = if reaction {
        types::REACTION
    } else {
        types::LIKED_CONSOLIDATED
    };
    let display_username = n.data.get("display_username").and_then(Value::as_str);
    let mut data = n.data.clone();
    data.insert(
        "username".into(),
        n.data
            .get("display_username")
            .cloned()
            .unwrap_or(Value::Null),
    );
    if reaction {
        data.insert(
            "name".into(),
            n.data.get("display_name").cloned().unwrap_or(Value::Null),
        );
        data.insert("consolidated".into(), json!(true));
    }

    // update_consolidated_notification!
    let consolidated: Option<(i64, String)> = if reaction {
        sqlx::query_as(
            "SELECT id, data FROM notifications WHERE user_id = $1 AND notification_type = $2 \
             AND created_at > now() - make_interval(mins => $3) \
             AND (data::json ->> 'consolidated')::bool AND data::json ->> 'display_username' = $4 \
             ORDER BY id LIMIT 1",
        )
        .bind(n.user_id)
        .bind(to)
        .bind(window)
        .bind(display_username.unwrap_or(""))
        .fetch_optional(&mut *conn)
        .await?
    } else {
        sqlx::query_as(
            "SELECT id, data FROM notifications WHERE user_id = $1 AND notification_type = $2 \
             AND created_at > now() - make_interval(mins => $3) \
             AND ($4::text IS NULL OR data::json ->> 'display_username' = $4) \
             ORDER BY id LIMIT 1",
        )
        .bind(n.user_id)
        .bind(to)
        .bind(window)
        .bind(display_username)
        .fetch_optional(&mut *conn)
        .await?
    };
    if let Some((id, existing)) = consolidated {
        let existing = data_hash(&existing);
        let mut merged = existing.clone();
        for (key, value) in &data {
            merged.insert(key.clone(), value.clone());
        }
        if let Some(count) = merged.get("count").filter(|c| !c.is_null()) {
            let next = count.as_i64().unwrap_or(0) + 1;
            merged.insert("count".into(), json!(next));
        }
        if reaction && existing.get("reaction_icon") != n.data.get("reaction_icon") {
            merged.shift_remove("reaction_icon");
        }
        sqlx::query(
            "UPDATE notifications SET data = $2, read = FALSE, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(id)
        .bind(Value::Object(merged).to_string())
        .execute(&mut *conn)
        .await?;
        // after_commit on update: refresh_notification_count only.
        crate::bus::publish_notifications_state(bus, &mut *conn, settings, n.user_id).await?;
        return Ok(id);
    }

    // create_consolidated_notification!: saving this one would pass the
    // threshold, so the unconsolidated ones become one dated as the
    // newest of them.
    let unconsolidated: Vec<(i64, Option<String>, NaiveDateTime)> = sqlx::query_as(
        "SELECT id, data::json ->> 'reaction_icon', created_at FROM notifications \
         WHERE user_id = $1 AND notification_type = $2 \
         AND created_at > now() - make_interval(mins => $3) AND data::json ->> 'username2' IS NULL \
         AND ($4 = FALSE OR data::json ->> 'consolidated' IS NULL) \
         AND ($5::text IS NULL OR data::json ->> 'display_username' = $5) ORDER BY id",
    )
    .bind(n.user_id)
    .bind(n.notification_type)
    .bind(window)
    .bind(reaction)
    // consolidated_reactions compares `data[:display_username].to_s`.
    .bind(if reaction {
        Some(display_username.unwrap_or(""))
    } else {
        display_username
    })
    .fetch_all(&mut *conn)
    .await?;
    let count_after = unconsolidated.len() as i64 + 1;
    let Some((_, _, timestamp)) = unconsolidated.last().filter(|_| count_after > threshold) else {
        return create(conn, bus, settings, n).await;
    };
    let timestamp = *timestamp;
    data.insert("count".into(), json!(count_after));
    if reaction
        && let Some(icon) = data.get("reaction_icon").and_then(Value::as_str)
        && unconsolidated
            .iter()
            .any(|(_, other, _)| other.as_deref() != Some(icon))
    {
        data.shift_remove("reaction_icon");
    }
    let ids: Vec<i64> = unconsolidated.iter().map(|(id, _, _)| *id).collect();
    sqlx::query("DELETE FROM notifications WHERE id = ANY($1)")
        .bind(&ids)
        .execute(&mut *conn)
        .await?;
    // Notification.create!(notification_type: to, user_id:, data:,
    // created_at: and updated_at: the newest's).
    let consolidated = NewNotification {
        notification_type: to,
        user_id: n.user_id,
        topic_id: None,
        post_number: None,
        data,
        read: false,
        high_priority: false,
        post_action_id: None,
        created_at: Some(timestamp),
        skip_send_email: false,
    };
    create(conn, bus, settings, &consolidated).await
}
