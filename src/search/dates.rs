//! `Search.word_to_date`, for the `before:` and `after:` filters.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, Utc};

const DAYS: [&str; 7] = [
    "sunday",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
];

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// In UTC (Rails' Time.zone): days ago, a date (`2024`, `2024-3`,
/// `2024-03-05`; a day past the month's end rolls over, as Time.new
/// does), `yesterday`, a weekday (its start this week) or a month (its
/// start, this year or last).
pub fn word_to_date(word: &str, now: DateTime<Utc>) -> Option<NaiveDateTime> {
    let today = now.date_naive();
    let midnight = |d: NaiveDate| d.and_hms_opt(0, 0, 0);
    if (1..=3).contains(&word.len()) && word.bytes().all(|b| b.is_ascii_digit()) {
        let days: i64 = word.parse().ok()?;
        return midnight(today - Duration::days(days));
    }
    static DATE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\A([12][0-9]{3})(-([0-1]?[0-9]))?(-([0-3]?[0-9]))?\z")
            .expect("static pattern")
    });
    if let Some(c) = DATE.captures(word) {
        let year: i32 = c[1].parse().ok()?;
        let month: u32 = c.get(3).map_or(Some(1), |m| m.as_str().parse().ok())?;
        let day: u32 = c.get(5).map_or(Some(1), |m| m.as_str().parse().ok())?;
        if day == 0 || month == 0 || day > 31 || month > 12 {
            return None;
        }
        let first = NaiveDate::from_ymd_opt(year, month, 1)?;
        return midnight(first + Duration::days(i64::from(day) - 1));
    }
    let lower = word.to_lowercase();
    if lower == "yesterday" {
        return midnight(today - Duration::days(1));
    }
    if let Some(start) = DAYS.iter().position(|d| *d == lower) {
        let wday = i64::from(today.weekday().num_days_from_sunday());
        let back = (wday - start as i64).rem_euclid(7);
        return midnight(today - Duration::days(back));
    }
    let idx = MONTHS.iter().position(|m| *m == lower || m[..3] == lower)? as i32 + 1;
    let mut delta = today.month() as i32 - idx;
    if delta < 0 {
        delta += 12;
    }
    let mut year = today.year();
    let mut month = today.month() as i32 - delta;
    if month < 1 {
        month += 12;
        year -= 1;
    }
    midnight(NaiveDate::from_ymd_opt(year, month as u32, 1)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn words_become_dates_as_ruby_reads_them() {
        // A Thursday.
        let now = Utc.with_ymd_and_hms(2026, 10, 15, 13, 5, 0).unwrap();
        let d = |w: &str| word_to_date(w, now).map(|t| t.to_string());
        assert_eq!(d("3").as_deref(), Some("2026-10-12 00:00:00"));
        assert_eq!(d("2024").as_deref(), Some("2024-01-01 00:00:00"));
        assert_eq!(d("1234").as_deref(), Some("1234-01-01 00:00:00"));
        assert_eq!(d("2024-3").as_deref(), Some("2024-03-01 00:00:00"));
        assert_eq!(d("2023-02-31").as_deref(), Some("2023-03-03 00:00:00"));
        assert_eq!(d("2024-13"), None);
        assert_eq!(d("Yesterday").as_deref(), Some("2026-10-14 00:00:00"));
        assert_eq!(d("monday").as_deref(), Some("2026-10-12 00:00:00"));
        assert_eq!(d("thursday").as_deref(), Some("2026-10-15 00:00:00"));
        assert_eq!(d("friday").as_deref(), Some("2026-10-09 00:00:00"));
        assert_eq!(d("dec").as_deref(), Some("2025-12-01 00:00:00"));
        assert_eq!(d("October").as_deref(), Some("2026-10-01 00:00:00"));
        assert_eq!(d("sept"), None);
        assert_eq!(d("soon"), None);
    }
}
