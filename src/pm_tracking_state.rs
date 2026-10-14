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

/// `publish_group_archived(topic:, group_id:, acting_user_id:)`: a message
/// archived by a group or moved back to its inbox, on the group's channel.
pub async fn publish_group_archived(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    topic_id: i32,
    group_id: i32,
    acting_user_id: Option<i32>,
) -> Result<(), AppError> {
    let message = json!({
        "message_type": "group_archive",
        "topic_id": topic_id,
        "payload": {
            "group_ids": [group_id],
            "acting_user_id": acting_user_id,
        },
    });
    bus.publish(
        conn,
        &group_channel(group_id),
        &message,
        Some(&[group_tag(group_id)]),
    )
    .await?;
    Ok(())
}

/// One row of `PrivateMessageTopicTrackingState.report(user)`
/// (PrivateMessageTopicTrackingStateSerializer).
#[derive(Debug, Clone)]
pub struct State {
    pub topic_id: i32,
    pub last_read_post_number: Option<i32>,
    pub notification_level: Option<i32>,
    pub highest_post_number: i32,
    pub group_ids: Vec<i32>,
}

impl State {
    /// The client's `_isNew` (`is_seen` is never in the report).
    fn is_new(&self) -> bool {
        self.last_read_post_number.is_none_or(|n| n == 0)
            && self.notification_level.is_none_or(|l| l >= 2)
    }

    /// The client's `_isUnread`.
    fn is_unread(&self) -> bool {
        self.last_read_post_number
            .is_some_and(|n| n != 0 && n < self.highest_post_number)
            && self.notification_level.is_some_and(|l| l >= 2)
    }
}

/// `TopicTrackingState::MAX_TOPICS`
const MAX_TOPICS: i64 = 5000;

/// `PrivateMessageTopicTrackingState.report(user)`: the user's new and
/// unread messages, as the client's pmTopicTrackingState loads them.
pub async fn report(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
) -> Result<Vec<State>, AppError> {
    let Some(user_id) = guardian.user_id() else {
        return Ok(Vec::new());
    };
    let whisperer = guardian.is_whisperer(settings)?;
    let first_unread_pm_at: Option<chrono::NaiveDateTime> = sqlx::query_scalar(
        "SELECT LEAST(MIN(user_stats.first_unread_pm_at), MIN(group_users.first_unread_pm_at)) \
         FROM group_users JOIN groups ON groups.id = group_users.group_id \
         JOIN user_stats ON user_stats.user_id = $1 WHERE group_users.user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    // Rails interpolates the time as Time#to_s, whole seconds.
    let first_unread_pm_at = first_unread_pm_at.map(|t| {
        use chrono::Timelike;
        t.with_nanosecond(0).unwrap_or(t)
    });
    let highest = if whisperer {
        "highest_staff_post_number"
    } else {
        "highest_post_number"
    };
    let unread_age = if first_unread_pm_at.is_some() {
        " AND topics.updated_at > $6"
    } else {
        " AND $6::timestamp IS NULL"
    };
    let raw = |new: bool| {
        let unread = if new {
            "1=0".to_string()
        } else {
            format!(
                "tu.last_read_post_number < topics.{highest} \
                 AND COALESCE(tu.notification_level, 1) >= 2{unread_age}"
            )
        };
        let new_filter = if new {
            format!(
                "topics.created_at >= {} AND tu.last_read_post_number IS NULL \
                 AND COALESCE(tu.notification_level, 2) >= 2 \
                 AND topics.created_at > $4 AND dismissed_topic_users.id IS NULL",
                crate::topic_tracking_report::treat_as_new()
            )
        } else {
            "1=0".to_string()
        };
        let dismissed = if new {
            "LEFT JOIN dismissed_topic_users ON dismissed_topic_users.topic_id = topics.id \
             AND dismissed_topic_users.user_id = $1"
        } else {
            ""
        };
        format!(
            "SELECT DISTINCT topics.id AS topic_id, u.id AS user_id, last_read_post_number, \
             tu.notification_level, topics.{highest} AS highest_post_number, \
             ARRAY(SELECT group_id FROM topic_allowed_groups WHERE topic_allowed_groups.topic_id = topics.id) AS group_ids \
             FROM topics \
             JOIN users u on u.id = $1 \
             JOIN user_stats AS us ON us.user_id = u.id \
             JOIN user_options AS uo ON uo.user_id = u.id \
             LEFT JOIN group_users gu ON gu.user_id = u.id \
             LEFT JOIN topic_allowed_groups tag ON tag.topic_id = topics.id AND tag.group_id = gu.group_id \
             LEFT JOIN topic_users tu ON tu.topic_id = topics.id AND tu.user_id = u.id \
             LEFT JOIN topic_allowed_users tau ON tau.topic_id = topics.id AND tau.user_id = u.id \
             {dismissed} \
             WHERE (tau.topic_id IS NOT NULL OR tag.topic_id IS NOT NULL) AND \
             topics.archetype = 'private_message' AND (({unread}) OR ({new_filter})) AND \
             topics.deleted_at IS NULL{}",
            if guardian.is_staff() {
                ""
            } else {
                " AND topics.visible"
            }
        )
    };
    let sql = format!("{}\nUNION ALL\n\n{}\n\n LIMIT $5", raw(true), raw(false));
    let min_date = chrono::DateTime::from_timestamp(settings.get("min_new_topics_time")?.to_i(), 0)
        .map(|t| t.naive_utc())
        .unwrap_or_default();
    type Row = (i32, i32, Option<i32>, Option<i32>, i32, Vec<i32>);
    let rows: Vec<Row> = sqlx::query_as(&sql)
        .bind(user_id)
        .bind(crate::clock::now_naive())
        .bind(
            settings
                .get("default_other_new_topic_duration_minutes")?
                .to_i(),
        )
        .bind(min_date)
        .bind(MAX_TOPICS)
        .bind(first_unread_pm_at)
        .fetch_all(&mut *conn)
        .await?;
    Ok(rows
        .into_iter()
        .map(
            |(
                topic_id,
                _,
                last_read_post_number,
                notification_level,
                highest_post_number,
                group_ids,
            )| {
                State {
                    topic_id,
                    last_read_post_number,
                    notification_level,
                    highest_post_number,
                    group_ids,
                }
            },
        )
        .collect())
}

/// Which messages a count looks at (`lookupCount`'s inboxFilter).
pub enum Inbox<'a> {
    /// `user`: messages to none of the viewer's visible groups
    /// (`_isPersonal`).
    User,
    /// `group`: messages to that group, one of the viewer's visible groups
    /// (`_isGroup`).
    Group(&'a str),
}

/// New or unread.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    New,
    Unread,
}

/// The viewer's visible groups (`currentUser.visibleGroups`): id, name
/// and has_messages.
pub async fn viewer_groups(
    conn: &mut PgConnection,
    guardian: &crate::guardian::Guardian,
) -> Result<Vec<(i32, String, bool)>, AppError> {
    let Some(user) = guardian.user() else {
        return Ok(Vec::new());
    };
    Ok(crate::current_user::visible_groups(conn, user.id, user.admin, user.moderator).await?)
}

/// MyMessagesSectionLink#totalCount: new and unread messages in the
/// viewer's inbox and in each of their groups with messages.
pub async fn inbox_count(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
) -> Result<i64, AppError> {
    let states = report(&mut *conn, settings, guardian).await?;
    if states.is_empty() {
        return Ok(0);
    }
    let groups = viewer_groups(conn, guardian).await?;
    let visible: Vec<(i32, String)> = groups
        .iter()
        .map(|(id, name, _)| (*id, name.clone()))
        .collect();
    let mut count = 0;
    for kind in [Kind::New, Kind::Unread] {
        count += lookup_count(&states, &visible, kind, Inbox::User);
        for (_, name, has_messages) in &groups {
            if *has_messages {
                count += lookup_count(&states, &visible, kind, Inbox::Group(name));
            }
        }
    }
    Ok(count)
}

/// pmTopicTrackingState#lookupCount over the report, the viewer's visible
/// groups (`currentUser.visibleGroups`, id and name) given.
pub fn lookup_count(states: &[State], groups: &[(i32, String)], kind: Kind, inbox: Inbox) -> i64 {
    states
        .iter()
        .filter(|s| match kind {
            Kind::New => s.is_new(),
            Kind::Unread => s.is_unread(),
        })
        .filter(|s| match &inbox {
            Inbox::User => !groups.iter().any(|(id, _)| s.group_ids.contains(id)),
            Inbox::Group(name) => groups
                .iter()
                .any(|(id, n)| n == name && s.group_ids.contains(id)),
        })
        .count() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(id: i32, last_read: Option<i32>, level: Option<i32>, groups: &[i32]) -> State {
        State {
            topic_id: id,
            last_read_post_number: last_read,
            notification_level: level,
            highest_post_number: 3,
            group_ids: groups.to_vec(),
        }
    }

    #[test]
    fn counts_follow_the_client() {
        let groups = vec![(2, "moderators".to_string())];
        let states = vec![
            // new, personal
            state(1, None, None, &[]),
            // new, to a group the viewer sees
            state(2, None, Some(2), &[2]),
            // unread, personal
            state(3, Some(1), Some(3), &[]),
            // read to the end: neither
            state(4, Some(3), Some(3), &[]),
            // regular: not unread
            state(5, Some(1), Some(1), &[]),
            // to a group the viewer does not see: personal
            state(6, None, Some(3), &[9]),
        ];
        let count = |kind, inbox| lookup_count(&states, &groups, kind, inbox);
        assert_eq!(count(Kind::New, Inbox::User), 2);
        assert_eq!(count(Kind::Unread, Inbox::User), 1);
        assert_eq!(count(Kind::New, Inbox::Group("moderators")), 1);
        assert_eq!(count(Kind::Unread, Inbox::Group("moderators")), 0);
        assert_eq!(count(Kind::New, Inbox::Group("staff")), 0);
    }
}
