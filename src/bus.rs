//! Live updates over pg-bus, where Discourse uses MessageBus: the channels
//! Rails publishes to and who may hear them. MessageBus's `user_ids` and
//! `group_ids` become audience tags, `user:<id>` and `group:<id>`.
//!
//! Messages are written in the transaction that made the change, so they
//! go out when it commits (Rails publishes from after_commit) and never if
//! it rolls back.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use chrono::NaiveDateTime;

use crate::posting::Ctx;
use crate::site_settings::SiteSettings;
use crate::topic_list::time_json;
use crate::url::Urls;
use crate::{AppError, Unsupported};

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

pub fn topic_channel(topic_id: i32) -> String {
    format!("/topic/{topic_id}")
}

/// Who hears a topic's messages: `None` when nobody would (MessageBus
/// skips a publish to an empty user or group list), `Some(None)` for
/// everyone, else the tags. `Topic#secure_audience_publish_messages`:
/// a message's human staff and participants (allowed users and the
/// members of allowed groups), a read-restricted category's groups.
async fn topic_audience(
    conn: &mut PgConnection,
    topic_id: i32,
) -> Result<Option<Option<Vec<String>>>, AppError> {
    let topic: Option<(String, Option<bool>, Option<i32>)> = sqlx::query_as(
        "SELECT t.archetype, c.read_restricted, c.id FROM topics t \
         LEFT JOIN categories c ON c.id = t.category_id WHERE t.id = $1",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((archetype, read_restricted, category_id)) = topic else {
        return Ok(None);
    };
    if archetype == "private_message" {
        let users: Vec<i32> = sqlx::query_scalar(
            "SELECT id FROM users WHERE id > 0 AND (admin OR moderator) \
             UNION SELECT user_id FROM topic_allowed_users WHERE topic_id = $1 \
             UNION SELECT gu.user_id FROM topic_allowed_groups tg \
               JOIN group_users gu ON gu.group_id = tg.group_id WHERE tg.topic_id = $1 \
             ORDER BY 1",
        )
        .bind(topic_id)
        .fetch_all(&mut *conn)
        .await?;
        return Ok(audience(users.into_iter().map(user_tag).collect()));
    }
    if read_restricted == Some(true) {
        let groups: Vec<i32> = sqlx::query_scalar(
            "SELECT group_id FROM category_groups WHERE category_id = $1 ORDER BY group_id",
        )
        .bind(category_id)
        .fetch_all(&mut *conn)
        .await?;
        return Ok(audience(groups.into_iter().map(group_tag).collect()));
    }
    Ok(Some(None))
}

/// `MessageBus.publish("/topic/:id", message,
/// topic.secure_audience_publish_messages)`.
pub async fn publish_to_topic(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    topic_id: i32,
    message: &Value,
) -> Result<(), AppError> {
    if let Some(tags) = topic_audience(conn, topic_id).await? {
        bus.publish(conn, &topic_channel(topic_id), message, tags.as_deref())
            .await?;
    }
    Ok(())
}

/// `MessageBus.publish(channel, message,
/// topic.secure_audience_publish_messages)` on a channel of the topic's
/// other than `/topic/:id`.
pub async fn publish_to_topic_channel(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    topic_id: i32,
    channel: &str,
    message: &Value,
) -> Result<(), AppError> {
    if let Some(tags) = topic_audience(conn, topic_id).await? {
        bus.publish(conn, channel, message, tags.as_deref()).await?;
    }
    Ok(())
}

fn audience(tags: Vec<String>) -> Option<Option<Vec<String>>> {
    if tags.is_empty() {
        None
    } else {
        Some(Some(tags))
    }
}

/// `Post#publish_change_to_clients!(type, opts)`: the change on
/// `/topic/<id>`, `opts` merged over the message, then the topic's stats
/// unless `skip_topic_stats`. Posts of a type everyone sees go to the
/// topic's audience; whispers to human staff and the author.
pub async fn publish_post_change(
    ctx: &Ctx<'_>,
    conn: &mut PgConnection,
    post_id: i32,
    kind: &str,
    opts: Map<String, Value>,
    skip_topic_stats: bool,
) -> Result<(), AppError> {
    type PostRow = (i32, i32, Option<i32>, Option<i32>, i32, i32, Option<String>);
    let post: Option<PostRow> = sqlx::query_as(
        "SELECT p.topic_id, p.post_number, p.user_id, p.last_editor_id, p.version, p.post_type, \
                u.username \
         FROM posts p JOIN topics t ON t.id = p.topic_id LEFT JOIN users u ON u.id = p.user_id \
         WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    // A post without its topic is skipped, as Rails does.
    let Some((topic_id, post_number, user_id, last_editor_id, version, post_type, username)) = post
    else {
        return Ok(());
    };
    let mut message = Map::new();
    message.insert("id".into(), json!(post_id));
    message.insert("post_number".into(), json!(post_number));
    message.insert("updated_at".into(), json!(now_json()));
    message.insert("user_id".into(), json!(user_id));
    message.insert("last_editor_id".into(), json!(last_editor_id));
    message.insert("type".into(), json!(kind));
    message.insert("version".into(), json!(version));
    if kind == "created" {
        message.insert("username".into(), json!(username));
    }
    message.extend(opts);

    // Topic.visible_post_types: regular, moderator_action, small_action.
    let tags = if [1, 2, 3].contains(&post_type) {
        topic_audience(conn, topic_id).await?
    } else {
        let users: Vec<i32> = sqlx::query_scalar(
            "SELECT id FROM users WHERE id > 0 AND (admin OR moderator OR id = $1) ORDER BY id",
        )
        .bind(user_id)
        .fetch_all(&mut *conn)
        .await?;
        audience(users.into_iter().map(user_tag).collect())
    };
    if let Some(tags) = tags {
        ctx.bus
            .publish(
                conn,
                &topic_channel(topic_id),
                &Value::Object(message),
                tags.as_deref(),
            )
            .await?;
    }
    if !skip_topic_stats {
        publish_topic_stats(ctx, conn, topic_id, kind).await?;
    }
    Ok(())
}

/// `Topic.publish_stats_to_clients!(topic_id, type)`: the like count after
/// a like, the post count and last poster after posts come and go.
pub async fn publish_topic_stats(
    ctx: &Ctx<'_>,
    conn: &mut PgConnection,
    topic_id: i32,
    kind: &str,
) -> Result<(), AppError> {
    let topic: Option<(i32, i32, Option<NaiveDateTime>, Option<i32>)> = sqlx::query_as(
        "SELECT like_count, posts_count, last_posted_at, last_post_user_id FROM topics WHERE id = $1",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((like_count, posts_count, last_posted_at, last_poster)) = topic else {
        return Ok(());
    };
    let mut message = match kind {
        "liked" | "unliked" => json!({ "like_count": like_count }),
        "created" | "destroyed" | "deleted" | "recovered" => {
            let last_poster = match last_poster {
                Some(id) => basic_user(ctx, conn, id).await?,
                None => Value::Null,
            };
            json!({
                "posts_count": posts_count,
                "last_posted_at": last_posted_at.map(time_json),
                "last_poster": last_poster,
            })
        }
        _ => return Ok(()),
    };
    let Some(tags) = topic_audience(conn, topic_id).await? else {
        return Ok(());
    };
    if let Some(m) = message.as_object_mut() {
        m.insert("id".into(), json!(topic_id));
        m.insert("updated_at".into(), json!(now_json()));
        m.insert("type".into(), json!("stats"));
    }
    ctx.bus
        .publish(conn, &topic_channel(topic_id), &message, tags.as_deref())
        .await?;
    Ok(())
}

/// `Time.now.as_json`.
fn now_json() -> String {
    time_json(crate::clock::now_naive())
}

/// `BasicUserSerializer`: the name only with enable_names.
async fn basic_user(
    ctx: &Ctx<'_>,
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<Value, AppError> {
    if ctx.settings.get("enable_user_status")?.truthy() {
        return Err(Unsupported("user status on BasicUserSerializer").into());
    }
    let user: Option<(String, Option<String>, Option<i32>)> =
        sqlx::query_as("SELECT username, name, uploaded_avatar_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((username, name, uploaded_avatar_id)) = user else {
        return Ok(Value::Null);
    };
    let urls = Urls {
        config: ctx.config,
        settings: ctx.settings,
    };
    let avatar =
        crate::avatar::avatar_template(&urls, user_id, &username, uploaded_avatar_id, None)?;
    let mut out = Map::new();
    out.insert("id".into(), json!(user_id));
    out.insert("username".into(), json!(username));
    if ctx.settings.get("enable_names")?.truthy() {
        out.insert("name".into(), json!(name));
    }
    out.insert("avatar_template".into(), json!(avatar));
    Ok(Value::Object(out))
}

/// `TopicUser.notification_level_change`: the user's own notification
/// level for a topic, on the topic's channel to them alone, with the
/// reason when one is given.
pub async fn publish_notification_level_change(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    user_id: i32,
    topic_id: i32,
    notification_level: i32,
    reason_id: Option<i32>,
) -> Result<(), AppError> {
    let mut message = json!({ "notification_level_change": notification_level });
    if let Some(reason) = reason_id {
        message["notifications_reason_id"] = json!(reason);
    }
    bus.publish(
        conn,
        &topic_channel(topic_id),
        &message,
        Some(&[user_tag(user_id)]),
    )
    .await?;
    Ok(())
}

/// The live half of `PostAlerter.create_notification_alert`: the alert
/// the browser pops up, to the user alone when they were seen in the last
/// 30 days (`allow_live_notifications?`).
pub async fn publish_notification_alert(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    user_id: i32,
    payload: &Value,
) -> Result<(), AppError> {
    let live: Option<bool> = sqlx::query_scalar(
        "SELECT COALESCE(last_seen_at >= now() - interval '30 days', FALSE) FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    if live != Some(true) {
        return Ok(());
    }
    bus.publish(
        conn,
        &format!("/notification-alert/{user_id}"),
        payload,
        Some(&[user_tag(user_id)]),
    )
    .await?;
    Ok(())
}

/// Where a page's live updates start: the bus's position taken before the
/// page reads its data, so a change committed while it renders is
/// delivered after it (at worst repeating what the page shows) rather
/// than missed. The page puts it in `<meta name="bus-position">` for
/// /bus/events and /bus/poll.
pub async fn page_position(bus: &pg_bus::Bus) -> Result<String, AppError> {
    Ok(bus.now().await?.to_string())
}

/// `User#all_unread_notifications_count`: unread notifications newer than
/// the last one the user saw, at most 99 (as in the published state).
pub async fn all_unread_notifications_count(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<i64, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM (SELECT 1 FROM notifications n LEFT JOIN topics t ON t.id = n.topic_id \
           WHERE t.deleted_at IS NULL AND n.user_id = $1 \
             AND n.id > (SELECT seen_notification_id FROM users WHERE id = $1) \
             AND NOT read LIMIT 99) x",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?)
}
