//! discourse-cakeday's client on the server-rendered pages: the cake or
//! balloon beside a poster's name (`addPosterIcon`), on the user card
//! (user-card-post-names) and on the profile (user-post-names), the
//! date of birth input among the profile preferences
//! (user-custom-preferences, with static/js/cakeday.js), the community
//! section's Anniversaries and Birthdays links, and the anniversaries and
//! birthdays pages (templates/cakeday/*), paged by htmx as DLoadMore
//! scrolls.
//!
//! "Today" is the viewer's date in their timezone, where the client reads
//! the browser's.

use chrono::{Datelike, Duration, NaiveDate, TimeZone, Utc};
use serde_json::Value;

use super::lists::{Kind, PAGE_SIZE};
use crate::emojis::Emojis;
use crate::site_settings::{SettingError, SiteSettings};
use crate::topic_list_view::{ListContext, escape, icon, t, t_with};

/// What the plugin's components read: its emoji settings, with birthdays
/// on or not, and the viewer's today.
#[derive(Debug, Clone)]
pub struct CakedayUi {
    /// `cakeday_emoji`
    pub cakeday_emoji: String,
    /// `cakeday_birthday_emoji` while `cakeday_birthday_enabled`.
    pub birthday_emoji: Option<String>,
    pub today: NaiveDate,
}

impl CakedayUi {
    /// None while the plugin is off (its client isn't loaded).
    pub fn load(settings: &SiteSettings, today: NaiveDate) -> Result<Option<Self>, SettingError> {
        if !super::enabled(settings)? {
            return Ok(None);
        }
        Ok(Some(CakedayUi {
            cakeday_emoji: settings.get("cakeday_emoji")?.to_s().to_string(),
            birthday_emoji: if super::birthday_enabled(settings)? {
                Some(settings.get("cakeday_birthday_emoji")?.to_s().to_string())
            } else {
                None
            },
            today,
        }))
    }

    /// `load` with the viewer's today.
    pub async fn for_viewer(
        conn: &mut sqlx::PgConnection,
        settings: &SiteSettings,
        viewer_id: Option<i32>,
    ) -> Result<Option<Self>, crate::plugins::PluginError> {
        if !super::enabled(settings)? {
            return Ok(None);
        }
        let today = super::viewer_today(conn, viewer_id).await?;
        Ok(Self::load(settings, today)?)
    }

    /// lib/cakeday's isSameDay: the date's month and day are today's, for
    /// an anniversary in an earlier year.
    fn same_day(&self, date: &Value, anniversary: bool) -> bool {
        let Some(date) = date
            .as_str()
            .and_then(|s| s.get(..10))
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        else {
            return false;
        };
        if anniversary && self.today.year() <= date.year() {
            return false;
        }
        (self.today.month(), self.today.day()) == (date.month(), date.day())
    }
}

/// `emojiUnescape(":name:", { title, skipTitle })` for one emoji code;
/// the code itself when it isn't one.
fn emoji(
    emojis: &Emojis,
    base_path: &str,
    name: &str,
    title: Option<&str>,
    skip_title: bool,
) -> String {
    let code = escape(&format!(":{name}:"));
    if !emojis.enabled {
        return code;
    }
    if emojis.is_denied(name) {
        return String::new();
    }
    let Some(url) = emojis.url(name, base_path) else {
        return code;
    };
    let class = if emojis.is_custom(name) {
        "emoji emoji-custom"
    } else {
        "emoji"
    };
    let title = escape(title.unwrap_or(name));
    let title_attr = if skip_title {
        String::new()
    } else {
        format!(" title='{title}'")
    };
    format!(
        "<img width=\"20\" height=\"20\" src='{}'{title_attr} alt='{title}' class='{class}'>",
        attr(&url)
    )
}

/// The title key of lib/cakeday's cakedayTitle and birthdayTitle: the
/// viewer's own, or another's.
fn title(cx: &ListContext, key: &str, own: bool) -> String {
    t(
        cx,
        &format!("user.{key}.{}", if own { "user_title" } else { "title" }),
    )
}

/// EmojiImages: the emoji list under the title, or the cake icon while
/// emoji are off.
fn emoji_images(cx: &ListContext, list: &str, title: &str) -> String {
    let emojis = &cx.settings.emojis;
    let inner = if emojis.enabled {
        let images: String = list
            .split('|')
            .map(|name| emoji(emojis, cx.base_path, name, None, true))
            .collect();
        format!("<div title=\"{}\">{images}</div>", escape(title))
    } else {
        format!(
            "<span class=\"svg-icon-title\" title=\"{}\">{}</span>",
            escape(title),
            icon("cake-candles", None)
        )
    };
    format!("<div class=\"emoji-images\">{inner}</div>")
}

/// The user-card-post-names and user-post-names connectors (UserCardCakeday,
/// UserCakeday): the birthday's emoji, then the anniversary's, for a user
/// whose `birthdate` or `cakedate` is today. `class` is the outlet's.
pub fn user_names(
    ui: &CakedayUi,
    cx: &ListContext,
    u: &Value,
    viewer_id: Option<i32>,
    class: &str,
) -> String {
    let own = viewer_id.is_some_and(|v| u["id"].as_i64() == Some(i64::from(v)));
    let mut out = format!("<div class=\"{class}\">");
    if let Some(list) = &ui.birthday_emoji
        && ui.same_day(&u["birthdate"], false)
    {
        out.push_str(&emoji_images(cx, list, &title(cx, "date_of_birth", own)));
    }
    if ui.same_day(&u["cakedate"], true) {
        out.push_str(&emoji_images(
            cx,
            &ui.cakeday_emoji,
            &title(cx, "anniversary", own),
        ));
    }
    out.push_str("</div>");
    out
}

/// The user card's connector.
pub fn user_card(ui: &CakedayUi, cx: &ListContext, u: &Value, viewer_id: Option<i32>) -> String {
    user_names(
        ui,
        cx,
        u,
        viewer_id,
        "user-card-post-names-outlet user-card-cakeday",
    )
}

/// The profile's connector, in its outlet's span.
pub fn profile(ui: &CakedayUi, cx: &ListContext, u: &Value, viewer_id: Option<i32>) -> String {
    format!(
        "<span>{}</span>",
        user_names(ui, cx, u, viewer_id, "user-post-names-outlet user-cakeday")
    )
}

/// The poster icons (`addPosterIcon`) for a post: the anniversary's,
/// then the birthday's (the order the initializer adds them), each a
/// `poster-icon` span with the emoji list or the cake icon.
pub fn poster_icons(ui: &CakedayUi, cx: &ListContext, p: &Value, viewer: Option<&str>) -> String {
    // The initializer adds nothing without a current user.
    let Some(viewer) = viewer else {
        return String::new();
    };
    let own = p["username"].as_str() == Some(viewer);
    let emojis = &cx.settings.emojis;
    let one = |list: &str, title: &str| {
        let content = if emojis.enabled {
            list.split('|')
                .map(|name| emoji(emojis, cx.base_path, name, Some(title), false))
                .collect()
        } else {
            icon("cake-candles", None)
        };
        format!(
            "<span class=\"poster-icon\" title=\"{}\">{content}</span>",
            escape(title)
        )
    };
    let mut out = String::new();
    if ui.same_day(&p["user_cakedate"], true) {
        out.push_str(&one(&ui.cakeday_emoji, &title(cx, "anniversary", own)));
    }
    if let Some(list) = &ui.birthday_emoji
        && ui.same_day(&p["user_birthdate"], false)
    {
        out.push_str(&one(list, &title(cx, "date_of_birth", own)));
    }
    out
}

/// moment.months()
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

/// UserDateOfBirthInput: the month and day combo boxes, which
/// static/js/cakeday.js turns into `date_of_birth` ("1904-M-D", or blank
/// unless both are set) when either changes. Nothing without birthdays.
pub fn date_of_birth_input(p: &crate::preferences_view::Prefs) -> Result<String, crate::AppError> {
    if !super::birthday_enabled(p.s)? {
        return Ok(String::new());
    }
    let cx = p.cx;
    let birthdate = p.u["birthdate"]
        .as_str()
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
    let none = Some(t(cx, "cakeday.none"));
    let month = crate::preferences_view::combo(
        cx,
        &crate::preferences_view::Combo {
            id: Some("cakeday-date-of-birth-month"),
            component_class: "",
            class: "",
            rows: MONTHS
                .iter()
                .enumerate()
                .map(|(i, m)| ((i + 1).to_string(), m.to_string()))
                .collect(),
            value: birthdate.map(|d| d.month().to_string()),
            none: none.clone(),
            key: None,
            filterable: false,
        },
    );
    let day = crate::preferences_view::combo(
        cx,
        &crate::preferences_view::Combo {
            id: Some("cakeday-date-of-birth-day"),
            component_class: "",
            class: "",
            rows: (1..=31).map(|d| (d.to_string(), d.to_string())).collect(),
            value: birthdate.map(|d| d.day().to_string()),
            none,
            key: None,
            filterable: false,
        },
    );
    Ok(format!(
        "<div class=\"user-custom-preferences-outlet user-date-of-birth-input\"><div class=\"control-group\"><label class=\"control-label\">{}</label><div class=\"controls\">{month}{day}<input type=\"hidden\" data-pref=\"date_of_birth\" value=\"\"></div></div></div>",
        escape(&t(cx, "user.date_of_birth.label"))
    ))
}

/// The community section's links (`addCommunitySectionLink`, secondary):
/// Anniversaries, then Birthdays, for a member.
pub(crate) fn community_more_links(
    cx: &crate::sidebar::Context,
) -> Result<Vec<crate::sidebar::Link>, SettingError> {
    use crate::sidebar::{Active, Link, Prefix};
    if cx.member.is_none() || !super::enabled(cx.settings)? {
        return Ok(Vec::new());
    }
    let mut kinds = vec![Kind::Anniversaries];
    if super::birthday_enabled(cx.settings)? {
        kinds.push(Kind::Birthdays);
    }
    Ok(kinds
        .into_iter()
        .map(|kind| {
            let key = kind.key();
            let text = cx
                .i18n
                .t(&format!("js.{key}.title"))
                .unwrap_or(key)
                .to_string();
            let route = format!("cakeday.{key}.today");
            Link {
                link_name: Some(key.to_string()),
                href: format!("{}/cakeday/{key}/today", cx.base_path),
                title: Some(text.clone()),
                content: escape(&text),
                prefix: Prefix::Icon {
                    name: "cake-candles".into(),
                    color: None,
                },
                active: *cx.active == Active::Route(route),
                ..Default::default()
            }
        })
        .collect())
}

/// The list pages' routes (cakeday.<kind>.<filter>).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Today,
    Tomorrow,
    Upcoming,
    All,
}

impl Filter {
    pub fn from_segment(s: &str) -> Option<Filter> {
        Some(match s {
            "today" => Filter::Today,
            "tomorrow" => Filter::Tomorrow,
            "upcoming" => Filter::Upcoming,
            "all" => Filter::All,
            _ => return None,
        })
    }

    pub fn segment(self) -> &'static str {
        match self {
            Filter::Today => "today",
            Filter::Tomorrow => "tomorrow",
            Filter::Upcoming => "upcoming",
            Filter::All => "all",
        }
    }

    /// The `filter` the route's model asks for: none for All, which asks
    /// by month.
    pub fn param(self) -> Option<String> {
        (self != Filter::All).then(|| self.segment().to_string())
    }
}

/// What a list page shows.
pub struct ListPage<'a> {
    pub kind: Kind,
    pub filter: Filter,
    /// The All page's month (its query param, else the current one).
    pub month: i64,
    pub page: i64,
    /// The lists' document.
    pub doc: &'a Value,
    pub birthdays: bool,
    pub today: NaiveDate,
}

/// A date the way moment formats `pattern`.
fn format_date(d: NaiveDate, pattern: &str) -> String {
    let at = Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap_or_default());
    crate::pretty_text::render::local_dates::format_utc(at, pattern).unwrap_or_default()
}

/// A URL in an attribute, as the client writes one (no escapeExpression,
/// which would turn its `=` into an entity).
fn attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The `<a>` a LinkTo draws, active on its route.
fn link_to(href: &str, label: &str, active: bool) -> String {
    format!(
        "<a href=\"{}\"{}>{}</a>",
        attr(href),
        if active { " class=\"active\"" } else { "" },
        escape(label)
    )
}

/// templates/cakeday with the kind's template and its filter's: the nav
/// pills, the title or the month chooser, the users (UserInfoList) and
/// DLoadMore's sentinel, which loads the next page.
pub fn page(cx: &ListContext, p: &ListPage) -> String {
    let base = cx.base_path;
    let key = p.kind.key();
    let mut out = String::from("<div class=\"container cakeday\"><ul class=\"nav-pills\">");
    out.push_str(&format!(
        "<li class=\"nav-item-anniversaries\">{}</li>",
        link_to(
            &format!("{base}/cakeday/anniversaries"),
            &t(cx, "anniversaries.title"),
            p.kind == Kind::Anniversaries
        )
    ));
    if p.birthdays {
        out.push_str(&format!(
            "<li class=\"nav-item-birthdays\">{}</li>",
            link_to(
                &format!("{base}/cakeday/birthdays"),
                &t(cx, "birthdays.title"),
                p.kind == Kind::Birthdays
            )
        ));
    }
    out.push_str(&format!(
        "</ul><div class=\"{key}\"><ul class=\"nav-pills\">"
    ));
    for filter in [
        Filter::Today,
        Filter::Tomorrow,
        Filter::Upcoming,
        Filter::All,
    ] {
        out.push_str(&format!(
            "<li>{}</li>",
            link_to(
                &format!("{base}/cakeday/{key}/{}", filter.segment()),
                &t(cx, &format!("cakeday.{}", filter.segment())),
                filter == p.filter
            )
        ));
    }
    out.push_str("</ul>");

    let no_year = t(cx, "dates.full_no_year_no_time");
    let (title, empty) = match p.filter {
        Filter::Today => (
            Some(t_with(
                cx,
                &format!("{key}.today.title"),
                &[("date", &format_date(p.today, &no_year))],
            )),
            "today",
        ),
        // The tomorrow page's title is today's, of tomorrow's date.
        Filter::Tomorrow => (
            Some(t_with(
                cx,
                &format!("{key}.today.title"),
                &[("date", &format_date(p.today + Duration::days(1), &no_year))],
            )),
            "tomorrow",
        ),
        Filter::Upcoming => {
            let start = p.today + Duration::days(2);
            let end = start + Duration::weeks(1);
            (
                Some(t_with(
                    cx,
                    &format!("{key}.upcoming.title"),
                    &[
                        ("start_date", &format_date(start, &no_year)),
                        ("end_date", &format_date(end, &no_year)),
                    ],
                )),
                "upcoming",
            )
        }
        Filter::All => (None, "month"),
    };
    if let Some(title) = &title {
        out.push_str(&format!(
            "<h2 class=\"cakeday-header\">{}</h2>",
            escape(title)
        ));
    }
    out.push_str("<div>");
    if p.filter == Filter::All {
        out.push_str(&format!(
            "<div class=\"cakeday-months\"><h2 class=\"cakeday-header\">{}</h2>{}</div>",
            escape(&t(cx, &format!("{key}.month.title"))),
            month_chooser(cx, p)
        ));
    }
    out.push_str("<div class=\"loading-container\"><ul class=\"user-info-list\">");
    out.push_str(&items(cx, p));
    let users = p.doc[key].as_array().map_or(0, Vec::len);
    if users == 0 && p.page == 0 {
        out.push_str(&format!(
            "<div class=\"user-info-empty-message\"><p>{}</p></div>",
            escape(&t(cx, &format!("{key}.{empty}.empty")))
        ));
    }
    out.push_str("</ul></div><div class=\"loading-container\"></div>");
    out.push_str(&sentinel(cx, p));
    out.push_str("</div></div></div>");
    out
}

/// UserInfoList's items: each user (DUserInfo) with their cakedate.
fn items(cx: &ListContext, p: &ListPage) -> String {
    let birthday = p.kind == Kind::Birthdays;
    let pattern = t(
        cx,
        if birthday {
            "dates.full_no_year_no_time"
        } else {
            "dates.full_with_year_no_time"
        },
    );
    let mut out = String::new();
    for u in p.doc[p.kind.key()].as_array().into_iter().flatten() {
        let date = u["cakedate"]
            .as_str()
            .and_then(|s| s.get(..10))
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .map(|d| format_date(d, &pattern))
            .unwrap_or_default();
        let info = crate::user_info_view::user_info(cx, u, None);
        // DUserInfo's block: the details after the title.
        let info = match info.strip_suffix("</div></div>") {
            Some(head) => format!(
                "{head}<div class=\"details\"><div>{}</div></div></div></div>",
                escape(&date)
            ),
            None => info,
        };
        out.push_str(&format!("<li class=\"user-info-item\">{info}</li>"));
    }
    out
}

/// The page's own path with its params.
fn page_path(cx: &ListContext, p: &ListPage, page: i64) -> String {
    let mut query = Vec::new();
    if p.filter == Filter::All {
        query.push(format!("month={}", p.month));
    }
    if page > 0 {
        query.push(format!("page={page}"));
    }
    format!(
        "{}/cakeday/{}/{}{}{}",
        cx.base_path,
        p.kind.key(),
        p.filter.segment(),
        if query.is_empty() { "" } else { "?" },
        query.join("&")
    )
}

/// DLoadMore's sentinel: while more users are left, revealing it asks for
/// the next page, whose items join the list and whose sentinel takes its
/// place.
fn sentinel(cx: &ListContext, p: &ListPage) -> String {
    let total = p.doc[format!("total_rows_{}", p.kind.key())]
        .as_i64()
        .unwrap_or(0);
    let more = (p.page + 1) * PAGE_SIZE < total;
    if !more {
        return "<div aria-hidden=\"true\" class=\"load-more-sentinel\" id=\"cakeday-load-more\"></div>".into();
    }
    format!(
        "<div aria-hidden=\"true\" class=\"load-more-sentinel\" id=\"cakeday-load-more\" hx-get=\"{}\" hx-trigger=\"revealed\" hx-select=\".user-info-list > li\" hx-target=\".cakeday .user-info-list\" hx-swap=\"beforeend\" hx-select-oob=\"#cakeday-load-more\" hx-push-url=\"false\"></div>",
        attr(&page_path(cx, p, p.page + 1))
    )
}

/// The All page's month ComboBox: each month a link to its list.
fn month_chooser(cx: &ListContext, p: &ListPage) -> String {
    let current = usize::try_from(p.month - 1)
        .ok()
        .and_then(|i| MONTHS.get(i))
        .copied()
        .unwrap_or_default();
    let mut rows = String::new();
    for (i, name) in MONTHS.iter().enumerate() {
        let value = i as i64 + 1;
        let selected = value == p.month;
        let path = page_path(
            cx,
            &ListPage {
                month: value,
                page: 0,
                ..*p
            },
            0,
        );
        rows.push_str(&format!(
            "<li aria-checked=\"{selected}\" role=\"menuitemradio\" data-index=\"{i}\" data-name=\"{name}\" data-value=\"{value}\" title=\"{name}\" class=\"select-kit-row{}\"><a href=\"{}\"><span class=\"name\">{name}</span></a></li>",
            if selected { " is-highlighted is-selected" } else { "" },
            attr(&path)
        ));
    }
    format!(
        "<details class=\"select-kit single-select combobox combo-box has-selection\"><summary class=\"select-kit-header single-select-header combo-box-header\" data-name=\"{current}\" data-value=\"{}\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{current}\" title=\"{current}\"><span class=\"name\">{current}</span></div>{}</div></summary><div class=\"select-kit-body\"><ul class=\"select-kit-collection\" role=\"menu\">{rows}</ul></div></details>",
        p.month,
        icon("angle-down", Some("angle-icon"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ui(today: NaiveDate) -> CakedayUi {
        CakedayUi {
            cakeday_emoji: "cake".into(),
            birthday_emoji: Some("birthday".into()),
            today,
        }
    }

    #[test]
    fn anniversaries_need_an_earlier_year_and_birthdays_dont() {
        let ui = ui(NaiveDate::from_ymd_opt(2020, 6, 10).unwrap());
        assert!(ui.same_day(&json!("2019-06-10"), true));
        assert!(!ui.same_day(&json!("2020-06-10"), true));
        assert!(!ui.same_day(&json!("2019-06-11"), true));
        assert!(ui.same_day(&json!("2006-06-10"), false));
        assert!(ui.same_day(&json!("2020-06-10"), false));
        assert!(!ui.same_day(&Value::Null, false));
    }
}
