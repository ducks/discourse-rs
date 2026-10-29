//! `Jobs::DiscoursePolicy::CheckPolicy` (every 6 hours): the reminders due
//! (a high priority topic_reminder to each member who has not accepted and
//! can see the post, the policy email to those who asked for it), the
//! renewals whose date passed, and the acceptances older than their
//! policy's renew period expired.

use chrono::{Duration, Months, NaiveDateTime};
use serde_json::json;
use sqlx::PgConnection;

use super::{PostPolicy, RENEW_INTERVALS, Users};
use crate::notification_create::{NewNotification, create};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// The job's name.
pub const JOB: &str = "discourse_policy_check_policy";

/// `Notification.types[:topic_reminder]`
const TOPIC_REMINDER: i32 = 18;

pub async fn run(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let now = crate::clock::now_naive();
    let post_ids: Vec<i32> = sqlx::query_scalar(
        "SELECT p.id FROM post_policies pp \
         JOIN posts p ON p.id = pp.post_id \
         JOIN topics t ON t.id = p.topic_id \
         WHERE t.deleted_at IS NULL AND p.deleted_at IS NULL AND t.archetype = 'regular' \
           AND ((reminder = 'weekly' AND last_reminded_at < $1) \
             OR (reminder = 'daily' AND last_reminded_at < $2)) \
         ORDER BY p.id",
    )
    .bind(now - Duration::weeks(1))
    .bind(now - Duration::days(1))
    .fetch_all(&mut *conn)
    .await?;
    for post_id in post_ids {
        remind(state, &mut conn, &settings, post_id, now).await?;
    }
    renew(&mut conn, now).await?;
    expire(&mut conn, now).await?;
    Ok(())
}

/// One post's reminders.
async fn remind(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    post_id: i32,
    now: NaiveDateTime,
) -> Result<(), AppError> {
    let Some(policy) = PostPolicy::for_post(conn, post_id).await? else {
        return Ok(());
    };
    let (topic_id, post_number, title): (i32, i32, String) = sqlx::query_as(
        "SELECT p.topic_id, p.post_number, t.title FROM posts p JOIN topics t ON t.id = p.topic_id \
         WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query("UPDATE post_policies SET last_reminded_at = $2, updated_at = $2 WHERE id = $1")
        .bind(policy.id)
        .bind(now)
        .execute(&mut *conn)
        .await?;
    let missing = policy.users(conn, Users::NotAccepted, None, 0).await?;
    for user_id in missing {
        let guardian = super::guardian_of(conn, Some(user_id)).await?;
        let visible = crate::posting::revisions::find_post_with_deleted_in(
            conn, settings, &guardian, post_id,
        )
        .await?
        .is_some();
        if !visible {
            continue;
        }
        // clear_existing_notification
        sqlx::query(
            "DELETE FROM notifications WHERE id = (SELECT id FROM notifications \
             WHERE notification_type = $1 AND topic_id = $2 AND post_number = $3 AND user_id = $4 \
             ORDER BY id LIMIT 1)",
        )
        .bind(TOPIC_REMINDER)
        .bind(topic_id)
        .bind(post_number)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
        let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
        create(
            conn,
            &state.bus,
            settings,
            &NewNotification::new(
                TOPIC_REMINDER,
                user_id,
                json!({ "topic_title": title, "display_username": username }),
            )
            .topic(topic_id, Some(post_number))
            .high_priority(true),
        )
        .await?;
    }
    for user_id in emailed_by(conn, &policy, now).await? {
        crate::jobs::enqueue(
            conn,
            "user_email",
            json!({ "type": "policy_email", "user_id": user_id, "post_id": post_id }),
        )
        .await?;
    }
    Ok(())
}

/// `PostPolicy#emailed_by`: the members who have not accepted and want
/// the email always, or when away and unseen for ten minutes.
async fn emailed_by(
    conn: &mut PgConnection,
    policy: &PostPolicy,
    now: NaiveDateTime,
) -> Result<Vec<i32>, sqlx::Error> {
    let missing = policy.users(conn, Users::NotAccepted, None, 0).await?;
    sqlx::query_scalar(
        "SELECT u.id FROM users u JOIN user_options o ON o.user_id = u.id \
         WHERE u.id = ANY($1) AND (o.policy_email_frequency = 2 \
           OR (o.policy_email_frequency = 1 AND u.last_seen_at < $2)) \
         ORDER BY u.id",
    )
    .bind(&missing)
    .bind(now - Duration::minutes(10))
    .fetch_all(conn)
    .await
}

/// The policies whose renewal date passed: their older acceptances
/// expired, the next date from their renew start on.
async fn renew(conn: &mut PgConnection, now: NaiveDateTime) -> Result<(), AppError> {
    #[derive(sqlx::FromRow)]
    struct Due {
        id: i64,
        post_id: i64,
        next_renew_at: NaiveDateTime,
        renew_start: Option<NaiveDateTime>,
        renew_days: Option<i32>,
        renew_interval: Option<i32>,
    }
    let due: Vec<Due> = sqlx::query_as(
        "SELECT id, post_id, next_renew_at, renew_start, renew_days, renew_interval \
         FROM post_policies WHERE next_renew_at < $1 ORDER BY id",
    )
    .bind(now)
    .fetch_all(&mut *conn)
    .await?;
    for policy in due {
        sqlx::query(
            "UPDATE policy_users SET expired_at = $3 WHERE post_policy_id = $1 \
             AND accepted_at IS NOT NULL AND revoked_at IS NULL AND expired_at IS NULL \
             AND accepted_at < $2",
        )
        .bind(policy.id)
        .bind(policy.next_renew_at)
        .bind(now)
        .execute(&mut *conn)
        .await?;
        let interval = policy
            .renew_interval
            .and_then(|i| RENEW_INTERVALS.get(usize::try_from(i).ok()?).copied());
        let days = policy.renew_days.unwrap_or(0);
        let mut next = policy.renew_start;
        if days < 1 && interval.is_none() {
            tracing::warn!("Invalid policy on post {}", policy.post_id);
        } else if let Some(mut at) = next {
            while at < now {
                at = match interval {
                    Some("monthly") => at + Months::new(1),
                    Some("quarterly") => at + Months::new(3),
                    Some("yearly") => at + Months::new(12),
                    _ => at + Duration::days(i64::from(days)),
                };
            }
            next = Some(at);
        }
        sqlx::query(
            "UPDATE post_policies SET next_renew_at = $2, updated_at = $3 \
             WHERE id = $1 AND next_renew_at IS DISTINCT FROM $2",
        )
        .bind(policy.id)
        .bind(next)
        .bind(now)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// The acceptances of policies without a renew start older than their
/// renew period.
async fn expire(conn: &mut PgConnection, now: NaiveDateTime) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE policy_users pu SET expired_at = $1 FROM post_policies pp \
         WHERE pp.id = pu.post_policy_id \
           AND pp.renew_start IS NULL \
           AND (pp.renew_days IS NOT NULL OR pp.renew_interval IS NOT NULL) \
           AND pu.accepted_at IS NOT NULL \
           AND pu.expired_at IS NULL \
           AND pu.revoked_at IS NULL \
           AND ( \
             (pp.renew_days IS NOT NULL AND pu.accepted_at < $1::timestamp - (INTERVAL '1 day' * pp.renew_days::integer)) OR \
             (pp.renew_interval = 0 AND pu.accepted_at < $1::timestamp - (INTERVAL '1 month')) OR \
             (pp.renew_interval = 1 AND pu.accepted_at < $1::timestamp - (INTERVAL '1 month' * 3)) OR \
             (pp.renew_interval = 2 AND pu.accepted_at < $1::timestamp - (INTERVAL '1 year')) \
           )",
    )
    .bind(now)
    .execute(conn)
    .await?;
    Ok(())
}
