//! The dashboard's rollup and pageview housekeeping jobs:
//! MaintainUserVisitDailyRollups, MaintainCategoryActivityDailyRollups,
//! RebuildCategoryActivityDailyRollups, MaintainBrowserPageviewRollups,
//! CleanUpBrowserPageviewEvents, CleanUpCrawlerStats and
//! DetectCrawlerPageviews.

use chrono::{Duration, NaiveDate};
use sqlx::PgConnection;

use crate::browser_pageviews as bpv;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `Jobs::MaintainUserVisitDailyRollups`, every 3 hours.
pub const MAINTAIN_USER_VISIT_DAILY_ROLLUPS: &str = "maintain_user_visit_daily_rollups";
/// `Jobs::MaintainCategoryActivityDailyRollups`, every 3 hours.
pub const MAINTAIN_CATEGORY_ACTIVITY_DAILY_ROLLUPS: &str =
    "maintain_category_activity_daily_rollups";
/// `Jobs::RebuildCategoryActivityDailyRollups`, daily (Rails: at 5am).
pub const REBUILD_CATEGORY_ACTIVITY_DAILY_ROLLUPS: &str = "rebuild_category_activity_daily_rollups";
/// `Jobs::MaintainBrowserPageviewRollups`, every 10 minutes.
pub const MAINTAIN_BROWSER_PAGEVIEW_ROLLUPS: &str = "maintain_browser_pageview_rollups";
/// `Jobs::CleanUpBrowserPageviewEvents`, daily.
pub const CLEAN_UP_BROWSER_PAGEVIEW_EVENTS: &str = "clean_up_browser_pageview_events";
/// `Jobs::CleanUpCrawlerStats`, daily.
pub const CLEAN_UP_CRAWLER_STATS: &str = "clean_up_crawler_stats";
/// `Jobs::DetectCrawlerPageviews`, every 10 minutes.
pub const DETECT_CRAWLER_PAGEVIEWS: &str = "detect_crawler_pageviews";

fn today() -> NaiveDate {
    crate::clock::now_naive().date()
}

async fn settings(conn: &mut PgConnection, state: &AppState) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// `Jobs::MaintainUserVisitDailyRollups`
pub async fn maintain_user_visit_daily_rollups(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let latest_rollup: Option<NaiveDate> =
        sqlx::query_scalar("SELECT MAX(date) FROM user_visit_daily_rollups")
            .fetch_one(&mut *tx)
            .await?;
    let latest_visit: Option<NaiveDate> =
        sqlx::query_scalar("SELECT MAX(visited_at) FROM user_visits")
            .fetch_one(&mut *tx)
            .await?;
    let end = today();
    let start = match latest_rollup {
        // The first aggregation, from the first visit.
        None => {
            let first: Option<NaiveDate> =
                sqlx::query_scalar("SELECT MIN(visited_at) FROM user_visits")
                    .fetch_one(&mut *tx)
                    .await?;
            let Some(first) = first else {
                return Ok(());
            };
            first
        }
        Some(latest) => {
            let mut start = end - Duration::days(1);
            if latest_visit.is_some_and(|v| v > latest) {
                start = start.min(latest + Duration::days(1));
            }
            start
        }
    };
    bpv::aggregate_user_visits(&mut tx, start, end).await?;
    tx.commit().await?;
    Ok(())
}

/// `Jobs::MaintainCategoryActivityDailyRollups`: the last two days, or a
/// rebuild when there are none.
pub async fn maintain_category_activity_daily_rollups(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let any: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM category_activity_daily_rollups)")
            .fetch_one(&mut *tx)
            .await?;
    if !any {
        bpv::rebuild_category_activity(&mut tx).await?;
    } else {
        bpv::aggregate_category_activity(&mut tx, today() - Duration::days(2), today()).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// `Jobs::RebuildCategoryActivityDailyRollups`
pub async fn rebuild_category_activity_daily_rollups(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    bpv::rebuild_category_activity(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}

/// `WebCrawlerRequest.max_record_age` (days) and `max_records_per_day`.
const CRAWLER_MAX_RECORD_DAYS: i64 = 30;
const CRAWLER_MAX_RECORDS_PER_DAY: i64 = 200;

/// `Jobs::CleanUpCrawlerStats`: old days gone, yesterday cut to its top
/// user agents.
pub async fn clean_up_crawler_stats(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM web_crawler_requests WHERE date < $1")
        .bind(crate::reports::bind_time(
            crate::clock::now_naive() - Duration::days(CRAWLER_MAX_RECORD_DAYS),
        ))
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "WITH ranked_requests AS ( \
           SELECT row_number() OVER (ORDER BY count DESC) as row_number, id \
           FROM web_crawler_requests WHERE date = $1 \
         ) \
         DELETE FROM web_crawler_requests WHERE id IN ( \
           SELECT ranked_requests.id FROM ranked_requests WHERE row_number > $2)",
    )
    .bind(today() - Duration::days(1))
    .bind(CRAWLER_MAX_RECORDS_PER_DAY)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// `Jobs::CleanUpBrowserPageviewEvents`: events (with their scores) and
/// session engagements past the retention period.
pub async fn clean_up_browser_pageview_events(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let s = settings(&mut tx, state).await?;
    if !s.get("clean_up_browser_pageview_events")?.truthy() {
        return Ok(());
    }
    let cutoff = bpv::retention_cutoff();
    sqlx::query(
        "DELETE FROM browser_pageview_event_scores WHERE event_id IN \
           (SELECT id FROM browser_pageview_events WHERE created_at < $1)",
    )
    .bind(cutoff)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM browser_pageview_events WHERE created_at < $1")
        .bind(cutoff)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM browser_pageview_session_engagements WHERE created_at < $1")
        .bind(cutoff)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// `Jobs::DetectCrawlerPageviews`: nothing unless the improved crawler
/// detection is on (CrawlerScorer is not ported).
pub async fn detect_crawler_pageviews(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(&mut conn, state).await?;
    if crate::reports::crawler_scorer_enabled(&s)? {
        return Err(Unsupported("scoring likely crawlers (CrawlerScorer)").into());
    }
    Ok(())
}

/// `Jobs::MaintainBrowserPageviewRollups`
pub async fn maintain_browser_pageview_rollups(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let s = settings(&mut tx, state).await?;
    // site_hostnames: every host name of the site, and current_hostname.
    let current = crate::url::Urls {
        config: &state.config,
        settings: &s,
    }
    .current_hostname()?;
    let hosts: Vec<String> = state
        .config
        .hostnames()
        .into_iter()
        .map(str::to_string)
        .chain([current])
        .collect();
    let end = today();
    let earliest: Option<NaiveDate> =
        sqlx::query_scalar("SELECT MIN(created_at)::date FROM browser_pageview_events")
            .fetch_one(&mut *tx)
            .await?;
    // aggregate_pageviews
    let none: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS (SELECT 1 FROM browser_pageview_country_daily_rollups) \
            AND NOT EXISTS (SELECT 1 FROM browser_pageview_referrer_daily_rollups)",
    )
    .fetch_one(&mut *tx)
    .await?;
    let start = if none {
        earliest
    } else {
        Some(end - Duration::days(1))
    };
    if let Some(start) = start {
        bpv::aggregate_countries(&mut tx, start, end).await?;
        bpv::aggregate_referrers(&mut tx, start, end).await?;
    }
    // aggregate_engagement
    let engagement_start: Option<NaiveDate> = sqlx::query_scalar(
        "SELECT COALESCE( \
           (SELECT MAX(date) FROM browser_pageview_session_engagement_daily_rollups WHERE date < $1), \
           (SELECT MIN(created_at)::date FROM browser_pageview_session_engagements))",
    )
    .bind(end)
    .fetch_one(&mut *tx)
    .await?;
    if let Some(start) = engagement_start {
        bpv::aggregate_engagement(&mut tx, start, end).await?;
    }
    // aggregate_crawlers
    if crate::reports::crawler_scorer_enabled(&s)? {
        let any: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM browser_pageview_crawler_daily_rollups)",
        )
        .fetch_one(&mut *tx)
        .await?;
        let start = if any {
            Some(end - Duration::days(1))
        } else {
            earliest
        };
        if let Some(start) = start {
            bpv::aggregate_crawlers(&mut tx, start, end).await?;
        }
    }
    let entry_urls_initialized: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM browser_pageview_entry_url_daily_rollups)",
    )
    .fetch_one(&mut *tx)
    .await?;
    let batch = s
        .get("browser_pageview_referrer_backfill_batch_size")?
        .to_i();
    let cleanup = s.get("clean_up_browser_pageview_events")?.truthy();
    backfill_referrers(&mut tx, &hosts, batch, cleanup, entry_urls_initialized).await?;
    backfill_urls(&mut tx, &hosts, batch, entry_urls_initialized).await?;
    if referrer_batch(&mut tx, 1, cleanup).await?.is_empty()
        && url_batch(&mut tx, 1).await?.is_empty()
    {
        let any: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM browser_pageview_entry_url_daily_rollups)",
        )
        .fetch_one(&mut *tx)
        .await?;
        let start = if any {
            Some(end - Duration::days(1))
        } else {
            earliest
        };
        if let Some(start) = start {
            bpv::aggregate_entry_urls(&mut tx, &hosts, start, end).await?;
        }
    }
    backfill_browsers(&mut tx, batch).await?;
    tx.commit().await?;
    Ok(())
}

/// `next_batch`: events whose referrer awaits normalization, newest first.
async fn referrer_batch(
    conn: &mut PgConnection,
    limit: i64,
    cleanup: bool,
) -> Result<Vec<(i64, Option<String>)>, AppError> {
    let retention = if cleanup {
        "AND created_at >= $3"
    } else {
        "AND $3::timestamp IS NOT NULL"
    };
    Ok(sqlx::query_as(&format!(
        "SELECT id, referrer FROM browser_pageview_events \
         WHERE referrer IS NOT NULL \
           AND (normalized_referrer_version IS NULL OR normalized_referrer_version < $1) {retention} \
         ORDER BY created_at DESC, id DESC LIMIT $2"
    ))
    .bind(bpv::REFERRER_VERSION)
    .bind(limit)
    .bind(bpv::retention_cutoff() + Duration::days(1))
    .fetch_all(conn)
    .await?)
}

/// `backfill_referrers`
async fn backfill_referrers(
    conn: &mut PgConnection,
    hosts: &[String],
    batch: i64,
    cleanup: bool,
    recompute_entry_urls: bool,
) -> Result<(), AppError> {
    let rows = referrer_batch(conn, batch, cleanup).await?;
    if rows.is_empty() {
        return Ok(());
    }
    let ids: Vec<i64> = rows.iter().map(|r| r.0).collect();
    let mut normalized: Vec<Option<String>> = Vec::new();
    for (_, referrer) in &rows {
        normalized.push(bpv::normalize_referrer(referrer.as_deref())?);
    }
    sqlx::query(
        "UPDATE browser_pageview_events AS e SET normalized_referrer = data.normalized_referrer \
         FROM (SELECT unnest($1::bigint[]) AS id, unnest($2::text[]) AS normalized_referrer) AS data \
         WHERE e.id = data.id",
    )
    .bind(&ids)
    .bind(&normalized)
    .execute(&mut *conn)
    .await?;
    // recomputable_dates: days whose other events are all normalized.
    let retention = if cleanup {
        "AND e.created_at >= $3"
    } else {
        "AND $3::timestamp IS NOT NULL"
    };
    let referrer_dates: Vec<NaiveDate> = sqlx::query_scalar(&format!(
        "WITH batch_ids AS (SELECT unnest($1::bigint[]) AS id), \
         touched_dates AS (SELECT DISTINCT created_at::date AS date FROM browser_pageview_events \
           WHERE id = ANY($1)) \
         SELECT touched_dates.date FROM touched_dates WHERE NOT EXISTS ( \
           SELECT 1 FROM browser_pageview_events e \
           WHERE e.created_at >= touched_dates.date AND e.created_at < touched_dates.date + 1 \
             AND e.referrer IS NOT NULL \
             AND NOT EXISTS (SELECT 1 FROM batch_ids WHERE batch_ids.id = e.id) \
             AND (e.normalized_referrer_version IS NULL OR e.normalized_referrer_version < $2) \
             {retention})"
    ))
    .bind(&ids)
    .bind(bpv::REFERRER_VERSION)
    .bind(bpv::retention_cutoff() + Duration::days(1))
    .fetch_all(&mut *conn)
    .await?;
    let entry_dates = entry_url_dates(conn, &ids, Backfill::Referrer).await?;
    bpv::recompute_referrers(conn, &referrer_dates).await?;
    if recompute_entry_urls {
        bpv::recompute_entry_urls(conn, hosts, &entry_dates).await?;
    }
    sqlx::query(
        "UPDATE browser_pageview_events SET normalized_referrer_version = $1 WHERE id = ANY($2)",
    )
    .bind(bpv::REFERRER_VERSION)
    .bind(&ids)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[derive(PartialEq)]
enum Backfill {
    Url,
    Referrer,
}

/// `entry_url_recomputable_dates`: days with no other event awaiting
/// either normalization.
async fn entry_url_dates(
    conn: &mut PgConnection,
    ids: &[i64],
    backfill: Backfill,
) -> Result<Vec<NaiveDate>, AppError> {
    let url_batch = if backfill == Backfill::Url {
        "AND events.id <> ALL($1)"
    } else {
        ""
    };
    let referrer_batch = if backfill == Backfill::Referrer {
        "AND events.id <> ALL($1)"
    } else {
        ""
    };
    Ok(sqlx::query_scalar(&format!(
        "WITH touched_dates AS (SELECT DISTINCT created_at::date AS date FROM browser_pageview_events \
           WHERE id = ANY($1)) \
         SELECT touched_dates.date FROM touched_dates WHERE NOT EXISTS ( \
           SELECT 1 FROM browser_pageview_events events \
           WHERE events.created_at >= touched_dates.date AND events.created_at < touched_dates.date + 1 \
             AND ( \
               ((events.normalized_url_version IS NULL OR events.normalized_url_version < $2) {url_batch}) \
               OR (events.referrer IS NOT NULL \
                 AND (events.normalized_referrer_version IS NULL OR events.normalized_referrer_version < $3) \
                 {referrer_batch})))"
    ))
    .bind(ids)
    .bind(bpv::SITE_PATH_VERSION)
    .bind(bpv::REFERRER_VERSION)
    .fetch_all(conn)
    .await?)
}

/// `url_batch`: events whose URL awaits normalization, newest first.
async fn url_batch(conn: &mut PgConnection, limit: i64) -> Result<Vec<(i64, String)>, AppError> {
    Ok(sqlx::query_as(
        "SELECT id, url FROM browser_pageview_events \
         WHERE created_at >= $1 AND (normalized_url_version IS NULL OR normalized_url_version < $2) \
         ORDER BY created_at DESC, id DESC LIMIT $3",
    )
    .bind(bpv::retention_cutoff())
    .bind(bpv::SITE_PATH_VERSION)
    .bind(limit)
    .fetch_all(conn)
    .await?)
}

/// `backfill_urls`
async fn backfill_urls(
    conn: &mut PgConnection,
    hosts: &[String],
    batch: i64,
    recompute_entry_urls: bool,
) -> Result<(), AppError> {
    let rows = url_batch(conn, batch).await?;
    if rows.is_empty() {
        return Ok(());
    }
    let ids: Vec<i64> = rows.iter().map(|r| r.0).collect();
    let normalized: Vec<Option<String>> = rows
        .iter()
        .map(|(_, url)| bpv::normalize_site_path(Some(url)))
        .collect();
    sqlx::query(
        "UPDATE browser_pageview_events AS e SET normalized_url = data.normalized_url \
         FROM unnest($1::bigint[], $2::text[]) AS data(id, normalized_url) WHERE e.id = data.id",
    )
    .bind(&ids)
    .bind(&normalized)
    .execute(&mut *conn)
    .await?;
    if recompute_entry_urls {
        let dates = entry_url_dates(conn, &ids, Backfill::Url).await?;
        bpv::recompute_entry_urls(conn, hosts, &dates).await?;
    }
    sqlx::query(
        "UPDATE browser_pageview_events SET normalized_url_version = $1 WHERE id = ANY($2)",
    )
    .bind(bpv::SITE_PATH_VERSION)
    .bind(&ids)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `backfill_browsers`
async fn backfill_browsers(conn: &mut PgConnection, batch: i64) -> Result<(), AppError> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, user_agent FROM browser_pageview_events \
         WHERE created_at >= $1 AND browser IS NULL ORDER BY created_at DESC, id DESC LIMIT $2",
    )
    .bind(bpv::retention_cutoff())
    .bind(batch)
    .fetch_all(&mut *conn)
    .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let ids: Vec<i64> = rows.iter().map(|r| r.0).collect();
    let browsers: Vec<i16> = rows.iter().map(|(_, ua)| bpv::browser_id(ua)).collect();
    sqlx::query(
        "UPDATE browser_pageview_events AS e SET browser = data.browser \
         FROM unnest($1::bigint[], $2::smallint[]) AS data(id, browser) WHERE e.id = data.id",
    )
    .bind(&ids)
    .bind(&browsers)
    .execute(conn)
    .await?;
    Ok(())
}

/// `Jobs::FlushBrowserPageviewEvents`, every 5 minutes.
pub const FLUSH_BROWSER_PAGEVIEW_EVENTS: &str = "flush_browser_pageview_events";

/// `Jobs::FlushBrowserPageviewEvents::MAX_FLUSH_SECONDS`
const MAX_FLUSH_SECONDS: u64 = 50;

/// `Jobs::FlushBrowserPageviewEvents`: the beacon events queued while
/// Postgres was read only, a batch at a time for up to 50 seconds.
pub async fn flush_browser_pageview_events(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    if crate::readonly::mode(&mut conn).await?.postgres {
        return Ok(());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(MAX_FLUSH_SECONDS);
    loop {
        if crate::request_tracking::queued_count(&state.site.pageview_queue) == 0 {
            return Ok(());
        }
        let processed =
            crate::request_tracking::flush_queued(&state.site.pageview_queue, &mut conn).await?;
        if processed < crate::request_tracking::FLUSH_BATCH_SIZE
            || std::time::Instant::now() >= deadline
        {
            return Ok(());
        }
    }
}
