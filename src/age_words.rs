//! `AgeWords` (lib/age_words.rb): Discourse's fork of Rails'
//! `distance_of_time_in_words`, which never says "about 1 month" and
//! counts days up to 90.

use chrono::{Datelike, NaiveDate, NaiveDateTime};

use crate::i18n::I18n;

/// A key of the scope, with its count where the translation has plural
/// forms; the verbose scope's "just now" has none.
fn t(i18n: &I18n, scope: &str, key: &str, count: i64) -> String {
    let full = format!("{scope}.{key}");
    i18n.t_count(&full, count, &[])
        .or_else(|| i18n.t(&full).map(str::to_string))
        .unwrap_or(full)
}

/// `AgeWords.distance_of_time_in_words(from, to, include_seconds, scope:)`
pub fn distance_of_time_in_words(
    i18n: &I18n,
    scope: &str,
    from: NaiveDateTime,
    to: NaiveDateTime,
    include_seconds: bool,
) -> String {
    let distance = ((to - from).num_microseconds().unwrap_or(i64::MAX) as f64 / 1e6).abs();
    let minutes = (distance / 60.0).round() as i64;
    let seconds = distance.round() as i64;
    let t = |key: &str, count: i64| t(i18n, scope, key, count);
    match minutes {
        0..=1 => {
            if !include_seconds {
                return if minutes == 0 {
                    t("less_than_x_minutes", 1)
                } else {
                    t("x_minutes", minutes)
                };
            }
            match seconds {
                0..=4 => t("less_than_x_seconds", 5),
                5..=9 => t("less_than_x_seconds", 10),
                10..=19 => t("less_than_x_seconds", 20),
                20..=39 => t("half_a_minute", 1),
                40..=59 => t("less_than_x_minutes", 1),
                _ => t("x_minutes", 1),
            }
        }
        2..=44 => t("x_minutes", minutes),
        45..=89 => t("about_x_hours", 1),
        90..=1439 => t("about_x_hours", (minutes as f64 / 60.0).round() as i64),
        1440..=2519 => t("x_days", 1),
        2520..=129_599 => t("x_days", (minutes as f64 / 1440.0).round() as i64),
        129_600..=525_599 => t("x_months", (minutes as f64 / 43200.0).round() as i64),
        _ => {
            let mut fyear = from.year();
            if from.month() >= 3 {
                fyear += 1;
            }
            let mut tyear = to.year();
            if to.month() < 3 {
                tyear -= 1;
            }
            let leap_years = if fyear > tyear {
                0
            } else {
                (fyear..=tyear)
                    .filter(|y| NaiveDate::from_ymd_opt(*y, 2, 29).is_some())
                    .count() as i64
            };
            let with_offset = minutes - leap_years * 1440;
            let remainder = with_offset.rem_euclid(525_600);
            let years = with_offset.div_euclid(525_600);
            if remainder < 131_400 {
                t("about_x_years", years)
            } else if remainder < 394_200 {
                t("over_x_years", years)
            } else {
                t("almost_x_years", years + 1)
            }
        }
    }
}

/// `AgeWords.time_ago_in_words(from, true, scope:
/// :"datetime.distance_in_words_verbose")`, as the penalty policies say
/// how long ago a user was suspended or silenced.
pub fn time_ago_verbose(i18n: &I18n, from: NaiveDateTime, now: NaiveDateTime) -> String {
    distance_of_time_in_words(i18n, "datetime.distance_in_words_verbose", from, now, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn ago(d: Duration) -> String {
        let i18n = I18n::vendored().unwrap();
        let now = NaiveDate::from_ymd_opt(2026, 10, 14)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();
        time_ago_verbose(&i18n, now - d, now)
    }

    #[test]
    fn verbose_boundaries() {
        assert_eq!(ago(Duration::seconds(3)), "just now");
        assert_eq!(ago(Duration::seconds(25)), "just now");
        assert_eq!(ago(Duration::seconds(45)), "less than 1 minute ago");
        assert_eq!(ago(Duration::seconds(60)), "1 minute ago");
        assert_eq!(ago(Duration::seconds(89)), "1 minute ago");
        assert_eq!(ago(Duration::minutes(2)), "2 minutes ago");
        assert_eq!(ago(Duration::minutes(44)), "44 minutes ago");
        assert_eq!(ago(Duration::minutes(50)), "1 hour ago");
        assert_eq!(ago(Duration::minutes(90)), "2 hours ago");
        assert_eq!(ago(Duration::hours(23)), "23 hours ago");
        assert_eq!(ago(Duration::hours(24)), "1 day ago");
        assert_eq!(ago(Duration::days(3)), "3 days ago");
        assert_eq!(ago(Duration::days(89)), "89 days ago");
        assert_eq!(ago(Duration::days(90)), "3 months ago");
        assert_eq!(ago(Duration::days(364)), "12 months ago");
        assert_eq!(ago(Duration::days(366)), "about 1 year ago");
        assert_eq!(ago(Duration::days(365 + 200)), "over 1 year ago");
        assert_eq!(ago(Duration::days(365 + 300)), "almost 2 years ago");
    }
}
