//! What reading writes: `PostTiming.process_timings` (the timings the
//! client reports as posts scroll by: post timings and reads, the time
//! read, the notifications on those posts, the topic user's last read post
//! and auto tracking, the day's visit) and `NotificationsController
//! #mark_read`.
//!
//! Refused: messages read through a group (TopicGroup.update_last_read).

use serde_json::json;
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::{AppError, Unsupported};

/// `PostTiming::MAX_READ_TIME_PER_BATCH`
const MAX_READ_TIME_PER_BATCH: i64 = 60_000;
/// `UserStat::MAX_TIME_READ_DIFF`, seconds.
const MAX_TIME_READ_DIFF: i64 = 100;
/// `TopicUser.notification_levels`
const REGULAR: i32 = 1;
const TRACKING: i32 = 2;

/// `UserStat.update_time_read!(id)`: the seconds since the user last
/// reported reading, when under MAX_TIME_READ_DIFF, then now cached.
async fn update_time_read(conn: &mut PgConnection, user_id: i32) -> Result<(), AppError> {
    let key = format!("user-last-seen:{user_id}");
    let now = crate::clock::now();
    let now_f = now.timestamp_micros() as f64 / 1_000_000.0;
    if let Some(last) = crate::owned_schema::cached_get(&mut *conn, &key).await? {
        let diff = (now_f - crate::ruby::to_f(&last)).round() as i64;
        if diff > 0 && diff < MAX_TIME_READ_DIFF {
            sqlx::query("UPDATE user_stats SET time_read = time_read + $2 WHERE user_id = $1")
                .bind(user_id)
                .bind(diff as i32)
                .execute(&mut *conn)
                .await?;
            sqlx::query(
                "UPDATE user_visits SET time_read = time_read + $2 WHERE user_id = $1 AND visited_at = $3",
            )
            .bind(user_id)
            .bind(diff as i32)
            .bind(now.date_naive())
            .execute(&mut *conn)
            .await?;
        }
    }
    crate::owned_schema::cached_setex(&mut *conn, &key, MAX_TIME_READ_DIFF, &now_f.to_string())
        .await?;
    Ok(())
}

/// `User#update_posts_read!(n)`: the day's visit gets the posts, made
/// (and counted in days_visited) if there is none yet.
async fn update_posts_read(conn: &mut PgConnection, user_id: i32, n: i64) -> Result<(), AppError> {
    let today = crate::clock::now().date_naive();
    let updated = sqlx::query(
        "UPDATE user_visits SET posts_read = posts_read + $3 WHERE user_id = $1 AND visited_at = $2",
    )
    .bind(user_id)
    .bind(today)
    .bind(n as i32)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if updated == 0 {
        // create_visit_record!
        sqlx::query(
            "INSERT INTO user_visits (user_id, visited_at, posts_read, mobile, time_read) \
             VALUES ($1, $2, $3, FALSE, 0)",
        )
        .bind(user_id)
        .bind(today)
        .bind(n as i32)
        .execute(&mut *conn)
        .await?;
        sqlx::query("UPDATE user_stats SET days_visited = days_visited + 1 WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// `PostTiming.record_new_timing`: a first timing counts a read of the
/// post, and outside messages a post read for the user.
async fn record_new_timing(
    conn: &mut PgConnection,
    topic_id: i32,
    user_id: i32,
    post_number: i32,
    msecs: i64,
) -> Result<(), AppError> {
    let inserted = sqlx::query(
        "INSERT INTO post_timings (topic_id, user_id, post_number, msecs) VALUES ($1, $2, $3, $4) \
         ON CONFLICT DO NOTHING",
    )
    .bind(topic_id)
    .bind(user_id)
    .bind(post_number)
    .bind(msecs as i32)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Ok(());
    }
    sqlx::query("UPDATE posts SET reads = reads + 1 WHERE topic_id = $1 AND post_number = $2")
        .bind(topic_id)
        .bind(post_number)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "UPDATE user_stats SET posts_read_count = posts_read_count + 1 WHERE user_id = $1 \
           AND NOT EXISTS (SELECT 1 FROM topics WHERE id = $2 AND archetype = 'private_message')",
    )
    .bind(user_id)
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `PostTiming.process_timings(user, topic_id, topic_time, timings)` for a
/// topic the user can see. Whether it marked any notifications read (Rails
/// then publishes the user's notification state).
pub async fn process_timings(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    guardian: &Guardian,
    topic_id: i32,
    topic_time: i64,
    timings: Vec<(i64, i64)>,
) -> Result<bool, AppError> {
    let user = guardian.user().ok_or(Unsupported("reading anonymously"))?;
    let whisperer = guardian.is_whisperer(s)?;
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
    let Some(highest) = highest else {
        return Ok(false);
    };
    let (allowed_groups, archetype): (bool, String) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM topic_allowed_groups WHERE topic_id = $1), archetype \
         FROM topics WHERE id = $1",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    if allowed_groups && archetype == "private_message" {
        return Err(Unsupported("group messages read (TopicGroup.update_last_read)").into());
    }

    update_time_read(&mut *conn, user.id).await?;

    // The most one batch may count per post: no more than the account's age.
    let created_at: chrono::NaiveDateTime =
        sqlx::query_scalar("SELECT created_at FROM users WHERE id = $1")
            .bind(user.id)
            .fetch_one(&mut *conn)
            .await?;
    let age_ms = (crate::clock::now_naive() - created_at).num_milliseconds();
    let max_time_per_post = age_ms.min(MAX_READ_TIME_PER_BATCH);

    let timings: Vec<(i32, i64)> = timings
        .into_iter()
        .filter(|(n, _)| *n >= 1 && *n <= i64::from(highest))
        .map(|(n, t)| (n as i32, t.min(max_time_per_post)))
        .collect();
    let highest_seen = timings.iter().map(|(n, _)| *n).max().unwrap_or(1).max(1);

    let mut new_posts_read = 0;
    let mut notifications_read = 0;
    if !timings.is_empty() {
        let mut existing = 0;
        let mut fresh = Vec::new();
        for (n, t) in &timings {
            let updated = sqlx::query(
                "UPDATE post_timings SET msecs = LEAST(msecs::bigint + $4, 2147483647) \
                 WHERE topic_id = $1 AND post_number = $2 AND user_id = $3",
            )
            .bind(topic_id)
            .bind(n)
            .bind(user.id)
            .bind(t)
            .execute(&mut *conn)
            .await?
            .rows_affected();
            if updated > 0 {
                existing += 1;
            } else {
                fresh.push((*n, *t));
            }
        }
        let regular: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND deleted_at IS NULL AND archetype = 'regular')",
        )
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
        if regular {
            new_posts_read = timings.len() as i64 - existing;
        }
        for (n, t) in fresh {
            record_new_timing(&mut *conn, topic_id, user.id, n, t).await?;
        }
        // Notification.mark_posts_read
        let numbers: Vec<i32> = timings.iter().map(|(n, _)| *n).collect();
        notifications_read = sqlx::query(
            "UPDATE notifications SET read = TRUE WHERE user_id = $1 AND topic_id = $2 \
               AND post_number = ANY($3) AND NOT read",
        )
        .bind(user.id)
        .bind(topic_id)
        .bind(&numbers)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    }
    let topic_time = topic_time.min(max_time_per_post);
    update_last_read(
        bus,
        conn,
        s,
        user.id,
        whisperer,
        topic_id,
        highest_seen,
        new_posts_read,
        topic_time,
    )
    .await?;
    Ok(notifications_read > 0)
}

/// `TopicUser.update_last_read`: the last read post (no further than the
/// topic goes), the time viewed, tracking once it passes the auto track
/// threshold; a first read makes the topic user.
#[allow(clippy::too_many_arguments)]
async fn update_last_read(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    user_id: i32,
    whisperer: bool,
    topic_id: i32,
    post_number: i32,
    new_posts_read: i64,
    msecs: i64,
) -> Result<(), AppError> {
    let msecs = msecs.max(0);
    let threshold = s.get("default_other_auto_track_topics_after_msecs")?.to_i();
    let row: Option<(i32, i32, Option<i32>, String)> = sqlx::query_as(
        "UPDATE topic_users SET \
           last_read_post_number = LEAST( \
             CASE WHEN $3 THEN t.highest_staff_post_number ELSE t.highest_post_number END, \
             GREATEST($4, tu.last_read_post_number)), \
           total_msecs_viewed = LEAST(tu.total_msecs_viewed + $5, 86400000), \
           notification_level = CASE WHEN tu.notifications_reason_id IS NULL \
               AND (tu.total_msecs_viewed + $5) > COALESCE(uo.auto_track_topics_after_msecs, $6) \
               AND COALESCE(uo.auto_track_topics_after_msecs, $6) >= 0 \
               AND t.archetype = 'regular' THEN $7 ELSE tu.notification_level END \
         FROM topic_users tu \
         JOIN topics t ON t.id = tu.topic_id \
         JOIN users u ON u.id = $1 \
         JOIN user_options uo ON uo.user_id = $1 \
         WHERE tu.topic_id = topic_users.topic_id AND tu.user_id = topic_users.user_id \
           AND tu.topic_id = $2 AND tu.user_id = $1 \
         RETURNING topic_users.notification_level, tu.notification_level, tu.last_read_post_number, t.archetype",
    )
    .bind(user_id)
    .bind(topic_id)
    .bind(whisperer)
    .bind(post_number)
    .bind(msecs)
    .bind(threshold)
    .bind(TRACKING)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((after, before, before_last_read, archetype)) = row {
        // The user read at least one new post. Messages' read state
        // (PrivateMessageTopicTrackingState) is not ported.
        if before_last_read.unwrap_or(0) < post_number && archetype == "regular" {
            crate::topic_tracking_state::publish_read(
                bus,
                s,
                &mut *conn,
                topic_id,
                post_number,
                user_id,
                Some(after),
            )
            .await?;
        }
        if new_posts_read > 0 {
            update_posts_read(&mut *conn, user_id, new_posts_read).await?;
        }
        if before != after {
            crate::bus::publish_notification_level_change(
                bus, &mut *conn, user_id, topic_id, after, None,
            )
            .await?;
        }
        return Ok(());
    }
    // A first read of the topic.
    let auto_track: Option<i32> = sqlx::query_scalar(
        "SELECT auto_track_topics_after_msecs FROM user_options WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?
    .flatten();
    let new_status = if i64::from(auto_track.unwrap_or(threshold as i32)) == 0 {
        TRACKING
    } else {
        REGULAR
    };
    let private_message: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND archetype = 'private_message')",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    if !private_message {
        crate::topic_tracking_state::publish_read(
            bus,
            s,
            &mut *conn,
            topic_id,
            post_number,
            user_id,
            Some(new_status),
        )
        .await?;
    }
    update_posts_read(&mut *conn, user_id, new_posts_read).await?;
    sqlx::query(
        "INSERT INTO topic_users (user_id, topic_id, last_read_post_number, last_visited_at, first_visited_at, \
                                  notification_level) \
         SELECT $1, $2, $3, now, now, $4 FROM clock_timestamp() AS now \
         WHERE EXISTS (SELECT 1 FROM topics WHERE id = $2) \
           AND NOT EXISTS (SELECT 1 FROM topic_users WHERE user_id = $1 AND topic_id = $2)",
    )
    .bind(user_id)
    .bind(topic_id)
    .bind(post_number)
    .bind(new_status)
    .execute(&mut *conn)
    .await?;
    crate::bus::publish_notification_level_change(
        bus, &mut *conn, user_id, topic_id, new_status, None,
    )
    .await?;
    Ok(())
}

/// How marking notifications read ends when it isn't a server error.
pub enum MarkRead {
    Done,
    /// `Discourse::InvalidParameters` with the message.
    Invalid(String),
}

/// `NotificationsController#mark_read`
pub async fn mark_read(
    conn: &mut PgConnection,
    user_id: i32,
    id: Option<i64>,
    dismiss_types: Option<&str>,
) -> Result<MarkRead, AppError> {
    if let Some(id) = id {
        // Notification.read
        sqlx::query(
            "UPDATE notifications SET read = TRUE WHERE id = $1 AND user_id = $2 AND NOT read",
        )
        .bind(id as i32)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
        return Ok(MarkRead::Done);
    }
    let types = match dismiss_types.map(str::trim).filter(|t| !t.is_empty()) {
        None => None,
        Some(list) => {
            let mut ids = Vec::new();
            let mut invalid = Vec::new();
            for name in list.split(',') {
                match crate::notifications::TYPES.iter().find(|(n, _)| *n == name) {
                    Some((_, id)) => ids.push(*id),
                    None => invalid.push(format!("{name:?}")),
                }
            }
            if !invalid.is_empty() {
                return Ok(MarkRead::Invalid(format!(
                    "invalid notification types: [{}]",
                    invalid.join(", ")
                )));
            }
            Some(ids)
        }
    };
    // Notification.read_types
    sqlx::query(
        "UPDATE notifications SET read = TRUE WHERE user_id = $1 AND NOT read \
           AND ($2::int[] IS NULL OR notification_type = ANY($2))",
    )
    .bind(user_id)
    .bind(&types)
    .execute(&mut *conn)
    .await?;
    // bump_last_seen_notification!: the newest visible notification past
    // the last seen, saved with the user.
    sqlx::query(
        "UPDATE users SET seen_notification_id = x.max_id, updated_at = clock_timestamp() \
         FROM (SELECT MAX(n.id) AS max_id FROM notifications n LEFT JOIN topics t ON t.id = n.topic_id \
               WHERE n.user_id = $1 AND (t.id IS NULL OR t.deleted_at IS NULL) \
                 AND n.id > COALESCE((SELECT seen_notification_id FROM users WHERE id = $1), 0)) x \
         WHERE users.id = $1 AND x.max_id IS NOT NULL",
    )
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    Ok(MarkRead::Done)
}

/// The success body.
pub fn success() -> serde_json::Value {
    json!({ "success": "OK" })
}
