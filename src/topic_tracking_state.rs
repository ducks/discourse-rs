//! Port of TopicTrackingState's publishing: what keeps topic lists and
//! their new and unread counts live for regular topics (`/new`,
//! `/latest`, `/unread`, `/unread/<user id>`, `/delete`, `/recover`).
//! Messages' tracking state is PrivateMessageTopicTrackingState, not
//! ported.

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::AppError;
use crate::bus::{group_tag, user_tag};
use crate::posting::post_types;
use crate::site_settings::SiteSettings;
use crate::topic_list::time_json;

/// `Group::AUTO_GROUPS[:admins]`, `[:staff]`
const ADMINS: i32 = 1;
const STAFF: i32 = 3;

struct Topic {
    id: i32,
    archetype: String,
    category_id: Option<i32>,
    user_id: Option<i32>,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
    bumped_at: NaiveDateTime,
    read_restricted: Option<bool>,
}

impl Topic {
    fn regular(&self) -> bool {
        self.archetype == "regular"
    }
}

async fn load(conn: &mut PgConnection, topic_id: i32) -> Result<Option<Topic>, AppError> {
    type Row = (
        i32,
        String,
        Option<i32>,
        Option<i32>,
        NaiveDateTime,
        NaiveDateTime,
        NaiveDateTime,
        Option<bool>,
    );
    let row: Option<Row> = sqlx::query_as(
        "SELECT t.id, t.archetype, t.category_id, t.user_id, t.created_at, t.updated_at, \
                t.bumped_at, c.read_restricted \
         FROM topics t LEFT JOIN categories c ON c.id = t.category_id WHERE t.id = $1",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(
        |(
            id,
            archetype,
            category_id,
            user_id,
            created_at,
            updated_at,
            bumped_at,
            read_restricted,
        )| {
            Topic {
                id,
                archetype,
                category_id,
                user_id,
                created_at,
                updated_at,
                bumped_at,
                read_restricted,
            }
        },
    ))
}

/// `secure_category_group_ids` as tags: admins alone without a
/// category, admins and the category's groups when it is read
/// restricted, everyone (`None`) otherwise.
async fn audience(conn: &mut PgConnection, topic: &Topic) -> Result<Option<Vec<String>>, AppError> {
    let Some(category_id) = topic.category_id else {
        return Ok(Some(vec![group_tag(ADMINS)]));
    };
    if topic.read_restricted != Some(true) {
        return Ok(None);
    }
    let mut ids: Vec<i32> = vec![ADMINS];
    let groups: Vec<i32> = sqlx::query_scalar(
        "SELECT group_id FROM category_groups WHERE category_id = $1 ORDER BY group_id",
    )
    .bind(category_id)
    .fetch_all(&mut *conn)
    .await?;
    for g in groups {
        if !ids.contains(&g) {
            ids.push(g);
        }
    }
    Ok(Some(ids.into_iter().map(group_tag).collect()))
}

/// `payload[:tags]` with tagging on: the topic's tag ids.
async fn tags(
    settings: &SiteSettings,
    conn: &mut PgConnection,
    topic_id: i32,
    payload: &mut Map<String, Value>,
) -> Result<(), AppError> {
    if !settings.get("tagging_enabled")?.truthy() {
        return Ok(());
    }
    let ids: Vec<i32> =
        sqlx::query_scalar("SELECT tag_id FROM topic_tags WHERE topic_id = $1 ORDER BY id")
            .bind(topic_id)
            .fetch_all(&mut *conn)
            .await?;
    payload.insert(
        "tags".into(),
        Value::Array(ids.into_iter().map(|id| json!({ "id": id })).collect()),
    );
    Ok(())
}

fn message(topic_id: i32, message_type: &str, payload: Option<Map<String, Value>>) -> Value {
    let mut m = Map::new();
    m.insert("topic_id".into(), json!(topic_id));
    m.insert("message_type".into(), json!(message_type));
    if let Some(payload) = payload {
        m.insert("payload".into(), Value::Object(payload));
    }
    Value::Object(m)
}

/// `TopicTrackingState.publish_new`: a new regular topic, then the
/// author's read of its first post.
pub async fn publish_new(
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    conn: &mut PgConnection,
    topic_id: i32,
) -> Result<(), AppError> {
    let Some(topic) = load(conn, topic_id).await? else {
        return Ok(());
    };
    if !topic.regular() {
        return Ok(());
    }
    let mut payload = Map::new();
    payload.insert("last_read_post_number".into(), Value::Null);
    payload.insert("highest_post_number".into(), json!(1));
    payload.insert("created_at".into(), json!(time_json(topic.created_at)));
    payload.insert("category_id".into(), json!(topic.category_id));
    payload.insert("archetype".into(), json!(topic.archetype));
    payload.insert("created_in_new_period".into(), json!(true));
    tags(settings, conn, topic.id, &mut payload).await?;
    let tags = audience(conn, &topic).await?;
    bus.publish(
        conn,
        "/new",
        &message(topic.id, "new_topic", Some(payload)),
        tags.as_deref(),
    )
    .await?;
    if let Some(user_id) = topic.user_id {
        publish_read(bus, settings, conn, topic.id, 1, user_id, None).await?;
    }
    Ok(())
}

/// `TopicTrackingState.publish_latest`: a regular topic bumped.
pub async fn publish_latest(
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    conn: &mut PgConnection,
    topic_id: i32,
) -> Result<(), AppError> {
    let Some(topic) = load(conn, topic_id).await? else {
        return Ok(());
    };
    if !topic.regular() {
        return Ok(());
    }
    let mut payload = Map::new();
    payload.insert("bumped_at".into(), json!(time_json(topic.bumped_at)));
    payload.insert("category_id".into(), json!(topic.category_id));
    payload.insert("archetype".into(), json!(topic.archetype));
    tags(settings, conn, topic.id, &mut payload).await?;
    let tags = audience(conn, &topic).await?;
    bus.publish(
        conn,
        "/latest",
        &message(topic.id, "latest", Some(payload)),
        tags.as_deref(),
    )
    .await?;
    Ok(())
}

/// `publish_muted`: to up to 100 users seen in the last week who muted
/// the topic.
pub async fn publish_muted(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    topic_id: i32,
) -> Result<(), AppError> {
    publish_to_recent_users(
        bus,
        conn,
        topic_id,
        "muted",
        "SELECT tu.user_id FROM topic_users tu JOIN users u ON u.id = tu.user_id \
         WHERE tu.topic_id = $1 AND tu.notification_level = 0 \
           AND u.last_seen_at > now() - interval '7 days' \
         ORDER BY u.last_seen_at DESC LIMIT 100",
    )
    .await
}

/// `publish_unmuted`: to up to 100 users seen in the last week who watch
/// or track the topic, its category or one of its tags (User.watching_topic).
pub async fn publish_unmuted(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    topic_id: i32,
) -> Result<(), AppError> {
    publish_to_recent_users(
        bus,
        conn,
        topic_id,
        "unmuted",
        "SELECT u.id FROM users u \
         LEFT JOIN category_users cu ON cu.user_id = u.id \
           AND cu.category_id = (SELECT category_id FROM topics WHERE id = $1) \
         LEFT JOIN topic_users tu ON tu.user_id = u.id AND tu.topic_id = $1 \
         LEFT JOIN tag_users tgu ON tgu.user_id = u.id \
           AND tgu.tag_id IN (SELECT tag_id FROM topic_tags WHERE topic_id = $1) \
         WHERE (cu.notification_level > 0 OR tu.notification_level > 0 OR tgu.notification_level > 0) \
           AND u.last_seen_at > now() - interval '7 days' \
         ORDER BY u.last_seen_at DESC LIMIT 100",
    )
    .await
}

async fn publish_to_recent_users(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    topic_id: i32,
    message_type: &str,
    users_sql: &str,
) -> Result<(), AppError> {
    let Some(topic) = load(conn, topic_id).await? else {
        return Ok(());
    };
    if !topic.regular() {
        return Ok(());
    }
    let users: Vec<i32> = sqlx::query_scalar(users_sql)
        .bind(topic_id)
        .fetch_all(&mut *conn)
        .await?;
    if users.is_empty() {
        return Ok(());
    }
    let tags = dedup_user_tags(users);
    bus.publish(
        conn,
        "/latest",
        &message(topic_id, message_type, None),
        Some(&tags),
    )
    .await?;
    Ok(())
}

fn dedup_user_tags(mut users: Vec<i32>) -> Vec<String> {
    users.sort();
    users.dedup();
    users.into_iter().map(user_tag).collect()
}

/// `publish_unread`: a reply, to the users tracking or watching the
/// topic other than its author; for a whisper only those in staff or the
/// whisperer groups, in a restricted category only its groups' members.
pub async fn publish_unread(
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    conn: &mut PgConnection,
    post_id: i32,
) -> Result<(), AppError> {
    let post: Option<(i32, i32, Option<i32>, i32, NaiveDateTime)> = sqlx::query_as(
        "SELECT topic_id, post_number, user_id, post_type, created_at FROM posts WHERE id = $1",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((topic_id, post_number, author, post_type, created_at)) = post else {
        return Ok(());
    };
    let Some(topic) = load(conn, topic_id).await? else {
        return Ok(());
    };
    if !topic.regular() || post_type == post_types::SMALL_ACTION {
        return Ok(());
    }
    let groups: Vec<i32> = if post_type == post_types::WHISPER {
        let mut ids = vec![STAFF];
        ids.extend(
            settings
                .group_ids("whispers_allowed_groups")?
                .into_iter()
                .map(|id| id as i32),
        );
        ids
    } else if topic.read_restricted == Some(true) {
        sqlx::query_scalar("SELECT group_id FROM category_groups WHERE category_id = $1")
            .bind(topic.category_id)
            .fetch_all(&mut *conn)
            .await?
    } else {
        Vec::new()
    };
    // TopicUser.tracking(topic_id): COALESCE(level, regular) >= tracking.
    let users: Vec<i32> = sqlx::query_scalar(
        "SELECT tu.user_id FROM topic_users tu \
         WHERE tu.topic_id = $1 AND COALESCE(tu.notification_level, 1) >= 2 \
           AND tu.user_id IS DISTINCT FROM $2 \
           AND (cardinality($3::int[]) = 0 OR EXISTS (SELECT 1 FROM group_users gu \
                WHERE gu.user_id = tu.user_id AND gu.group_id = ANY($3::int[])))",
    )
    .bind(topic_id)
    .bind(author)
    .bind(&groups)
    .fetch_all(&mut *conn)
    .await?;
    if users.is_empty() {
        return Ok(());
    }
    let mut payload = Map::new();
    payload.insert("highest_post_number".into(), json!(post_number));
    payload.insert("updated_at".into(), json!(time_json(topic.updated_at)));
    payload.insert("created_at".into(), json!(time_json(created_at)));
    payload.insert("category_id".into(), json!(topic.category_id));
    payload.insert("archetype".into(), json!(topic.archetype));
    tags(settings, conn, topic_id, &mut payload).await?;
    let tags = dedup_user_tags(users);
    bus.publish(
        conn,
        "/unread",
        &message(topic_id, "unread", Some(payload)),
        Some(&tags),
    )
    .await?;
    Ok(())
}

/// `TopicTrackingState.publish_read`: the user's read position in a
/// regular topic, on their own `/unread/<id>` channel. Messages' reads
/// go through PrivateMessageTopicTrackingState instead, not ported.
pub async fn publish_read(
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    conn: &mut PgConnection,
    topic_id: i32,
    last_read_post_number: i32,
    user_id: i32,
    notification_level: Option<i32>,
) -> Result<(), AppError> {
    // user.whisperer?: staff, or in whispers_allowed_groups.
    let groups = settings.group_ids("whispers_allowed_groups")?;
    let whisperer: bool = sqlx::query_scalar(
        "SELECT (admin OR moderator) OR EXISTS (SELECT 1 FROM group_users \
           WHERE user_id = $1 AND group_id::bigint = ANY($2::bigint[])) \
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .bind(&groups)
    .fetch_optional(&mut *conn)
    .await?
    .unwrap_or(false);
    let column = if whisperer {
        "highest_staff_post_number"
    } else {
        "highest_post_number"
    };
    let highest: Option<i32> =
        sqlx::query_scalar(&format!("SELECT {column} FROM topics WHERE id = $1"))
            .bind(topic_id)
            .fetch_optional(&mut *conn)
            .await?;
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
        &format!("/unread/{user_id}"),
        &message,
        Some(&[user_tag(user_id)]),
    )
    .await?;
    Ok(())
}

/// `publish_delete`: a regular topic trashed.
pub async fn publish_delete(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    topic_id: i32,
) -> Result<(), AppError> {
    publish_plain(bus, conn, topic_id, "/delete", "delete").await
}

/// `publish_recover`: a regular topic back from the trash.
pub async fn publish_recover(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    topic_id: i32,
) -> Result<(), AppError> {
    publish_plain(bus, conn, topic_id, "/recover", "recover").await
}

async fn publish_plain(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    topic_id: i32,
    channel: &str,
    message_type: &str,
) -> Result<(), AppError> {
    let Some(topic) = load(conn, topic_id).await? else {
        return Ok(());
    };
    if !topic.regular() {
        return Ok(());
    }
    let tags = audience(conn, &topic).await?;
    bus.publish(
        conn,
        channel,
        &message(topic_id, message_type, None),
        tags.as_deref(),
    )
    .await?;
    Ok(())
}
