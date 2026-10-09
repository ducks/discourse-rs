//! discourse-topic-voting's handlers for core's topic events, and the
//! jobs they enqueue: VoteRelease archives a topic's votes when voting no
//! longer applies (closed, archived, trashed, moved out of a voting
//! category), telling the voters unless it was trashed; VoteReclaim
//! restores them when it applies again.

use serde_json::{Value, json};
use sqlx::PgConnection;

use super::votes::update_vote_count;
use crate::AppError;
use crate::site_settings::SiteSettings;

const RELEASE: &str = "Jobs::DiscourseTopicVoting::VoteRelease";
const RECLAIM: &str = "Jobs::DiscourseTopicVoting::VoteReclaim";
/// `Notification.types[:votes_released]`
const VOTES_RELEASED: i32 = 26;

/// on(:topic_status_updated)
pub async fn topic_status_updated(
    conn: &mut PgConnection,
    topic_id: i32,
    status: &str,
    enabled: bool,
) -> Result<(), sqlx::Error> {
    if !matches!(status, "closed" | "autoclosed" | "archived") {
        return Ok(());
    }
    let (closed, archived, deleted): (bool, bool, bool) =
        sqlx::query_as("SELECT closed, archived, deleted_at IS NOT NULL FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
    if deleted {
        return Ok(());
    }
    if enabled {
        return crate::jobs::enqueue(conn, RELEASE, json!({ "topic_id": topic_id })).await;
    }
    let closing_unarchived = matches!(status, "closed" | "autoclosed") && !archived;
    let archiving_open = status == "archived" && !closed;
    if closing_unarchived || archiving_open {
        crate::jobs::enqueue(conn, RECLAIM, json!({ "topic_id": topic_id })).await?;
    }
    Ok(())
}

/// on(:topic_trashed)
pub async fn topic_trashed(
    conn: &mut PgConnection,
    topic_id: i32,
    closed: bool,
    archived: bool,
) -> Result<(), sqlx::Error> {
    if closed || archived {
        return Ok(());
    }
    crate::jobs::enqueue(
        conn,
        RELEASE,
        json!({ "topic_id": topic_id, "trashed": true }),
    )
    .await
}

/// Jobs::DiscourseTopicVoting::VoteRelease
pub async fn vote_release(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    args: &Value,
) -> Result<(), AppError> {
    let Some(topic_id) = topic_arg(conn, args).await? else {
        return Ok(());
    };
    sqlx::query("UPDATE topic_voting_votes SET archive = TRUE WHERE topic_id = $1")
        .bind(topic_id)
        .execute(&mut *conn)
        .await?;
    update_vote_count(conn, topic_id).await?;
    // The voter can't reach a trashed topic from the notification.
    if args.get("trashed") == Some(&Value::Bool(true)) {
        return Ok(());
    }
    // votes.find_each, by id.
    let voters: Vec<i32> = sqlx::query_scalar(
        "SELECT user_id FROM topic_voting_votes WHERE topic_id = $1 ORDER BY id",
    )
    .bind(topic_id)
    .fetch_all(&mut *conn)
    .await?;
    let data = json!({ "message": "votes_released", "title": "votes_released" }).to_string();
    for user_id in voters {
        sqlx::query(
            "INSERT INTO notifications (notification_type, user_id, topic_id, data, read, \
                                        high_priority, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, FALSE, FALSE, clock_timestamp(), clock_timestamp())",
        )
        .bind(VOTES_RELEASED)
        .bind(user_id)
        .bind(topic_id)
        .bind(&data)
        .execute(&mut *conn)
        .await?;
        crate::bus::publish_notifications_state(bus, &mut *conn, settings, user_id).await?;
    }
    Ok(())
}

/// Jobs::DiscourseTopicVoting::VoteReclaim
pub async fn vote_reclaim(conn: &mut PgConnection, args: &Value) -> Result<(), AppError> {
    let Some(topic_id) = topic_arg(conn, args).await? else {
        return Ok(());
    };
    sqlx::query("UPDATE topic_voting_votes SET archive = FALSE WHERE topic_id = $1")
        .bind(topic_id)
        .execute(&mut *conn)
        .await?;
    update_vote_count(conn, topic_id).await?;
    crate::jobs::enqueue(
        conn,
        "Jobs::DiscourseTopicVoting::BackfillBadges",
        json!({ "topic_id": topic_id }),
    )
    .await?;
    Ok(())
}

/// `Topic.with_deleted.find_by(id: args[:topic_id])`
async fn topic_arg(conn: &mut PgConnection, args: &Value) -> Result<Option<i32>, AppError> {
    let Some(id) = args.get("topic_id").and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    }) else {
        return Ok(None);
    };
    Ok(sqlx::query_scalar("SELECT id FROM topics WHERE id = $1")
        .bind(id as i32)
        .fetch_optional(conn)
        .await?)
}
