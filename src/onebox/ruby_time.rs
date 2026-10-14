//! `Time.parse` (lib/time.rb over `Date._parse`, crate::ruby_date) for
//! the dates onebox pages give (published times, a movie's duration), and
//! the `strftime` formats the engines print. Only the year, month, day,
//! hour and minute are printed and Time.parse keeps the fields it read
//! whatever the zone, so the zone is not read. What depends on today's
//! date or on how Ruby rolls fields over is refused rather than guessed.

use crate::Unsupported;

/// What `Time.parse` came to.
#[derive(Debug, PartialEq)]
pub enum Parsed {
    Time(Fields),
    /// ArgumentError (out of range, no time information).
    Raised,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fields {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: i64,
    pub minute: i64,
    pub second: i64,
    /// What `%Z` prints: "UTC" for UTC and for the server's local time
    /// (which is UTC); None for an offset (not ported).
    pub zone: Option<&'static str>,
}

/// What `%Z` prints for a time Time.parse read with this zone.
fn zone_name(zone: Option<&str>) -> Option<&'static str> {
    static UTC: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)\A(?:[-+]00:?00|[-+]00|UTC|Z|UT|GMT)\z").unwrap()
    });
    match zone {
        None => Some("UTC"),
        Some(z) if UTC.is_match(z) => Some("UTC"),
        Some(_) => None,
    }
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        _ => 28,
    }
}

/// `Date._parse(s)`, refusing what isn't ASCII and raising past the
/// 128 character limit.
fn date_parse(s: &str) -> Result<Option<crate::ruby_date::DateHash>, Unsupported> {
    if !s.is_ascii() {
        return Err(Unsupported("dates that aren't ASCII"));
    }
    if s.len() > 128 {
        return Ok(None);
    }
    let h = crate::ruby_date::parse(s);
    let none = h.year.is_none()
        && h.yday.is_none()
        && h.mon.is_none()
        && h.mday.is_none()
        && h.hour.is_none()
        && h.min.is_none()
        && h.sec.is_none()
        && !h.sec_fraction;
    if none {
        return Ok(None);
    }
    if h.yday.is_some() {
        return Err(Unsupported("dates given as a day of the year"));
    }
    Ok(Some(h))
}

/// `Time.parse(s)` as a published time is read: its date must be its own
/// (not today's) and its fields in range.
pub fn parse(s: &str) -> Result<Parsed, Unsupported> {
    let Some(h) = date_parse(s)? else {
        return Ok(Parsed::Raised);
    };
    let Some(mut year) = h.year else {
        return Err(Unsupported(
            "dates without a year (Time.parse takes today's)",
        ));
    };
    let mut month = h.mon.unwrap_or(1);
    let mut day = h.mday.unwrap_or(1);
    let mut hour = h.hour.unwrap_or(0);
    let minute = h.min.unwrap_or(0);
    let sec = h.sec.unwrap_or(0);
    if !(1..=12).contains(&month) {
        return Ok(Parsed::Raised);
    }
    if !(0..=9999).contains(&year) {
        return Err(Unsupported("dates past the years strftime pads"));
    }
    let valid = (1..=days_in_month(year, month)).contains(&day)
        && (0..=23).contains(&hour)
        && (0..=59).contains(&minute)
        && (0..=59).contains(&sec);
    if !valid {
        // With a zone the offset is applied field by field before Ruby
        // normalizes; that isn't ported.
        if h.zone.is_some() {
            return Err(Unsupported("dates with a zone Time.parse rolls over"));
        }
        // Time.local's ranges.
        if !(1..=31).contains(&day)
            || hour > 24
            || (hour == 24 && (minute > 0 || sec > 0))
            || minute > 59
            || sec > 60
        {
            return Ok(Parsed::Raised);
        }
        if sec == 60 {
            return Err(Unsupported("leap seconds"));
        }
        // A day past the month's end and hour 24 roll forward.
        if hour == 24 {
            hour = 0;
            day += 1;
        }
        let days = days_in_month(year, month);
        if day > days {
            day -= days;
            month += 1;
        }
        if month > 12 {
            month = 1;
            year += 1;
        }
    }
    Ok(Parsed::Time(Fields {
        year,
        month,
        day,
        hour,
        minute,
        second: sec,
        zone: zone_name(h.zone.as_deref()),
    }))
}

/// A movie's duration as `Time.parse(duration).strftime("%H:%M")` reads
/// it: the date (today's where it names none) doesn't show.
pub fn duration(s: &str) -> Result<Parsed, Unsupported> {
    let Some(h) = date_parse(s)? else {
        return Ok(Parsed::Raised);
    };
    let hour = h.hour.unwrap_or(0);
    let minute = h.min.unwrap_or(0);
    let sec = h.sec.unwrap_or(0);
    if h.mon.is_some_and(|m| !(1..=12).contains(&m))
        || h.mday.is_some_and(|d| !(1..=31).contains(&d))
    {
        return Ok(Parsed::Raised);
    }
    if h.year.is_some_and(|y| !(0..=9999).contains(&y))
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&sec)
    {
        return Err(Unsupported("movie durations Time.parse rolls over"));
    }
    Ok(Parsed::Time(Fields {
        year: h.year.unwrap_or(0),
        month: h.mon.unwrap_or(1),
        day: h.mday.unwrap_or(1),
        hour,
        minute,
        second: sec,
        zone: None,
    }))
}

const MONTH_ABBR: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTH_NAME: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

impl Fields {
    /// `strftime("%-d %b %y")`
    pub fn short(&self) -> String {
        format!(
            "{} {} {:02}",
            self.day,
            MONTH_ABBR[self.month as usize - 1],
            self.year.rem_euclid(100)
        )
    }

    /// `strftime("%I:%M%p - %d %B %Y")`
    pub fn long(&self) -> String {
        let hour12 = match self.hour % 12 {
            0 => 12,
            h => h,
        };
        format!(
            "{:02}:{:02}{} - {:02} {} {:04}",
            hour12,
            self.minute,
            if self.hour < 12 { "AM" } else { "PM" },
            self.day,
            MONTH_NAME[self.month as usize - 1],
            self.year
        )
    }

    /// `strftime("%H:%M")`
    pub fn hours_minutes(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    /// `strftime(format)` for the directives the engines use; refused for
    /// `%Z` where the zone's name isn't known.
    pub fn strftime(&self, format: &str) -> Result<String, Unsupported> {
        let hour12 = match self.hour % 12 {
            0 => 12,
            h => h,
        };
        let mut out = String::new();
        let mut chars = format.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            let mut d = chars.next().unwrap_or('%');
            let unpadded = d == '-';
            if unpadded {
                d = chars.next().unwrap_or('%');
            }
            let num = |n: i64, width: usize| {
                if unpadded {
                    n.to_string()
                } else {
                    format!("{n:0width$}")
                }
            };
            match d {
                'Y' => out.push_str(&num(self.year, 4)),
                'y' => out.push_str(&num(self.year.rem_euclid(100), 2)),
                'm' => out.push_str(&num(self.month, 2)),
                'd' => out.push_str(&num(self.day, 2)),
                'e' => out.push_str(&format!("{:>2}", self.day)),
                'H' => out.push_str(&num(self.hour, 2)),
                'I' => out.push_str(&num(hour12, 2)),
                'l' => out.push_str(&if unpadded {
                    hour12.to_string()
                } else {
                    format!("{hour12:>2}")
                }),
                'M' => out.push_str(&num(self.minute, 2)),
                'S' => out.push_str(&num(self.second, 2)),
                'p' => out.push_str(if self.hour < 12 { "AM" } else { "PM" }),
                'b' => out.push_str(MONTH_ABBR[self.month as usize - 1]),
                'B' => out.push_str(MONTH_NAME[self.month as usize - 1]),
                'F' => out.push_str(&format!(
                    "{:04}-{:02}-{:02}",
                    self.year, self.month, self.day
                )),
                'T' => out.push_str(&format!(
                    "{:02}:{:02}:{:02}",
                    self.hour, self.minute, self.second
                )),
                'Z' => out.push_str(
                    self.zone
                        .ok_or(Unsupported("time zones whose name isn't known"))?,
                ),
                '%' => out.push('%'),
                _ => return Err(Unsupported("strftime directives not ported")),
            }
        }
        Ok(out)
    }
}

/// `Time.at(seconds)` in the server's zone (UTC).
pub fn at(seconds: i64) -> Result<Fields, Unsupported> {
    use chrono::{Datelike, Timelike};
    let t = chrono::DateTime::from_timestamp(seconds, 0)
        .ok_or(Unsupported("times past chrono's range"))?;
    Ok(Fields {
        year: i64::from(t.year()),
        month: i64::from(t.month()),
        day: i64::from(t.day()),
        hour: i64::from(t.hour()),
        minute: i64::from(t.minute()),
        second: i64::from(t.second()),
        zone: Some("UTC"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn both(s: &str) -> (String, String) {
        match parse(s).unwrap() {
            Parsed::Time(f) => (f.short(), f.long()),
            Parsed::Raised => panic!("raised"),
        }
    }

    #[test]
    fn parses_as_time_parse() {
        let jan = (
            "15 Jan 23".to_string(),
            "10:30AM - 15 January 2023".to_string(),
        );
        assert_eq!(both("2023-01-15T10:30:00+05:00"), jan);
        assert_eq!(both("2023-01-15T10:30:00.123-0800"), jan);
        assert_eq!(both("Mon, 15 Jan 2023 10:30:00 GMT"), jan);
        assert_eq!(both("2023-01-15T10:30"), jan);
        assert_eq!(both("2023-01-15T22:30:00Z").1, "10:30PM - 15 January 2023");
        assert_eq!(both("2023-01-15").1, "12:00AM - 15 January 2023");
        assert_eq!(both("January 15, 2023").0, "15 Jan 23");
        assert_eq!(both("20230115").0, "15 Jan 23");
        assert_eq!(
            both("2014-08-08T20:03:49.008Zyyyy-MM-dd HH:mm:ss z"),
            (
                "8 Aug 14".to_string(),
                "08:03PM - 08 August 2014".to_string()
            )
        );
        assert_eq!(parse("2023-13-01").unwrap(), Parsed::Raised);
        assert_eq!(parse("garbage").unwrap(), Parsed::Raised);
        match duration("PT2H30M15S").unwrap() {
            Parsed::Time(f) => assert_eq!(f.hours_minutes(), "02:30"),
            Parsed::Raised => panic!(),
        }
        assert_eq!(duration("PT90M").unwrap(), Parsed::Raised);
    }
}
