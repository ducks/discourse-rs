//! The user profile as the Ember app renders it (templates/user.gjs,
//! user/collapsed-info.gjs, components/user-nav.gjs, user/summary.gjs and
//! the summary components, user-activity.gjs and the user stream's
//! PostList), from the users#show, users#summary and user_actions
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
    /// `currentUser.can_send_private_messages`
    pub can_send_private_messages: bool,
    /// `currentUser.draft_count`
    pub draft_count: i64,
    /// chat's `userCanDirectMessage`, for its Chat button.
    pub can_direct_message: bool,
}

/// The routed tab and what it shows.
pub enum Tab<'a> {
    /// `user.summary`; None where the viewer may not see it.
    Summary(Option<&'a Value>),
    /// `userActivity.*`: the filter and the user stream, collapsed
    /// (UserAction.collapseStream).
    Activity {
        filter: ActivityFilter,
        stream: &'a [Value],
        /// The votes list's topics (discourse-topic-voting's
        /// userActivity.votes), empty elsewhere.
        topics: &'a [Value],
        /// The reactions list (discourse-reactions' userActivity.reactions,
        /// UserReactionSerializer), empty elsewhere.
        reactions: &'a [Value],
        /// The solved posts (discourse-solved's userActivity.solved,
        /// SolvedPostSerializer), empty elsewhere.
        solved: &'a [Value],
    },
}

/// The activity sub-navigation's stream routes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActivityFilter {
    All,
    Topics,
    Replies,
    LikesGiven,
    /// discourse-topic-voting's: the topics the user voted on, a topic
    /// list rather than a stream.
    Votes,
    /// discourse-reactions': the user's reactions, a post list of them.
    Reactions,
    /// discourse-solved's: the user's accepted answers, a post list.
    Solved,
}

impl ActivityFilter {
    /// `/u/:username/activity/<path>`
    pub fn from_path(path: &str) -> Option<ActivityFilter> {
        Some(match path {
            "" => ActivityFilter::All,
            "topics" => ActivityFilter::Topics,
            "replies" => ActivityFilter::Replies,
            "likes-given" => ActivityFilter::LikesGiven,
            "votes" => ActivityFilter::Votes,
            "reactions" => ActivityFilter::Reactions,
            "solved" => ActivityFilter::Solved,
            _ => return None,
        })
    }

    /// UserStream#filterParam: the user_actions types the stream loads.
    pub fn action_types(self) -> &'static [i32] {
        match self {
            ActivityFilter::All => &[4, 5],
            ActivityFilter::Topics => &[4],
            // TYPES.posts: the replies route's own posts.
            ActivityFilter::Replies => &[5],
            ActivityFilter::LikesGiven => &[1],
            ActivityFilter::Votes | ActivityFilter::Reactions | ActivityFilter::Solved => &[],
        }
    }

    /// `user-stream` plus the filter's class (UserStream#filterClassName).
    fn class_name(self) -> &'static str {
        match self {
            ActivityFilter::All => "",
            ActivityFilter::Topics => " filter-4",
            ActivityFilter::Replies => " filter-5",
            ActivityFilter::LikesGiven => " filter-1",
            ActivityFilter::Votes | ActivityFilter::Reactions | ActivityFilter::Solved => "",
        }
    }
}

/// The site settings the profile reads.
pub struct ProfileSettings {
    pub enable_badges: bool,
    pub hide_user_activity_tab: bool,
    /// discourse-topic-voting's votes tab (topic_voting_show_votes_on_profile).
    pub show_votes: bool,
    /// discourse-reactions' settings, when it is on: its reactions tab.
    pub reactions: Option<crate::plugins::reactions::view::ReactionsUi>,
    /// discourse-solved is on: its Solved tab and summary stat.
    pub solved: bool,
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
/// `relativeAge(format: "medium")`; `leave_ago` is "medium-with-ago".
fn age_medium(cx: &ListContext, at: DateTime<Utc>, leave_ago: bool) -> String {
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
        let scope = if leave_ago {
            "medium_with_ago"
        } else {
            "medium"
        };
        t_count(cx, &format!("dates.{scope}.{unit}"), count, &[])
    };
    format!(
        "<span class=\"relative-date date\" title=\"{}\" data-time=\"{}\" data-format=\"{}\">{}</span>",
        escape(&long_date(cx, at)),
        at.timestamp_millis(),
        if leave_ago {
            "medium-with-ago"
        } else {
            "medium"
        },
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
    tab: &Tab,
) -> String {
    let u = &show["user"];
    let base = cx.base_path;
    let username = s(&u["username"]);
    let name = u["name"].as_str().filter(|n| !n.trim().is_empty());
    let viewing_self = viewer.id.is_some() && viewer.id == u["id"].as_i64().map(|i| i as i32);
    let hidden = u["profile_hidden"] == true;
    // collapsedInfo: hidden profiles, one's own, and every route but the
    // summary.
    let on_summary = matches!(tab, Tab::Summary(_));
    let collapsed = hidden || viewing_self || !on_summary;
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
    // chat's user-profile-controls connector: ChatDirectMessageButton.
    if u["can_chat_user"] == true && !viewing_self && viewer.can_direct_message {
        out.push_str(&format!(
            "<li class=\"user-card-below-message-button chat-button\"><button class=\"btn btn-icon-text btn-primary chat-direct-message-btn\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button></li>",
            crate::post_view::d_icon("d-chat", None),
            escape(&t(cx, "chat.title_capitalized"))
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
    let nav_item = |class: &str, href: &str, icon_name: &str, label: &str, current: bool| {
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
        out.push_str(&nav_item(
            "user-nav__summary",
            &format!("{user_path}/summary"),
            "user",
            &t(cx, "user.summary.title"),
            on_summary,
        ));
        if viewing_self || viewer.admin || !settings.hide_user_activity_tab {
            out.push_str(&nav_item(
                "user-nav__activity",
                &format!("{user_path}/activity"),
                "bars-staggered",
                &t(cx, "user.activity_stream"),
                !on_summary,
            ));
        }
    }
    if viewing_self || viewer.admin {
        out.push_str(&nav_item(
            "user-nav__notifications",
            &format!("{user_path}/notifications"),
            "bell",
            &t(cx, "user.notifications"),
            false,
        ));
    }
    if viewer.can_send_private_messages && (viewing_self || viewer.admin) {
        out.push_str(&nav_item(
            "user-nav__personal-messages",
            &format!("{user_path}/messages"),
            "envelope",
            &t(cx, "user.private_messages"),
            false,
        ));
    }
    if settings.enable_badges && u["badge_count"].as_i64().unwrap_or(0) > 0 {
        out.push_str(&nav_item(
            "user-nav__badges",
            &format!("{user_path}/badges"),
            "certificate",
            &t(cx, "badges.title"),
            false,
        ));
    }
    if u["can_edit"] == true {
        out.push_str(&nav_item(
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
    } else {
        match tab {
            Tab::Summary(Some(summary)) => out.push_str(&summary_content(
                cx, settings, &user_path, username, summary,
            )),
            Tab::Summary(None) => {}
            Tab::Activity {
                filter,
                stream,
                topics,
                reactions,
                solved,
            } => out.push_str(&activity_content(
                cx,
                settings,
                viewer,
                viewing_self,
                &user_path,
                *filter,
                stream,
                topics,
                reactions,
                solved,
                u["pending_posts_count"].as_i64().unwrap_or(0),
            )),
        }
    }
    out.push_str("</div></div></section></div>");
    out
}

/// discourse-topic-voting's userActivity.votes: the user's voted topics
/// as a paginated topic list without posters, or its empty state.
fn votes_list(cx: &ListContext, viewing_self: bool, user_path: &str, topics: &[Value]) -> String {
    if topics.is_empty() {
        let title = if viewing_self {
            t(cx, "topic_voting.no_votes_title_self")
        } else {
            t_with(
                cx,
                "topic_voting.no_votes_title_others",
                &[("username", user_path.rsplit('/').next().unwrap_or(""))],
            )
        };
        return format!(
            "<div class=\"empty-state\"><span class=\"empty-state__title\">{}</span></div>",
            escape(&title)
        );
    }
    let header = |class: &str, key: &str, sortable: bool| {
        let label = escape(&t(cx, key));
        if sortable {
            format!(
                "<th class=\"topic-list-data {class} sortable num\" data-sort-order=\"{class}\" scope=\"col\"><button>{label}</button></th>"
            )
        } else {
            format!(
                "<th class=\"topic-list-data {class}\" data-sort-order=\"{class}\" scope=\"col\"><span>{label}</span></th>"
            )
        }
    };
    let rows: String = topics
        .iter()
        .map(|topic| crate::topic_list_view::suggested_row(cx, topic))
        .collect();
    format!(
        "<div class=\"paginated-topics-list\"><div class=\"row dismiss-container-top\"></div><div><div class=\"loading-container\">\
         <table class=\"topic-list\"><caption class=\"sr-only\">{}</caption><thead class=\"topic-list-header\"><tr>{}{}{}{}</tr></thead>\
         <tbody class=\"topic-list-body\">{rows}</tbody></table></div></div><div class=\"loading-container\"></div>\
         <div aria-hidden=\"true\" class=\"load-more-sentinel\"></div></div>",
        escape(&t(cx, "sr_topic_list_caption")),
        header("default", "topic.title", false),
        header("posts", "replies", true),
        header("views", "views", true),
        header("activity", "activity", true),
    )
}

/// `user-activity.gjs`: the secondary navigation, then the user stream
/// (`user/stream.gjs`).
#[allow(clippy::too_many_arguments)]
fn activity_content(
    cx: &ListContext,
    settings: &ProfileSettings,
    viewer: &Viewer,
    viewing_self: bool,
    user_path: &str,
    filter: ActivityFilter,
    stream: &[Value],
    topics: &[Value],
    reactions: &[Value],
    solved: &[Value],
    pending_posts_count: i64,
) -> String {
    let mut out = String::from(
        "<div class=\"user-navigation user-navigation-secondary\">\
         <nav aria-label=\"User secondary - activity\" class=\"horizontal-overflow-nav\">\
         <div class=\"d-overflow-controls --owned-scroller horizontal-overflow-nav__controls\">\
         <ul class=\"nav-pills action-list\" data-d-scroll-axis=\"horizontal\">",
    );
    let item = |class: &str, path: &str, icon_name: &str, label: &str, title: Option<&str>| {
        let current = ActivityFilter::from_path(path) == Some(filter);
        format!(
            "<li{} class=\"{class}\"{}><a class=\"{}\" href=\"{user_path}/activity{}{path}\">{} <span>{}</span></a></li>",
            if current {
                " aria-current=\"location\""
            } else {
                ""
            },
            title
                .map(|t| format!(" title=\"{}\"", escape(t)))
                .unwrap_or_default(),
            if current { "active" } else { "" },
            if path.is_empty() { "" } else { "/" },
            icon(icon_name, None),
            escape(label)
        )
    };
    out.push_str(&item(
        "user-nav__activity-all",
        "",
        "bars-staggered",
        &t(cx, "user.filters.all"),
        None,
    ));
    out.push_str(&item(
        "user-nav__activity-topics",
        "topics",
        "list-ul",
        &t(cx, "user_action_groups.4"),
        None,
    ));
    out.push_str(&item(
        "user-nav__activity-replies",
        "replies",
        "reply",
        &t(cx, "user_action_groups.5"),
        None,
    ));
    if viewing_self {
        out.push_str(&item(
            "user-nav__activity-read",
            "read",
            "clock-rotate-left",
            &t(cx, "user.read"),
            Some(&t(cx, "user.read_help")),
        ));
        let drafts = if viewer.draft_count > 0 {
            t_count(cx, "drafts.label_with_count", viewer.draft_count, &[])
        } else {
            t(cx, "drafts.label")
        };
        out.push_str(&item(
            "user-nav__activity-drafts",
            "drafts",
            "pencil",
            &drafts,
            None,
        ));
    }
    if pending_posts_count > 0 {
        out.push_str(&item(
            "user-nav__activity-pending",
            "pending",
            "clock",
            &t_count(
                cx,
                "pending_posts.label_with_count",
                pending_posts_count,
                &[],
            ),
            None,
        ));
    }
    out.push_str(&item(
        "user-nav__activity-likes",
        "likes-given",
        "heart",
        &t(cx, "user_action_groups.1"),
        None,
    ));
    if viewing_self || viewer.admin {
        out.push_str(&item(
            "user-nav__activity-bookmarks",
            "bookmarks",
            "bookmark",
            &t(cx, "user_action_groups.3"),
            None,
        ));
    }
    // The user-activity-bottom outlet: discourse-reactions', then
    // discourse-topic-voting's.
    if settings.reactions.is_some() {
        out.push_str(&item(
            "user-activity-bottom-outlet discourse-reactions-user-activity-reactions",
            "reactions",
            "far-face-smile",
            &t(cx, "discourse_reactions.reactions_title"),
            None,
        ));
    }
    // The plugins' links that put no span around their label.
    let bare_item = |class: &str, path: &str, icon_name: &str, label: &str| {
        item(class, path, icon_name, label, None).replacen(
            &format!(" <span>{}</span>", escape(label)),
            &format!(" {}", escape(label)),
            1,
        )
    };
    if settings.solved {
        out.push_str(&bare_item(
            "user-activity-bottom-outlet solved-list",
            "solved",
            "square-check",
            &t(cx, "solved.title"),
        ));
    }
    if settings.show_votes {
        out.push_str(&bare_item(
            "user-nav__activity-votes",
            "votes",
            "check-to-slot",
            &t(cx, "topic_voting.vote_title_plural"),
        ));
    }
    if filter == ActivityFilter::Reactions
        && let Some(ui) = settings.reactions.as_ref()
    {
        out.push_str(
            "</ul></div></nav></div><section class=\"user-content\" id=\"user-content\"><div>",
        );
        out.push_str(&reactions_list(cx, ui, reactions));
        out.push_str("</div></section>");
        return out;
    }
    if filter == ActivityFilter::Solved && settings.solved {
        out.push_str("</ul></div></nav></div><section class=\"user-content\" id=\"user-content\">");
        out.push_str(&solved_list(cx, viewing_self, user_path, solved));
        out.push_str("</section>");
        return out;
    }
    if filter == ActivityFilter::Votes {
        out.push_str("</ul></div></nav></div><section class=\"user-content\" id=\"user-content\">");
        out.push_str(&votes_list(cx, viewing_self, user_path, topics));
        out.push_str("</section>");
        return out;
    }
    out.push_str(
        "</ul></div></nav></div><section class=\"user-content\" id=\"user-content\"><div>",
    );

    let items = collapse_stream(stream);
    if items.is_empty() {
        // The route's emptyState (DEmptyState, text only).
        let (title, body) = match filter {
            // Votes renders its own list (votes_list).
            ActivityFilter::All
            | ActivityFilter::Topics
            | ActivityFilter::Votes
            | ActivityFilter::Reactions
            | ActivityFilter::Solved => (t(cx, "user_activity.no_activity_title"), String::new()),
            ActivityFilter::Replies if viewing_self => (
                t(cx, "user_activity.no_replies_title"),
                t_with(
                    cx,
                    "user_activity.no_replies_body",
                    &[("searchUrl", &format!("{}/search", cx.base_path))],
                ),
            ),
            ActivityFilter::Replies => (
                t_with(
                    cx,
                    "user_activity.no_replies_title_others",
                    &[("username", user_path.rsplit('/').next().unwrap_or(""))],
                ),
                String::new(),
            ),
            ActivityFilter::LikesGiven => (
                if viewing_self {
                    t(cx, "user_activity.no_likes_title")
                } else {
                    t_with(
                        cx,
                        "user_activity.no_likes_title_others",
                        &[("username", user_path.rsplit('/').next().unwrap_or(""))],
                    )
                },
                t_with(
                    cx,
                    "user_activity.no_likes_body",
                    &[("heartIcon", &icon("heart", None))],
                ),
            ),
        };
        out.push_str(&format!(
            "<div class=\"empty-state__container --text-only\"><div class=\"empty-state\"><div class=\"empty-state__title\" data-test-title>{}</div>",
            escape(&title)
        ));
        if !body.is_empty() {
            out.push_str(&format!(
                "<div class=\"empty-state__body\"><p data-test-body>{body}</p></div>"
            ));
        }
        out.push_str("</div></div>");
    }

    // PostList
    out.push_str(&format!(
        "<div class=\"post-list user-stream{}\">",
        filter.class_name()
    ));
    if items.is_empty() {
        out.push_str(&format!(
            "<div class=\"post-list__empty-text\">{}</div>",
            escape(&t(cx, "post_list.empty"))
        ));
    }
    for item in &items {
        out.push_str(&stream_item(cx, item));
    }
    out.push_str("</div></div></section>");
    out
}

/// A collapsed user action with its likes, stars, edits and bookmarks
/// (UserAction#children), each the acting users' actions.
struct StreamItem<'a> {
    action: &'a Value,
    children: Vec<(&'static str, Vec<&'a Value>)>,
}

/// `UserAction.collapseStream`: one item per post; likes, edits and
/// bookmarks of it gather under the first as children.
fn collapse_stream(stream: &[Value]) -> Vec<StreamItem<'_>> {
    // likes_given, likes_received, edits, bookmarks
    const TO_COLLAPSE: [i64; 4] = [1, 2, 11, 3];
    let mut items: Vec<StreamItem> = Vec::new();
    let mut seen: Vec<(i64, i64)> = Vec::new();
    for action in stream {
        let key = (
            action["topic_id"].as_i64().unwrap_or(0),
            action["post_number"].as_i64().unwrap_or(0),
        );
        let action_type = action["action_type"].as_i64().unwrap_or(0);
        let bucket = match action_type {
            1 | 2 => Some("heart"),
            11 => Some("pencil"),
            3 => Some("bookmark"),
            _ => None,
        };
        let pos = match seen.iter().position(|k| *k == key) {
            Some(pos) => pos,
            None => {
                seen.push(key);
                items.push(StreamItem {
                    action,
                    // likes, stars, edits, bookmarks, in that order.
                    children: vec![
                        ("heart", Vec::new()),
                        ("star", Vec::new()),
                        ("pencil", Vec::new()),
                        ("bookmark", Vec::new()),
                    ],
                });
                items.len() - 1
            }
        };
        // A later uncollapsed action only lends the item its action_type
        // and description, neither of which the item draws.
        if TO_COLLAPSE.contains(&action_type)
            && let Some(group) = items[pos].children.iter_mut().find(|g| Some(g.0) == bucket)
        {
            group.1.push(action);
        }
    }
    for item in &mut items {
        item.children.retain(|(_, actions)| !actions.is_empty());
    }
    items
}

/// PostListItem, with UserStream's blocks.
fn stream_item(cx: &ListContext, item: &StreamItem) -> String {
    let a = item.action;
    let base = cx.base_path;
    let username = s(&a["username"]);
    let user_href = format!("{base}/u/{}", escape(&username.to_lowercase()));
    let mut out = String::from(
        "<div class=\"post-list-item user-stream-item\"><div class=\"post-list-item__header info\">",
    );
    let u = escape(username);
    out.push_str(&format!(
        "<a class=\"avatar-link\" data-user-card=\"{u}\" href=\"{user_href}\"><div class=\"avatar-wrapper\">{}</div></a>",
        avatar(cx, s(&a["avatar_template"]), 48).replacen(
            "class=\"avatar\"",
            &format!("class=\"avatar actor\" title=\"{u}\""),
            1
        )
    ));
    // PostListItemDetails
    let title = s(&a["title"]);
    let title_html = crate::topic_list_view::emoji_unescape(&escape(title), &cx.settings, base);
    let post_number = a["post_number"].as_i64().unwrap_or(0);
    let slug = a["slug"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("topic");
    let mut url = format!("{base}/t/{slug}/{}", a["topic_id"].as_i64().unwrap_or(0));
    if post_number > 1 {
        url.push_str(&format!("/{post_number}"));
    }
    let aria = if post_number > 0 && !title.is_empty() {
        t_with(
            cx,
            "post_list.aria_post_number",
            &[("title", title), ("postNumber", &post_number.to_string())],
        )
    } else {
        title.to_string()
    };
    out.push_str(&format!(
        "<div class=\"post-list-item__details\"><div class=\"stream-topic-title\"><span class=\"topic-statuses\">{}</span><span class=\"title\"><a aria-label=\"{}\" href=\"{}\">{title_html}</a></span></div><div class=\"post-list-item__metadata\">",
        crate::topic_list_view::topic_statuses(cx, a),
        escape(&aria),
        escape(&url)
    ));
    if let Some(category) = a["category_id"]
        .as_i64()
        .and_then(|id| cx.categories.get(&id))
    {
        out.push_str(&format!(
            "<span class=\"category stream-post-category\">{}</span>",
            category_badge(cx, category)
        ));
    }
    if let Some(at) = date(&a["created_at"]) {
        out.push_str(&format!(
            // The template's whitespace spaces the date from the bullet.
            "<span class=\"time\"> {} </span>",
            age_medium(cx, at, true)
        ));
    }
    out.push_str("</div></div>");
    if a["truncated"] == true {
        out.push_str(&format!(
            "<button class=\"btn no-text btn-icon btn-transparent expand-item\" title=\"{}\" type=\"button\">{}</button>",
            escape(&t(cx, "post.expand_collapse")),
            icon("chevron-down", None)
        ));
    }
    out.push_str("<span></span></div>");
    // PostActionDescription (no createdAt from the stream: `when` is empty).
    if let Some(code) = a["action_code"].as_str().filter(|c| !c.is_empty()) {
        let who = a["action_code_who"]
            .as_str()
            .map(|u| {
                format!(
                    "<a class=\"mention\" href=\"{base}/u/{}\">@{}</a>",
                    uri_component(u),
                    escape(u)
                )
            })
            .unwrap_or_default();
        out.push_str(&format!(
            "<p class=\"excerpt\">{}</p>",
            t_with(
                cx,
                &format!("action_codes.{code}"),
                &[
                    ("who", &who),
                    ("when", ""),
                    ("path", s(&a["action_code_path"]))
                ]
            )
        ));
    }
    for (icon_name, actions) in &item.children {
        out.push_str(&format!(
            "<div class=\"user-stream-item-actions\">{}",
            icon(icon_name, Some("icon"))
        ));
        for child in actions {
            let acting = s(&child["acting_username"]);
            out.push_str(&format!(
                "<a class=\"avatar-link\" data-user-card=\"{u}\" href=\"{base}/u/{}\"><div class=\"avatar-wrapper\">{}</div></a>",
                escape(&acting.to_lowercase()),
                avatar(cx, s(&child["acting_avatar_template"]), 24)
                    .replacen("class=\"avatar\"", "class=\"avatar actor\"", 1),
                u = escape(acting)
            ));
            if let Some(reason) = child["edit_reason"].as_str().filter(|r| !r.is_empty()) {
                out.push_str(&format!(
                    " &mdash; <span class=\"edit-reason\">{}</span>",
                    escape(reason)
                ));
            }
        }
        out.push_str("</div>");
    }
    out.push_str(&format!(
        "<div class=\"excerpt\"{} data-topic-id=\"{}\" data-user-id=\"{}\"><div class=\"cooked\">{}</div></div></div>",
        a["post_id"]
            .as_i64()
            .map(|id| format!(" data-post-id=\"{id}\""))
            .unwrap_or_default(),
        a["topic_id"].as_i64().unwrap_or(0),
        a["user_id"].as_i64().unwrap_or(0),
        s(&a["excerpt"])
    ));
    out
}

/// discourse-solved's user-activity/solved: the user's accepted answers
/// as UserStream draws them, or the route's empty state.
fn solved_list(cx: &ListContext, viewing_self: bool, user_path: &str, posts: &[Value]) -> String {
    let base = cx.base_path;
    if posts.is_empty() {
        let (title, body) = if viewing_self {
            (
                t(cx, "solved.no_solved_topics_title"),
                t(cx, "solved.no_solved_topics_body"),
            )
        } else {
            (
                t_with(
                    cx,
                    "solved.no_solved_topics_title_others",
                    &[("username", user_path.rsplit('/').next().unwrap_or(""))],
                ),
                String::new(),
            )
        };
        let mut out = format!(
            "<div class=\"empty-state__container --text-only\"><div class=\"empty-state\"><div class=\"empty-state__title\" data-test-title>{}</div>",
            escape(&title)
        );
        if !body.is_empty() {
            out.push_str(&format!(
                "<div class=\"empty-state__body\"><p data-test-body>{}</p></div>",
                escape(&body)
            ));
        }
        out.push_str("</div></div>");
        return out;
    }
    let mut out = String::from("<div><div class=\"post-list user-stream\">");
    for p in posts {
        let username = s(&p["username"]);
        let u = escape(username);
        out.push_str(&format!(
            "<div class=\"post-list-item user-stream-item\"><div class=\"post-list-item__header info\"><a class=\"avatar-link\" data-user-card=\"{u}\" href=\"{base}/u/{}\"><div class=\"avatar-wrapper\">{}</div></a>",
            escape(&username.to_lowercase()),
            avatar(cx, s(&p["avatar_template"]), 48).replacen(
                "class=\"avatar\"",
                &format!("class=\"avatar actor\" title=\"{u}\""),
                1
            )
        ));
        // titleHtml is the topic's fancy title; the link is the post's url.
        let title = s(&p["topic_title"]);
        let aria = t_with(
            cx,
            "post_list.aria_post_number",
            &[
                ("title", title),
                (
                    "postNumber",
                    &p["post_number"].as_i64().unwrap_or(0).to_string(),
                ),
            ],
        );
        out.push_str(&format!(
            "<div class=\"post-list-item__details\"><div class=\"stream-topic-title\"><span class=\"topic-statuses\"></span><span class=\"title\"><a aria-label=\"{}\" href=\"{}\">{title}</a></span></div><div class=\"post-list-item__metadata\">",
            escape(&aria),
            escape(s(&p["url"]))
        ));
        if let Some(category) = p["category_id"]
            .as_i64()
            .and_then(|id| cx.categories.get(&id))
        {
            out.push_str(&format!(
                "<span class=\"category stream-post-category\">{}</span>",
                category_badge(cx, category)
            ));
        }
        if let Some(at) = date(&p["created_at"]) {
            out.push_str(&format!(
                "<span class=\"time\"> {} </span>",
                age_medium(cx, at, true)
            ));
        }
        out.push_str(&format!(
            "</div></div></div><div class=\"excerpt\" data-post-id=\"{}\" data-topic-id=\"{}\" data-user-id=\"{}\"><div class=\"cooked\">{}</div></div></div>",
            p["post_id"].as_i64().unwrap_or(0),
            p["topic_id"].as_i64().unwrap_or(0),
            p["user_id"].as_i64().unwrap_or(0),
            s(&p["excerpt"])
        ));
    }
    out.push_str("</div><div class=\"loading-container\"></div><div aria-hidden=\"true\" class=\"load-more-sentinel\"></div></div>");
    out
}

/// discourse-reactions' user-activity/reactions: a PostList of the
/// reactions (flattenForPostList), each with the reaction and its user
/// above the excerpt (DiscourseReactionsReactionEmoji).
fn reactions_list(
    cx: &ListContext,
    ui: &crate::plugins::reactions::view::ReactionsUi,
    reactions: &[Value],
) -> String {
    let base = cx.base_path;
    let mut out = String::from("<div class=\"post-list user-stream\">");
    if reactions.is_empty() {
        out.push_str(&format!(
            "<div class=\"post-list__empty-text\">{}</div>",
            escape(&t(cx, "notifications.empty"))
        ));
    }
    for r in reactions {
        let post = &r["post"];
        let username = s(&post["username"]);
        let u = escape(username);
        out.push_str(&format!(
            "<div class=\"post-list-item user-stream-item\"><div class=\"post-list-item__header info\"><a class=\"avatar-link\" data-user-card=\"{u}\" href=\"{base}/u/{}\"><div class=\"avatar-wrapper\">{}</div></a>",
            escape(&username.to_lowercase()),
            avatar(cx, s(&post["avatar_template"]), 48).replacen(
                "class=\"avatar\"",
                &format!("class=\"avatar actor\" title=\"{u}\""),
                1
            )
        ));
        // PostListItemDetails: the flattened item has no post number.
        let title_html = crate::topic_list_view::emoji_unescape(
            s(&post["topic"]["fancy_title"]),
            &cx.settings,
            base,
        );
        let url = format!(
            "{base}/t/{}/{}",
            s(&post["topic_slug"]),
            post["topic_id"].as_i64().unwrap_or(0)
        );
        out.push_str(&format!(
            "<div class=\"post-list-item__details\"><div class=\"stream-topic-title\"><span class=\"topic-statuses\"></span><span class=\"title\"><a href=\"{}\">{title_html}</a></span></div><div class=\"post-list-item__metadata\">",
            escape(&url)
        ));
        if let Some(category) = post["category_id"]
            .as_i64()
            .and_then(|id| cx.categories.get(&id))
        {
            out.push_str(&format!(
                "<span class=\"category stream-post-category\">{}</span>",
                category_badge(cx, category)
            ));
        }
        if let Some(at) = date(&r["created_at"]) {
            out.push_str(&format!(
                "<span class=\"time\"> {} </span>",
                age_medium(cx, at, true)
            ));
        }
        out.push_str("</div></div></div>");
        let reaction = &r["reaction"];
        if reaction["reaction_users_count"].as_i64().unwrap_or(0) > 0 {
            let reactor = s(&r["user"]["username"]);
            let value = s(&reaction["reaction_value"]);
            let emoji = if value.is_empty() {
                String::new()
            } else {
                format!(
                    "<img width=\"20\" height=\"20\" src=\"{}\" title=\"{v}\" alt=\"{v}\" class=\"emoji reaction-emoji\">",
                    crate::category_badge::html_escape(&ui.emoji_url(base, value)),
                    v = escape(value)
                )
            };
            out.push_str(&format!(
                "<div class=\"discourse-reactions-my-reaction\">{emoji}<a class=\"avatar-link\" data-user-card=\"{ru}\">{}</a></div>",
                avatar(cx, s(&r["user"]["avatar_template"]), 24).replacen(
                    "class=\"avatar\"",
                    &format!("class=\"avatar actor\" title=\"{}\"", escape(reactor)),
                    1
                ),
                ru = escape(reactor)
            ));
        }
        out.push_str(&format!(
            "<div class=\"excerpt\" data-post-id=\"{}\" data-topic-id=\"{}\" data-user-id=\"{}\"><div class=\"cooked\">{}</div></div></div>",
            post["id"].as_i64().unwrap_or(0),
            post["topic_id"].as_i64().unwrap_or(0),
            post["user_id"].as_i64().unwrap_or(0),
            s(&post["excerpt"])
        ));
    }
    out.push_str("</div><div class=\"loading-container\"></div><div aria-hidden=\"true\" class=\"load-more-sentinel\"></div>");
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
                age_medium(cx, at, false)
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
        // discourse-solved's user-summary-stat outlet (SolvedCount).
        let solved = n("solved_count");
        if settings.solved && solved > 0 {
            out.push_str(&format!(
                "<li class=\"user-summary-stat-outlet solved-count linked-stat\"><a href=\"{user_path}/activity/solved\">{}</a></li>",
                stat(
                    number(cx, solved),
                    escape(&t_count(cx, "solved.solution_summary", solved, &[])),
                    Some("square-check"),
                    None
                )
            ));
        }
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
