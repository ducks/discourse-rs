//! AdminDashboardSiteTrafficExplorer: the beacon's pageview events in a
//! date range, newest first up to site_traffic_explorer_event_limit,
//! filtered by traffic type and dimension, summarized (pageviews,
//! sessions, logged in share, bounce rate, engaged time), as a daily
//! series, and broken down by URL, entry URL, referrer, country, network,
//! browser, language and address. One query, as Rails runs it, under a
//! ten second statement timeout.
//!
//! Country and network labels come from MaxMind (DiscourseIpInfo) when a
//! database is configured; there is none here, so they are the codes, as
//! Rails labels them without one.

use std::net::IpAddr;

use chrono::{Duration, NaiveDate, NaiveDateTime};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::browser_pageviews as bpv;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `DIMENSION_LIMIT`
const DIMENSION_LIMIT: i32 = 50;
/// `STATEMENT_TIMEOUT_MS`
const STATEMENT_TIMEOUT_MS: i64 = 10_000;
/// `BrowserPageviewEvent::MAX_LANGUAGE_LENGTH`
const MAX_LANGUAGE_LENGTH: usize = 255;

/// `FILTER_KEYS`, with each one's dimension (`FILTER_DIMENSIONS`).
pub const FILTER_KEYS: [(&str, &str); 9] = [
    ("traffic_type", ""),
    ("top_url", "top_urls"),
    ("entry_url", "entry_urls"),
    ("referrer", "referrers"),
    ("country", "countries"),
    ("network", "networks"),
    ("browser", "browsers"),
    ("language", "languages"),
    ("ip", "ip_addresses"),
];

/// `BrowserPageviewEvent.browsers`
const BROWSERS: [&str; 13] = [
    "unknown",
    "chrome",
    "edge",
    "safari",
    "firefox",
    "opera",
    "ie",
    "samsung_browser",
    "uc_browser",
    "android_browser",
    "qq_browser",
    "baidu_browser",
    "kaios_browser",
];

/// `TRAFFIC_TYPE_VALUES`
const TRAFFIC_TYPES: [&str; 3] = ["logged_in", "anonymous", "likely_crawler"];

/// Why the request is refused: `Discourse::InvalidParameters`, with the
/// parameter's name or without one.
#[derive(Debug, PartialEq)]
pub struct Invalid(pub Option<&'static str>);

/// A filter value after normalization.
#[derive(Clone, Debug, PartialEq)]
enum Item {
    Text(String),
    Asn(i64),
}

impl Item {
    fn text(&self) -> String {
        match self {
            Item::Text(s) => s.clone(),
            Item::Asn(n) => n.to_string(),
        }
    }
}

/// The request's parameters as the service reads them.
pub struct Params {
    start_date: NaiveDate,
    end_date: NaiveDate,
    filters: Vec<(&'static str, Option<Vec<Item>>)>,
}

/// `Integer(value, exception: false)`: Ruby's integer literal syntax
/// (prefixes, a leading 0 for octal, underscores between digits, spaces
/// around). None for anything else; too large is refused.
fn ruby_integer(s: &str) -> Result<Option<i128>, Unsupported> {
    let t = s.trim_matches(|c: char| c.is_ascii_whitespace() || c == '\u{b}');
    let (neg, t) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let lower = t.to_ascii_lowercase();
    let (radix, digits) = if let Some(r) = lower.strip_prefix("0x") {
        (16, r)
    } else if let Some(r) = lower.strip_prefix("0b") {
        (2, r)
    } else if let Some(r) = lower.strip_prefix("0o") {
        (8, r)
    } else if let Some(r) = lower.strip_prefix("0d") {
        (10, r)
    } else if lower.len() > 1 && lower.starts_with('0') {
        // The leading 0 is a digit: an underscore may follow it.
        let rest = &lower[1..];
        (
            8,
            rest.strip_prefix('_')
                .filter(|r| !r.starts_with('_'))
                .unwrap_or(rest),
        )
    } else {
        (10, lower.as_str())
    };
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
    {
        return Ok(None);
    }
    let mut value: i128 = 0;
    for c in digits.chars().filter(|c| *c != '_') {
        let Some(d) = c.to_digit(radix) else {
            return Ok(None);
        };
        value = value
            .checked_mul(i128::from(radix))
            .and_then(|v| v.checked_add(i128::from(d)))
            .ok_or(Unsupported("a filter value that is a very large integer"))?;
    }
    Ok(Some(if neg { -value } else { value }))
}

/// `IPAddr.new(value).to_s`: the address, masked by a prefix length.
fn ip_to_s(value: &str) -> Result<Option<String>, Unsupported> {
    if value.contains('%') {
        return Err(Unsupported("an IPv6 address with a zone in a filter"));
    }
    let (addr, prefix) = match value.split_once('/') {
        Some((a, p)) => (a, Some(p)),
        None => (value, None),
    };
    let addr = addr
        .strip_prefix('[')
        .and_then(|a| a.strip_suffix(']'))
        .unwrap_or(addr);
    let Ok(ip) = addr.parse::<IpAddr>() else {
        return Ok(None);
    };
    let Some(prefix) = prefix else {
        return Ok(Some(ip.to_string()));
    };
    if !prefix.bytes().all(|b| b.is_ascii_digit()) || prefix.is_empty() {
        return Err(Unsupported("an address with a netmask in a filter"));
    }
    let Ok(bits) = prefix.parse::<u32>() else {
        return Ok(None);
    };
    Ok(match ip {
        IpAddr::V4(v4) if bits <= 32 => {
            let mask = if bits == 0 {
                0
            } else {
                u32::MAX << (32 - bits)
            };
            Some(std::net::Ipv4Addr::from(u32::from(v4) & mask).to_string())
        }
        IpAddr::V6(v6) if bits <= 128 => {
            let mask = if bits == 0 {
                0
            } else {
                u128::MAX << (128 - bits)
            };
            Some(std::net::Ipv6Addr::from(u128::from(v6) & mask).to_string())
        }
        _ => None,
    })
}

/// `normalize_filter_value(key, value)`, the value as the :array type
/// cast it (a string Ruby reads as an integer becomes that integer).
fn normalize_value(key: &'static str, raw: &str) -> Result<Result<Item, Invalid>, Unsupported> {
    let invalid = Ok(Err(Invalid(Some(key))));
    let value = match ruby_integer(raw)? {
        Some(n) => n.to_string(),
        None => raw.to_string(),
    };
    if matches!(key, "referrer" | "language") && value.is_empty() {
        return Ok(Ok(Item::Text(String::new())));
    }
    if crate::ruby::is_blank(&value) {
        return invalid;
    }
    Ok(Ok(match key {
        "traffic_type" => {
            if !TRAFFIC_TYPES.contains(&value.as_str()) {
                return invalid;
            }
            Item::Text(value)
        }
        "top_url" | "entry_url" => match bpv::normalize_site_path(Some(&value)) {
            Some(path) => Item::Text(path),
            None => return invalid,
        },
        "country" => {
            let country = value.to_uppercase();
            if !(country.len() == 2 && country.bytes().all(|b| b.is_ascii_uppercase())) {
                return invalid;
            }
            Item::Text(country)
        }
        "network" => {
            let Some(digits) = value.strip_prefix("AS") else {
                return invalid;
            };
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return invalid;
            }
            let digits = digits.trim_start_matches('0');
            let asn = if digits.is_empty() {
                0
            } else {
                digits
                    .parse::<i64>()
                    .map_err(|_| Unsupported("a network number past 64 bits"))?
            };
            Item::Asn(asn)
        }
        "browser" => {
            if !BROWSERS.contains(&value.as_str()) {
                return invalid;
            }
            Item::Text(value)
        }
        "language" => {
            if value.chars().count() > MAX_LANGUAGE_LENGTH {
                return invalid;
            }
            Item::Text(bpv::normalize_language(Some(&value)).unwrap_or_default())
        }
        "ip" => match ip_to_s(&value)? {
            Some(ip) => Item::Text(ip),
            None => return invalid,
        },
        "referrer" => match bpv::normalize_referrer(Some(&format!("https://{value}")))? {
            Some(r) if !r.is_empty() => Item::Text(r),
            _ => return invalid,
        },
        _ => unreachable!("a filter key"),
    }))
}

/// ActiveModel's date cast: a YYYY-MM-DD date. Other spellings Date._parse
/// reads are refused; text without digits names no date.
fn cast_date(v: Option<&Value>) -> Result<Option<NaiveDate>, Unsupported> {
    let Some(Value::String(s)) = v else {
        return Ok(None);
    };
    let b = s.as_bytes();
    if b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return Ok(NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
    }
    if !s.bytes().any(|c| c.is_ascii_digit()) {
        return Ok(None);
    }
    Err(Unsupported(
        "a traffic explorer date not written YYYY-MM-DD",
    ))
}

/// `traffic_params`, the contract and `params.filters`.
pub fn parse(p: &Map<String, Value>) -> Result<Result<Params, Invalid>, Unsupported> {
    // traffic_params: each filter an array of strings, traffic types
    // split on commas.
    let mut raw: Vec<(&'static str, Option<Vec<String>>)> = Vec::new();
    for (key, _) in FILTER_KEYS {
        let Some(value) = p.get(key) else {
            raw.push((key, None));
            continue;
        };
        let Value::Array(items) = value else {
            return Ok(Err(Invalid(Some(key))));
        };
        let mut strings = Vec::new();
        for item in items {
            let Value::String(s) = item else {
                return Ok(Err(Invalid(Some(key))));
            };
            if key == "traffic_type" {
                // String#split(","): trailing empty fields dropped.
                let mut parts: Vec<String> = s.split(',').map(str::to_string).collect();
                while parts.last().is_some_and(String::is_empty) {
                    parts.pop();
                }
                strings.extend(parts);
            } else {
                strings.push(s.clone());
            }
        }
        raw.push((key, Some(strings)));
    }
    // The contract: both dates, the start not after the end.
    let (Some(start_date), Some(end_date)) = (
        cast_date(p.get("start_date"))?,
        cast_date(p.get("end_date"))?,
    ) else {
        return Ok(Err(Invalid(None)));
    };
    if start_date > end_date {
        return Ok(Err(Invalid(None)));
    }
    // params.filters
    let mut filters = Vec::new();
    for (key, values) in raw {
        let Some(values) = values else {
            filters.push((key, None));
            continue;
        };
        if values.is_empty() || values.len() > DIMENSION_LIMIT as usize {
            return Ok(Err(Invalid(Some(key))));
        }
        let mut items: Vec<Item> = Vec::new();
        for v in &values {
            match normalize_value(key, v)? {
                Ok(item) => {
                    if !items.contains(&item) {
                        items.push(item);
                    }
                }
                Err(e) => return Ok(Err(e)),
            }
        }
        filters.push((key, Some(items)));
    }
    Ok(Ok(Params {
        start_date,
        end_date,
        filters,
    }))
}

/// `filter_predicate`, over the bound parameters.
const FILTER_PREDICATE: &str = "(NOT $5 OR normalized_url = ANY($6::text[]))
          AND (NOT $7 OR entry_url = ANY($8::text[]))
          AND (NOT $9 OR referrer = ANY($10::text[]))
          AND (NOT $11 OR country_code = ANY($12::text[]))
          AND (NOT $13 OR asn = ANY($14::bigint[]))
          AND (NOT $15 OR browser = ANY($16::int[]))
          AND (NOT $17 OR language = ANY($18::text[]))
          AND (NOT $19 OR host(ip_address) = ANY($20::text[]))
          AND (NOT $21
            OR ($22 AND NOT likely_crawler AND user_id IS NOT NULL)
            OR ($23 AND NOT likely_crawler AND user_id IS NULL)
            OR ($24 AND likely_crawler))";

fn dimension_sql(name: &str, value: &str, extra: &str, condition: &str, group: &str) -> String {
    let rep = if extra.is_empty() {
        String::new()
    } else {
        ", 'representative_ip', representative_ip".to_string()
    };
    format!(
        "'{name}', (
            SELECT COALESCE(
              jsonb_agg(
                jsonb_build_object('value', value, 'pageviews', pageviews{rep})
                ORDER BY pageviews DESC, value
              ),
              '[]'::jsonb
            )
            FROM (
              SELECT
                {value} AS value,
                COUNT(*)::integer AS pageviews{extra}
              FROM dimensioned
              WHERE {FILTER_PREDICATE}{condition}
              GROUP BY {group}
              ORDER BY pageviews DESC, value
              LIMIT $28
            ) rows
          )"
    )
}

/// The service's query.
fn query() -> String {
    let rep = ",\n                host(MIN(ip_address)) AS representative_ip";
    let dimensions = [
        dimension_sql(
            "top_urls",
            "normalized_url",
            "",
            "\n                AND normalized_url IS NOT NULL",
            "normalized_url",
        ),
        dimension_sql(
            "entry_urls",
            "entry_url",
            "",
            "\n                AND entry_url IS NOT NULL",
            "entry_url",
        ),
        dimension_sql(
            "referrers",
            "referrer",
            "",
            "\n                AND referrer IS NOT NULL",
            "referrer",
        ),
        dimension_sql(
            "countries",
            "country_code",
            rep,
            "\n                AND country_code IS NOT NULL",
            "country_code",
        ),
        dimension_sql(
            "networks",
            "'AS' || asn",
            rep,
            "\n                AND asn IS NOT NULL",
            "asn",
        ),
        dimension_sql("browsers", "browser", "", "", "browser"),
        dimension_sql("languages", "language", "", "", "language"),
        dimension_sql("ip_addresses", "host(ip_address)", "", "", "ip_address"),
    ]
    .join(",\n          ");
    format!(
        "WITH population AS MATERIALIZED (
        SELECT
          bpe.id, bpe.created_at, bpe.session_id, bpe.user_id, bpe.normalized_url,
          bpe.normalized_referrer, bpe.country_code, bpe.asn, bpe.ip_address,
          COALESCE(bpe.browser, 0) AS browser,
          COALESCE(bpe.normalized_language, '') AS language,
          bpe.score
        FROM browser_pageview_events bpe
        WHERE bpe.created_at >= $1
          AND bpe.created_at < $2
        ORDER BY bpe.created_at DESC, bpe.id DESC
        LIMIT $3
      ),
      population_stats AS MATERIALIZED (
        SELECT COUNT(*) AS row_count, MIN(created_at) AS oldest_created_at
        FROM population
      ),
      population_boundary AS MATERIALIZED (
        SELECT population_stats.row_count, population_stats.oldest_created_at,
          MIN(population.id) AS oldest_id
        FROM population_stats
        LEFT JOIN population ON population.created_at = population_stats.oldest_created_at
        GROUP BY population_stats.row_count, population_stats.oldest_created_at
      ),
      classified AS (
        SELECT
          population.created_at, population.session_id, population.user_id,
          population.normalized_url, population.normalized_referrer, population.country_code,
          population.asn, population.ip_address, population.browser, population.language,
          (
            population.normalized_referrer IS NULL
            OR split_part(split_part(population.normalized_referrer, '/', 1), '?', 1) <> $4
          ) AS acquisition_entry,
          ($25 AND COALESCE(population.score, 0) > $26) AS likely_crawler
        FROM population
      ),
      dimensioned AS MATERIALIZED (
        SELECT
          classified.created_at, classified.session_id, classified.user_id,
          classified.normalized_url, classified.country_code, classified.asn,
          classified.ip_address, classified.browser, classified.language,
          classified.likely_crawler,
          CASE WHEN classified.acquisition_entry THEN classified.normalized_url END AS entry_url,
          CASE WHEN classified.acquisition_entry
            THEN COALESCE(classified.normalized_referrer, '') END AS referrer
        FROM classified
      ),
      filtered AS NOT MATERIALIZED (
        SELECT * FROM dimensioned WHERE {FILTER_PREDICATE}
      ),
      sessions AS (
        SELECT session_id, COUNT(*) AS pageviews FROM filtered GROUP BY session_id
      ),
      session_summary AS (
        SELECT
          COUNT(*)::integer AS distinct_sessions,
          COUNT(*) FILTER (
            WHERE sessions.pageviews = 1
              AND COALESCE(engagement.engaged_seconds, 0) < $27
          )::integer AS bounced_sessions,
          COALESCE(SUM(engagement.engaged_seconds), 0)::bigint AS engaged_seconds
        FROM sessions
        LEFT JOIN browser_pageview_session_engagements engagement
          ON engagement.session_id = sessions.session_id
      ),
      series_rows AS (
        SELECT
          created_at::date AS date,
          COUNT(*)::integer AS pageviews,
          COUNT(*) FILTER (WHERE NOT likely_crawler AND user_id IS NOT NULL)::integer
            AS logged_in_human_pageviews,
          COUNT(*) FILTER (WHERE NOT likely_crawler AND user_id IS NULL)::integer
            AS anonymous_human_pageviews,
          COUNT(*) FILTER (WHERE likely_crawler)::integer AS likely_crawler_pageviews
        FROM filtered
        GROUP BY created_at::date
      ),
      traffic_summary AS (
        SELECT
          COALESCE(SUM(pageviews), 0)::integer AS pageviews,
          COALESCE(SUM(logged_in_human_pageviews), 0)::integer AS logged_in_pageviews
        FROM series_rows
      )
      SELECT
        (
          SELECT population_boundary.row_count = $3
            AND EXISTS (
              SELECT 1 FROM browser_pageview_events bpe
              WHERE bpe.created_at >= $1 AND bpe.created_at < $2
                AND (bpe.created_at, bpe.id) < (
                  population_boundary.oldest_created_at, population_boundary.oldest_id
                )
              LIMIT 1
            )
          FROM population_boundary
        ) AS pageview_limited,
        (SELECT population_boundary.oldest_created_at FROM population_boundary)
          AS oldest_pageview_at,
        jsonb_build_object(
          'country', (
            SELECT COALESCE(jsonb_object_agg(country_code, representative_ip), '{{}}'::jsonb)
            FROM (
              SELECT country_code, host(MIN(ip_address)) AS representative_ip
              FROM population
              WHERE $11 AND country_code = ANY($12::text[])
              GROUP BY country_code
            ) rows
          ),
          'network', (
            SELECT COALESCE(jsonb_object_agg(asn, representative_ip), '{{}}'::jsonb)
            FROM (
              SELECT asn, host(MIN(ip_address)) AS representative_ip
              FROM population
              WHERE $13 AND asn = ANY($14::bigint[])
              GROUP BY asn
            ) rows
          )
        )::text AS active_filter_representative_ips,
        jsonb_build_object(
          'pageviews', traffic_summary.pageviews,
          'distinct_sessions', session_summary.distinct_sessions,
          'logged_in_share', CASE
            WHEN traffic_summary.pageviews = 0 THEN 0
            ELSE ROUND(traffic_summary.logged_in_pageviews::numeric * 100 /
              traffic_summary.pageviews)::integer
          END,
          'bounce_rate', CASE
            WHEN session_summary.distinct_sessions = 0 THEN 0
            ELSE ROUND(session_summary.bounced_sessions::numeric * 100 /
              session_summary.distinct_sessions)::integer
          END,
          'average_session_duration_seconds', CASE
            WHEN session_summary.distinct_sessions = 0 THEN 0
            ELSE session_summary.engaged_seconds::numeric / session_summary.distinct_sessions
          END
        )::text AS summary,
        COALESCE(
          (
            SELECT jsonb_agg(
              jsonb_build_object(
                'date', date,
                'pageviews', pageviews,
                'logged_in_human_pageviews', logged_in_human_pageviews,
                'anonymous_human_pageviews', anonymous_human_pageviews,
                'likely_crawler_pageviews', likely_crawler_pageviews
              )
              ORDER BY date
            )
            FROM series_rows
          ),
          '[]'::jsonb
        )::text AS series,
        jsonb_build_object(
          {dimensions}
        )::text AS dimensions
      FROM traffic_summary
      CROSS JOIN session_summary"
    )
}

/// A jsonb value as Ruby reads it back and renders it: numbers with a
/// fraction are Floats, written to 16 significant digits.
fn ruby_json(text: &str) -> Result<Value, AppError> {
    fn walk(v: Value) -> Value {
        match v {
            Value::Number(n) if !(n.is_i64() || n.is_u64()) => {
                let f = n.as_f64().unwrap_or(0.0);
                let rounded: f64 = format!("{f:.15e}").parse().unwrap_or(f);
                crate::reports::float_json(rounded)
            }
            Value::Array(a) => Value::Array(a.into_iter().map(walk).collect()),
            Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, walk(v))).collect()),
            other => other,
        }
    }
    Ok(walk(serde_json::from_str(text)?))
}

/// What the service reads besides the database.
pub struct Cx<'a> {
    pub settings: &'a SiteSettings,
    pub i18n: &'a I18n,
    /// `Discourse.current_hostname`
    pub hostname: String,
}

impl Cx<'_> {
    fn t(&self, key: &str) -> String {
        self.i18n
            .t(key)
            .map(str::to_string)
            .unwrap_or_else(|| format!("Translation missing: en.{key}"))
    }

    /// `dimension_label(dimension, value, representative_ip)`, without
    /// MaxMind for countries and networks.
    fn label(&self, dimension: &str, value: &str) -> String {
        match dimension {
            "referrers" if value.is_empty() => {
                self.t("admin_site_traffic_explorer.direct_or_unknown")
            }
            "browsers" => self
                .i18n
                .t(&format!("browsers.{value}"))
                .map_or_else(|| value.to_string(), str::to_string),
            "languages" if value.is_empty() => self.t("admin_site_traffic_explorer.unknown"),
            _ => value.to_string(),
        }
    }
}

/// The traffic, or `traffic_query_timeout` when the query ran too long.
pub enum Outcome {
    Traffic(Value),
    Timeout,
}

#[derive(sqlx::FromRow)]
struct Row {
    pageview_limited: Option<bool>,
    oldest_pageview_at: Option<NaiveDateTime>,
    summary: String,
    series: String,
    dimensions: String,
}

/// `load_traffic`
pub async fn load(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    params: &Params,
) -> Result<Outcome, AppError> {
    let cutoff = bpv::retention_cutoff().date();
    let effective_start = params.start_date.max(cutoff);
    let cap = cx.settings.get("site_traffic_explorer_event_limit")?.to_i();
    let site_host = bpv::normalize_host(Some(&cx.hostname))?;
    let filter = |key: &str| {
        params
            .filters
            .iter()
            .find(|(k, _)| *k == key)
            .and_then(|(_, v)| v.as_ref())
    };
    let texts = |key: &str| -> Vec<Option<String>> {
        match filter(key) {
            Some(items) => items.iter().map(|i| Some(i.text())).collect(),
            None => vec![None],
        }
    };
    let asns: Vec<Option<i64>> = match filter("network") {
        Some(items) => items
            .iter()
            .map(|i| match i {
                Item::Asn(n) => Some(*n),
                Item::Text(_) => None,
            })
            .collect(),
        None => vec![None],
    };
    let browsers: Vec<Option<i32>> = match filter("browser") {
        Some(items) => items
            .iter()
            .map(|i| {
                BROWSERS
                    .iter()
                    .position(|b| *b == i.text())
                    .map(|p| p as i32)
            })
            .collect(),
        None => vec![None],
    };
    let types = filter("traffic_type").map(|items| {
        let has = |t: &str| items.iter().any(|i| i.text() == t);
        (has("logged_in"), has("anonymous"), has("likely_crawler"))
    });
    let crawler_detection = crate::reports::crawler_scorer_enabled(cx.settings)?;

    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    sqlx::query(&format!(
        "SET LOCAL statement_timeout = {STATEMENT_TIMEOUT_MS}"
    ))
    .execute(&mut *tx)
    .await?;
    let sql = query();
    let result = sqlx::query_as::<_, Row>(&sql)
        .bind(effective_start)
        .bind(params.end_date + Duration::days(1))
        .bind(cap)
        .bind(site_host)
        .bind(filter("top_url").is_some())
        .bind(texts("top_url"))
        .bind(filter("entry_url").is_some())
        .bind(texts("entry_url"))
        .bind(filter("referrer").is_some())
        .bind(texts("referrer"))
        .bind(filter("country").is_some())
        .bind(texts("country"))
        .bind(filter("network").is_some())
        .bind(asns)
        .bind(filter("browser").is_some())
        .bind(browsers)
        .bind(filter("language").is_some())
        .bind(texts("language"))
        .bind(filter("ip").is_some())
        .bind(texts("ip"))
        .bind(types.is_some())
        .bind(types.map(|t| t.0))
        .bind(types.map(|t| t.1))
        .bind(types.map(|t| t.2))
        .bind(crawler_detection)
        .bind(bpv::BOT_SCORE_THRESHOLD)
        .bind(bpv::BOUNCE_ENGAGED_SECONDS)
        .bind(DIMENSION_LIMIT)
        .fetch_one(&mut *tx)
        .await;
    let row = match result {
        Ok(row) => row,
        Err(e) => {
            let e = AppError::from(e);
            if e.sql_state().as_deref() == Some("57014") {
                return Ok(Outcome::Timeout);
            }
            return Err(e);
        }
    };
    tx.commit().await?;

    let mut traffic = Map::new();
    traffic.insert(
        "partial_data".into(),
        partial_data(
            cx,
            row.pageview_limited.unwrap_or(false),
            row.oldest_pageview_at,
            params.start_date,
            cutoff,
        )?,
    );
    traffic.insert("summary".into(), ruby_json(&row.summary)?);
    traffic.insert("series".into(), ruby_json(&row.series)?);
    traffic.insert("series_colors".into(), series_colors());
    traffic.insert(
        "dimensions".into(),
        decorate_dimensions(cx, ruby_json(&row.dimensions)?),
    );
    let active = active_filters(cx, params);
    if !active.is_empty() {
        traffic.insert("active_filters".into(), Value::Array(active));
    }
    Ok(Outcome::Traffic(Value::Object(traffic)))
}

/// `partial_data`: what the range leaves out, the retention period or
/// the event limit.
fn partial_data(
    cx: &Cx<'_>,
    pageview_limited: bool,
    limit_start_at: Option<NaiveDateTime>,
    start_date: NaiveDate,
    cutoff: NaiveDate,
) -> Result<Value, AppError> {
    let retention_limited = start_date < cutoff;
    if !retention_limited && !pageview_limited {
        return Ok(Value::Null);
    }
    let limit = cx.settings.get("site_traffic_explorer_event_limit")?.to_i();
    let at = || {
        limit_start_at
            .map(|t| Value::String(t.format("%Y-%m-%dT%H:%M:%SZ").to_string()))
            .unwrap_or(Value::Null)
    };
    let available = cutoff.format("%Y-%m-%d").to_string();
    Ok(if retention_limited && pageview_limited {
        json!({
            "reason": "retention_and_pageview_limit",
            "available_start_date": available,
            "pageview_limit": limit,
            "pageview_limit_start_at": at(),
        })
    } else if retention_limited {
        json!({"reason": "retention", "available_start_date": available})
    } else {
        json!({
            "reason": "pageview_limit",
            "pageview_limit": limit,
            "pageview_limit_start_at": at(),
        })
    })
}

/// `series_colors`, from Reports::SiteTraffic::SERIES_COLORS.
fn series_colors() -> Value {
    json!({
        "logged_in_human_pageviews": "#4B3CE0",
        "anonymous_human_pageviews": "#9C8DEC",
        "likely_crawler_pageviews": "#B3AAC9",
    })
}

/// `decorate_dimensions`: each row's value, label and pageviews, a
/// browser by name.
fn decorate_dimensions(cx: &Cx<'_>, dimensions: Value) -> Value {
    let Value::Object(dimensions) = dimensions else {
        return dimensions;
    };
    let out: Map<String, Value> = dimensions
        .into_iter()
        .map(|(dimension, rows)| {
            let rows = rows
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .map(|row| {
                            let value = if dimension == "browsers" {
                                let id = row["value"].as_i64().unwrap_or(-1);
                                usize::try_from(id)
                                    .ok()
                                    .and_then(|i| BROWSERS.get(i))
                                    .copied()
                                    .unwrap_or("unknown")
                                    .to_string()
                            } else {
                                match &row["value"] {
                                    Value::String(s) => s.clone(),
                                    other => other.to_string(),
                                }
                            };
                            json!({
                                "value": value,
                                "label": cx.label(&dimension, &value),
                                "pageviews": row["pageviews"],
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            (dimension, Value::Array(rows))
        })
        .collect();
    Value::Object(out)
}

/// `decorate_active_filters`
fn active_filters(cx: &Cx<'_>, params: &Params) -> Vec<Value> {
    let mut out = Vec::new();
    for (key, items) in &params.filters {
        let Some(items) = items else { continue };
        let dimension = FILTER_KEYS
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, d)| *d)
            .unwrap_or_default();
        for item in items {
            if *key == "traffic_type" {
                let t = item.text();
                out.push(json!({
                    "key": key,
                    "value": t,
                    "label": cx.t(&format!("admin_site_traffic_explorer.traffic_types.{t}")),
                }));
                continue;
            }
            let value = match item {
                Item::Asn(n) => format!("AS{n}"),
                Item::Text(s) => s.clone(),
            };
            out.push(json!({
                "key": key,
                "value": value,
                "label": cx.label(dimension, &value),
            }));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_as_ruby_reads_them() {
        assert_eq!(ruby_integer("42").unwrap(), Some(42));
        assert_eq!(ruby_integer(" -0x1A ").unwrap(), Some(-26));
        assert_eq!(ruby_integer("007").unwrap(), Some(7));
        assert_eq!(ruby_integer("08").unwrap(), None);
        assert_eq!(ruby_integer("1_000").unwrap(), Some(1000));
        assert_eq!(ruby_integer("1__0").unwrap(), None);
        assert_eq!(ruby_integer("0b101").unwrap(), Some(5));
        assert_eq!(ruby_integer("").unwrap(), None);
        assert_eq!(ruby_integer("1e5").unwrap(), None);
        assert_eq!(ruby_integer("0").unwrap(), Some(0));
        assert_eq!(ruby_integer("0_7").unwrap(), Some(7));
        assert_eq!(ruby_integer("+5").unwrap(), Some(5));
        assert_eq!(ruby_integer("-007").unwrap(), Some(-7));
        for none in [
            "0x", "- 5", "0_", "0__7", "0x_1", "0_x1", "1_", "_1", "0o_7",
        ] {
            assert_eq!(ruby_integer(none).unwrap(), None, "{none}");
        }
        assert_eq!(ruby_integer("00").unwrap(), Some(0));
    }

    #[test]
    fn addresses_as_ipaddr_writes_them() {
        assert_eq!(
            ip_to_s("2001:0db8::1").unwrap().as_deref(),
            Some("2001:db8::1")
        );
        assert_eq!(ip_to_s("1.2.3.4/24").unwrap().as_deref(), Some("1.2.3.0"));
        assert_eq!(ip_to_s("[::1]").unwrap().as_deref(), Some("::1"));
        assert_eq!(ip_to_s("nope").unwrap(), None);
        assert_eq!(ip_to_s("01.2.3.4").unwrap(), None);
    }

    #[test]
    fn floats_as_ruby_renders_jsonb_numbers() {
        assert_eq!(
            ruby_json(r#"{"a": 11.6666666666666667, "b": 40.0000000000000000, "c": 0}"#)
                .unwrap()
                .to_string(),
            r#"{"a":11.66666666666667,"b":40.0,"c":0}"#
        );
    }
}
