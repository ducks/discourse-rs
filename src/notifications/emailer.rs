//! `NotificationEmailer.process_notification`: the user_email job a new
//! notification enqueues, by its type, once the user's options and state
//! let it through. No plugin registers an email notification filter on
//! the reference, so none is consulted.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `UserOption.email_level_types[:never]`
const NEVER: i32 = 2;

/// `EMAILABLE_POST_TYPES`: regular, whisper, moderator_action.
const EMAILABLE_POST_TYPES: [i64; 3] = [1, 4, 2];

/// The notification as `process_notification` reads it: the row's
/// columns and its `data_hash` as the in-memory record has it (a
/// consolidation plan may have changed it after the row's data was
/// built).
pub struct Notification<'a> {
    pub id: i64,
    pub notification_type: i32,
    pub user_id: i32,
    pub topic_id: Option<i32>,
    pub post_number: Option<i32>,
    pub data: &'a Map<String, Value>,
}

/// How `EmailUser` enqueues for a type.
enum Enqueue {
    /// `enqueue(type)`: email_level, default delay.
    Default,
    /// `enqueue(type, private_delay)`: email_level, private delay.
    PrivateDelay,
    /// `enqueue_private(type)`: email_messages_level, private delay.
    Private,
}

/// `EmailUser#<type>`: the email type and how it is enqueued; None for
/// the types EmailUser has no method for.
fn email_for(notification_type: &str) -> Option<(&'static str, Enqueue)> {
    Some(match notification_type {
        "group_mentioned" => ("group_mentioned", Enqueue::Default),
        "mentioned" => ("user_mentioned", Enqueue::Default),
        "posted" | "watching_category_or_tag" => ("user_posted", Enqueue::Default),
        "quoted" => ("user_quoted", Enqueue::Default),
        "replied" => ("user_replied", Enqueue::Default),
        "linked" => ("user_linked", Enqueue::Default),
        "watching_first_post" => ("user_watching_first_post", Enqueue::Default),
        "post_approved" => ("post_approved", Enqueue::Default),
        "private_message" => ("user_private_message", Enqueue::Private),
        "invited_to_private_message" => ("user_invited_to_private_message", Enqueue::PrivateDelay),
        "invited_to_topic" => ("user_invited_to_topic", Enqueue::PrivateDelay),
        _ => return None,
    })
}

/// `Notification.types[type]`
fn type_name(notification_type: i32) -> Option<&'static str> {
    crate::notifications::TYPES
        .iter()
        .find(|(_, t)| *t == notification_type)
        .map(|(name, _)| *name)
}

/// `Notification#post_id` / `#post`: the live post at the notification's
/// topic and post number.
async fn post_at(
    conn: &mut PgConnection,
    n: &Notification<'_>,
) -> Result<Option<(i32, i32)>, AppError> {
    let (Some(topic_id), Some(post_number)) = (n.topic_id, n.post_number) else {
        return Ok(None);
    };
    Ok(sqlx::query_as(
        "SELECT id, post_type FROM posts WHERE topic_id = $1 AND post_number = $2 \
         AND deleted_at IS NULL LIMIT 1",
    )
    .bind(topic_id)
    .bind(post_number)
    .fetch_optional(&mut *conn)
    .await?)
}

/// Ruby's `to_i` on a JSON value: integers as they are, strings by their
/// leading digits, nil 0.
fn value_to_i(value: &Value) -> Result<i64, AppError> {
    Ok(match value {
        Value::Null => 0,
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f.trunc() as i64))
            .unwrap_or(0),
        Value::String(s) => crate::ruby::to_i(s),
        _ => {
            return Err(Unsupported("a notification original_post_id that is not a number").into());
        }
    })
}

/// `NotificationEmailer.process_notification(notification, no_delay:)`
pub async fn process_notification(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    n: &Notification<'_>,
    no_delay: bool,
) -> Result<(), AppError> {
    let Some(name) = type_name(n.notification_type) else {
        return Ok(());
    };
    let Some((email_type, how)) = email_for(name) else {
        return Ok(());
    };
    #[derive(sqlx::FromRow)]
    struct Recipient {
        active: bool,
        staged: bool,
        approved: bool,
        email_level: Option<i32>,
        email_messages_level: Option<i32>,
        has_options: bool,
    }
    let user: Option<Recipient> = sqlx::query_as(
        "SELECT u.active, u.staged, u.approved, o.email_level, o.email_messages_level, \
                o.user_id IS NOT NULL AS has_options \
         FROM users u LEFT JOIN user_options o ON o.user_id = u.id WHERE u.id = $1",
    )
    .bind(n.user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(Recipient {
        active,
        staged,
        approved,
        email_level,
        email_messages_level,
        has_options,
    }) = user
    else {
        return Err(Unsupported("a notification email for a user that is gone").into());
    };
    let delay = |private: bool| -> Result<i64, AppError> {
        Ok(if no_delay {
            0
        } else if private {
            settings.get("personal_email_time_window_seconds")?.to_i()
        } else {
            settings.get("email_time_window_mins")?.to_i() * 60
        })
    };
    let delay = match how {
        Enqueue::Default | Enqueue::PrivateDelay => {
            // `notification.user.user_option.email_level` raises on nil.
            if !has_options {
                return Err(
                    Unsupported("a notification email for a user without user options").into(),
                );
            }
            if email_level == Some(NEVER) {
                return Ok(());
            }
            delay(matches!(how, Enqueue::PrivateDelay))?
        }
        Enqueue::Private => {
            if !has_options || email_messages_level == Some(NEVER) {
                return Ok(());
            }
            delay(true)?
        }
    };

    // perform_enqueue
    if !(active || staged) {
        return Ok(());
    }
    if settings.get("must_approve_users")?.truthy() && !approved && !staged {
        return Ok(());
    }
    if staged
        && [
            "user_linked",
            "user_quoted",
            "user_mentioned",
            "group_mentioned",
        ]
        .contains(&email_type)
    {
        return Ok(());
    }
    // post_type: the data's original_post_type, else the post's.
    let mut post = None;
    let post_type = match n.data.get("original_post_type") {
        Some(Value::Null) | None => {
            post = Some(post_at(conn, n).await?);
            post.flatten().map(|(_, t)| i64::from(t))
        }
        Some(Value::Number(t)) => t.as_i64(),
        Some(_) => None,
    };
    if !post_type.is_some_and(|t| EMAILABLE_POST_TYPES.contains(&t)) {
        return Ok(());
    }

    // notification_params
    let post_id = match n.data.get("original_post_id") {
        Some(v) if !v.is_null() && v != &Value::Bool(false) => value_to_i(v)?,
        _ => {
            let post = match post {
                Some(p) => p,
                None => post_at(conn, n).await?,
            };
            post.map(|(id, _)| i64::from(id)).unwrap_or(0)
        }
    };
    let mut args = json!({
        "type": email_type,
        "user_id": n.user_id,
        "notification_id": n.id,
        "notification_data_hash": Value::Object(n.data.clone()),
        "notification_type": name,
    });
    if post_id > 0 && name != "post_approved" {
        args["post_id"] = json!(post_id);
    }
    crate::jobs::enqueue_in(&mut *conn, delay, "user_email", args).await?;
    Ok(())
}
