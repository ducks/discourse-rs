//! The local dates plugin's markdown (discourse-local-dates.js): `[date=...]`
//! and `[date-range from=... to=...]` found by text-post-process become
//! `<span class="discourse-local-date" data-*>` around the date in UTC, in
//! the tag's `format` or moment's default. The server cooks with moment
//! 2.30 and moment-timezone 0.6 (the full data build) in English; chrono-tz
//! stands in for the zone data and `format` below for moment's formatter.
//!
//! What moment would only make sense of through the browser's `Date`
//! parsing (a time like `9:00`), or a date with no year, is refused.

use chrono::{
    DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Offset, TimeZone, Timelike,
    Utc,
};
use chrono_tz::{OffsetName, Tz};
use markdown_it::Node;
use markdown_it::parser::inline::Text;

use super::RenderSettings;
use super::bbcode::{data_attributes, parse_tag};
use super::context::Context;
use super::element::Element;
use crate::pretty_text::sanitizer::AllowList;

const INVALID: &str = "Invalid date";

/// `moment.tz.link(["Asia/Kolkata|IST", "Asia/Seoul|KST", "Asia/Tokyo|JST"])`
fn zone(name: &str) -> Option<Tz> {
    let name = match name {
        "IST" => "Asia/Kolkata",
        "KST" => "Asia/Seoul",
        "JST" => "Asia/Tokyo",
        other => other,
    };
    name.parse().ok()
}

/// `moment(dateStr, "YYYY-M-D").format("YYYY-MM-DD")`, forgiving as moment
/// is: each part is the first run of its digits anywhere in what is left.
/// Err when there is no year, which moment would take from today.
fn normalize_date(s: &str) -> Result<String, &'static str> {
    let mut rest = s;
    let mut take = |max: usize| -> Option<&str> {
        let start = rest.find(|c: char| c.is_ascii_digit())?;
        let len = rest[start..]
            .bytes()
            .take(max)
            .take_while(u8::is_ascii_digit)
            .count();
        let found = &rest[start..start + len];
        rest = &rest[start + len..];
        Some(found)
    };
    let Some(year) = take(4) else {
        return Ok(INVALID.to_string());
    };
    let year = if year.len() == 2 {
        // parseTwoDigitYear
        let y: i32 = year.parse().unwrap_or(0);
        y + if y > 68 { 1900 } else { 2000 }
    } else {
        year.parse().unwrap_or(0)
    };
    let month: u32 = take(2).map_or(1, |m| m.parse().unwrap_or(0));
    let day: u32 = take(2).map_or(1, |d| d.parse().unwrap_or(0));
    if year > 9999 {
        return Err("a local date year past 9999");
    }
    Ok(match NaiveDate::from_ymd_opt(year, month, day) {
        Some(date) => format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day()),
        None => INVALID.to_string(),
    })
}

/// A moment: an instant and the offset and zone it is shown in.
struct Moment {
    utc: DateTime<Utc>,
    offset: i32,
    abbr: String,
    /// `zz`: core moment's "Coordinated Universal Time" after `.utc()`,
    /// moment-timezone's abbreviation in a zone.
    name: String,
}

impl Moment {
    fn utc(utc: DateTime<Utc>) -> Self {
        Moment {
            utc,
            offset: 0,
            abbr: "UTC".to_string(),
            name: "Coordinated Universal Time".to_string(),
        }
    }

    fn in_zone(utc: DateTime<Utc>, tz: Tz) -> Option<Self> {
        let local = utc.with_timezone(&tz);
        let offset = local.offset();
        Some(Moment {
            utc,
            offset: offset.fix().local_minus_utc(),
            abbr: offset.abbreviation()?.to_string(),
            name: offset.abbreviation()?.to_string(),
        })
    }

    fn local(&self) -> NaiveDateTime {
        self.utc.naive_utc() + Duration::seconds(i64::from(self.offset))
    }
}

/// `moment.tz("YYYY-MM-DD[THH:mm[:ss[.SSS]]]", zone)`: the wall time read
/// in the zone (a time in a gap moves forward, one that happens twice is
/// the first), or an explicit offset when the time has one. Ok(None) is
/// moment's invalid date.
fn parse_in_zone(
    date: &str,
    time: Option<&str>,
    tz: Tz,
) -> Result<Option<DateTime<Utc>>, &'static str> {
    let Ok(day) = NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
        return Ok(None);
    };
    let Some(time) = time else {
        return Ok(wall(day.and_time(NaiveTime::MIN), tz));
    };
    // The extended ISO times moment knows, and an offset after them.
    let (clock, offset) = match time.find(['Z', 'z', '+', '-']) {
        Some(i) => (&time[..i], Some(time[i..].trim())),
        None => (time, None),
    };
    let parts: Vec<&str> = clock.split(':').collect();
    let two = |s: &str| s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit());
    if parts.is_empty() || parts.len() > 3 || !parts[..parts.len().min(2)].iter().all(|p| two(p)) {
        return Err("a local date time that is not HH:mm[:ss]");
    }
    let (seconds, millis) = match parts.get(2) {
        None => (0, 0),
        Some(s) => {
            let (whole, fraction) = s.split_once(['.', ',']).unwrap_or((s, ""));
            if !two(whole) || !fraction.bytes().all(|b| b.is_ascii_digit()) {
                return Err("a local date time that is not HH:mm[:ss]");
            }
            let millis = format!("{fraction:0<3}")[..3].parse::<u32>().unwrap_or(0);
            (whole.parse::<u32>().unwrap_or(99), millis)
        }
    };
    let hour: u32 = parts[0].parse().unwrap_or(99);
    let minute: u32 = parts.get(1).map_or(0, |m| m.parse().unwrap_or(99));
    let naive = if hour == 24 && minute == 0 && seconds == 0 && millis == 0 {
        day.succ_opt().map(|d| d.and_time(NaiveTime::MIN))
    } else {
        day.and_hms_milli_opt(hour, minute, seconds, millis)
    };
    let Some(naive) = naive else {
        return Ok(None);
    };
    Ok(match offset {
        None => wall(naive, tz),
        Some("Z" | "z") => Some(Utc.from_utc_datetime(&naive)),
        Some(offset) => {
            let sign = if offset.starts_with('-') { -1 } else { 1 };
            let digits: String = offset[1..].chars().filter(char::is_ascii_digit).collect();
            if !(digits.len() == 2 || digits.len() == 4) {
                return Err("a local date time with an offset moment would not read");
            }
            let hours: i64 = digits[..2].parse().unwrap_or(0);
            let minutes: i64 = digits.get(2..).map_or(0, |m| m.parse().unwrap_or(0));
            let shift = Duration::minutes(sign * (hours * 60 + minutes));
            Some(Utc.from_utc_datetime(&(naive - shift)))
        }
    })
}

/// moment-timezone's `Zone.parse`, defaults: `moveInvalidForward` on,
/// `moveAmbiguousForward` off.
fn wall(naive: NaiveDateTime, tz: Tz) -> Option<DateTime<Utc>> {
    match tz.from_local_datetime(&naive) {
        chrono::LocalResult::Single(t) => Some(t.with_timezone(&Utc)),
        chrono::LocalResult::Ambiguous(first, _) => Some(first.with_timezone(&Utc)),
        chrono::LocalResult::None => {
            // In a gap: read with the offset from before it.
            let before = tz
                .from_local_datetime(&(naive - Duration::days(1)))
                .earliest()?;
            let offset = before.offset().fix().local_minus_utc();
            Some(Utc.from_utc_datetime(&(naive - Duration::seconds(i64::from(offset)))))
        }
    }
}

const MONTHS: [&str; 12] = [
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
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const WEEKDAYS_MIN: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

/// The English locale's ordinal.
fn ordinal(n: i64) -> String {
    let suffix = if (n % 100) / 10 == 1 {
        "th"
    } else {
        match n % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    };
    format!("{n}{suffix}")
}

/// moment's `zeroFill`.
fn zero_fill(n: i64, width: usize, force_sign: bool) -> String {
    let sign = if n < 0 {
        "-"
    } else if force_sign {
        "+"
    } else {
        ""
    };
    format!("{sign}{:0width$}", n.abs())
}

/// The English long date formats, as `expandFormat` replaces them.
fn long_format(token: &str) -> Option<&'static str> {
    Some(match token {
        "LTS" => "h:mm:ss A",
        "LT" => "h:mm A",
        "L" => "MM/DD/YYYY",
        "LL" => "MMMM D, YYYY",
        "LLL" => "MMMM D, YYYY h:mm A",
        "LLLL" => "dddd, MMMM D, YYYY h:mm A",
        "l" => "M/D/YYYY",
        "ll" => "MMM D, YYYY",
        "lll" => "MMM D, YYYY h:mm A",
        "llll" => "ddd, MMM D, YYYY h:mm A",
        _ => return None,
    })
}

/// A `[...]` literal (`\[[^\[]*\]`) at the start of `rest`: its length
/// with the brackets.
fn literal(rest: &str) -> Option<usize> {
    let end = rest.strip_prefix('[')?.find(']')?;
    (!rest[1..1 + end].contains('[')).then_some(end + 2)
}

/// `expandFormat`: long date tokens outside brackets, until none is left.
fn expand(format: &str) -> String {
    let mut current = format.to_string();
    for _ in 0..5 {
        let mut out = String::new();
        let mut rest = current.as_str();
        let mut changed = false;
        while let Some(c) = rest.chars().next() {
            if let Some(len) = literal(rest) {
                out.push_str(&rest[..len]);
                rest = &rest[len..];
                continue;
            }
            if c == '\\' && rest.len() > 1 {
                let next = rest[1..].chars().next().unwrap();
                out.push(c);
                out.push(next);
                rest = &rest[1 + next.len_utf8()..];
                continue;
            }
            let token = [
                "LTS", "LLLL", "LLL", "LL", "LT", "L", "llll", "lll", "ll", "l",
            ]
            .into_iter()
            .find(|t| rest.starts_with(t));
            match token {
                Some(t) => {
                    out.push_str(long_format(t).unwrap());
                    rest = &rest[t.len()..];
                    changed = true;
                }
                None => {
                    out.push(c);
                    rest = &rest[c.len_utf8()..];
                }
            }
        }
        current = out;
        if !changed {
            break;
        }
    }
    current
}

/// moment's `formattingTokens`, longest first where they share a start.
const TOKENS: &[&str] = &[
    "YYYYYY",
    "YYYYY",
    "YYYY",
    "YY",
    "MMMM",
    "MMM",
    "MM",
    "Mo",
    "M",
    "DDDD",
    "DDDo",
    "DDD",
    "DD",
    "Do",
    "D",
    "dddd",
    "ddd",
    "dd",
    "do",
    "d",
    "e",
    "E",
    "ww",
    "wo",
    "w",
    "WW",
    "Wo",
    "W",
    "Qo",
    "Q",
    "gggg",
    "ggggg",
    "gg",
    "GGGG",
    "GGGGG",
    "GG",
    "a",
    "A",
    "hh",
    "h",
    "HH",
    "H",
    "kk",
    "k",
    "mm",
    "m",
    "ss",
    "s",
    "SSSSSSSSS",
    "SSSSSSSS",
    "SSSSSSS",
    "SSSSSS",
    "SSSSS",
    "SSSS",
    "SSS",
    "SS",
    "S",
    "x",
    "X",
    "zz",
    "z",
    "ZZ",
    "Z",
    "N",
    "y",
];

/// The week of a year that starts on `dow` and whose first week holds
/// January `7 + dow - doy`: moment's `weekOfYear`, as (week, year).
fn week_of_year(date: NaiveDate, dow: i64, doy: i64) -> (i64, i64) {
    fn first_week_offset(year: i32, dow: i64, doy: i64) -> i64 {
        let fwd = 7 + dow - doy;
        let jan = NaiveDate::from_ymd_opt(year, 1, fwd as u32).unwrap();
        let fwdlw = (7 + i64::from(jan.weekday().num_days_from_sunday()) - dow) % 7;
        -fwdlw + fwd - 1
    }
    fn weeks_in_year(year: i32, dow: i64, doy: i64) -> i64 {
        let days = if NaiveDate::from_ymd_opt(year, 2, 29).is_some() {
            366
        } else {
            365
        };
        (days - first_week_offset(year, dow, doy) + first_week_offset(year + 1, dow, doy)) / 7
    }
    let year = date.year();
    let offset = first_week_offset(year, dow, doy);
    let week = (i64::from(date.ordinal()) - offset - 1).div_euclid(7) + 1;
    if week < 1 {
        (
            week + weeks_in_year(year - 1, dow, doy),
            i64::from(year - 1),
        )
    } else if week > weeks_in_year(year, dow, doy) {
        (week - weeks_in_year(year, dow, doy), i64::from(year + 1))
    } else {
        (week, i64::from(year))
    }
}

/// `moment#format`, English, for a valid moment.
fn format(m: &Moment, format: &str) -> Result<String, &'static str> {
    let format = expand(format);
    let t = m.local();
    let date = t.date();
    let mut out = String::new();
    let mut rest = format.as_str();
    while let Some(c) = rest.chars().next() {
        if let Some(len) = literal(rest) {
            out.push_str(&rest[1..len - 1]);
            rest = &rest[len..];
            continue;
        }
        if c == '\\' && rest.len() > 1 {
            let next = rest[1..].chars().next().unwrap();
            out.push(next);
            rest = &rest[1 + next.len_utf8()..];
            continue;
        }
        let Some(token) = TOKENS
            .iter()
            .filter(|t| rest.starts_with(**t))
            .max_by_key(|t| t.len())
        else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        };
        rest = &rest[token.len()..];
        let year = i64::from(date.year());
        let month = i64::from(date.month());
        let day = i64::from(date.day());
        let weekday = i64::from(date.weekday().num_days_from_sunday());
        let hour = i64::from(t.hour());
        let millis = i64::from(t.and_utc().timestamp_subsec_millis());
        let offset_minutes = i64::from(m.offset / 60);
        let offset = |separator: &str| {
            let sign = if offset_minutes < 0 { '-' } else { '+' };
            let abs = offset_minutes.abs();
            format!("{sign}{:02}{separator}{:02}", abs / 60, abs % 60)
        };
        let piece = match *token {
            "YYYYYY" => zero_fill(year, 6, true),
            "YYYYY" => zero_fill(year, 5, false),
            "YYYY" => {
                if year <= 9999 {
                    zero_fill(year, 4, false)
                } else {
                    zero_fill(year, 4, true)
                }
            }
            "YY" => zero_fill(year % 100, 2, false),
            "MMMM" => MONTHS[month as usize - 1].to_string(),
            "MMM" => MONTHS[month as usize - 1][..3].to_string(),
            "MM" => zero_fill(month, 2, false),
            "Mo" => ordinal(month),
            "M" => month.to_string(),
            "DDDD" => zero_fill(i64::from(date.ordinal()), 3, false),
            "DDDo" => ordinal(i64::from(date.ordinal())),
            "DDD" => date.ordinal().to_string(),
            "DD" => zero_fill(day, 2, false),
            "Do" => ordinal(day),
            "D" => day.to_string(),
            "dddd" => WEEKDAYS[weekday as usize].to_string(),
            "ddd" => WEEKDAYS[weekday as usize][..3].to_string(),
            "dd" => WEEKDAYS_MIN[weekday as usize].to_string(),
            "do" => ordinal(weekday),
            "d" | "e" => weekday.to_string(),
            "E" => i64::from(date.weekday().number_from_monday()).to_string(),
            "ww" => zero_fill(week_of_year(date, 0, 6).0, 2, false),
            "wo" => ordinal(week_of_year(date, 0, 6).0),
            "w" => week_of_year(date, 0, 6).0.to_string(),
            "WW" => zero_fill(week_of_year(date, 1, 4).0, 2, false),
            "Wo" => ordinal(week_of_year(date, 1, 4).0),
            "W" => week_of_year(date, 1, 4).0.to_string(),
            "Qo" => ordinal((month - 1) / 3 + 1),
            "Q" => ((month - 1) / 3 + 1).to_string(),
            "gg" => zero_fill(week_of_year(date, 0, 6).1 % 100, 2, false),
            "gggg" => zero_fill(week_of_year(date, 0, 6).1, 4, false),
            "ggggg" => zero_fill(week_of_year(date, 0, 6).1, 5, false),
            "GG" => zero_fill(week_of_year(date, 1, 4).1 % 100, 2, false),
            "GGGG" => zero_fill(week_of_year(date, 1, 4).1, 4, false),
            "GGGGG" => zero_fill(week_of_year(date, 1, 4).1, 5, false),
            "a" => if hour < 12 { "am" } else { "pm" }.to_string(),
            "A" => if hour < 12 { "AM" } else { "PM" }.to_string(),
            "hh" => zero_fill(if hour % 12 == 0 { 12 } else { hour % 12 }, 2, false),
            "h" => (if hour % 12 == 0 { 12 } else { hour % 12 }).to_string(),
            "HH" => zero_fill(hour, 2, false),
            "H" => hour.to_string(),
            "kk" => zero_fill(if hour == 0 { 24 } else { hour }, 2, false),
            "k" => (if hour == 0 { 24 } else { hour }).to_string(),
            "mm" => zero_fill(i64::from(t.minute()), 2, false),
            "m" => t.minute().to_string(),
            "ss" => zero_fill(i64::from(t.second()), 2, false),
            "s" => t.second().to_string(),
            s if s.starts_with('S') => {
                let digits = format!("{millis:03}");
                if s.len() <= 3 {
                    digits[..s.len()].to_string()
                } else {
                    format!("{digits}{}", "0".repeat(s.len() - 3))
                }
            }
            "x" => m.utc.timestamp_millis().to_string(),
            "X" => m.utc.timestamp().to_string(),
            "z" => m.abbr.clone(),
            "zz" => m.name.clone(),
            "ZZ" => offset(""),
            "Z" => offset(":"),
            _ => return Err("an era (N or y) in a local date format"),
        };
        out.push_str(&piece);
    }
    Ok(out)
}

/// `addLocalDate`: the span, from the tag's attributes in their order.
fn local_date(
    mut attrs: Vec<(String, String)>,
    settings: &RenderSettings,
    ctx: &Context,
) -> Option<Node> {
    let get = |attrs: &[(String, String)], key: &str| {
        attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    };
    for key in ["timezone", "displayedTimezone"] {
        if get(&attrs, key).is_some_and(|tz| zone(&tz).is_none()) {
            attrs.retain(|(k, _)| k != key);
        }
    }
    for (key, value) in attrs.iter_mut() {
        match key.as_str() {
            "timezones" => {
                *value = value
                    .split('|')
                    .filter(|tz| zone(tz).is_some())
                    .collect::<Vec<_>>()
                    .join("|");
            }
            "_default" | "date" if !value.is_empty() => match normalize_date(value) {
                Ok(date) => *value = date,
                Err(what) => {
                    ctx.refuse(what);
                    return None;
                }
            },
            _ => {}
        }
    }
    let date = get(&attrs, "_default")
        .filter(|d| !d.is_empty())
        .or_else(|| get(&attrs, "date").filter(|d| !d.is_empty()));
    let time = get(&attrs, "time").filter(|t| !t.is_empty());
    let tz = get(&attrs, "timezone")
        .and_then(|tz| zone(&tz))
        .unwrap_or(chrono_tz::Etc::UTC);
    let instant = match date {
        Some(date) => match parse_in_zone(&date, time.as_deref(), tz) {
            Ok(instant) => instant,
            Err(what) => {
                ctx.refuse(what);
                return None;
            }
        },
        // `moment.tz("10:00", zone)`, or nothing at all: the browser's Date.
        None => {
            ctx.refuse("a local date without a date");
            return None;
        }
    };
    let Some(email_zone) = zone(&settings.local_dates_email_timezone) else {
        ctx.refuse("discourse_local_dates_email_timezone that moment does not know");
        return None;
    };
    let email_format = if settings.local_dates_email_format.is_empty() {
        "YYYY-MM-DDTHH:mm:ssZ"
    } else {
        settings.local_dates_email_format.as_str()
    };
    let (preview, text) = match instant {
        Some(utc) => {
            let Some(emailed) = Moment::in_zone(utc, email_zone) else {
                ctx.refuse("a local date email zone without an abbreviation");
                return None;
            };
            let shown = get(&attrs, "format").filter(|f| !f.is_empty());
            let shown = shown.as_deref().unwrap_or("YYYY-MM-DDTHH:mm:ss[Z]");
            match (
                format(&emailed, email_format),
                format(&Moment::utc(utc), shown),
            ) {
                (Ok(preview), Ok(text)) => (preview, text),
                (Err(what), _) | (_, Err(what)) => {
                    ctx.refuse(what);
                    return None;
                }
            }
        }
        None => (INVALID.to_string(), INVALID.to_string()),
    };
    attrs.push(("emailPreview".to_string(), preview));
    let mut span = Element::inline("span", &[("class", "discourse-local-date")]);
    if let Some(element) = span.cast_mut::<Element>() {
        element.attrs.extend(data_attributes(&attrs, Some("date")));
    }
    span.children.push(Node::new(Text { content: text }));
    Some(span)
}

/// The `date` rule: `[date=...]`, else the match as text.
pub fn date(matched: &str, settings: &RenderSettings, ctx: &Context) -> Vec<Node> {
    let parsed =
        parse_tag(matched, false).filter(|tag| tag.tag == "date" && tag.length == matched.len());
    match parsed.and_then(|tag| local_date(tag.attrs, settings, ctx)) {
        Some(span) => vec![span],
        None => vec![text(matched)],
    }
}

/// The `date-range` rule: a span for `from`, an arrow, a span for `to`.
pub fn range(matched: &str, settings: &RenderSettings, ctx: &Context) -> Vec<Node> {
    let Some(tag) = parse_tag(matched, false)
        .filter(|tag| tag.tag == "date-range" && tag.length == matched.len())
    else {
        return vec![text(matched)];
    };
    let mut nodes = Vec::new();
    let side = |which: &str, nodes: &mut Vec<Node>| -> bool {
        let Some(value) = tag.attr(which).map(str::to_string) else {
            return true;
        };
        let mut attrs: Vec<(String, String)> = tag
            .attrs
            .iter()
            .filter(|(k, _)| k != "from" && k != "to")
            .cloned()
            .collect();
        attrs.push(("range".to_string(), which.to_string()));
        let (date, time) = match value.split_once('T') {
            Some((date, time)) => (date.to_string(), Some(time.to_string())),
            None => (value.clone(), None),
        };
        set(&mut attrs, "date", date);
        match time {
            Some(time) => set(&mut attrs, "time", time),
            // `[attributes.date, attributes.time] = from.split("T")`
            None => attrs.retain(|(k, _)| k != "time"),
        }
        match local_date(attrs, settings, ctx) {
            Some(span) => {
                nodes.push(span);
                true
            }
            None => false,
        }
    };
    if !side("from", &mut nodes) {
        return vec![text(matched)];
    }
    if tag.attr("from").is_some() && tag.attr("to").is_some() {
        nodes.push(text("→"));
    }
    if !side("to", &mut nodes) {
        return vec![text(matched)];
    }
    nodes
}

pub fn allow(list: &mut AllowList) {
    list.allow(&[
        "span.discourse-local-date",
        "span[aria-label]",
        "span[data-calendar]",
        "span[data-countdown]",
        "span[data-date]",
        "span[data-displayed-timezone]",
        "span[data-email-preview]",
        "span[data-format]",
        "span[data-ics]",
        "span[data-recurring]",
        "span[data-time]",
        "span[data-timezone]",
        "span[data-timezones]",
    ]);
}

fn set(attrs: &mut Vec<(String, String)>, key: &str, value: String) {
    match attrs.iter_mut().find(|(k, _)| k == key) {
        Some(entry) => entry.1 = value,
        None => attrs.push((key.to_string(), value)),
    }
}

fn text(content: &str) -> Node {
    Node::new(Text {
        content: content.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_normalize_as_moment_reads_them() {
        assert_eq!(normalize_date("2020-1-5").unwrap(), "2020-01-05");
        assert_eq!(normalize_date("2020/12/31").unwrap(), "2020-12-31");
        assert_eq!(normalize_date("20-3-4").unwrap(), "2020-03-04");
        assert_eq!(normalize_date("2020").unwrap(), "2020-01-01");
        assert_eq!(normalize_date("2020-13-01").unwrap(), INVALID);
        assert_eq!(normalize_date("soon").unwrap(), INVALID);
    }

    #[test]
    fn formats_as_moment_writes_them() {
        let utc = Utc.with_ymd_and_hms(2018, 9, 1, 21, 0, 0).unwrap();
        let m = Moment::utc(utc);
        assert_eq!(
            format(&m, "YYYY-MM-DDTHH:mm:ss[Z]").unwrap(),
            "2018-09-01T21:00:00Z"
        );
        assert_eq!(format(&m, "LL").unwrap(), "September 1, 2018");
        assert_eq!(
            format(&m, "llll z").unwrap(),
            "Sat, Sep 1, 2018 9:00 PM UTC"
        );
        assert_eq!(
            format(&m, "Do [of] MMMM, dddd w W Q").unwrap(),
            "1st of September, Saturday 35 35 3"
        );
        assert_eq!(format(&m, "Z ZZ X").unwrap(), "+00:00 +0000 1535835600");
    }

    #[test]
    fn wall_times_read_in_their_zone() {
        let paris: Tz = "Europe/Paris".parse().unwrap();
        let read = |date, time| {
            parse_in_zone(date, time, paris)
                .unwrap()
                .unwrap()
                .to_rfc3339()
        };
        assert_eq!(read("2019-10-11", None), "2019-10-10T22:00:00+00:00");
        // Spring forward: 02:30 does not exist and moves to 03:30.
        assert_eq!(
            read("2024-03-31", Some("02:30")),
            "2024-03-31T01:30:00+00:00"
        );
        // Fall back: 02:30 happens twice; the first is taken.
        assert_eq!(
            read("2024-10-27", Some("02:30")),
            "2024-10-27T00:30:00+00:00"
        );
        assert!(parse_in_zone("2024-10-27", Some("9:00"), paris).is_err());
    }
}
