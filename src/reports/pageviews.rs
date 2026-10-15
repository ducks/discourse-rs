//! The traffic reports: the `*_reqs` request counts (req_report), the
//! consolidated pageviews and API requests, site traffic, web crawlers,
//! the browser pageview rollups' top entry URLs, countries and
//! referrers, and the storage stats.

use chrono::{Duration, NaiveDate, NaiveDateTime};
use serde_json::{Value, json};
use sqlx::PgConnection;

use super::counts::lit;
use super::{Cx, Report, colors, crawler_scorer_enabled, decimal_json, points, req_type};
use crate::{AppError, Unsupported};

/// `ApplicationRequest.browser_pageviews`' condition: each cohort read
/// from its beacons on the days they counted, else from the header.
pub fn browser_pageviews_where(alias: &str) -> String {
    let pairs = [
        ("page_view_anon_browser", "page_view_anon_browser_beacon"),
        (
            "page_view_logged_in_browser",
            "page_view_logged_in_browser_beacon",
        ),
    ];
    let mut ids = Vec::new();
    let mut conds = Vec::new();
    for (legacy, beacon) in pairs {
        let l = req_type(legacy).unwrap_or(-1);
        let b = req_type(beacon).unwrap_or(-1);
        ids.push(l);
        ids.push(b);
        conds.push(format!(
            "({alias}.req_type IN ({l}, {b}) AND ({alias}.req_type = {b}) = EXISTS ( \
               SELECT 1 FROM application_requests beacons \
               WHERE beacons.date = {alias}.date AND beacons.req_type = {b} AND beacons.count > 0))"
        ));
    }
    format!(
        "{alias}.req_type IN ({}) AND ({})",
        ids.iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(", "),
        conds.join(" OR ")
    )
}

/// The legacy pageview types: crawler, anonymous and logged in.
fn legacy_where() -> String {
    "application_requests.req_type IN (6, 8, 7)".into()
}

/// `Report.req_report(report, filter)`
pub async fn req_report(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
    filter: &str,
) -> Result<(), AppError> {
    let cond = match filter {
        "page_view_total" => {
            if cx.settings.get("use_legacy_pageviews")?.truthy() {
                legacy_where()
            } else {
                browser_pageviews_where("application_requests")
            }
        }
        "page_view_legacy_total" => legacy_where(),
        _ => match req_type(filter) {
            Some(id) => format!("application_requests.req_type = {id}"),
            None => "application_requests.req_type IS NULL".into(),
        },
    };
    if filter == "page_view_total" {
        r.icon = Some("file".into());
    }
    let rows: Vec<(NaiveDate, i64)> = sqlx::query_as(&format!(
        "SELECT date, SUM(count)::bigint FROM application_requests WHERE {cond} \
           AND date >= {} AND date <= {} GROUP BY date ORDER BY date ASC",
        lit(r.start_date),
        lit(r.end_date)
    ))
    .fetch_all(&mut *conn)
    .await?;
    r.data = points(&rows);
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT COALESCE(SUM(count), 0)::bigint FROM application_requests WHERE {cond}"
    ))
    .fetch_one(&mut *conn)
    .await?;
    r.total = Some(json!(total));
    let prev: i64 = sqlx::query_scalar(&format!(
        "SELECT COALESCE(SUM(count), 0)::bigint FROM application_requests WHERE {cond} \
           AND date >= {} AND date < {}",
        lit(r.start_date - Duration::days(31)),
        lit(r.start_date)
    ))
    .fetch_one(&mut *conn)
    .await?;
    r.prev30_days = Some(json!(prev));
    Ok(())
}

/// One request type's sums per day, as a stacked series.
async fn series(
    conn: &mut PgConnection,
    r: &Report,
    req: &str,
    label: String,
    color: &str,
) -> Result<Value, AppError> {
    let id = req_type(req).ok_or(Unsupported("an unknown request type"))?;
    let rows: Vec<(NaiveDate, i64)> = sqlx::query_as(&format!(
        "SELECT date, SUM(count)::bigint FROM application_requests WHERE req_type = {id} \
           AND date >= {} AND date <= {} GROUP BY date ORDER BY date ASC",
        lit(r.start_date),
        lit(r.end_date)
    ))
    .fetch_all(&mut *conn)
    .await?;
    Ok(json!({"req": req, "label": label, "color": color, "data": points(&rows)}))
}

/// `report_consolidated_page_views`
pub async fn consolidated_page_views(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["stacked_chart".into()];
    let mut data = Vec::new();
    for (req, color) in [
        ("page_view_logged_in", colors::TURQUOISE),
        ("page_view_anon", colors::LIME),
        ("page_view_crawler", colors::PURPLE),
    ] {
        let label = cx.t(&format!("reports.consolidated_page_views.xaxis.{req}"))?;
        data.push(series(conn, r, req, label, color).await?);
    }
    r.data = json!(data);
    Ok(())
}

/// `report_consolidated_api_requests`
pub async fn consolidated_api_requests(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["stacked_chart".into()];
    let mut data = Vec::new();
    for (req, color) in [("api", colors::TURQUOISE), ("user_api", colors::LIME)] {
        let label = cx.t(&format!("reports.consolidated_api_requests.xaxis.{req}"))?;
        data.push(series(conn, r, req, label, color).await?);
    }
    r.data = json!(data);
    Ok(())
}

/// `Reports::SiteTraffic::SERIES_COLORS`
const SERIES_COLORS: [(&str, &str); 6] = [
    ("page_view_logged_in_browser", "#4B3CE0"),
    ("page_view_anon_browser", "#9C8DEC"),
    ("page_view_crawler", "#D5CDF7"),
    ("page_view_likely_crawler", "#B3AAC9"),
    ("page_view_embed", "#E6E1F8"),
    ("page_view_other", "#E84A5F"),
];

/// `report_site_traffic`
pub async fn site_traffic(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["stacked_chart".into()];
    let browser = browser_pageviews_where("application_requests");
    let first: Option<NaiveDate> = sqlx::query_scalar(&format!(
        "SELECT MIN(date) FROM application_requests WHERE {browser}"
    ))
    .fetch_one(&mut *conn)
    .await?;
    let likely = crawler_scorer_enabled(cx.settings)?;
    let start = lit(r.start_date);
    let end = lit(r.end_date);
    let first = match first {
        Some(d) => format!("'{d}'::date"),
        None => "NULL".into(),
    };
    let sql = format!(
        "WITH browser_pageviews AS ( \
           SELECT date, \
             SUM(count) FILTER (WHERE req_type IN (15, 19)) AS logged_in, \
             SUM(count) FILTER (WHERE req_type IN (13, 17)) AS anonymous \
           FROM (SELECT application_requests.* FROM application_requests \
                 WHERE {browser} AND application_requests.date BETWEEN {start} AND {end}) requests \
           GROUP BY date \
         ), likely_crawlers AS ( \
           SELECT date, \
             COALESCE(SUM(count) FILTER (WHERE logged_in), 0)::bigint AS logged_in, \
             COALESCE(SUM(count) FILTER (WHERE NOT logged_in), 0)::bigint AS anonymous \
           FROM browser_pageview_crawler_daily_rollups \
           WHERE {likely} AND date >= {start} AND date <= {end} \
           GROUP BY date \
         ) \
         SELECT ar.date, \
           GREATEST(0, COALESCE(MAX(bp.logged_in), 0) - COALESCE(MAX(lc.logged_in), 0))::bigint AS page_view_logged_in_browser, \
           GREATEST(0, COALESCE(MAX(bp.anonymous), 0) - COALESCE(MAX(lc.anonymous), 0))::bigint AS page_view_anon_browser, \
           (COALESCE(MAX(lc.logged_in), 0) + COALESCE(MAX(lc.anonymous), 0))::bigint AS page_view_likely_crawler, \
           SUM(CASE WHEN ar.req_type = 6 THEN ar.count ELSE 0 END)::bigint AS page_view_crawler, \
           SUM(CASE WHEN ar.req_type = 21 THEN ar.count ELSE 0 END)::bigint AS page_view_embed, \
           GREATEST(0, SUM(CASE WHEN ar.req_type = 8 THEN ar.count WHEN ar.req_type = 7 THEN ar.count ELSE 0 END) \
             - COALESCE(MAX(bp.logged_in), 0) - COALESCE(MAX(bp.anonymous), 0))::bigint AS page_view_other \
         FROM application_requests ar \
         LEFT JOIN browser_pageviews bp ON bp.date = ar.date \
         LEFT JOIN likely_crawlers lc ON lc.date = ar.date \
         WHERE ar.date >= {start} AND ar.date <= {end} AND ar.date >= {first} \
         GROUP BY ar.date ORDER BY ar.date ASC"
    );
    #[allow(clippy::type_complexity)]
    let rows: Vec<(NaiveDate, i64, i64, i64, i64, i64, i64)> =
        sqlx::query_as(&sql).fetch_all(&mut *conn).await?;
    let pick = |i: usize| -> Value {
        Value::Array(
            rows.iter()
                .map(|row| {
                    let y = match i {
                        0 => row.1,
                        1 => row.2,
                        2 => row.3,
                        3 => row.4,
                        4 => row.5,
                        _ => row.6,
                    };
                    json!({"x": super::date_json(row.0), "y": y})
                })
                .collect(),
        )
    };
    let color = |req: &str| {
        SERIES_COLORS
            .iter()
            .find(|(k, _)| *k == req)
            .map(|(_, c)| *c)
            .unwrap_or_default()
    };
    let entry = |req: &str, i: usize| -> Result<Value, AppError> {
        Ok(json!({
            "req": req,
            "label": cx.t(&format!("reports.site_traffic.xaxis.{req}"))?,
            "color": color(req),
            "data": pick(i),
        }))
    };
    let mut data = vec![
        entry("page_view_logged_in_browser", 0)?,
        entry("page_view_anon_browser", 1)?,
    ];
    if likely {
        data.push(entry("page_view_likely_crawler", 2)?);
    }
    data.push(entry("page_view_crawler", 3)?);
    let embeds: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM embeddable_hosts)")
        .fetch_one(&mut *conn)
        .await?;
    if embeds {
        data.push(entry("page_view_embed", 4)?);
    }
    data.push(entry("page_view_other", 5)?);
    r.data = Value::Array(data);
    Ok(())
}

/// `report_web_crawlers`
pub async fn web_crawlers(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.labels = json!([
        {"type": "string", "property": "user_agent", "title": cx.t("reports.web_crawlers.labels.user_agent")?},
        {"property": "count", "type": "number", "title": cx.t("reports.web_crawlers.labels.page_views")?},
    ]);
    r.modes = vec!["table".into()];
    let rows: Vec<(String, i64)> = sqlx::query_as(&format!(
        "SELECT user_agent, SUM(count)::bigint AS sum_count FROM web_crawler_requests \
         WHERE date >= {} and date <= {} GROUP BY user_agent ORDER BY sum_count DESC LIMIT 200",
        lit(r.start_date),
        lit(r.end_date)
    ))
    .fetch_all(&mut *conn)
    .await?;
    r.data = Value::Array(
        rows.into_iter()
            .map(|(ua, n)| json!({"user_agent": ua, "count": n}))
            .collect(),
    );
    Ok(())
}

/// `BrowserPageviewEvent.rollup_count_sql`
fn rollup_count_sql(cx: &Cx<'_>) -> Result<String, AppError> {
    let logged_in_only = cx.settings.get("login_required")?.truthy();
    let count = if logged_in_only {
        "logged_in_count"
    } else {
        "count"
    };
    if !crawler_scorer_enabled(cx.settings)? {
        return Ok(count.into());
    }
    let crawler = if logged_in_only {
        "likely_crawler_logged_in_count"
    } else {
        "likely_crawler_count"
    };
    Ok(format!("GREATEST({count} - {crawler}, 0)"))
}

/// The rows of a ranked rollup query: (key, count as numeric text,
/// percent).
async fn ranked(
    conn: &mut PgConnection,
    sql: &str,
) -> Result<Vec<(String, String, i32)>, AppError> {
    Ok(sqlx::query_as(sql).fetch_all(conn).await?)
}

/// `report_top_entry_urls`
pub async fn top_entry_urls(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    r.labels = json!([
        {"properties": ["entry_url", "entry_url"], "type": "link", "title": cx.t("reports.top_entry_urls.labels.entry_url")?},
        {"property": "count", "type": "number", "title": cx.t("reports.top_entry_urls.labels.count")?},
    ]);
    let c = rollup_count_sql(cx)?;
    let rows = ranked(
        conn,
        &format!(
            "WITH ranked AS ( \
               SELECT entry_url, SUM({c}) AS count, SUM(SUM({c})) OVER () AS total \
               FROM browser_pageview_entry_url_daily_rollups \
               WHERE date >= '{}' AND date <= '{}' GROUP BY entry_url HAVING SUM({c}) > 0 \
             ) \
             SELECT entry_url, count::text AS count_text, CASE WHEN total = 0 THEN 0 \
               ELSE ROUND((count::numeric / total) * 100)::integer END AS percent \
             FROM ranked ORDER BY count DESC, entry_url ASC LIMIT 200",
            r.start_date.date(),
            r.end_date.date()
        ),
    )
    .await?;
    r.data = Value::Array(
        rows.into_iter()
            .map(|(url, n, p)| json!({"entry_url": url, "count": decimal_json(&n), "percent": p}))
            .collect(),
    );
    Ok(())
}

/// `TopCountriesByBrowserPageviews::EXCLUDED_COUNTRY_CODES`
const EXCLUDED_COUNTRY_CODES: &str = "('ZZ', 'T1', 'A1', 'A2', 'O1', 'XX', 'EU', 'AP')";

/// `report_top_countries_by_browser_pageviews`
pub async fn top_countries(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    r.labels = json!([
        {"property": "country_code", "type": "text", "title": cx.t("reports.top_countries_by_browser_pageviews.labels.country_code")?},
        {"property": "count", "type": "number", "title": cx.t("reports.top_countries_by_browser_pageviews.labels.count")?},
    ]);
    let c = rollup_count_sql(cx)?;
    let end_exclusive = r.end_date.date() + Duration::days(1);
    let rows = ranked(
        conn,
        &format!(
            "WITH ranked AS ( \
               SELECT country_code, SUM({c}) AS count, SUM(SUM({c})) OVER () AS total \
               FROM browser_pageview_country_daily_rollups \
               WHERE date >= {} AND date < '{end_exclusive}' GROUP BY country_code HAVING SUM({c}) > 0 \
             ) \
             SELECT country_code, count::text AS count_text, CASE WHEN total = 0 THEN 0 \
               ELSE ROUND((count::numeric / total) * 100)::integer END AS percent \
             FROM ranked WHERE country_code IS NOT NULL AND country_code NOT IN {EXCLUDED_COUNTRY_CODES} \
             ORDER BY count DESC, country_code ASC LIMIT 200",
            lit(r.start_date)
        ),
    )
    .await?;
    r.data = Value::Array(
        rows.into_iter()
            .map(|(code, n, p)| json!({"country_code": code, "count": decimal_json(&n), "percent": p}))
            .collect(),
    );
    Ok(())
}

/// `BrowserPageviewEventUrlNormalizer.normalize_host` for the site's
/// hostname: lowercased, without `www.` or a trailing dot.
fn normalize_host(host: &str) -> Result<String, AppError> {
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return Err(Unsupported("normalizing a hostname Addressable would encode").into());
    }
    let lower = host.to_ascii_lowercase();
    let lower = lower.strip_prefix("www.").unwrap_or(&lower);
    Ok(lower.strip_suffix('.').unwrap_or(lower).to_string())
}

/// `report_top_referrers_by_browser_pageviews`
pub async fn top_referrers(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    r.labels = json!([
        {"property": "normalized_referrer", "type": "text", "title": cx.t("reports.top_referrers_by_browser_pageviews.labels.normalized_referrer")?},
        {"property": "count", "type": "number", "title": cx.t("reports.top_referrers_by_browser_pageviews.labels.count")?},
    ]);
    let c = rollup_count_sql(cx)?;
    let end_exclusive = r.end_date.date() + Duration::days(1);
    let host = normalize_host(&cx.urls.current_hostname()?)?;
    let escaped: String = host
        .chars()
        .flat_map(|ch| {
            if matches!(ch, '\\' | '_' | '%') {
                vec!['\\', ch]
            } else {
                vec![ch]
            }
        })
        .collect();
    let rows: Vec<(String, String, i32)> = sqlx::query_as(&format!(
        "WITH ranked AS ( \
           SELECT normalized_referrer, SUM({c}) AS count, SUM(SUM({c})) OVER () AS total \
           FROM browser_pageview_referrer_daily_rollups \
           WHERE date >= {} AND date < '{end_exclusive}' AND normalized_referrer IS NOT NULL \
             AND normalized_referrer <> $1 \
             AND normalized_referrer NOT LIKE $2 ESCAPE '\\' \
             AND normalized_referrer NOT LIKE $3 ESCAPE '\\' \
           GROUP BY normalized_referrer HAVING SUM({c}) > 0 \
         ) \
         SELECT normalized_referrer, count::text AS count_text, CASE WHEN total = 0 THEN 0 \
           ELSE ROUND((count::numeric / total) * 100)::integer END AS percent \
         FROM ranked ORDER BY count DESC, normalized_referrer ASC LIMIT 200",
        lit(r.start_date)
    ))
    .bind(&host)
    .bind(format!("{escaped}/%"))
    .bind(format!("{escaped}?%"))
    .fetch_all(&mut *conn)
    .await?;
    r.data = Value::Array(
        rows.into_iter()
            .map(|(referrer, n, p)| {
                json!({"normalized_referrer": referrer, "count": decimal_json(&n), "percent": p})
            })
            .collect(),
    );
    Ok(())
}

/// Free bytes on the filesystem holding `path` (`df -Pk`'s available
/// column).
fn free_bytes(path: &std::path::Path) -> Result<i64, AppError> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Unsupported("a storage path with a NUL byte"))?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: statvfs reads the NUL-terminated path and fills `stat`.
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // df -k rounds the available blocks down to whole kilobytes.
    let available = stat.f_bavail as u128 * stat.f_frsize as u128;
    Ok((available / 1024 * 1024) as i64)
}

/// `report_storage_stats`: the local backup store and the uploads.
pub async fn storage_stats(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Report,
) -> Result<(), AppError> {
    if cx.settings.get("backup_location")?.to_s() != "local" {
        return Err(Unsupported("storage stats of a remote backup store").into());
    }
    let config = cx.urls.config;
    if config.globals.use_s3() || cx.settings.get("enable_s3_uploads")?.truthy() {
        return Err(Unsupported("storage stats of uploads on S3").into());
    }
    let public = &config.public_dir;
    // LocalBackupStore.base_directory makes the directory if need be.
    let backups_dir = public.join("backups").join("default");
    std::fs::create_dir_all(&backups_dir)?;
    let mut files: Vec<(u64, NaiveDateTime)> = Vec::new();
    for entry in std::fs::read_dir(&backups_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !(name.ends_with(".gz") || name.ends_with(".tgz")) || name.starts_with('.') {
            continue;
        }
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        let modified: chrono::DateTime<chrono::Utc> = meta.modified()?.into();
        files.push((meta.len(), modified.naive_utc()));
    }
    let used: u64 = files.iter().map(|f| f.0).sum();
    let latest = files.iter().map(|f| f.1).max();
    let uploads_dir = public.join("uploads").join("default");
    let uploads_free_at = if uploads_dir.exists() {
        uploads_dir
    } else {
        public.clone()
    };
    r.data = json!({
        "backups": {
            "used_bytes": used,
            "free_bytes": free_bytes(&backups_dir)?,
            "count": files.len(),
            "last_backup_taken_at": latest.map(|t| t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()),
        },
        "uploads": {
            "used_bytes": uploads_used_bytes(conn).await?,
            "free_bytes": free_bytes(&uploads_free_at)?,
        },
    });
    Ok(())
}

/// `DiskSpace.uploads_used_bytes`: the uploads' and optimized images'
/// sizes, as stored.
pub async fn uploads_used_bytes(conn: &mut PgConnection) -> Result<i64, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT (SELECT COALESCE(SUM(filesize), 0) FROM uploads)::bigint \
              + (SELECT COALESCE(SUM(filesize), 0) FROM optimized_images)::bigint",
    )
    .fetch_one(conn)
    .await?)
}
