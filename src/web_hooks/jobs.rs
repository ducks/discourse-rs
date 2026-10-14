//! The web hooks' scheduled jobs: purging old events and daily aggregates,
//! aggregating a day's events, and redelivering the events an admin asked
//! for again.

use serde_json::{Value, json};

use super::admin::{Event, WebHook};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `Jobs::PurgeOldWebHookEvents`, daily.
pub const PURGE_OLD_WEB_HOOK_EVENTS: &str = "purge_old_web_hook_events";
/// `Jobs::PurgeOldWebHookEventsDailyAggregate`, daily.
pub const PURGE_OLD_WEB_HOOK_EVENTS_DAILY_AGGREGATE: &str =
    "purge_old_web_hook_events_daily_aggregate";
/// `Jobs::AggregateWebHooksEvents`, daily.
pub const AGGREGATE_WEB_HOOKS_EVENTS: &str = "aggregate_web_hooks_events";
/// `Jobs::CleanupRedeliveringWebHookEvents`, hourly.
pub const CLEANUP_REDELIVERING_WEB_HOOK_EVENTS: &str = "cleanup_redelivering_web_hook_events";
/// `Jobs::RedeliverWebHookEvents`, every minute.
pub const REDELIVER_WEB_HOOK_EVENTS: &str = "redeliver_web_hook_events";

/// `Group::AUTO_GROUPS[:admins]`
const ADMINS: i32 = 1;
/// `RedeliverWebHookEvents::LIMIT`
const LIMIT: i64 = 20;

async fn settings(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// `WebHookEvent.purge_old`
pub async fn purge_old_web_hook_events(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    let days = s.get("retain_web_hook_events_period_days")?.to_i();
    sqlx::query("DELETE FROM web_hook_events WHERE created_at < now() - make_interval(days => $1)")
        .bind(days as i32)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `WebHookEventsDailyAggregate.purge_old`
pub async fn purge_old_web_hook_events_daily_aggregate(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    let days = s.get("retain_web_hook_events_aggregate_days")?.to_i();
    sqlx::query(
        "DELETE FROM web_hook_events_daily_aggregates \
         WHERE created_at < now() - make_interval(days => $1)",
    )
    .bind(days as i32)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `Jobs::AggregateWebHooksEvents`: for each active hook without an
/// aggregate of the day (`date`, else today in UTC), one, its
/// `before_create aggregate!` counting the day's events.
pub async fn aggregate_web_hooks_events(state: &AppState, args: &Value) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let date: chrono::NaiveDate = match crate::params::scalar(&args["date"])
        .filter(|d| !d.trim().is_empty())
    {
        Some(d) => match chrono::NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d") {
            Ok(date) => date,
            Err(_) => {
                return Err(crate::Unsupported("an aggregate date other than YYYY-MM-DD").into());
            }
        },
        None => {
            sqlx::query_scalar("SELECT (now() AT TIME ZONE 'UTC')::date")
                .fetch_one(&mut *conn)
                .await?
        }
    };
    let hooks: Vec<i32> = sqlx::query_scalar(
        "SELECT DISTINCT w.id FROM web_hooks w \
         LEFT JOIN web_hook_events_daily_aggregates a ON w.id = a.web_hook_id AND a.date = $1 \
         WHERE w.active AND a.id IS NULL ORDER BY w.id ASC",
    )
    .bind(date)
    .fetch_all(&mut *conn)
    .await?;
    for hook_id in hooks {
        let (count, sum, successful, failed): (i64, Option<i64>, i64, i64) = sqlx::query_as(
            "SELECT count(*), sum(duration)::bigint, \
                    count(*) FILTER (WHERE status >= 200 AND status <= 299), \
                    count(*) FILTER (WHERE status < 200 OR status > 299) \
             FROM web_hook_events WHERE created_at >= $1 AND created_at < $1 + interval '1 day' \
               AND web_hook_id = $2",
        )
        .bind(date.and_hms_opt(0, 0, 0))
        .bind(hook_id)
        .fetch_one(&mut *conn)
        .await?;
        // Integer division of the sum (nil durations count as nothing).
        let mean = if count == 0 {
            0
        } else {
            sum.unwrap_or(0).div_euclid(count)
        };
        sqlx::query(
            "INSERT INTO web_hook_events_daily_aggregates (web_hook_id, date, successful_event_count, \
                    failed_event_count, mean_duration, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, clock_timestamp(), clock_timestamp())",
        )
        .bind(i64::from(hook_id))
        .bind(date)
        .bind(successful as i32)
        .bind(failed as i32)
        .bind(mean as i32)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `Jobs::CleanupRedeliveringWebHookEvents`: those asked for over eight
/// hours ago.
pub async fn cleanup_redelivering_web_hook_events(state: &AppState) -> Result<(), AppError> {
    sqlx::query(
        "DELETE FROM redelivering_webhook_events WHERE created_at < now() - interval '8 hours'",
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// `Jobs::RedeliverWebHookEvents`: up to twenty events not yet being
/// redelivered, marked so, each sent again with its stored headers and
/// body, announced to the admins and forgotten (Rails sleeps two seconds
/// between them: pacing, left out).
pub async fn redeliver_web_hook_events(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM redelivering_webhook_events WHERE NOT processing ORDER BY ctid LIMIT $1",
    )
    .bind(LIMIT)
    .fetch_all(&mut *conn)
    .await?;
    if ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE redelivering_webhook_events SET processing = TRUE, updated_at = clock_timestamp() \
         WHERE id = ANY($1)",
    )
    .bind(&ids)
    .execute(&mut *conn)
    .await?;
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT id, web_hook_event_id FROM redelivering_webhook_events WHERE id = ANY($1) ORDER BY ctid",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    for (id, event_id) in rows {
        let event = Event::find(&mut conn, event_id).await?;
        let hook = match &event {
            Some(e) => WebHook::find(&mut conn, i64::from(e.web_hook_id)).await?,
            None => None,
        };
        let (Some(event), Some(hook)) = (event, hook) else {
            // The event or its hook is gone: Rails' rescue can't publish
            // about it either (NoMethodError on nil) and the job fails.
            return Err(std::io::Error::other("a redelivery of a missing web hook event").into());
        };
        let kind = match super::emit::redeliver(&mut conn, state, &s, &hook, &event).await {
            Ok(()) => "redelivered",
            Err(e) => {
                tracing::warn!(event = event.id, error = %e, "redelivering a web hook event");
                "redelivery_failed"
            }
        };
        let event = Event::find(&mut conn, event.id)
            .await?
            .ok_or_else(|| std::io::Error::other("a redelivered event vanished"))?;
        let hook = WebHook::find(&mut conn, i64::from(hook.id))
            .await?
            .unwrap_or(hook);
        state
            .bus
            .publish(
                &mut conn,
                &format!("/web_hook_events/{}", hook.id),
                &json!({
                    "type": kind,
                    "web_hook_event": super::admin::serialize_event(&event, &hook.payload_url),
                }),
                Some(&[crate::bus::group_tag(ADMINS)]),
            )
            .await?;
        sqlx::query("DELETE FROM redelivering_webhook_events WHERE id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}
