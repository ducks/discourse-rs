//! The user profile as the Ember app renders it (templates/user.gjs,
//! user/collapsed-info.gjs, components/user-nav.gjs, user/summary.gjs and
//! the summary components), from the users#show and users#summary
//! documents.
//!
//! Not drawn yet: the staff counters, the profile background, user fields,
//! the featured topic, status messages and the mute/ignore dropdown.

use chrono::{DateTime, Datelike, Utc};
use serde_json::Value;

use crate::topic_list_view::{
    ListContext, category_badge, escape, icon, long_date, relative_age_tiny, t, t_count, t_with,
};

/// Who is looking.
pub struct Viewer {
    pub id: Option<i32>,
    pub admin: bool,
    pub staff: bool,
}

/// The site settings the profile reads.
pub struct ProfileSettings {
    pub enable_badges: bool,
    pub hide_user_activity_tab: bool,
}

fn date(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// `dNumber`: the short form, titled with the full number when they differ.
fn number(cx: &ListContext, n: i64) -> String {
    let short = if n > 999_999 {
        t_with(
            cx,
            "number.short.millions",
            &[("number", &format!("{:.1}", n as f64 / 1_000_000.0))],
        )
    } else if n > 99_999 {
        t_with(
            cx,
            "number.short.thousands",
            &[("number", &(n / 1000).to_string())],
        )
    } else if n > 999 {
        t_with(
            cx,
            "number.short.thousands",
            &[("number", &format!("{:.1}", n as f64 / 1000.0))],
        )
    } else {
        n.to_string()
    };
    let full = n.to_string();
    if short == full {
        format!("<span class=\"number\">{short}</span>")
    } else {
        format!(
            "<span class=\"number\" title=\"{full}\">{}</span>",
            escape(&short)
        )
    }
}

/// `duration(seconds, {format})`
fn duration(cx: &ListContext, seconds: i64, format: &str) -> String {
    let minutes = ((seconds as f64 / 60.0).round() as i64).max(1);
    let key = |k: &str| format!("dates.{format}.{k}");
    match () {
        _ if seconds <= 59 => t_count(cx, &key("less_than_x_minutes"), 1, &[]),
        _ if minutes <= 44 => t_count(cx, &key("x_minutes"), minutes, &[]),
        _ if minutes <= 89 => t_count(cx, &key("about_x_hours"), 1, &[]),
        _ if minutes <= 1409 => t_count(
            cx,
            &key("about_x_hours"),
            (minutes as f64 / 60.0).round() as i64,
            &[],
        ),
        _ if minutes <= 2519 => t_count(cx, &key("x_days"), 1, &[]),
        _ if minutes <= 129_599 => t_count(
            cx,
            &key("x_days"),
            (minutes as f64 / 1440.0).round() as i64,
            &[],
        ),
        _ if minutes <= 525_599 => t_count(
            cx,
            &key("x_months"),
            (minutes as f64 / 43200.0).round() as i64,
            &[],
        ),
        _ => {
            let years = minutes as f64 / 525_600.0;
            let remainder = years % 1.0;
            let whole = years.floor() as i64;
            if remainder < 0.25 {
                t_count(cx, &key("about_x_years"), whole, &[])
            } else if remainder < 0.75 {
                t_count(cx, &key("over_x_years"), whole, &[])
            } else {
                t_count(cx, &key("almost_x_years"), whole + 1, &[])
            }
        }
    }
}

/// `dAgeWithTooltip(date, format="medium")`: relativeAgeMedium unwrapped.
fn age_medium(cx: &ListContext, at: DateTime<Utc>) -> String {
    let distance = ((cx.now - at).num_milliseconds() as f64 / 1000.0).round() as i64;
    let text = if distance < 60 {
        t(cx, "now")
    } else if distance > 432_000 {
        let format = if at.year() == cx.now.year() {
            t(cx, "dates.tiny.date_month")
        } else {
            t(cx, "dates.tiny.date_year")
        };
        crate::pretty_text::render::local_dates::format_utc(at, &format).unwrap_or_default()
    } else {
        let minutes = (distance as f64 / 60.0).round() as i64;
        let (unit, count) = match minutes {
            1..=55 => ("x_minutes", minutes),
            56..=89 => ("x_hours", 1),
            90..=1409 => ("x_hours", (minutes as f64 / 60.0).round() as i64),
            1410..=2519 => ("x_days", 1),
            2520..=129_599 => ("x_days", (minutes as f64 / 1440.0).round() as i64),
            129_600..=525_599 => ("x_months", (minutes as f64 / 43200.0).round() as i64),
            _ => ("x_years", (minutes as f64 / 525_600.0).round() as i64),
        };
        t_count(cx, &format!("dates.medium.{unit}"), count, &[])
    };
    format!(
        "<span class=\"relative-date date\" title=\"{}\" data-time=\"{}\" data-format=\"medium\">{}</span>",
        escape(&long_date(cx, at)),
        at.timestamp_millis(),
        escape(&text)
    )
}

/// `i18n(key, {count})`: the plural form when the key has them, else the
/// plain string.
fn label(cx: &ListContext, key: &str, count: i64) -> String {
    cx.i18n
        .t_count(&format!("js.{key}"), count, &[])
        .unwrap_or_else(|| t(cx, key))
}

/// `iconHTML(name, {label})`
fn labeled_icon(name: &str, label: &str) -> String {
    format!(
        "{}<span class=\"sr-only\">{}</span>",
        icon(name, None),
        escape(label)
    )
}

fn avatar(cx: &ListContext, template: &str, size: u32) -> String {
    format!(
        "<img alt=\"\" width=\"{size}\" height=\"{size}\" src=\"{}{}\" class=\"avatar\">",
        if template.starts_with('/') && !template.starts_with("//") {
            cx.base_path
        } else {
            ""
        },
        escape(&template.replace("{size}", &size.to_string()))
    )
}

/// The `.user-main` section: the about panel, the user navigation and,
/// unless the profile is hidden, the summary.
pub fn render(
    cx: &ListContext,
    settings: &ProfileSettings,
    viewer: &Viewer,
    show: &Value,
    summary: Option<&Value>,
) -> String {
    let u = &show["user"];
    let base = cx.base_path;
    let username = s(&u["username"]);
    let name = u["name"].as_str().filter(|n| !n.trim().is_empty());
    let viewing_self = viewer.id.is_some() && viewer.id == u["id"].as_i64().map(|i| i as i32);
    let hidden = u["profile_hidden"] == true;
    // collapsedInfo on the summary route: hidden profiles and one's own.
    let collapsed = hidden || viewing_self;
    let name_first = cx.settings.prioritize_name && name.is_some();
    let user_path = format!("{base}/u/{}", escape(&username.to_lowercase()));

    let mut out = String::new();
    out.push_str(&format!(
        "<div class=\"container{}{}{}\"><section class=\"user-main\">",
        if viewing_self { " viewing-self" } else { "" },
        if hidden { " profile-hidden" } else { "" },
        u["primary_group_name"]
            .as_str()
            .map(|g| format!(" group-{}", escape(g)))
            .unwrap_or_default()
    ));
    out.push_str(&format!(
        "<a class=\"skip-link__user-nav\" href=\"#user-content\" id=\"user-nav-skip-link\">{}</a>",
        escape(&t(cx, "skip_user_nav"))
    ));
    out.push_str(&format!(
        "<section class=\"{}about no-background\">",
        if collapsed { "collapsed-info " } else { "" }
    ));
    if !collapsed {
        out.push_str("<div class=\"user-profile-image\"></div>");
    }
    out.push_str("<div class=\"details\"><div class=\"primary\">");
    // UserProfileAvatar
    out.push_str(&format!(
        "<div class=\"user-profile-avatar\">{}<div></div></div>",
        avatar(cx, s(&u["avatar_template"]), 144)
    ));
    // primary-textual
    out.push_str("<div class=\"primary-textual\"><div class=\"user-profile-names\">");
    let status = {
        let label_name = name.unwrap_or_default();
        if u["admin"] == true && viewer.staff {
            labeled_icon(
                "shield-halved",
                &t_with(cx, "user.admin", &[("user", label_name)]),
            )
        } else if u["moderator"] == true {
            labeled_icon(
                "shield-halved",
                &t_with(cx, "user.moderator", &[("user", label_name)]),
            )
        } else {
            String::new()
        }
    };
    let (first, second, first_class, second_class) = if name_first {
        (name.unwrap_or_default(), username, "full-name", "username")
    } else {
        (username, name.unwrap_or_default(), "username", "full-name")
    };
    out.push_str(&format!(
        "<div class=\"{first_class} user-profile-names__primary\">{} {status}</div>\
         <div class=\"{second_class} user-profile-names__secondary\">{}</div>",
        escape(first),
        escape(second)
    ));
    if u["staged"] == true {
        out.push_str(&format!(
            "<div class=\"staged user-profile-names__secondary\">{}</div>",
            escape(&t(cx, "user.staged"))
        ));
    }
    if let Some(title) = u["title"].as_str().filter(|t| !t.is_empty()) {
        out.push_str(&format!(
            "<div class=\"user-profile-names__title\">{}</div>",
            escape(title)
        ));
    }
    out.push_str("</div><div class=\"location-and-website user-profile__location-and-website\">");
    if let Some(location) = u["location"].as_str().filter(|l| !l.is_empty()) {
        out.push_str(&format!(
            "<div class=\"user-profile-location\">{}{}</div>",
            icon("location-dot", None),
            escape(location)
        ));
    }
    if let Some(site) = u["website_name"].as_str().filter(|w| !w.is_empty()) {
        // linkWebsite: not for basic users; nofollow below TL3.
        let basic = u["trust_level"].as_i64().unwrap_or(0) < 2 && u["admin"] != true;
        let link = if basic {
            format!(
                "<span title=\"{}\">{}</span>",
                escape(s(&u["website"])),
                escape(site)
            )
        } else {
            let rel = if u["trust_level"].as_i64().unwrap_or(0) > 2 {
                "noopener"
            } else {
                "noopener nofollow ugc"
            };
            format!(
                "<a href=\"{}\" rel=\"{rel}\" target=\"_blank\">{}</a>",
                escape(s(&u["website"])),
                escape(site)
            )
        };
        out.push_str(&format!(
            "<div class=\"user-profile-website\">{}{link}</div>",
            icon("globe", None)
        ));
    }
    out.push_str("</div><div class=\"bio\"><div>");
    let restricted = u["suspended_till"].is_string() || u["silenced_till"].is_string();
    if !restricted || viewer.staff {
        out.push_str(s(&u["bio_cooked"]));
    }
    out.push_str("</div></div></div>");
    // controls
    out.push_str("<section class=\"controls\"><ul>");
    if u["can_send_private_message_to_user"] == true && !viewing_self {
        out.push_str(&format!(
            "<li><button class=\"btn btn-icon-text btn-primary compose-pm\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button></li>",
            icon("envelope", None),
            escape(&t(cx, "user.private_message"))
        ));
    }
    if !hidden && viewing_self {
        out.push_str(&format!(
            "<li><button aria-controls=\"collapsed-info-panel\" aria-expanded=\"false\" aria-label=\"{}\" class=\"btn btn-icon-text btn-default user-profile-toggle-btn\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button></li>",
            escape(&t(cx, "user.sr_expand_profile")),
            icon("angles-down", None),
            escape(&t(cx, "user.expand_profile"))
        ));
    }
    out.push_str("</ul></section></div>");
    if !collapsed {
        out.push_str(&collapsed_info(cx, u, viewer));
    }
    out.push_str("</div></section>");

    // UserNav
    out.push_str(
        "<div class=\"new-user-wrapper\"><section class=\"user-navigation user-navigation-primary\">\
         <nav aria-label=\"User primary\" class=\"horizontal-overflow-nav\">\
         <div class=\"d-overflow-controls --owned-scroller horizontal-overflow-nav__controls\">\
         <ul class=\"nav-pills action-list main-nav nav user-nav\" data-d-scroll-axis=\"horizontal\">",
    );
    let tab = |class: &str, href: &str, icon_name: &str, label: &str, current: bool| {
        format!(
            "<li{} class=\"{class}\"><a class=\"{}\" href=\"{href}\">{} <span>{}</span></a></li>",
            if current {
                " aria-current=\"page\""
            } else {
                ""
            },
            if current { "active" } else { "" },
            icon(icon_name, None),
            escape(label)
        )
    };
    if !hidden {
        out.push_str(&tab(
            "user-nav__summary",
            &format!("{user_path}/summary"),
            "user",
            &t(cx, "user.summary.title"),
            true,
        ));
        if viewing_self || viewer.admin || !settings.hide_user_activity_tab {
            out.push_str(&tab(
                "user-nav__activity",
                &format!("{user_path}/activity"),
                "bars-staggered",
                &t(cx, "user.activity_stream"),
                false,
            ));
        }
    }
    if viewing_self || viewer.admin {
        out.push_str(&tab(
            "user-nav__notifications",
            &format!("{user_path}/notifications"),
            "bell",
            &t(cx, "user.notifications"),
            false,
        ));
    }
    if settings.enable_badges && u["badge_count"].as_i64().unwrap_or(0) > 0 {
        out.push_str(&tab(
            "user-nav__badges",
            &format!("{user_path}/badges"),
            "certificate",
            &t(cx, "badges.title"),
            false,
        ));
    }
    if u["can_edit"] == true {
        out.push_str(&tab(
            "user-nav__preferences",
            &format!("{user_path}/preferences"),
            "gear",
            &t(cx, "user.preferences.title"),
            false,
        ));
    }
    out.push_str("</ul></div></nav></section><div class=\"new-user-content-wrapper\">");
    if hidden {
        // user/profile-hidden.gjs
        out.push_str(&format!(
            "<p class=\"user-profile-hidden\">{}</p>",
            escape(&t(cx, "user.profile_hidden"))
        ));
    } else if let Some(summary) = summary {
        out.push_str(&summary_content(
            cx, settings, &user_path, username, summary,
        ));
    }
    out.push_str("</div></div></section></div>");
    out
}

/// CollapsedInfo's secondary panel.
fn collapsed_info(cx: &ListContext, u: &Value, viewer: &Viewer) -> String {
    let mut out = String::from("<div class=\"secondary\" id=\"collapsed-info-panel\"><dl>");
    for (key, class, label) in [
        ("created_at", "created-at", "user.created"),
        ("last_posted_at", "last-posted-at", "user.last_posted"),
        ("last_seen_at", "last-seen-at", "user.last_seen"),
    ] {
        if let Some(at) = date(&u[key]) {
            out.push_str(&format!(
                "<div><dt class=\"{class}\">{}</dt><dd class=\"{class}\">{}</dd></div>",
                escape(&t(cx, label)),
                age_medium(cx, at)
            ));
        }
    }
    if let Some(views) = u["profile_view_count"].as_i64().filter(|v| *v > 0) {
        out.push_str(&format!(
            "<div><dt class=\"profile-view-count\">{}</dt><dd class=\"profile-view-count\">{views}</dd></div>",
            escape(&t(cx, "views"))
        ));
    }
    if let Some(by) = u["invited_by"]["username"].as_str() {
        out.push_str(&format!(
            "<div><dt class=\"invited-by\">{}</dt><dd class=\"invited-by\"><a href=\"{}/u/{}\">{}</a></dd></div>",
            escape(&t(cx, "user.invited_by")),
            cx.base_path,
            escape(&by.to_lowercase()),
            escape(by)
        ));
    }
    // hasTrustLevel: any level, zero included.
    if let Some(level) = u["trust_level"].as_i64() {
        let names = ["newuser", "basic", "member", "regular", "leader"];
        let name = names.get(level as usize).copied().unwrap_or_default();
        out.push_str(&format!(
            "<div><dt class=\"trust-level\">{}</dt><dd class=\"trust-level\">{}</dd></div>",
            escape(&t(cx, "user.trust_level")),
            escape(&t(cx, &format!("trust_levels.names.{name}")))
        ));
    }
    let _ = viewer;
    out.push_str("</dl></div>");
    out
}

/// `user/summary.gjs`
fn summary_content(
    cx: &ListContext,
    settings: &ProfileSettings,
    user_path: &str,
    username: &str,
    doc: &Value,
) -> String {
    let m = &doc["user_summary"];
    let mut out = String::from("<div class=\"user-content\" id=\"user-content\">");
    let n = |key: &str| m[key].as_i64().unwrap_or(0);
    let stat = |value: String, label: String, icon_name: Option<&str>, title: Option<String>| {
        format!(
            "<div class=\"user-stat\"><span class=\"value\"{}>{value}</span> <span class=\"label\">{}{label}</span></div>",
            title
                .map(|t| format!(" title=\"{}\"", escape(&t)))
                .unwrap_or_default(),
            icon_name.map(|i| icon(i, None)).unwrap_or_default()
        )
    };
    let count_stat = |key: &str, label_key: &str, icon_name: Option<&str>| {
        stat(
            number(cx, n(key)),
            label(cx, &format!("user.summary.{label_key}"), n(key)),
            icon_name,
            None,
        )
    };
    let linked = m["can_see_user_actions"] == true;
    let li = |class: &str, href: Option<String>, inner: String| match href.filter(|_| linked) {
        Some(href) => {
            format!("<li class=\"{class} linked-stat\"><a href=\"{href}\">{inner}</a></li>")
        }
        None => format!("<li class=\"{class}\">{inner}</li>"),
    };
    if m["can_see_summary_stats"] == true {
        out.push_str(&format!(
            "<div class=\"top-section stats-section\"><h3 class=\"stats-title\">{}</h3><ul>",
            escape(&t(cx, "user.summary.stats"))
        ));
        out.push_str(&li(
            "stats-days-visited",
            None,
            count_stat("days_visited", "days_visited", None),
        ));
        let time_read = n("time_read");
        out.push_str(&li(
            "stats-time-read",
            None,
            stat(
                escape(&duration(cx, time_read, "tiny")),
                label(cx, "user.summary.time_read", time_read),
                None,
                Some(t_with(
                    cx,
                    "user.summary.time_read_title",
                    &[("duration", &duration(cx, time_read, "medium"))],
                )),
            ),
        ));
        let recent = n("recent_time_read");
        if recent != time_read && recent != 0 {
            out.push_str(&li(
                "stats-recent-read",
                None,
                stat(
                    escape(&duration(cx, recent, "tiny")),
                    label(cx, "user.summary.recent_time_read", recent),
                    None,
                    Some(t_with(
                        cx,
                        "user.summary.recent_time_read_title",
                        &[("duration", &duration(cx, recent, "medium"))],
                    )),
                ),
            ));
        }
        out.push_str(&li(
            "stats-topics-entered",
            None,
            count_stat("topics_entered", "topics_entered", None),
        ));
        out.push_str(&li(
            "stats-posts-read",
            None,
            count_stat("posts_read_count", "posts_read", None),
        ));
        out.push_str(&li(
            "stats-likes-given",
            Some(format!("{user_path}/activity/likes-given")),
            count_stat("likes_given", "likes_given", Some("heart")),
        ));
        out.push_str(&li(
            "stats-likes-received",
            None,
            count_stat("likes_received", "likes_received", Some("heart")),
        ));
        if n("bookmark_count") > 0 {
            out.push_str(&li(
                "stats-bookmark-count",
                Some(format!("{user_path}/activity/bookmarks")),
                count_stat("bookmark_count", "bookmark_count", None),
            ));
        }
        out.push_str(&li(
            "stats-topic-count",
            Some(format!("{user_path}/activity/topics")),
            count_stat("topic_count", "topic_count", None),
        ));
        out.push_str(&li(
            "stats-post-count",
            Some(format!("{user_path}/activity/replies")),
            count_stat("post_count", "post_count", None),
        ));
        out.push_str("</ul></div>");
    }

    let section = |class: &str, title: &str, inner: String| {
        format!(
            "<div class=\"top-sub-section {class}\"><h3 class=\"stats-title\">{}</h3>{inner}</div>",
            escape(&t(cx, &format!("user.summary.{title}")))
        )
    };
    let topics: Vec<&Value> = doc["topics"].as_array().into_iter().flatten().collect();
    let topic_by_id = |id: i64| {
        topics
            .iter()
            .find(|t| t["id"].as_i64() == Some(id))
            .copied()
    };
    let topic_item = |title: &str, url: &str, created: &Value, likes: i64| {
        let mut info = created
            .as_str()
            .and_then(|_| date(created))
            .map(|at| {
                format!(
                    "<span class=\"relative-date\" data-time=\"{}\" data-format=\"tiny\">{}</span>",
                    at.timestamp_millis(),
                    escape(&relative_age_tiny(cx, at))
                )
            })
            .unwrap_or_default();
        if likes > 0 {
            info.push_str(&format!(
                " &middot; {}&nbsp;<span class=\"like-count\">{}</span>",
                icon("heart", None),
                number(cx, likes)
            ));
        }
        format!(
            "<li><span class=\"topic-info\">{info}</span><br><a href=\"{url}\">{title}</a></li>"
        )
    };
    let list_or_none = |items: Vec<String>, more: Option<(String, String)>, none: &str| {
        if items.is_empty() {
            format!("<p>{}</p>", escape(&t(cx, &format!("user.summary.{none}"))))
        } else {
            let mut s = format!("<ul>{}</ul>", items.join(""));
            if let Some((href, label)) = more {
                s.push_str(&format!(
                    "<p><a class=\"more\" href=\"{href}\">{label}</a></p>"
                ));
            }
            s
        }
    };
    // UserSummaryTopicsList#hasMore: a full list of six.
    let more = |kind: &str, len: usize| {
        (len >= 6).then(|| {
            (
                format!("{user_path}/activity/{kind}"),
                escape(&t(cx, &format!("user.summary.more_{kind}"))),
            )
        })
    };
    let replies: Vec<String> = m["replies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let topic = topic_by_id(r["topic_id"].as_i64()?)?;
            let url = format!(
                "{}/t/{}/{}/{}",
                cx.base_path,
                s(&topic["slug"]),
                topic["id"],
                r["post_number"]
            );
            Some(topic_item(
                s(&topic["fancy_title"]),
                &url,
                &r["created_at"],
                r["like_count"].as_i64().unwrap_or(0),
            ))
        })
        .collect();
    let replies_len = replies.len();
    let top_topics: Vec<String> = m["topic_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|id| {
            let topic = topic_by_id(id.as_i64()?)?;
            let url = format!("{}/t/{}/{}", cx.base_path, s(&topic["slug"]), topic["id"]);
            Some(topic_item(
                s(&topic["fancy_title"]),
                &url,
                &topic["created_at"],
                topic["like_count"].as_i64().unwrap_or(0),
            ))
        })
        .collect();
    let topics_len = top_topics.len();
    out.push_str(&format!(
        "<div class=\"top-section replies-and-topics-section\">{}{}</div>",
        section(
            "replies-section pull-left",
            "top_replies",
            list_or_none(replies, more("replies", replies_len), "no_replies")
        ),
        section(
            "topics-section pull-right",
            "top_topics",
            list_or_none(top_topics, more("topics", topics_len), "no_topics")
        )
    ));

    let links: Vec<String> = m["links"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|l| {
            let url = s(&l["url"]);
            let clicks = l["clicks"].as_i64().unwrap_or(0);
            let post_url = format!(
                "{}/t/{}/{}/{}",
                cx.base_path,
                s(&l["topic"]["slug"]),
                l["topic"]["id"],
                l["post_number"]
            );
            format!(
                "<li><a aria-label=\"{}\" class=\"domain\" data-clicks=\"{clicks}\" href=\"{}\" rel=\"noopener nofollow ugc\" target=\"_blank\" title=\"{}\">{}</a><br><a href=\"{post_url}\">{}</a></li>",
                escape(&t_count(cx, "topic_map.clicks", clicks, &[])),
                escape(url),
                escape(s(&l["title"])),
                escape(&shorten_url(url)),
                s(&l["topic"]["fancy_title"])
            )
        })
        .collect();
    let users_list = |key: &str, count_class: &str, icon_name: &str, none: &str| {
        let users: Vec<String> = m[key]
            .as_array()
            .into_iter()
            .flatten()
            .map(|u| {
                format!(
                    "<li>{}</li>",
                    user_info(
                        cx,
                        u,
                        &format!(
                            "{} <span class=\"{count_class}\">{}</span>",
                            icon(icon_name, None),
                            number(cx, u["count"].as_i64().unwrap_or(0))
                        )
                    )
                )
            })
            .collect();
        if users.is_empty() {
            format!(
                "<div><p>{}</p></div>",
                escape(&t(cx, &format!("user.summary.{none}")))
            )
        } else {
            format!("<div><ul>{}</ul></div>", users.join(""))
        }
    };
    out.push_str(&format!(
        "<div class=\"top-section links-and-replied-to-section\">{}{}</div>",
        section(
            "links-section pull-left",
            "top_links",
            list_or_none(links, None, "no_links")
        ),
        section(
            "summary-user-list replied-section pull-right",
            "most_replied_to_users",
            users_list("most_replied_to_users", "replies", "reply", "no_replies")
        )
    ));
    out.push_str(&format!(
        "<div class=\"top-section most-liked-section\">{}{}</div>",
        section(
            "summary-user-list liked-by-section pull-left",
            "most_liked_by",
            users_list("most_liked_by_users", "likes", "heart", "no_likes")
        ),
        section(
            "summary-user-list liked-section pull-right",
            "most_liked_users",
            users_list("most_liked_users", "likes", "heart", "no_likes")
        )
    ));

    let categories: Vec<&Value> = m["top_categories"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    if !categories.is_empty() {
        let mut rows = String::new();
        for c in categories {
            let Some(category) = cx.categories.get(&c["id"].as_i64().unwrap_or(0)) else {
                continue;
            };
            let search = |count: i64, first: bool| {
                if count == 0 {
                    return "&ndash;".to_string();
                }
                let mut q = format!("@{username} #{}", category.slug);
                if first {
                    q.push_str(" in:first");
                }
                format!(
                    "<a href=\"{}/search?q={}\">{count}</a>",
                    cx.base_path,
                    uri_component(&q)
                )
            };
            rows.push_str(&format!(
                "<tr><td class=\"category-link\">{}</td><td class=\"topic-count\">{}</td><td class=\"reply-count\">{}</td></tr>",
                category_badge(cx, category),
                search(c["topic_count"].as_i64().unwrap_or(0), true),
                search(c["post_count"].as_i64().unwrap_or(0), false)
            ));
        }
        out.push_str(&format!(
            "<div class=\"top-section top-categories-section\">{}</div>",
            section(
                "summary-category-list pull-left",
                "top_categories",
                format!(
                    "<table><thead><tr><th class=\"category-link\"></th><th class=\"topic-count\">{}</th><th class=\"reply-count\">{}</th></tr></thead><tbody>{rows}</tbody></table>",
                    escape(&t(cx, "user.summary.topics")),
                    escape(&t(cx, "user.summary.replies"))
                )
            )
        ));
    }

    if settings.enable_badges {
        out.push_str(&format!(
            "<div class=\"top-section badges-section\"><h3 class=\"stats-title\">{}</h3>",
            escape(&t(cx, "user.summary.top_badges"))
        ));
        let user_badges: Vec<&Value> = m["badges"].as_array().into_iter().flatten().collect();
        let defs: Vec<&Value> = doc["badges"].as_array().into_iter().flatten().collect();
        if user_badges.is_empty() {
            out.push_str(&format!(
                "<p>{}</p>",
                escape(&t(cx, "user.summary.no_badges"))
            ));
        } else {
            out.push_str("<div class=\"badge-group-list\">");
            for ub in &user_badges {
                let Some(b) = defs.iter().find(|d| d["id"] == ub["badge_id"]).copied() else {
                    continue;
                };
                out.push_str(&badge_card(cx, b, ub["count"].as_i64()));
            }
            out.push_str("</div>");
        }
        if user_badges.len() >= 6 {
            out.push_str(&format!(
                "<a class=\"more\" href=\"{user_path}/badges\">{}</a>",
                escape(&t(cx, "user.summary.more_badges"))
            ));
        }
        out.push_str("</div>");
    }
    out.push_str("</div>");
    out
}

/// `DUserInfo` with its yielded details.
fn user_info(cx: &ListContext, u: &Value, details: &str) -> String {
    let username = s(&u["username"]);
    let path = format!("{}/u/{}", cx.base_path, escape(&username.to_lowercase()));
    let name_first =
        cx.settings.prioritize_name && u["name"].as_str().is_some_and(|n| !n.trim().is_empty());
    let mut name = String::new();
    if let Some(n) = u["name"].as_str().filter(|n| !n.is_empty()) {
        name = format!(
            "<span class=\"name-wrapper\"><a data-user-card=\"{}\" href=\"{path}\"><span class=\"name\">{}</span></a></span>",
            escape(username),
            escape(n)
        );
    }
    format!(
        "<div class=\"user-info medium\" data-username=\"{0}\"><div class=\"user-image\"><div class=\"user-image-inner\"><a aria-hidden=\"true\" data-user-card=\"{0}\" href=\"{path}\">{1}</a></div></div>\
         <div class=\"user-detail\"><div class=\"name-line{2}\"><span class=\"username-wrapper\"><a data-user-card=\"{0}\" href=\"{path}\"><span class=\"username\">{0}</span></a></span>{name}</div>\
         <div class=\"title\">{3}</div><div class=\"details\">{details}</div></div></div>",
        escape(username),
        avatar(cx, s(&u["avatar_template"]), 48),
        if name_first { " --name-first" } else { "" },
        escape(s(&u["title"]))
    )
}

/// `DBadgeCard` at the medium size.
fn badge_card(cx: &ListContext, b: &Value, count: Option<i64>) -> String {
    let slug = s(&b["slug"]);
    let type_class = match b["badge_type_id"].as_i64() {
        Some(1) => "badge-type-gold",
        Some(2) => "badge-type-silver",
        _ => "badge-type-bronze",
    };
    let image = match b["image_url"].as_str().filter(|i| !i.is_empty()) {
        Some(url) => format!("<img src=\"{}\" alt=\"\">", escape(url)),
        None => icon(s(&b["icon"]), None),
    };
    let display = match count {
        None => b["grant_count"].as_i64(),
        Some(c) if c > 1 => Some(c),
        Some(_) => None,
    };
    let mut described = format!("badge-summary-{slug}");
    let mut granted = String::new();
    if let Some(c) = display {
        described.push_str(&format!(" badge-granted-{slug}"));
        granted = format!(
            "<div class=\"badge-granted\" id=\"badge-granted-{slug}\">{}</div>",
            t_count(cx, "badges.awarded", c, &[("number", &c.to_string())])
        );
    }
    format!(
        "<div class=\"badge-card --badge-medium\" data-badge-slug=\"{slug}\"><div class=\"badge-contents\">\
         <span aria-hidden=\"true\" class=\"badge-icon {type_class}\">{image}</span>\
         <div class=\"badge-info\"><div class=\"badge-info-item\"><h3><a aria-describedby=\"{described}\" class=\"badge-link\" href=\"{}/badges/{}/{slug}\">{}</a></h3>\
         <div class=\"badge-summary\" id=\"badge-summary-{slug}\">{}</div>{granted}</div></div></div></div>",
        cx.base_path,
        b["id"],
        escape(s(&b["name"])),
        s(&b["description"])
    )
}

/// `shortenUrl`: a trailing slash off a bare host, the scheme and www off,
/// 80 characters at most.
fn shorten_url(url: &str) -> String {
    let mut u = url.to_string();
    if u.matches('/').count() == 3 && u.ends_with('/') {
        u.pop();
    }
    let u = u
        .strip_prefix("https://")
        .or_else(|| u.strip_prefix("http://"))
        .unwrap_or(&u);
    let u = u.strip_prefix("www.").unwrap_or(u);
    // substring(0, 80) counts UTF-16 units; ASCII URLs are the common case.
    u.chars().take(80).collect()
}

/// `encodeURIComponent`
fn uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
