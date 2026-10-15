//! The admin dashboard's traffic section (AdminDashboardSiteTraffic):
//! pageview KPIs and series over the request counts and the browser
//! pageview rollups, and the top countries, referrers and entry URLs
//! cards.

use chrono::{Duration, NaiveDate};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::AppError;
use crate::reports::{self, Cx, Opts, Viewer, req_type};

/// `TOP_CARD_LIMIT`
const TOP_CARD_LIMIT: usize = 5;

/// `Reports::SiteTraffic::SERIES_COLORS`
fn series_color(req: &str) -> &'static str {
    match req {
        "page_view_logged_in_browser" => "#4B3CE0",
        "page_view_anon_browser" => "#9C8DEC",
        "page_view_crawler" => "#D5CDF7",
        "page_view_likely_crawler" => "#B3AAC9",
        "page_view_embed" => "#E6E1F8",
        _ => "#E84A5F",
    }
}

/// A series of the pageview chart: its id and its day's value.
type Series = (&'static str, fn(&Row) -> i64);

/// A day of traffic.
#[derive(sqlx::FromRow)]
struct Row {
    date: NaiveDate,
    logged_in: i64,
    anonymous: i64,
    crawlers: i64,
    embedded: i64,
    likely_crawlers: i64,
}

struct Ctx {
    legacy: bool,
    login_required: bool,
    likely: bool,
}

impl Ctx {
    /// `selected_request_type_names`
    fn logged_in_name(&self) -> &'static str {
        if self.legacy {
            "page_view_logged_in"
        } else {
            "page_view_logged_in_browser"
        }
    }

    fn anonymous_name(&self) -> &'static str {
        if self.legacy {
            "page_view_anon"
        } else {
            "page_view_anon_browser"
        }
    }

    /// The logged in and anonymous request types with their beacons.
    fn logged_in_types(&self) -> String {
        format!(
            "{},{}",
            req_type(self.logged_in_name()).unwrap_or(-1),
            req_type("page_view_logged_in_browser_beacon").unwrap_or(-1)
        )
    }

    fn anonymous_types(&self) -> String {
        format!(
            "{},{}",
            req_type(self.anonymous_name()).unwrap_or(-1),
            req_type("page_view_anon_browser_beacon").unwrap_or(-1)
        )
    }
}

/// `traffic_rows(range_start_date, range_end_date)`
async fn traffic_rows(
    conn: &mut PgConnection,
    cx: &Ctx,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<Row>, AppError> {
    let requests = if cx.legacy {
        format!(
            "SELECT application_requests.* FROM application_requests \
             WHERE application_requests.req_type IN ({}, {})",
            req_type(cx.logged_in_name()).unwrap_or(-1),
            req_type(cx.anonymous_name()).unwrap_or(-1)
        )
    } else {
        format!(
            "SELECT application_requests.* FROM application_requests WHERE {}",
            crate::reports::pageviews_browser_where("application_requests")
        )
    };
    let sql = format!(
        "WITH dates AS ( \
           SELECT request_date::date AS date FROM generate_series( \
             CAST($1 AS date), CAST($2 AS date), INTERVAL '1 day') request_date \
         ), likely_crawlers AS ( \
           SELECT date, \
             COALESCE(SUM(count) FILTER (WHERE logged_in), 0)::bigint AS logged_in, \
             COALESCE(SUM(count) FILTER (WHERE NOT logged_in), 0)::bigint AS anonymous \
           FROM browser_pageview_crawler_daily_rollups \
           WHERE $3 AND date >= CAST($1 AS date) AND date <= CAST($2 AS date) GROUP BY date \
         ), human_pageviews AS ( \
           SELECT date, \
             COALESCE(SUM(count) FILTER (WHERE req_type IN ({logged})), 0)::bigint AS logged_in, \
             COALESCE(SUM(count) FILTER (WHERE req_type IN ({anon})), 0)::bigint AS anonymous \
           FROM ({requests} AND application_requests.date BETWEEN $1 AND $2) requests GROUP BY date \
         ) \
         SELECT dates.date, \
           GREATEST(0, COALESCE(MAX(hp.logged_in), 0) - COALESCE(MAX(lc.logged_in), 0))::bigint AS logged_in, \
           GREATEST(0, COALESCE(MAX(hp.anonymous), 0) - COALESCE(MAX(lc.anonymous), 0))::bigint AS anonymous, \
           COALESCE(SUM(CASE WHEN ar.req_type = 6 THEN ar.count ELSE 0 END), 0)::bigint AS crawlers, \
           COALESCE(SUM(CASE WHEN ar.req_type = 21 THEN ar.count ELSE 0 END), 0)::bigint AS embedded, \
           (COALESCE(MAX(lc.logged_in), 0) \
             + CASE WHEN $4 THEN 0 ELSE COALESCE(MAX(lc.anonymous), 0) END)::bigint AS likely_crawlers \
         FROM dates \
         LEFT JOIN application_requests ar ON ar.date = dates.date AND ar.req_type IN (6, 21) \
         LEFT JOIN human_pageviews hp ON hp.date = dates.date \
         LEFT JOIN likely_crawlers lc ON lc.date = dates.date \
         GROUP BY dates.date ORDER BY dates.date ASC",
        logged = cx.logged_in_types(),
        anon = cx.anonymous_types(),
    );
    Ok(sqlx::query_as(&sql)
        .bind(start)
        .bind(end)
        .bind(cx.likely)
        .bind(cx.login_required)
        .fetch_all(conn)
        .await?)
}

/// `AdminDashboardSiteTraffic.build`
pub async fn build(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    start: Option<&str>,
    end: Option<&str>,
) -> Result<Value, AppError> {
    let now = crate::clock::now_naive();
    let default_start = reports::beginning_of_day((now - Duration::days(29)).date());
    let default_end = reports::end_of_day(now.date());
    let mut s = crate::admin_dashboard::zone_parse(start)?
        .map(reports::beginning_of_day)
        .unwrap_or(default_start);
    let mut e = crate::admin_dashboard::zone_parse(end)?
        .map(reports::end_of_day)
        .unwrap_or(default_end);
    if s.date() > e.date() {
        s = default_start;
        e = default_end;
    }
    let ctx = Ctx {
        legacy: cx.settings.get("use_legacy_pageviews")?.truthy(),
        login_required: cx.settings.get("login_required")?.truthy(),
        likely: reports::crawler_scorer_enabled(cx.settings)?,
    };
    let day_count = (e.date() - s.date()).num_days() + 1;
    let prior_end = s.date() - Duration::days(1);
    let prior_start = prior_end - Duration::days(day_count - 1);
    let current = traffic_rows(conn, &ctx, s.date(), e.date()).await?;
    let prior = traffic_rows(conn, &ctx, prior_start, prior_end).await?;
    // include_embedded_series?
    let embedding = cx.settings.get("embed_topics_list")?.truthy()
        || cx.settings.get("embed_full_app")?.truthy();
    let include_embedded = !ctx.login_required
        && embedding
        && sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM embeddable_hosts)")
            .fetch_one(&mut *conn)
            .await?;
    let sum = |rows: &[Row], f: fn(&Row) -> i64| rows.iter().map(f).sum::<i64>();
    let logged_in = sum(&current, |r| r.logged_in);
    let anonymous = if ctx.login_required {
        0
    } else {
        sum(&current, |r| r.anonymous)
    };
    let human = logged_in + anonymous;
    // browser_pageviews: the value, with the trend over a complete prior
    // period.
    let mut pageviews = Map::new();
    pageviews.insert("value".into(), json!(human));
    if prior_tracking_started(conn, &ctx, prior_start).await? {
        let prev = sum(&prior, |r| r.logged_in)
            + if ctx.login_required {
                0
            } else {
                sum(&prior, |r| r.anonymous)
            };
        if prev != 0 && human != prev {
            let change = (human - prev) as f64 / prev as f64 * 100.0;
            if change.abs() >= 0.05 {
                let formatted = if change.abs() < 1.0 {
                    reports::float_json(reports::round_to(change, 1))
                } else {
                    json!(change.round() as i64)
                };
                pageviews.insert("percent_change".into(), formatted);
                pageviews.insert(
                    "comparison_period".into(),
                    json!({
                        "start_date": prior_start.format("%Y-%m-%d").to_string(),
                        "end_date": prior_end.format("%Y-%m-%d").to_string(),
                    }),
                );
            }
        }
    }
    let mut kpis = Map::new();
    kpis.insert("browser_pageviews".into(), Value::Object(pageviews));
    if !ctx.login_required {
        let share = if human > 0 {
            (logged_in as f64 / human as f64 * 100.0).round() as i64
        } else {
            0
        };
        kpis.insert("logged_in_share".into(), json!({ "value": share }));
    }
    let count_sql = rollup_count_sql(cx)?;
    let (total, direct): (i64, i64) = sqlx::query_as(&format!(
        "SELECT COALESCE(SUM({count_sql}), 0)::bigint AS total, \
           COALESCE(SUM({count_sql}) FILTER (WHERE normalized_referrer IS NULL), 0)::bigint AS direct \
         FROM browser_pageview_referrer_daily_rollups WHERE date >= $1 AND date <= $2"
    ))
    .bind(s.date())
    .bind(e.date())
    .fetch_one(&mut *conn)
    .await?;
    if total != 0 {
        kpis.insert(
            "direct_traffic".into(),
            json!({ "value": (direct as f64 / total as f64 * 100.0).round() as i64 }),
        );
    }
    let human_col = |col: &str, crawler: &str| {
        if ctx.likely {
            format!("GREATEST({col} - {crawler}, 0)")
        } else {
            col.to_string()
        }
    };
    let (sessions, bounced, engaged): (i64, i64, i64) = sqlx::query_as(&format!(
        "SELECT COALESCE(SUM({}), 0)::bigint, COALESCE(SUM({}), 0)::bigint, COALESCE(SUM({}), 0)::bigint \
         FROM browser_pageview_session_engagement_daily_rollups WHERE date >= $1 AND date <= $2",
        human_col("sessions", "likely_crawler_sessions"),
        human_col("bounced", "likely_crawler_bounced"),
        human_col("engaged_seconds_total", "likely_crawler_engaged_seconds_total"),
    ))
    .bind(s.date())
    .bind(e.date())
    .fetch_one(&mut *conn)
    .await?;
    let per_session = |n: i64| -> Value {
        if sessions == 0 {
            Value::Null
        } else {
            json!((n as f64 / sessions as f64).round() as i64)
        }
    };
    kpis.insert(
        "bounce_rate".into(),
        json!({ "value": if sessions == 0 { Value::Null } else { json!((bounced as f64 / sessions as f64 * 100.0).round() as i64) } }),
    );
    kpis.insert(
        "average_session_duration_seconds".into(),
        json!({ "value": per_session(engaged) }),
    );
    // series_ids
    let mut series: Vec<Series> = vec![("logged_in", |r| r.logged_in)];
    if !ctx.login_required {
        series.push(("anonymous", |r| r.anonymous));
    }
    if !ctx.login_required && include_embedded {
        series.push(("embedded", |r| r.embedded));
    }
    if ctx.likely {
        series.push(("likely_crawlers", |r| r.likely_crawlers));
    }
    if !ctx.login_required {
        series.push(("crawlers", |r| r.crawlers));
    }
    let mut pageview_series = Vec::new();
    for (id, value) in series {
        let label_req = match id {
            "logged_in" => "page_view_logged_in_browser",
            "anonymous" => "page_view_anon_browser",
            "embedded" => "page_view_embed",
            "crawlers" => "page_view_crawler",
            _ => "page_view_likely_crawler",
        };
        let req = match id {
            "logged_in" => ctx.logged_in_name(),
            "anonymous" => ctx.anonymous_name(),
            "embedded" => "page_view_embed",
            "crawlers" => "page_view_crawler",
            _ => "page_view_likely_crawler",
        };
        pageview_series.push(json!({
            "req": req,
            "label": cx.t(&format!("reports.site_traffic.xaxis.{label_req}"))?,
            "color": series_color(label_req),
            "data": current.iter().map(|r| json!({"x": r.date.format("%Y-%m-%d").to_string(), "y": value(r)})).collect::<Vec<_>>(),
        }));
    }
    let mut out = Map::new();
    out.insert("kpis".into(), Value::Object(kpis));
    out.insert("pageview_series".into(), Value::Array(pageview_series));
    if !ctx.legacy {
        for (key, kind) in [
            ("top_countries", "top_countries_by_browser_pageviews"),
            ("top_referrers", "top_referrers_by_browser_pageviews"),
            ("top_entry_urls", "top_entry_urls"),
        ] {
            if let Some(card) = fetch_card(conn, cx, viewer, kind, s, e).await? {
                out.insert(key.into(), card);
            }
        }
    }
    Ok(Value::Object(out))
}

/// `prior_period_tracking_started?`
async fn prior_tracking_started(
    conn: &mut PgConnection,
    cx: &Ctx,
    prior_start: NaiveDate,
) -> Result<bool, AppError> {
    let types = if cx.login_required {
        cx.logged_in_types()
    } else {
        format!("{},{}", cx.logged_in_types(), cx.anonymous_types())
    };
    Ok(sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM application_requests WHERE date <= $1 AND req_type IN ({types}))"
    ))
    .bind(prior_start)
    .fetch_one(conn)
    .await?)
}

fn rollup_count_sql(cx: &Cx<'_>) -> Result<String, AppError> {
    let logged_in_only = cx.settings.get("login_required")?.truthy();
    let count = if logged_in_only {
        "logged_in_count"
    } else {
        "count"
    };
    if !reports::crawler_scorer_enabled(cx.settings)? {
        return Ok(count.into());
    }
    let crawler = if logged_in_only {
        "likely_crawler_logged_in_count"
    } else {
        "likely_crawler_count"
    };
    Ok(format!("GREATEST({count} - {crawler}, 0)"))
}

/// `fetch_card(type)`: the report's first rows, or its error.
async fn fetch_card(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    viewer: &Viewer<'_>,
    kind: &str,
    start: chrono::NaiveDateTime,
    end: chrono::NaiveDateTime,
) -> Result<Option<Value>, AppError> {
    if reports::hidden(kind, viewer, cx.settings)? {
        return Ok(None);
    }
    let mut filters = Map::new();
    filters.insert(
        "login_required".into(),
        json!(cx.settings.get("login_required")?.truthy()),
    );
    filters.insert("host".into(), json!(cx.urls.config.hostname()));
    let opts = Opts {
        start_date: Some(start),
        end_date: Some(end),
        filters: Some(filters),
        ..Opts::default()
    };
    let rows = |data: &Value| -> Value {
        Value::Array(
            data.as_array()
                .map(|a| a.iter().take(TOP_CARD_LIMIT).cloned().collect())
                .unwrap_or_default(),
        )
    };
    if let Some(cached) = reports::find_cached(conn, cx, viewer, kind, &opts).await? {
        if let Some(err) = cached.get("error").filter(|e| !e.is_null()) {
            return Ok(Some(json!({"rows": [], "error": err})));
        }
        return Ok(Some(json!({"rows": rows(&cached["data"]), "error": null})));
    }
    let Some(report) = reports::find(conn, cx, viewer, kind, &opts).await? else {
        return Ok(Some(json!({"rows": [], "error": "exception"})));
    };
    if report.error != Some("timeout") {
        reports::cache(conn, cx, &report).await?;
    }
    if let Some(err) = report.error {
        return Ok(Some(json!({"rows": [], "error": err})));
    }
    Ok(Some(json!({"rows": rows(&report.data), "error": null})))
}
