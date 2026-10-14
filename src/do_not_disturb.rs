//! Do not disturb: DoNotDisturbTiming and ShelvedNotification, the
//! user's `publish_do_not_disturb`, and releasing what was shelved
//! (`ShelvedNotification#process`, DoNotDisturbController#destroy's and
//! Jobs::ProcessShelvedNotifications').
//!
//! A notification created while its user is in do not disturb is shelved
//! by `notification_create`'s send_email; push notifications skip such a
//! user (`jobs::post_alert::push_notification`).

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::notification_emailer;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `Jobs::ProcessShelvedNotifications`, every 5 minutes.
pub const PROCESS_SHELVED_NOTIFICATIONS: &str = "process_shelved_notifications";

/// `User#publish_do_not_disturb(ends_at:)`: the end as an HTTP date (or
/// null), to the user alone.
pub async fn publish(
    bus: &pg_bus::Bus,
    conn: &mut PgConnection,
    user_id: i32,
    ends_at: Option<NaiveDateTime>,
) -> Result<(), AppError> {
    let ends_at = ends_at.map(httpdate);
    bus.publish(
        conn,
        &format!("/do-not-disturb/{user_id}"),
        &json!({ "ends_at": ends_at }),
        Some(&[crate::bus::user_tag(user_id)]),
    )
    .await?;
    Ok(())
}

/// `Time#httpdate`: RFC 1123 in GMT, whole seconds.
fn httpdate(t: NaiveDateTime) -> String {
    t.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

/// `User#do_not_disturb_until`:the latest end of the active timings.
pub async fn until(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<Option<NaiveDateTime>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT MAX(ends_at) FROM do_not_disturb_timings WHERE user_id = $1 \
         AND starts_at <= now() AND ends_at > now()",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?)
}

/// A shelved notification and what `process` reads of its notification.
#[derive(sqlx::FromRow)]
struct Shelved {
    id: i64,
    notification_id: i64,
    notification_type: i32,
    user_id: i32,
    topic_id: Option<i32>,
    post_number: Option<i32>,
    data: String,
}

const SHELVED_COLUMNS: &str = "sn.id, sn.notification_id, n.notification_type, n.user_id, \
     n.topic_id, n.post_number, n.data";

/// `ShelvedNotification#process`: the notification's email, now.
async fn process(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    shelved: &Shelved,
) -> Result<(), AppError> {
    let data: Map<String, Value> = serde_json::from_str(&shelved.data).unwrap_or_default();
    notification_emailer::process_notification(
        conn,
        settings,
        &notification_emailer::Notification {
            id: shelved.notification_id,
            notification_type: shelved.notification_type,
            user_id: shelved.user_id,
            topic_id: shelved.topic_id,
            post_number: shelved.post_number,
            data: &data,
        },
        true,
    )
    .await
}

/// `DoNotDisturbController#destroy`'s work: the active timings go (future
/// scheduled ones stay), the end is published, then each shelved
/// notification of the user is processed and its row destroyed.
pub async fn end(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<(), AppError> {
    sqlx::query(
        "DELETE FROM do_not_disturb_timings WHERE user_id = $1 \
         AND starts_at <= now() AND ends_at > now()",
    )
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    publish(bus, &mut *conn, user_id, None).await?;
    let shelved: Vec<Shelved> = sqlx::query_as(&format!(
        "SELECT {SHELVED_COLUMNS} FROM shelved_notifications sn \
         JOIN notifications n ON n.id = sn.notification_id WHERE n.user_id = $1 ORDER BY sn.id"
    ))
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    for s in &shelved {
        process(conn, settings, s).await?;
    }
    let ids: Vec<i64> = shelved.iter().map(|s| s.id).collect();
    sqlx::query("DELETE FROM shelved_notifications WHERE id = ANY($1)")
        .bind(&ids)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `Jobs::ProcessShelvedNotifications`: the shelved notifications of
/// users with any timing ended by now (an active one beside it doesn't
/// matter) are processed and destroyed, then the timings ended before now
/// go.
pub async fn process_shelved_notifications(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let now: DateTime<Utc> = crate::clock::now();
    let now = now.naive_utc();
    // ShelvedNotification.where(id: ids): each once, in the table's order.
    let shelved: Vec<Shelved> = sqlx::query_as(&format!(
        "SELECT {SHELVED_COLUMNS} FROM shelved_notifications sn \
         JOIN notifications n ON n.id = sn.notification_id \
         WHERE sn.id IN (SELECT sn.id FROM shelved_notifications sn \
                         JOIN notifications n ON sn.notification_id = n.id \
                         JOIN do_not_disturb_timings dndt ON n.user_id = dndt.user_id \
                           AND dndt.ends_at <= $1) \
         ORDER BY sn.id"
    ))
    .bind(now)
    .fetch_all(&mut *tx)
    .await?;
    for s in &shelved {
        process(&mut tx, &settings, s).await?;
    }
    let ids: Vec<i64> = shelved.iter().map(|s| s.id).collect();
    sqlx::query("DELETE FROM shelved_notifications WHERE id = ANY($1)")
        .bind(&ids)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM do_not_disturb_timings WHERE ends_at < $1")
        .bind(now)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn httpdate_is_rfc1123_gmt() {
        let t = chrono::NaiveDate::from_ymd_opt(2026, 10, 13)
            .and_then(|d| d.and_hms_micro_opt(23, 59, 59, 999_999))
            .expect("a valid time");
        assert_eq!(httpdate(t), "Tue, 13 Oct 2026 23:59:59 GMT");
        let t = chrono::NaiveDate::from_ymd_opt(2026, 1, 4)
            .and_then(|d| d.and_hms_opt(7, 5, 0))
            .expect("a valid time");
        assert_eq!(httpdate(t), "Sun, 04 Jan 2026 07:05:00 GMT");
    }
}
