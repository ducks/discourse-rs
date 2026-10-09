//! discourse-solved's SharedIssue::Toggle (POST /solution/shared_issue):
//! a member says they have the same problem ("Me too"), or takes it back.
//! Saying it starts tracking the topic for them. The topic's channel
//! hears the count. The per-topic rate limit (RateLimiter, Redis) is not
//! ported, as likes' isn't.

use serde_json::{Map, json};
use sqlx::PgConnection;

use super::{TopicFacts, TopicView};
use crate::AppError;
use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;

/// `TopicUser.notification_levels[:tracking]`
const TRACKING: i32 = 2;
/// `TopicUser.notification_reasons[:user_changed]`
const USER_CHANGED: i32 = 2;

pub enum Outcome {
    Done { count: i64, created: bool },
    NotFound,
    Forbidden,
}

pub async fn toggle(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    guardian: &Guardian,
    topic_id: i32,
) -> Result<Outcome, AppError> {
    let Some(user_id) = guardian.user_id() else {
        return Ok(Outcome::Forbidden);
    };
    // model :topic (a live topic)
    let topic: Option<TopicFacts> = sqlx::query_as(
        "SELECT id, user_id, category_id, archetype, closed, archived, deleted_at IS NOT NULL AS deleted \
         FROM topics WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(topic) = topic else {
        return Ok(Outcome::NotFound);
    };
    // policy :can_create_shared_issue: the topic view's answer, and
    // can_see_topic?.
    let view = TopicView::load(&mut *conn, settings, guardian, topic).await?;
    let mut keys = Map::new();
    view.shared_issue_keys(&mut *conn, settings, guardian, &mut keys)
        .await?;
    let can_see =
        match crate::topic_guardian::TopicCtx::load(&mut *conn, settings, guardian, topic_id)
            .await?
        {
            Some(ctx) => {
                let secure = guardian.secure_category_ids(&mut *conn, settings).await?;
                guardian.can_see_topic(settings, &ctx, true, &secure)?
            }
            None => false,
        };
    if keys["can_create_shared_issue"] != true || !can_see {
        return Ok(Outcome::Forbidden);
    }

    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM discourse_solved_shared_issues WHERE topic_id = $1 AND user_id = $2",
    )
    .bind(topic_id)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = existing {
        sqlx::query("DELETE FROM discourse_solved_shared_issues WHERE id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
    } else {
        sqlx::query(
            "INSERT INTO discourse_solved_shared_issues (topic_id, user_id, created_at, updated_at) \
             VALUES ($1, $2, clock_timestamp(), clock_timestamp())",
        )
        .bind(topic_id)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
        // start_tracking_topic, below tracking (no row counts as regular).
        let level: Option<i32> = sqlx::query_scalar(
            "SELECT notification_level FROM topic_users WHERE topic_id = $1 AND user_id = $2",
        )
        .bind(topic_id)
        .bind(user_id)
        .fetch_optional(&mut *conn)
        .await?
        .flatten();
        if level.unwrap_or(1) < TRACKING {
            crate::posting::change_topic_user(
                &mut *conn,
                user_id,
                topic_id,
                &[crate::posting::TopicUserAttr::NotificationLevel(
                    TRACKING,
                    USER_CHANGED,
                )],
            )
            .await?;
            crate::bus::publish_notification_level_change(
                bus,
                &mut *conn,
                user_id,
                topic_id,
                TRACKING,
                Some(USER_CHANGED),
            )
            .await?;
        }
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM discourse_solved_shared_issues WHERE topic_id = $1",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    let created = existing.is_none();
    crate::bus::publish_to_topic(
        bus,
        &mut *conn,
        topic_id,
        &json!({"type": "shared_issue", "count": count, "user_created_shared_issue": created}),
    )
    .await?;
    Ok(Outcome::Done { count, created })
}
