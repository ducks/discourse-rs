//! Admin::SearchLogsController#index: `SearchLog.trending(period,
//! search_type)` through SearchLogsSerializer.
//!
//! Refused: `human_only`, which depends on the improved_crawler_detection
//! upcoming change.

use chrono::{Datelike, Months, NaiveDate};
use serde_json::{Value, json};
use sqlx::PgConnection;

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

pub async fn trending(
    conn: &mut PgConnection,
    period: &str,
    search_type: &str,
) -> Result<Value, AppError> {
    let start = start_of(period, crate::clock::now_naive().date());
    let filter = match search_type {
        "all" => String::new(),
        "non_staff_only" => "AND search_logs.user_id IN \
             (SELECT id FROM users WHERE NOT admin AND NOT moderator)"
            .into(),
        "human_only" => {
            return Err(Unsupported("human_only search logs (improved crawler detection)").into());
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_through_rates() {
        assert_eq!(ctr(1, 3), json!(33.4));
        assert_eq!(ctr(2, 3), json!(66.7));
        assert_eq!(ctr(1, 1), json!(100.0));
        assert_eq!(ctr(0, 4), json!(0));
    }
}
