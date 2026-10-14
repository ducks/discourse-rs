//! Browser pageviews: the events the beacon records
//! (BrowserPageviewEvent), their URL and referrer normalization
//! (BrowserPageviewEventUrlNormalizer), and the daily rollups the dashboard
//! reads (country, referrer, entry URL, session engagement and likely
//! crawler rollups, CategoryActivityDailyRollup, UserVisitDailyRollup).

use chrono::{Duration, NaiveDate, NaiveDateTime};
use sqlx::PgConnection;

use crate::{AppError, Unsupported};

/// `BrowserPageviewEventUrlNormalizer::REFERRER_VERSION`
pub const REFERRER_VERSION: i16 = 1;
/// `BrowserPageviewEventUrlNormalizer::SITE_PATH_VERSION`
pub const SITE_PATH_VERSION: i32 = 1;
/// `BrowserPageviewEventUrlNormalizer::MAX_LENGTH`
const MAX_LENGTH: usize = 2000;
/// `CrawlerScorer::BOT_SCORE_THRESHOLD`
pub const BOT_SCORE_THRESHOLD: i32 = 55;
/// `BrowserPageviewEvent::RETENTION_PERIOD`, in months.
const RETENTION_MONTHS: u32 = 3;

/// `TRACKING_PARAMS`
const TRACKING_PARAMS: [&str; 12] = [
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "fbclid",
    "gclid",
    "mc_cid",
    "mc_eid",
    "ref_src",
    "_hsenc",
    "_hsmi",
];

/// An Addressable::URI's parts, as `Addressable::URI.parse` splits them.
struct Parsed {
    scheme: Option<String>,
    host: Option<String>,
    path: String,
    query: Option<String>,
}

/// `Addressable::URI.parse(raw.strip)`: None for blank input or what
/// Addressable rejects (InvalidURIError).
fn parse_uri(raw: &str) -> Option<Parsed> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    static SPLIT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\A(?:([^:/?#]+):)?(?://([^/?#]*))?([^?#]*)(?:\?([^#]*))?(?:#(.*))?\z")
            .expect("a literal pattern")
    });
    let caps = SPLIT.captures(s)?;
    let scheme = caps.get(1).map(|m| m.as_str().to_string());
    let authority = caps.get(2).map(|m| m.as_str());
    let host = match authority {
        None => None,
        Some(a) => {
            let hostport = a.rsplit_once('@').map_or(a, |(_, h)| h);
            if let Some(rest) = hostport.strip_prefix('[') {
                let end = rest.find(']')?;
                let after = &rest[end + 1..];
                if !after.is_empty() {
                    let port = after.strip_prefix(':')?;
                    if !port.chars().all(|c| c.is_ascii_digit()) {
                        return None;
                    }
                }
                Some(format!("[{}]", &rest[..end]))
            } else {
                let (host, port) = match hostport.rsplit_once(':') {
                    Some((h, p)) => (h, Some(p)),
                    None => (hostport, None),
                };
                if let Some(p) = port
                    && !p.chars().all(|c| c.is_ascii_digit())
                {
                    // "Invalid port number"
                    return None;
                }
                Some(host.to_string())
            }
        }
    };
    Some(Parsed {
        scheme,
        host,
        path: caps.get(3).map_or("", |m| m.as_str()).to_string(),
        query: caps.get(4).map(|m| m.as_str().to_string()),
    })
}

/// `normalize_host(host)`: lowercased, without `www.` or a trailing dot.
pub fn normalize_host(host: Option<&str>) -> Result<Option<String>, Unsupported> {
    let Some(host) = host.filter(|h| !crate::ruby::is_blank(h)) else {
        return Ok(None);
    };
    if !host.is_ascii() {
        return Err(Unsupported(
            "normalizing a hostname that isn't ASCII (IDNA)",
        ));
    }
    if host.contains('%') {
        return Err(Unsupported("normalizing a percent-encoded hostname"));
    }
    let lower = host.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return Ok(None);
    }
    let lower = lower.strip_prefix("www.").unwrap_or(&lower);
    Ok(Some(lower.strip_suffix('.').unwrap_or(lower).to_string()))
}

/// `filter_query`: the pairs that aren't tracking parameters, as written.
fn filter_query(query: Option<&str>) -> String {
    let Some(q) = query.filter(|q| !q.is_empty()) else {
        return String::new();
    };
    q.split('&')
        .filter(|pair| {
            let key = pair.split_once('=').map_or(*pair, |(k, _)| k);
            !TRACKING_PARAMS.contains(&key)
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// `uri.path.sub(%r{/+\z}, "")`
fn normalized_path(path: &str) -> String {
    path.trim_end_matches('/').to_string()
}

/// `byteslice(0, MAX_LENGTH).scrub("")`
fn clip(s: &str) -> String {
    if s.len() <= MAX_LENGTH {
        return s.to_string();
    }
    let mut end = MAX_LENGTH;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// `normalize_referrer(raw)`: host, path and the kept query.
pub fn normalize_referrer(raw: Option<&str>) -> Result<Option<String>, Unsupported> {
    let Some(uri) = raw.and_then(parse_uri) else {
        return Ok(None);
    };
    let Some(host) = normalize_host(uri.host.as_deref())? else {
        return Ok(None);
    };
    let path = normalized_path(&uri.path);
    let query = filter_query(uri.query.as_deref());
    let query = if query.is_empty() {
        String::new()
    } else {
        format!("?{query}")
    };
    Ok(Some(clip(&format!("{host}{path}{query}"))))
}

/// `normalize_site_path(raw)`: the path alone, rooted.
pub fn normalize_site_path(raw: Option<&str>) -> Option<String> {
    let uri = raw.and_then(parse_uri)?;
    if let Some(scheme) = uri.scheme.as_deref().filter(|s| !s.is_empty())
        && (!matches!(scheme, "http" | "https")
            || uri.host.as_deref().is_none_or(crate::ruby::is_blank))
    {
        return None;
    }
    let mut path = normalized_path(&uri.path);
    if !path.starts_with('/') {
        path = format!("/{path}");
    }
    Some(clip(&path))
}

/// `BrowserPageviewEvent.browsers` for a user agent.
pub fn browser_id(user_agent: &str) -> i16 {
    match crate::user_private::browser_detection(user_agent).0 {
        "chrome" => 1,
        "edge" => 2,
        "safari" => 3,
        "firefox" => 4,
        "opera" => 5,
        "ie" => 6,
        "samsung_browser" => 7,
        "uc_browser" => 8,
        "android_browser" => 9,
        "qq_browser" => 10,
        "baidu_browser" => 11,
        "kaios_browser" => 12,
        _ => 0,
    }
}

/// `BrowserPageviewEvent.retention_cutoff`: three months ago, at midnight.
pub fn retention_cutoff() -> NaiveDateTime {
    let now = crate::clock::now_naive();
    let date = now
        .date()
        .checked_sub_months(chrono::Months::new(RETENTION_MONTHS))
        .unwrap_or(now.date());
    date.and_hms_opt(0, 0, 0).expect("midnight")
}

/// `CrawlerScorer.likely_crawler_condition(table:)`
pub fn likely_crawler(table: Option<&str>) -> String {
    let prefix = table.map(|t| format!("{t}.")).unwrap_or_default();
    format!("{prefix}score > {BOT_SCORE_THRESHOLD}")
}

/// `BrowserPageviewCountryDailyRollup.aggregate` and
/// `BrowserPageviewReferrerDailyRollup.aggregate` (by `column`).
async fn aggregate_counts(
    conn: &mut PgConnection,
    table: &str,
    column: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<(), AppError> {
    let crawler = likely_crawler(None);
    sqlx::query(&format!(
        "INSERT INTO {table} (date, {column}, count, logged_in_count, likely_crawler_count, \
                              likely_crawler_logged_in_count) \
         SELECT created_at::date AS date, {column}, COUNT(*) AS count, \
           COUNT(*) FILTER (WHERE user_id IS NOT NULL) AS logged_in_count, \
           COUNT(*) FILTER (WHERE {crawler}) AS likely_crawler_count, \
           COUNT(*) FILTER (WHERE user_id IS NOT NULL AND {crawler}) AS likely_crawler_logged_in_count \
         FROM browser_pageview_events WHERE created_at >= $1 AND created_at < $2 \
         GROUP BY date, {column} \
         ON CONFLICT (date, {column}) DO UPDATE SET count = EXCLUDED.count, \
           logged_in_count = EXCLUDED.logged_in_count, \
           likely_crawler_count = EXCLUDED.likely_crawler_count, \
           likely_crawler_logged_in_count = EXCLUDED.likely_crawler_logged_in_count"
    ))
    .bind(start)
    .bind(end + Duration::days(1))
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn aggregate_countries(
    conn: &mut PgConnection,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<(), AppError> {
    aggregate_counts(
        conn,
        "browser_pageview_country_daily_rollups",
        "country_code",
        start,
        end,
    )
    .await
}

pub async fn aggregate_referrers(
    conn: &mut PgConnection,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<(), AppError> {
    aggregate_counts(
        conn,
        "browser_pageview_referrer_daily_rollups",
        "normalized_referrer",
        start,
        end,
    )
    .await
}

/// `BrowserPageviewReferrerDailyRollup.recompute(dates)`: the dates that
/// still have events, rebuilt.
pub async fn recompute_referrers(
    conn: &mut PgConnection,
    dates: &[NaiveDate],
) -> Result<(), AppError> {
    let mut dates = dates.to_vec();
    dates.sort();
    dates.dedup();
    if dates.is_empty() {
        return Ok(());
    }
    let dates: Vec<NaiveDate> = sqlx::query_scalar(
        "SELECT d.date FROM unnest($1::date[]) AS d(date) \
         WHERE EXISTS (SELECT 1 FROM browser_pageview_events e \
           WHERE e.created_at >= d.date AND e.created_at < d.date + 1)",
    )
    .bind(&dates)
    .fetch_all(&mut *conn)
    .await?;
    if dates.is_empty() {
        return Ok(());
    }
    sqlx::query("DELETE FROM browser_pageview_referrer_daily_rollups WHERE date = ANY($1)")
        .bind(&dates)
        .execute(&mut *conn)
        .await?;
    for d in dates {
        aggregate_referrers(conn, d, d).await?;
    }
    Ok(())
}

/// `BrowserPageviewCrawlerDailyRollup.aggregate`
pub async fn aggregate_crawlers(
    conn: &mut PgConnection,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO browser_pageview_crawler_daily_rollups (date, logged_in, count) \
         SELECT created_at::date AS date, user_id IS NOT NULL AS logged_in, COUNT(*) AS count \
         FROM browser_pageview_events WHERE created_at >= $1 AND created_at < $2 AND score > $3 \
         GROUP BY date, logged_in \
         ON CONFLICT (date, logged_in) DO UPDATE SET count = EXCLUDED.count",
    )
    .bind(start)
    .bind(end + Duration::days(1))
    .bind(BOT_SCORE_THRESHOLD)
    .execute(conn)
    .await?;
    Ok(())
}

/// `BrowserPageviewSessionEngagement::BEACON_SETTLE_PERIOD`, in minutes.
pub const BEACON_SETTLE_MINUTES: i64 = 30;
/// `BOUNCE_ENGAGED_SECONDS_THRESHOLD`
const BOUNCE_ENGAGED_SECONDS: i32 = 10;

/// `BrowserPageviewSessionEngagementDailyRollup.aggregate`
pub async fn aggregate_engagement(
    conn: &mut PgConnection,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<(), AppError> {
    let end = end + Duration::days(1);
    sqlx::query(
        "DELETE FROM browser_pageview_session_engagement_daily_rollups rollup \
         WHERE rollup.date >= $1 AND rollup.date < $2 \
           AND EXISTS (SELECT 1 FROM browser_pageview_events \
             WHERE created_at >= rollup.date AND created_at < rollup.date + 1)",
    )
    .bind(start)
    .bind(end)
    .execute(&mut *conn)
    .await?;
    let crawler = likely_crawler(Some("bpe"));
    let settle_before = crate::clock::now_naive() - Duration::minutes(BEACON_SETTLE_MINUTES);
    sqlx::query(&format!(
        "WITH active_sessions AS ( \
           SELECT DISTINCT session_id FROM browser_pageview_events \
           WHERE created_at >= $1 AND created_at < LEAST($2::timestamp, $3::timestamp) \
         ), session_pageviews AS ( \
           SELECT sp.* FROM active_sessions CROSS JOIN LATERAL ( \
             SELECT bpe.session_id, MIN(bpe.created_at)::date AS date, COUNT(*) AS pageview_count, \
               bool_or(bpe.user_id IS NOT NULL) AS logged_in, bool_or({crawler}) AS likely_crawler \
             FROM browser_pageview_events bpe WHERE bpe.session_id = active_sessions.session_id \
             GROUP BY bpe.session_id HAVING MIN(bpe.created_at) >= $1 \
           ) sp \
         ) \
         INSERT INTO browser_pageview_session_engagement_daily_rollups \
           (date, logged_in, sessions, bounced, engaged_seconds_total, \
            likely_crawler_sessions, likely_crawler_bounced, likely_crawler_engaged_seconds_total) \
         SELECT session_pageviews.date, session_pageviews.logged_in, COUNT(*) AS sessions, \
           COUNT(*) FILTER (WHERE session_pageviews.pageview_count = 1 \
             AND COALESCE(engagement.engaged_seconds, 0) < $4) AS bounced, \
           COALESCE(SUM(engagement.engaged_seconds), 0) AS engaged_seconds_total, \
           COUNT(*) FILTER (WHERE session_pageviews.likely_crawler) AS likely_crawler_sessions, \
           COUNT(*) FILTER (WHERE session_pageviews.likely_crawler AND session_pageviews.pageview_count = 1 \
             AND COALESCE(engagement.engaged_seconds, 0) < $4) AS likely_crawler_bounced, \
           COALESCE(SUM(engagement.engaged_seconds) FILTER (WHERE session_pageviews.likely_crawler), 0) \
             AS likely_crawler_engaged_seconds_total \
         FROM session_pageviews \
         LEFT JOIN browser_pageview_session_engagements engagement \
           ON engagement.session_id = session_pageviews.session_id \
         GROUP BY session_pageviews.date, session_pageviews.logged_in"
    ))
    .bind(start.and_hms_opt(0, 0, 0).expect("midnight"))
    .bind(end.and_hms_opt(0, 0, 0).expect("midnight"))
    .bind(crate::reports::bind_time(settle_before))
    .bind(BOUNCE_ENGAGED_SECONDS)
    .execute(conn)
    .await?;
    Ok(())
}

/// `BrowserPageviewEntryUrlDailyRollup.site_hostnames`
fn site_hostnames(host: &str) -> Result<Vec<String>, AppError> {
    Ok(normalize_host(Some(host))?.into_iter().collect())
}

/// `BrowserPageviewEntryUrlDailyRollup.aggregate`
pub async fn aggregate_entry_urls(
    conn: &mut PgConnection,
    hostname: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<(), AppError> {
    let cutoff = retention_cutoff().date();
    let mut start = start.max(cutoff);
    // rebuildable_start_date
    let earliest: Option<NaiveDateTime> = sqlx::query_scalar(
        "SELECT MIN(created_at) FROM browser_pageview_events WHERE created_at >= $1",
    )
    .bind(retention_cutoff())
    .fetch_one(&mut *conn)
    .await?;
    if let Some(earliest) = earliest.map(|e| e.date())
        && start <= earliest
    {
        let older: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM browser_pageview_entry_url_daily_rollups WHERE date < $1)",
        )
        .bind(earliest)
        .fetch_one(&mut *conn)
        .await?;
        if older {
            start = earliest + Duration::days(1);
        }
    }
    if start > end {
        return Ok(());
    }
    let exclusive = end + Duration::days(1);
    sqlx::query(
        "DELETE FROM browser_pageview_entry_url_daily_rollups rollup \
         WHERE rollup.date >= $1 AND rollup.date < $2 \
           AND EXISTS (SELECT 1 FROM browser_pageview_events events \
             WHERE events.created_at >= rollup.date AND events.created_at < rollup.date + 1)",
    )
    .bind(start)
    .bind(exclusive)
    .execute(&mut *conn)
    .await?;
    let crawler = likely_crawler(Some("events"));
    sqlx::query(&format!(
        "INSERT INTO browser_pageview_entry_url_daily_rollups \
           (date, entry_url, count, logged_in_count, likely_crawler_count, likely_crawler_logged_in_count) \
         SELECT events.created_at::date, events.normalized_url, COUNT(*), \
           COUNT(*) FILTER (WHERE events.user_id IS NOT NULL), \
           COUNT(*) FILTER (WHERE COALESCE({crawler}, false)), \
           COUNT(*) FILTER (WHERE events.user_id IS NOT NULL AND COALESCE({crawler}, false)) \
         FROM browser_pageview_events events \
         WHERE events.created_at >= $1 AND events.created_at < $2 \
           AND events.normalized_url IS NOT NULL \
           AND (NULLIF(events.referrer, '') IS NULL \
             OR (events.normalized_referrer IS NOT NULL \
               AND split_part(split_part(events.normalized_referrer, '/', 1), '?', 1) <> ALL($3))) \
         GROUP BY events.created_at::date, events.normalized_url"
    ))
    .bind(start)
    .bind(exclusive)
    .bind(site_hostnames(hostname)?)
    .execute(conn)
    .await?;
    Ok(())
}

/// `BrowserPageviewEntryUrlDailyRollup.recompute(dates)`
pub async fn recompute_entry_urls(
    conn: &mut PgConnection,
    hostname: &str,
    dates: &[NaiveDate],
) -> Result<(), AppError> {
    let mut seen = Vec::new();
    for d in dates {
        if seen.contains(d) {
            continue;
        }
        seen.push(*d);
        aggregate_entry_urls(conn, hostname, *d, *d).await?;
    }
    Ok(())
}

/// `UserVisitDailyRollup.aggregate(start_date:, end_date:)`: DAU and the
/// rolling MAU per day, replacing the window's rows.
pub async fn aggregate_user_visits(
    conn: &mut PgConnection,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<(), AppError> {
    let rows: Vec<(NaiveDate, i64, i64)> = sqlx::query_as(
        "WITH island_boundaries AS ( \
           SELECT d, prev_d, next_d FROM ( \
             SELECT visited_at AS d, lag(visited_at) OVER w AS prev_d, lead(visited_at) OVER w AS next_d \
             FROM user_visits WHERE visited_at >= $1::DATE - 29 AND visited_at <= $2::DATE \
             WINDOW w AS (PARTITION BY user_id ORDER BY visited_at) \
           ) visits \
           WHERE prev_d IS NULL OR prev_d < d - 30 OR next_d IS NULL OR next_d > d + 30 \
         ), active_range_events AS ( \
           SELECT d AS day, 1 AS delta FROM island_boundaries WHERE prev_d IS NULL OR prev_d < d - 30 \
           UNION ALL \
           SELECT d + 30 AS day, -1 AS delta FROM island_boundaries WHERE next_d IS NULL OR next_d > d + 30 \
         ), daily_active_deltas AS ( \
           SELECT day, sum(delta) AS delta FROM active_range_events GROUP BY day \
         ), days AS ( \
           SELECT generate_series((SELECT min(day) FROM daily_active_deltas), $2::DATE, INTERVAL '1 day')::DATE AS day \
         ), rolling_mau AS ( \
           SELECT days.day, sum(coalesce(daily_active_deltas.delta, 0)) OVER (ORDER BY days.day) AS mau \
           FROM days LEFT JOIN daily_active_deltas ON daily_active_deltas.day = days.day \
         ), dau AS ( \
           SELECT visited_at AS date, count(*) AS dau FROM user_visits \
           WHERE visited_at >= $1::DATE AND visited_at <= $2::DATE GROUP BY visited_at \
         ) \
         SELECT dau.date, dau.dau, rolling_mau.mau::bigint FROM dau \
         JOIN rolling_mau ON rolling_mau.day = dau.date ORDER BY dau.date",
    )
    .bind(start)
    .bind(end)
    .fetch_all(&mut *conn)
    .await?;
    sqlx::query("DELETE FROM user_visit_daily_rollups WHERE date BETWEEN $1 AND $2")
        .bind(start)
        .bind(end)
        .execute(&mut *conn)
        .await?;
    for (date, dau, mau) in rows {
        sqlx::query("INSERT INTO user_visit_daily_rollups (date, dau, mau) VALUES ($1, $2, $3)")
            .bind(date)
            .bind(dau)
            .bind(mau)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// `CategoryActivityDailyRollup::ELIGIBLE_TOPICS`
const ELIGIBLE_TOPICS: &str =
    "topics.deleted_at IS NULL AND topics.archetype = 'regular' AND topics.category_id IS NOT NULL";
/// `AGGREGATION_CHUNK_DAYS`
const AGGREGATION_CHUNK_DAYS: i64 = 90;

/// `CategoryActivityDailyRollup.aggregate(start_date:, end_date:)`, in
/// 90 day chunks.
pub async fn aggregate_category_activity(
    conn: &mut PgConnection,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<(), AppError> {
    let mut start = start;
    while start <= end {
        let chunk_end = (start + Duration::days(AGGREGATION_CHUNK_DAYS - 1)).min(end);
        let crawler = likely_crawler(Some("bpe"));
        #[allow(clippy::type_complexity)]
        let rows: Vec<(NaiveDate, i32, i32, i32, i64, i64)> = sqlx::query_as(&format!(
            "WITH topic_counts AS ( \
               SELECT topics.created_at::date AS date, topics.category_id, COUNT(*) AS topics FROM topics \
               WHERE topics.created_at >= $1 AND topics.created_at < $2 AND {ELIGIBLE_TOPICS} GROUP BY 1, 2 \
             ), post_counts AS ( \
               SELECT posts.created_at::date AS date, topics.category_id, COUNT(*) AS posts FROM posts \
               INNER JOIN topics ON topics.id = posts.topic_id \
               WHERE posts.created_at >= $1 AND posts.created_at < $2 AND posts.deleted_at IS NULL \
                 AND posts.post_type = 1 AND {ELIGIBLE_TOPICS} GROUP BY 1, 2 \
             ), view_counts AS ( \
               SELECT tvs.viewed_at AS date, topics.category_id, \
                 SUM(tvs.anonymous_views + tvs.logged_in_views) AS page_views \
               FROM topic_view_stats tvs INNER JOIN topics ON topics.id = tvs.topic_id \
               WHERE tvs.viewed_at >= $1 AND tvs.viewed_at < $2 AND {ELIGIBLE_TOPICS} GROUP BY 1, 2 \
             ), crawler_view_counts AS ( \
               SELECT bpe.created_at::date AS date, topics.category_id, \
                 COUNT(DISTINCT (bpe.topic_id, COALESCE(bpe.user_id::text, 'ip-' || bpe.ip_address::text))) \
                   AS likely_crawler_page_views \
               FROM browser_pageview_events bpe INNER JOIN topics ON topics.id = bpe.topic_id \
               WHERE bpe.created_at >= $1 AND bpe.created_at < $2 AND {crawler} AND {ELIGIBLE_TOPICS} \
               GROUP BY 1, 2 \
             ) \
             SELECT combined.date, combined.category_id, SUM(combined.topics)::int AS topics, \
               SUM(combined.posts)::int AS posts, SUM(combined.page_views)::bigint AS page_views, \
               COALESCE(MAX(crawler_view_counts.likely_crawler_page_views), 0)::bigint AS likely_crawler_page_views \
             FROM ( \
               SELECT date, category_id, topics, 0 AS posts, 0 AS page_views FROM topic_counts \
               UNION ALL SELECT date, category_id, 0, posts, 0 FROM post_counts \
               UNION ALL SELECT date, category_id, 0, 0, page_views FROM view_counts \
             ) combined \
             LEFT JOIN crawler_view_counts ON crawler_view_counts.date = combined.date \
               AND crawler_view_counts.category_id = combined.category_id \
             GROUP BY combined.date, combined.category_id"
        ))
        .bind(start)
        .bind(chunk_end + Duration::days(1))
        .fetch_all(&mut *conn)
        .await?;
        replace_category_activity(conn, start, chunk_end, rows).await?;
        start = chunk_end + Duration::days(1);
    }
    Ok(())
}

/// `CategoryActivityDailyRollup.replace!`: the window's rows replaced,
/// likely crawler counts kept for days whose events are gone.
async fn replace_category_activity(
    conn: &mut PgConnection,
    start: NaiveDate,
    end: NaiveDate,
    rows: Vec<(NaiveDate, i32, i32, i32, i64, i64)>,
) -> Result<(), AppError> {
    if rows.is_empty() {
        sqlx::query("DELETE FROM category_activity_daily_rollups WHERE date BETWEEN $1 AND $2")
            .bind(start)
            .bind(end)
            .execute(&mut *conn)
            .await?;
        return Ok(());
    }
    // crawler_page_views_without_events
    let event_dates: Vec<NaiveDate> = sqlx::query_scalar(
        "SELECT DISTINCT created_at::date FROM browser_pageview_events \
         WHERE created_at >= $1 AND created_at < $2",
    )
    .bind(start)
    .bind(end + Duration::days(1))
    .fetch_all(&mut *conn)
    .await?;
    let preserved: Vec<(NaiveDate, i32, i64)> = sqlx::query_as(
        "SELECT date, category_id, likely_crawler_page_views FROM category_activity_daily_rollups \
         WHERE date BETWEEN $1 AND $2 AND likely_crawler_page_views > 0 AND NOT (date = ANY($3))",
    )
    .bind(start)
    .bind(end)
    .bind(&event_dates)
    .fetch_all(&mut *conn)
    .await?;
    sqlx::query("DELETE FROM category_activity_daily_rollups WHERE date BETWEEN $1 AND $2")
        .bind(start)
        .bind(end)
        .execute(&mut *conn)
        .await?;
    for (date, category_id, topics, posts, page_views, crawler) in rows {
        let crawler = preserved
            .iter()
            .find(|(d, c, _)| *d == date && *c == category_id)
            .map_or(crawler, |p| p.2);
        sqlx::query(
            "INSERT INTO category_activity_daily_rollups \
               (date, category_id, topics, posts, page_views, likely_crawler_page_views) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(date)
        .bind(category_id)
        .bind(topics)
        .bind(posts)
        .bind(page_views)
        .bind(crawler)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `CategoryActivityDailyRollup.rebuild!`: from the earliest eligible
/// topic or rollup to today.
pub async fn rebuild_category_activity(conn: &mut PgConnection) -> Result<(), AppError> {
    let earliest: Option<NaiveDateTime> = sqlx::query_scalar(&format!(
        "SELECT MIN(created_at) FROM topics WHERE {ELIGIBLE_TOPICS}"
    ))
    .fetch_one(&mut *conn)
    .await?;
    let min_rollup: Option<NaiveDate> =
        sqlx::query_scalar("SELECT MIN(date) FROM category_activity_daily_rollups")
            .fetch_one(&mut *conn)
            .await?;
    let start = [earliest.map(|e| e.date()), min_rollup]
        .into_iter()
        .flatten()
        .min();
    let Some(start) = start else {
        return Ok(());
    };
    aggregate_category_activity(conn, start, crate::clock::now_naive().date()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As the reference normalized them (scheduled/browser_pageview_rollups).
    #[test]
    fn normalizes_referrers_and_paths_as_rails() {
        let r = |s: &str| normalize_referrer(Some(s)).unwrap();
        assert_eq!(
            r("https://www.Google.com/search?q=x&utm_source=y#frag").as_deref(),
            Some("google.com/search?q=x")
        );
        assert_eq!(
            r("https://news.example.com:8443/a/b/?ref_src=z&k=1").as_deref(),
            Some("news.example.com/a/b?k=1")
        );
        assert_eq!(
            r("http://localhost/latest").as_deref(),
            Some("localhost/latest")
        );
        assert_eq!(r("www.example.com/no-scheme"), None);
        assert_eq!(r(""), None);
        let p = |s: &str| normalize_site_path(Some(s));
        assert_eq!(p("https://localhost/t/35?u=x").as_deref(), Some("/t/35"));
        assert_eq!(p("/latest/").as_deref(), Some("/latest"));
        assert_eq!(p("/").as_deref(), Some("/"));
        assert_eq!(p("mailto:x@example.com"), None);
        assert_eq!(p("relative/path").as_deref(), Some("/relative/path"));
    }
}
