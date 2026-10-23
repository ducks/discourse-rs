//! Dates in the viewer's timezone (ROADMAP 209). The pages carry each date
//! as UTC epoch milliseconds and static/js/dates.js draws it in the
//! browser's zone before the page paints, so one rendered page serves every
//! viewer (the anonymous cache). The server still draws the text, in UTC
//! at the time of the request, for what runs no script (crawlers, tests).
//!
//! Ember's own markup, `<span class="relative-date" data-time data-format>`
//! (autoUpdatingRelativeAge), needs nothing more. Any other date sits in a
//! template, `%{<ms>:<format>}` for each date, in a `data-tz-text`,
//! `data-tz-title`, `data-tz-aria-label`, `data-tz-value` or
//! `data-tz-datetime` attribute: the format is a moment format string
//! (resolved here from the locale's key) or `@tiny`, `@medium`,
//! `@medium-with-ago` or `@medium-with-ago-and-on` for relativeAge.
//! `data-tz-zone` names the zone where Ember uses the member's timezone
//! option instead of the browser's.

use chrono::{DateTime, Datelike, Utc};

use crate::i18n::I18n;
use crate::pretty_text::render::local_dates::format_utc;
use crate::topic_list_view::{ListContext, escape};

/// What the formatters read: the strings, the page's time and the
/// relative_date_duration site setting.
pub(crate) trait DateContext {
    fn i18n(&self) -> &I18n;
    fn now(&self) -> DateTime<Utc>;
    fn relative_date_duration(&self) -> i64;
}

impl DateContext for ListContext<'_> {
    fn i18n(&self) -> &I18n {
        self.i18n
    }
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
    fn relative_date_duration(&self) -> i64 {
        self.settings.relative_date_duration
    }
}

impl DateContext for crate::admin_view::AdminCx<'_> {
    fn i18n(&self) -> &I18n {
        self.i18n
    }
    fn now(&self) -> DateTime<Utc> {
        crate::clock::now()
    }
    fn relative_date_duration(&self) -> i64 {
        self.settings
            .get("relative_date_duration")
            .map(|v| v.to_i())
            .unwrap_or(30)
    }
}

impl DateContext for crate::user_menu_view::MenuContext<'_> {
    fn i18n(&self) -> &I18n {
        self.i18n
    }
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
    fn relative_date_duration(&self) -> i64 {
        self.relative_date_duration
    }
}

fn t<C: DateContext + ?Sized>(cx: &C, key: &str) -> String {
    cx.i18n().t(&format!("js.{key}")).unwrap_or(key).to_string()
}

fn t_count<C: DateContext + ?Sized>(cx: &C, key: &str, count: i64) -> String {
    cx.i18n()
        .t_count(&format!("js.{key}"), count, &[])
        .unwrap_or_else(|| key.to_string())
}

/// A date of a template, in a moment format.
pub(crate) fn marker(at: DateTime<Utc>, format: &str) -> String {
    format!("%{{{}:{}}}", at.timestamp_millis(), format)
}

/// A date of a template, in the format the locale's `js.<key>` names.
pub(crate) fn marker_key<C: DateContext + ?Sized>(cx: &C, at: DateTime<Utc>, key: &str) -> String {
    marker(at, &t(cx, key))
}

/// `longDate` in a template: dates.long_with_year.
pub(crate) fn long_marker<C: DateContext + ?Sized>(cx: &C, at: DateTime<Utc>) -> String {
    marker_key(cx, at, "dates.long_with_year")
}

/// A relative age in a template (`relativeAge` with the format).
pub(crate) fn relative_marker(at: DateTime<Utc>, format: &str) -> String {
    marker(at, &format!("@{format}"))
}

/// The template drawn in UTC at the page's time: what a viewer without
/// the script reads.
pub(crate) fn fill_utc<C: DateContext + ?Sized>(cx: &C, template: &str) -> String {
    fill_in(cx, template, chrono_tz::UTC)
}

/// The moment as a UTC time reading the zone's wall clock, for format_utc.
fn wall(at: DateTime<Utc>, zone: chrono_tz::Tz) -> DateTime<Utc> {
    use chrono::TimeZone;
    Utc.from_utc_datetime(&at.with_timezone(&zone).naive_local())
}

/// The template drawn in a zone at the page's time (the formats and the
/// bookmark reminders; the relative ages don't depend on it).
pub(crate) fn fill_in<C: DateContext + ?Sized>(
    cx: &C,
    template: &str,
    zone: chrono_tz::Tz,
) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("%{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let parsed = after.find('}').and_then(|end| {
            let (ms, format) = after[..end].split_once(':')?;
            let ms: i64 = ms.parse().ok()?;
            Some((end, ms, DateTime::from_timestamp_millis(ms)?, format))
        });
        let Some((end, ms, at, format)) = parsed else {
            out.push_str("%{");
            rest = after;
            continue;
        };
        out.push_str(&match format.strip_prefix('@') {
            Some("reminder") => formatted_reminder(cx, at, zone),
            Some(named) => named_format(cx, ms, at, named),
            None => format_utc(wall(at, zone), format).unwrap_or_default(),
        });
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// ` <name>="<utc>" data-tz-<name>="<template>"`, both escaped.
pub(crate) fn attr<C: DateContext + ?Sized>(cx: &C, name: &str, template: &str) -> String {
    if !template.contains("%{") {
        return format!(" {name}=\"{}\"", escape(template));
    }
    format!(
        " {name}=\"{}\" data-tz-{name}=\"{}\"",
        escape(&fill_utc(cx, template)),
        escape(template)
    )
}

/// ` data-tz-<name>="<template>"` alone, for an attribute the caller writes
/// in UTC itself.
pub(crate) fn tz_attr(name: &str, template: &str) -> String {
    format!(" data-tz-{name}=\"{}\"", escape(template))
}

/// An element's text: ` data-tz-text="<template>"` for its start tag and
/// the escaped text in UTC.
pub(crate) fn text<C: DateContext + ?Sized>(cx: &C, template: &str) -> (String, String) {
    if !template.contains("%{") {
        return (String::new(), escape(template));
    }
    (tz_attr("text", template), escape(&fill_utc(cx, template)))
}

/// `text` in the member's zone where Ember reads their timezone option:
/// with one, ` data-tz-zone` names it and the text is drawn in it here;
/// without one, the browser's zone (moment.tz.guess), UTC here.
pub(crate) fn text_in<C: DateContext + ?Sized>(
    cx: &C,
    template: &str,
    zone: Option<chrono_tz::Tz>,
) -> (String, String) {
    let attrs = match zone {
        Some(z) => format!(
            "{} data-tz-zone=\"{}\"",
            tz_attr("text", template),
            escape(z.name())
        ),
        None => tz_attr("text", template),
    };
    (
        attrs,
        escape(&fill_in(cx, template, zone.unwrap_or(chrono_tz::UTC))),
    )
}

/// `formattedReminderTime(reminder_at, timezone)` capitalized
/// (Bookmark#formattedReminder): today's or tomorrow's time, else the
/// long date, in the zone.
pub(crate) fn formatted_reminder<C: DateContext + ?Sized>(
    cx: &C,
    at: DateTime<Utc>,
    zone: chrono_tz::Tz,
) -> String {
    let local = at.with_timezone(&zone).date_naive();
    let today = cx.now().with_timezone(&zone).date_naive();
    let format = |key: &str| format_utc(wall(at, zone), &t(cx, key)).unwrap_or_default();
    let i18n = cx.i18n();
    let text = if local == today + chrono::Duration::days(1) {
        i18n.t_with(
            "js.bookmarks.reminders.tomorrow_with_time",
            &[("time", &format("dates.time"))],
        )
    } else if local == today {
        i18n.t_with(
            "js.bookmarks.reminders.today_with_time",
            &[("time", &format("dates.time"))],
        )
    } else {
        i18n.t_with(
            "js.bookmarks.reminders.at_time",
            &[("date_time", &format("dates.long_with_year"))],
        )
    }
    .unwrap_or_default();
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `<tag attrs data-tz-text>text</tag>` for an element holding only the
/// template's text.
pub(crate) fn element<C: DateContext + ?Sized>(
    cx: &C,
    tag: &str,
    attrs: &str,
    template: &str,
) -> String {
    let (tz, text) = text(cx, template);
    format!("<{tag}{attrs}{tz}>{text}</{tag}>")
}

/// `title="<longDate>" data-tz-title=...`: the tooltip of
/// autoUpdatingRelativeAge's `title: true`.
pub(crate) fn long_title<C: DateContext + ?Sized>(cx: &C, at: DateTime<Utc>) -> String {
    attr(cx, "title", &long_marker(cx, at))
}

/// `dAgeWithTooltip(date)`: autoUpdatingRelativeAge in the tiny format with
/// the longDate title.
pub(crate) fn tiny_span<C: DateContext + ?Sized>(cx: &C, at: DateTime<Utc>) -> String {
    format!(
        "<span class=\"relative-date\"{} data-time=\"{}\" data-format=\"tiny\">{}</span>",
        long_title(cx, at),
        at.timestamp_millis(),
        escape(&relative_age_tiny(cx, at))
    )
}

/// A template's `@` formats (static/js/dates.js has the same):
///   tiny, medium, medium-with-ago, medium-with-ago-and-on  relativeAge
///   timeline           timelineDate
///   timeline-ago       the timeline's last post: relativeAgeTiny with
///                      addAgo and timelineDate past the relative ages
///   medium-span-ago    relativeAgeMediumSpan with ago (no dates)
///   reminder           Bookmark#formattedReminder (fill_in's zone)
///   timeline-days-ago  the number is days: timelineDate of today less them
///   tiny-ago           relativeAge with addAgo (poll-info's closed age)
///   humanize           moment().to(date, true) (poll-info's time left)
///   year:A|B           format A in the current year, else B
fn named_format<C: DateContext + ?Sized>(
    cx: &C,
    ms: i64,
    at: DateTime<Utc>,
    named: &str,
) -> String {
    if let Some(formats) = named.strip_prefix("year:") {
        let (this_year, other) = formats.split_once('|').unwrap_or((formats, formats));
        let format = if at.year() == cx.now().year() {
            this_year
        } else {
            other
        };
        return format_utc(at, format).unwrap_or_default();
    }
    match named {
        "timeline" => timeline_date(cx, at),
        "timeline-days-ago" => timeline_date(cx, cx.now() - chrono::Duration::days(ms)),
        "timeline-ago" => tiny(cx, at, Tiny::Timeline),
        "tiny-ago" => tiny(cx, at, Tiny::Ago),
        "humanize" => humanize(cx.now(), at),
        "medium-span-ago" => {
            let distance = (cx.now() - at).num_milliseconds() as f64 / 1000.0;
            medium_span(cx, distance, true)
        }
        _ => relative_age(cx, at, named),
    }
}

/// `timelineDate`: "Sep 30" this year, else "Sep 2025".
pub(crate) fn timeline_date<C: DateContext + ?Sized>(cx: &C, at: DateTime<Utc>) -> String {
    let key = if at.year() == cx.now().year() {
        "dates.long_no_year_no_time"
    } else {
        "dates.timeline_date"
    };
    format_utc(at, &t(cx, key)).unwrap_or_default()
}

/// `relativeAge(date, {format})`, unwrapped, at the page's time in UTC.
pub(crate) fn relative_age<C: DateContext + ?Sized>(
    cx: &C,
    at: DateTime<Utc>,
    format: &str,
) -> String {
    match format {
        "medium" => relative_age_medium(cx, at, false, false),
        "medium-with-ago" => relative_age_medium(cx, at, true, false),
        "medium-with-ago-and-on" => relative_age_medium(cx, at, true, true),
        _ => relative_age_tiny(cx, at),
    }
}

/// `relativeAgeTiny`
pub(crate) fn relative_age_tiny<C: DateContext + ?Sized>(cx: &C, at: DateTime<Utc>) -> String {
    tiny(cx, at, Tiny::Plain)
}

/// relativeAgeTiny's options.
#[derive(Clone, Copy, PartialEq)]
enum Tiny {
    Plain,
    /// `addAgo` (relativeAge(date, { addAgo: true })).
    Ago,
    /// The timeline's: addAgo, and timelineDate as the defaultFormat.
    Timeline,
}

/// `moment().to(at, true)` in English: moment's relativeTime of the time
/// between now and `at`, without a suffix ("7 months", "a day").
pub(crate) fn humanize(now: DateTime<Utc>, at: DateTime<Utc>) -> String {
    let (from, to) = if at < now { (at, now) } else { (now, at) };
    // positiveMomentsDifference: whole calendar months, then the rest.
    let add_months = |d: DateTime<Utc>, months: i32| -> DateTime<Utc> {
        let total = d.year() * 12 + d.month0() as i32 + months;
        let (year, month0) = (total.div_euclid(12), total.rem_euclid(12) as u32);
        let last_day = (28..=31)
            .rev()
            .find(|day| chrono::NaiveDate::from_ymd_opt(year, month0 + 1, *day).is_some())
            .unwrap_or(28);
        let day = d.day().min(last_day);
        chrono::NaiveDate::from_ymd_opt(year, month0 + 1, day)
            .map(|date| date.and_time(d.time()).and_utc())
            .unwrap_or(d)
    };
    let mut months = (to.month0() as i32 - from.month0() as i32) + (to.year() - from.year()) * 12;
    if add_months(from, months) > to {
        months -= 1;
    }
    let ms = (to - add_months(from, months)).num_milliseconds() as f64;
    // Duration#as: months to days and back as moment converts them.
    let days_of_months = (f64::from(months) * 146_097.0 / 4800.0).round();
    let as_seconds = days_of_months * 86_400.0 + ms / 1000.0;
    let as_days = days_of_months + ms / 864e5;
    let as_months = f64::from(months) + (ms / 864e5) * 4800.0 / 146_097.0;
    let round = |x: f64| (x + 0.5).floor() as i64;
    let seconds = round(as_seconds);
    let minutes = round(as_seconds / 60.0);
    let hours = round(as_seconds / 3600.0);
    let days = round(as_days);
    let months = round(as_months);
    let years = round(as_months / 12.0);
    match () {
        _ if seconds <= 44 => "a few seconds".to_string(),
        _ if seconds < 45 => format!("{seconds} seconds"),
        _ if minutes <= 1 => "a minute".to_string(),
        _ if minutes < 45 => format!("{minutes} minutes"),
        _ if hours <= 1 => "an hour".to_string(),
        _ if hours < 22 => format!("{hours} hours"),
        _ if days <= 1 => "a day".to_string(),
        _ if days < 26 => format!("{days} days"),
        _ if months <= 1 => "a month".to_string(),
        _ if months < 11 => format!("{months} months"),
        _ if years <= 1 => "a year".to_string(),
        _ => format!("{years} years"),
    }
}

/// relativeAgeTiny with its options.
fn tiny<C: DateContext + ?Sized>(cx: &C, at: DateTime<Utc>, options: Tiny) -> String {
    let timeline = options == Tiny::Timeline;
    let add_ago = options != Tiny::Plain;
    let now = cx.now();
    let distance = ((now - at).num_milliseconds() as f64 / 1000.0)
        .round()
        .abs();
    let minutes = (distance / 60.0).round().max(1.0) as i64;
    let duration = cx.relative_date_duration();
    let date = |key: &str| format_utc(at, &t(cx, key)).unwrap_or_default();
    let age = |key: &str, count: i64| {
        let text = t_count(cx, &format!("dates.tiny.{key}"), count);
        if add_ago {
            cx.i18n()
                .t_with("js.dates.wrap_ago", &[("date", &text)])
                .unwrap_or(text)
        } else {
            text
        }
    };
    match minutes {
        0..=44 => age("x_minutes", minutes),
        45..=89 => age("about_x_hours", 1),
        90..=1409 => age("about_x_hours", (minutes as f64 / 60.0).round() as i64),
        _ if duration == 0 && minutes <= 525_599 => date("dates.tiny.date_month"),
        1410..=2519 => age("x_days", 1),
        m if m >= 2520 && m <= (if duration == 0 { 14 } else { duration }) * 1440 => {
            age("x_days", (minutes as f64 / 1440.0).round() as i64)
        }
        _ if timeline => timeline_date(cx, at),
        // smartShortDate
        _ if at.year() == now.year() => date("dates.tiny.date_month"),
        _ => date("dates.tiny.date_year"),
    }
}

/// `relativeAgeMedium` with wrapInSpan false.
fn relative_age_medium<C: DateContext + ?Sized>(
    cx: &C,
    at: DateTime<Utc>,
    leave_ago: bool,
    wrap_on: bool,
) -> String {
    let now = cx.now();
    let distance = ((now - at).num_milliseconds() as f64 / 1000.0).round() as i64;
    if distance < 60 {
        return t(cx, "now");
    }
    if distance > 432_000 {
        // smartShortDate(date, shortDate)
        let key = if at.year() == now.year() {
            "dates.tiny.date_month"
        } else {
            "dates.medium.date_year"
        };
        let short = format_utc(at, &t(cx, key)).unwrap_or_default();
        return if wrap_on {
            cx.i18n()
                .t_with("js.dates.wrap_on", &[("date", &short)])
                .unwrap_or(short)
        } else {
            short
        };
    }
    medium_span(cx, distance as f64, leave_ago)
}

/// `relativeAgeMediumSpan(distance, leaveAgo)`, the distance in seconds.
fn medium_span<C: DateContext + ?Sized>(cx: &C, distance: f64, leave_ago: bool) -> String {
    let minutes = (distance / 60.0).round() as i64;
    let (unit, count) = match minutes {
        1..=55 => ("x_minutes", minutes),
        56..=89 => ("x_hours", 1),
        90..=1409 => ("x_hours", (minutes as f64 / 60.0).round() as i64),
        1410..=2519 => ("x_days", 1),
        2520..=129_599 => ("x_days", (minutes as f64 / 1440.0).round() as i64),
        129_600..=525_599 => ("x_months", (minutes as f64 / 43200.0).round() as i64),
        _ => ("x_years", (minutes as f64 / 525_600.0).round() as i64),
    };
    let scope = if leave_ago {
        "medium_with_ago"
    } else {
        "medium"
    };
    t_count(cx, &format!("dates.{scope}.{unit}"), count)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dates<'a> {
        i18n: &'a I18n,
        now: DateTime<Utc>,
        relative_date_duration: i64,
    }

    impl DateContext for Dates<'_> {
        fn i18n(&self) -> &I18n {
            self.i18n
        }
        fn now(&self) -> DateTime<Utc> {
            self.now
        }
        fn relative_date_duration(&self) -> i64 {
            self.relative_date_duration
        }
    }

    fn cx(i18n: &I18n) -> Dates<'_> {
        Dates {
            i18n,
            now: "2026-10-17T12:00:00Z".parse().unwrap(),
            relative_date_duration: 30,
        }
    }

    #[test]
    fn templates_fill_in_utc() {
        let i18n = I18n::vendored().unwrap();
        let cx = cx(&i18n);
        let at: DateTime<Utc> = "2026-10-03T23:30:00Z".parse().unwrap();
        let template = format!("Created: {}", long_marker(&cx, at));
        assert_eq!(template, "Created: %{1791070200000:MMM D, YYYY h:mm a}");
        assert_eq!(fill_utc(&cx, &template), "Created: Oct 3, 2026 11:30 pm");
        assert_eq!(
            fill_utc(&cx, &relative_marker(at, "medium-with-ago-and-on")),
            "on Oct 3"
        );
        assert_eq!(
            attr(&cx, "title", &template),
            " title=\"Created: Oct 3, 2026 11:30 pm\" \
             data-tz-title=\"Created: %{1791070200000:MMM D, YYYY h:mm a}\""
        );
        // Not a placeholder: left as it is.
        assert_eq!(fill_utc(&cx, "%{name} %{x:y}"), "%{name} %{x:y}");
    }

    #[test]
    fn reminders_are_drawn_in_the_members_zone() {
        let i18n = I18n::vendored().unwrap();
        let cx = cx(&i18n);
        // 7 pm on the 17th in Los Angeles, 2 am on the 18th in UTC.
        let at: DateTime<Utc> = "2026-10-18T02:00:00Z".parse().unwrap();
        let template = relative_marker(at, "reminder");
        let la: chrono_tz::Tz = "America/Los_Angeles".parse().unwrap();
        assert_eq!(fill_utc(&cx, &template), "Tomorrow at 2:00 am");
        assert_eq!(fill_in(&cx, &template, la), "Today at 7:00 pm");
        assert_eq!(
            text_in(&cx, &template, Some(la)),
            (
                " data-tz-text=\"%{1792288800000:@reminder}\" \
                 data-tz-zone=\"America/Los_Angeles\""
                    .to_string(),
                "Today at 7:00 pm".to_string()
            )
        );
        // Without the option, the browser's zone: no data-tz-zone.
        assert_eq!(
            text_in(&cx, &template, None).0,
            " data-tz-text=\"%{1792288800000:@reminder}\""
        );
    }

    #[test]
    fn the_timeline_and_year_formats() {
        let i18n = I18n::vendored().unwrap();
        let cx = cx(&i18n);
        let at: DateTime<Utc> = "2026-09-30T07:28:29Z".parse().unwrap();
        assert_eq!(fill_utc(&cx, &relative_marker(at, "timeline")), "Sep 30");
        assert_eq!(
            fill_utc(&cx, &relative_marker(at, "timeline-ago")),
            "17d ago"
        );
        // The number is days before the page's time.
        assert_eq!(fill_utc(&cx, "%{400:@timeline-days-ago}"), "Sep 2025");
        let last_year: DateTime<Utc> = "2025-09-30T07:28:29Z".parse().unwrap();
        assert_eq!(
            fill_utc(&cx, &relative_marker(last_year, "year:MMM D|MMM YYYY")),
            "Sep 2025"
        );
        assert_eq!(
            fill_utc(&cx, &relative_marker(at, "year:MMM D|MMM YYYY")),
            "Sep 30"
        );
        // Two days back: relativeAgeMediumSpan, never a date.
        let earlier: DateTime<Utc> = "2026-10-09T12:00:00Z".parse().unwrap();
        assert_eq!(
            fill_utc(&cx, &relative_marker(earlier, "medium-span-ago")),
            "8 days ago"
        );
        // Text without a date carries no template.
        assert_eq!(text(&cx, "plain"), (String::new(), "plain".to_string()));
        assert_eq!(attr(&cx, "title", "a & b"), " title=\"a &amp; b\"");
    }

    #[test]
    fn medium_past_five_days_shows_the_short_date() {
        let i18n = I18n::vendored().unwrap();
        let cx = cx(&i18n);
        let last_year: DateTime<Utc> = "2025-10-03T23:30:00Z".parse().unwrap();
        // shortDate (dates.medium.date_year), not the tiny one
        assert_eq!(relative_age(&cx, last_year, "medium"), "Oct 3, 2025");
        assert_eq!(
            relative_age(&cx, last_year, "medium-with-ago-and-on"),
            "on Oct 3, 2025"
        );
        assert_eq!(relative_age(&cx, last_year, "tiny"), "Oct 2025");
    }

    #[test]
    fn humanize_as_moment_to_without_suffix() {
        let now: DateTime<Utc> = "2026-10-18T12:00:00Z".parse().unwrap();
        let at = |s: &str| s.parse::<DateTime<Utc>>().unwrap();
        assert_eq!(humanize(now, at("2027-06-01T12:00:00Z")), "7 months");
        assert_eq!(humanize(now, at("2026-10-18T12:00:30Z")), "a few seconds");
        assert_eq!(humanize(now, at("2026-10-18T12:01:00Z")), "a minute");
        assert_eq!(humanize(now, at("2026-10-18T14:00:00Z")), "2 hours");
        assert_eq!(humanize(now, at("2026-10-19T12:00:00Z")), "a day");
        assert_eq!(humanize(now, at("2026-10-28T12:00:00Z")), "10 days");
        assert_eq!(humanize(now, at("2026-11-18T12:00:00Z")), "a month");
        assert_eq!(humanize(now, at("2027-10-18T12:00:00Z")), "a year");
        assert_eq!(humanize(now, at("2029-10-18T12:00:00Z")), "3 years");
        // Past dates read the same.
        assert_eq!(humanize(now, at("2026-10-16T12:00:00Z")), "2 days");
    }

    #[test]
    fn tiny_with_ago() {
        let i18n = I18n::vendored().unwrap();
        let cx = cx(&i18n);
        let hours_ago: DateTime<Utc> = "2026-10-17T09:00:00Z".parse().unwrap();
        assert_eq!(fill_utc(&cx, &marker(hours_ago, "@tiny-ago")), "3h ago");
        let last_year: DateTime<Utc> = "2025-10-03T23:30:00Z".parse().unwrap();
        assert_eq!(fill_utc(&cx, &marker(last_year, "@tiny-ago")), "Oct 2025");
    }
}
