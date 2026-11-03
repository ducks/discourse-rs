//! Admin::SearchLogsController#index: `SearchLog.trending(period,
//! search_type)` through SearchLogsSerializer.
//!
//! And #term: `SearchLog.term_details` with the term's search results.
//!
//! Refused: `human_only` with the improved_crawler_detection
//! upcoming change on (CrawlerScorer's scores).

use chrono::{Datelike, Months, NaiveDate};
use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `SearchLog.start_of(period)`: a date, the window's start.
fn start_of(period: &str, today: NaiveDate) -> NaiveDate {
    match period {
        "yearly" => today.checked_sub_months(Months::new(12)),
        "monthly" => today.checked_sub_months(Months::new(1)),
        "quarterly" => today.checked_sub_months(Months::new(3)),
        "weekly" => Some(today - chrono::Duration::days(7)),
        "daily" => Some(today),
        _ => today.with_year(today.year() - 1000),
    }
    .unwrap_or(today)
}

/// `SearchLog.search_types`
fn search_type_id(name: &str) -> Option<i32> {
    match name {
        "header" => Some(1),
        "full_page" => Some(2),
        _ => None,
    }
}

/// `SearchLog#ctr`: the click-through percentage rounded up to a tenth,
/// an integer 0 when nothing was clicked.
fn ctr(click_through: i64, searches: i64) -> Value {
    if click_through == 0 || searches == 0 {
        return json!(0);
    }
    let pct = click_through as f64 / searches as f64 * 100.0;
    json!((pct * 10.0).ceil() / 10.0)
}

/// The `non_staff` scope (an inner join on users: no anonymous rows).
const NON_STAFF: &str = "AND search_logs.user_id IN \
     (SELECT id FROM users WHERE NOT admin AND NOT moderator)";

/// The `human_only` scope: `non_staff` while improved_crawler_detection
/// is off (CrawlerScorer.enabled?); its crawler scoring isn't ported.
fn human_only(settings: &SiteSettings) -> Result<&'static str, AppError> {
    if settings
        .get("improved_crawler_detection")
        .map(|v| v.truthy())
        .unwrap_or(false)
    {
        return Err(Unsupported("human_only search logs (improved crawler detection)").into());
    }
    Ok(NON_STAFF)
}

pub async fn trending(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    period: &str,
    search_type: &str,
) -> Result<Value, AppError> {
    let start = start_of(period, crate::clock::now_naive().date());
    let filter = match search_type {
        "all" => String::new(),
        "non_staff_only" => NON_STAFF.into(),
        "human_only" => human_only(settings)?.into(),
        // `where("search_type = ?", nil)` for anything else: none.
        other => match search_type_id(other) {
            Some(id) => format!("AND search_type = {id}"),
            None => "AND FALSE".into(),
        },
    };
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(&format!(
        "SELECT lower(term) AS term, COUNT(*) AS searches, \
                SUM(CASE WHEN search_result_id IS NOT NULL THEN 1 ELSE 0 END)::int8 AS click_through \
         FROM search_logs WHERE search_logs.created_at > $1 {filter} \
         GROUP BY lower(term) ORDER BY searches DESC, click_through DESC, term ASC LIMIT 100"
    ))
    .bind(start)
    .fetch_all(&mut *conn)
    .await?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|(term, searches, clicks)| {
                json!({ "term": term, "searches": searches, "ctr": ctr(clicks, searches) })
            })
            .collect(),
    ))
}

/// `Time#to_date` as Ruby writes the date: before the Gregorian reform
/// (1582-10-15) Date counts in the Julian calendar (`1000.years.ago`
/// becomes a date six days earlier than the proleptic one).
fn ruby_date(date: NaiveDate) -> String {
    let reform = NaiveDate::from_ymd_opt(1582, 10, 15).expect("a date");
    if date >= reform {
        return date.to_string();
    }
    // The Julian day number, then the Julian calendar's date for it.
    let jdn = i64::from(date.num_days_from_ce()) + 1_721_425;
    let c = jdn + 32082;
    let d = (4 * c + 3) / 1461;
    let e = c - 1461 * d / 4;
    let m = (5 * e + 2) / 153;
    let day = e - (153 * m + 2) / 5 + 1;
    let month = m + 3 - 12 * (m / 10);
    let year = d - 4800 + m / 10;
    format!("{year:04}-{month:02}-{day:02}")
}

/// `SearchLog.term_details(term, period, search_type)`: the searches
/// for the term a day, since the period's start (no filter for a search
/// type it doesn't know).
pub async fn term_details(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    term: &str,
    period: &str,
    search_type: &str,
) -> Result<serde_json::Map<String, Value>, AppError> {
    let now = crate::clock::now();
    let start = start_of(period, now.date_naive());
    let filter = match search_type {
        "header" => "AND search_type = 1",
        "full_page" => "AND search_type = 2",
        "click_through_only" => "AND search_result_id IS NOT NULL",
        "non_staff_only" => NON_STAFF,
        "human_only" => human_only(settings)?,
        _ => "",
    };
    let rows: Vec<(i64, NaiveDate)> = sqlx::query_as(&format!(
        "SELECT COUNT(*) AS count, search_logs.created_at::date AS date FROM search_logs \
         WHERE lower(search_logs.term) = $1 AND search_logs.created_at > $2 {filter} \
         GROUP BY search_logs.created_at::date ORDER BY date"
    ))
    .bind(term.to_lowercase())
    .bind(start)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = serde_json::Map::new();
    out.insert("type".into(), json!("search_log_term"));
    out.insert(
        "title".into(),
        json!(i18n.t("search_logs.graph_title").unwrap_or("Search Count")),
    );
    out.insert("start_date".into(), json!(ruby_date(start)));
    out.insert(
        "end_date".into(),
        json!(now.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()),
    );
    out.insert(
        "data".into(),
        Value::Array(
            rows.into_iter()
                .map(|(count, date)| json!({ "x": date.to_string(), "y": count }))
                .collect(),
        ),
    );
    out.insert("period".into(), json!(period));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_before_the_reform_are_julian() {
        let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
        assert_eq!(ruby_date(d(1026, 10, 14)), "1026-10-08");
        assert_eq!(ruby_date(d(1582, 10, 15)), "1582-10-15");
        assert_eq!(ruby_date(d(1582, 10, 14)), "1582-10-04");
        assert_eq!(ruby_date(d(2026, 7, 14)), "2026-07-14");
    }

    #[test]
    fn click_through_rates() {
        assert_eq!(ctr(1, 3), json!(33.4));
        assert_eq!(ctr(2, 3), json!(66.7));
        assert_eq!(ctr(1, 1), json!(100.0));
        assert_eq!(ctr(0, 4), json!(0));
    }
}
