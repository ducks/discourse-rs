//! A topic's posts as the Ember client renders them on a desktop
//! (components/post.gjs and post/*), from the post serializer JSON: the
//! avatar, the poster's names, the post infos, the cooked content, the
//! post menu and the actions summary; small actions and time gaps between
//! posts. Dates are shown in UTC, as the topic list's are.
//!
//! The post menu shows the buttons whose actions are ported: like (with
//! its count), copy link, bookmark and reply. Flag, edit, delete, admin,
//! read and replies open modals, the composer or lists that are not, so
//! they are left out; with them gone at most one button could collapse,
//! so the menu is never collapsed behind show more.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::site_settings::{SettingError, SiteSettings};
use crate::topic_list_view::{
    ListContext, escape, icon, long_date, relative_age_tiny, t, t_count, t_with,
};

/// `post_type`s (Post.types)
const MODERATOR_ACTION: i64 = 2;
const SMALL_ACTION: i64 = 3;
const WHISPER: i64 = 4;
/// PostActionType.types[:like]
const LIKE: i64 = 2;

/// The site settings the posts read.
pub struct PostSettings {
    pub prioritize_username_in_ux: bool,
    pub display_name_on_posts: bool,
    pub hide_user_profiles_from_public: bool,
    pub avatar_size_24: i64,
    pub avatar_size_48: i64,
    pub enable_badges: bool,
    pub allow_username_in_share_links: bool,
    pub suppress_reply_directly_above: bool,
    pub show_time_gap_days: i64,
    pub old_post_notice_days: i64,
}

impl PostSettings {
    pub fn load(settings: &SiteSettings) -> Result<PostSettings, SettingError> {
        let flag = |name: &str| -> Result<bool, SettingError> { Ok(settings.get(name)?.truthy()) };
        let mut sizes: Vec<i64> = settings
            .get("avatar_sizes")?
            .to_s()
            .split('|')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        sizes.sort_unstable();
        // getRawAvatarSize at a device pixel ratio of 1.
        let raw = |size: i64| {
            sizes
                .iter()
                .copied()
                .find(|s| *s >= size)
                .or(sizes.last().copied())
                .unwrap_or(size)
        };
        Ok(PostSettings {
            prioritize_username_in_ux: flag("prioritize_username_in_ux")?,
            display_name_on_posts: flag("display_name_on_posts")?,
            hide_user_profiles_from_public: flag("hide_user_profiles_from_public")?,
            avatar_size_24: raw(24),
            avatar_size_48: raw(48),
            enable_badges: flag("enable_badges")?,
            allow_username_in_share_links: flag("allow_username_in_share_links")?,
            suppress_reply_directly_above: flag("suppress_reply_directly_above")?,
            show_time_gap_days: settings.get("show_time_gap_days")?.to_i(),
            old_post_notice_days: settings.get("old_post_notice_days")?.to_i(),
        })
    }
}

/// What the posts need of their topic.
pub struct TopicInfo {
    pub id: i64,
    pub slug: String,
    /// `details.created_by.id`
    pub created_by_id: Option<i64>,
    pub archived: bool,
    /// `details.can_create_post`: the reply button.
    pub can_create_post: bool,
}

impl TopicInfo {
    /// From the topic view JSON.
    pub fn from_view(view: &Value) -> TopicInfo {
        TopicInfo {
            id: view["id"].as_i64().unwrap_or(0),
            slug: view["slug"].as_str().unwrap_or("").to_string(),
            created_by_id: view["details"]["created_by"]["id"].as_i64(),
            archived: view["archived"] == true,
            can_create_post: view["details"]["can_create_post"] == true,
        }
    }
}

/// The post rendered just above, for the reply-to tab and time gaps.
#[derive(Clone, Copy)]
pub struct Prev {
    pub post_number: i64,
    pub created_at: DateTime<Utc>,
}

pub struct PostContext<'a> {
    pub list: &'a ListContext<'a>,
    pub settings: &'a PostSettings,
    pub topic: &'a TopicInfo,
    /// The viewer's username, for a member.
    pub viewer: Option<&'a str>,
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn date(v: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// `iconHTML` with lib/icon-library's replacements: the class keeps the
/// name unless it has a dot.
fn d_icon(name: &str, extra: Option<&str>) -> String {
    // "thumbtack unpinned": the icon, with the rest as classes.
    if let Some((base, classes)) = name.split_once(' ') {
        return d_icon(base, extra).replacen(
            &format!("d-icon-{base} "),
            &format!("d-icon-{base} {classes} "),
            1,
        );
    }
    let replaced = match name {
        "d-liked" => "heart",
        "d-unliked" => "far-heart",
        "d-post-share" => "arrow-up-from-bracket",
        "topic.closed" => "lock",
        "topic.opened" => "unlock",
        other => other,
    };
    if name == replaced {
        return icon(name, extra);
    }
    let class_name = if name.contains('.') { replaced } else { name };
    icon(replaced, extra).replacen(
        &format!("d-icon-{replaced} "),
        &format!("d-icon-{class_name} "),
        1,
    )
}

/// `relativeAgeMedium`, unwrapped: "just now", "3 hours ago", or a short
/// date past five days ("on Sep 30" with `wrap_on`).
fn relative_age_medium(cx: &PostContext, at: DateTime<Utc>, wrap_on: bool) -> String {
    use chrono::Datelike;
    let l = cx.list;
    let distance = ((l.now - at).num_milliseconds() as f64 / 1000.0).round() as i64;
    if distance < 60 {
        return t(l, "now");
    }
    if distance > 432_000 {
        let format = if at.year() == l.now.year() {
            t(l, "dates.tiny.date_month")
        } else {
            t(l, "dates.medium.date_year")
        };
        let short =
            crate::pretty_text::render::local_dates::format_utc(at, &format).unwrap_or_default();
        return if wrap_on {
            t_with(l, "dates.wrap_on", &[("date", &short)])
        } else {
            short
        };
    }
    // relativeAgeMediumSpan, with ago
    let minutes = (distance as f64 / 60.0).round() as i64;
    let key = "dates.medium_with_ago";
    let (unit, count) = match minutes {
        1..=55 => ("x_minutes", minutes),
        56..=89 => ("x_hours", 1),
        90..=1409 => ("x_hours", (minutes as f64 / 60.0).round() as i64),
        1410..=2519 => ("x_days", 1),
        2520..=129_599 => ("x_days", (minutes as f64 / 1440.0).round() as i64),
        129_600..=525_599 => ("x_months", (minutes as f64 / 43200.0).round() as i64),
        _ => ("x_years", (minutes as f64 / 525_600.0).round() as i64),
    };
    t_count(l, &format!("{key}.{unit}"), count, &[])
}

/// The tiny auto-updating date span.
fn tiny_date(cx: &PostContext, at: DateTime<Utc>) -> String {
    format!(
        "<span class=\"relative-date\" title=\"{}\" data-time=\"{}\" data-format=\"tiny\">{}</span>",
        escape(&long_date(cx.list, at)),
        at.timestamp_millis(),
        escape(&relative_age_tiny(cx.list, at))
    )
}

/// The name a post shows first: the full name only with
/// display_name_on_posts and names prioritized.
fn display_name<'p>(cx: &PostContext, username: &'p str, name: Option<&'p str>) -> &'p str {
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) if cx.settings.display_name_on_posts && !cx.settings.prioritize_username_in_ux => n,
        _ => username,
    }
}

fn user_path(cx: &PostContext, username: &str) -> String {
    format!("{}/u/{}", cx.list.base_path, username.to_lowercase())
}

/// DUserLink's anchor attributes: no link to a profile hidden from
/// anonymous visitors.
fn user_link_attrs(cx: &PostContext, username: &str, aria_hidden: bool) -> String {
    let hidden = cx.settings.hide_user_profiles_from_public && cx.viewer.is_none();
    let mut out = String::new();
    if hidden {
        out.push_str(" class=\"non-clickable\"");
    }
    if aria_hidden {
        out.push_str(" aria-hidden=\"true\"");
    } else {
        out.push_str(&format!(
            " aria-label=\"{}\"",
            escape(&t_with(
                cx.list,
                "user.profile_possessive",
                &[("username", username)]
            ))
        ));
    }
    out.push_str(&format!(" data-user-card=\"{}\"", escape(username)));
    if !hidden {
        out.push_str(&format!(" href=\"{}\"", escape(&user_path(cx, username))));
    }
    out.push_str(if aria_hidden {
        " tabindex=\"-1\""
    } else {
        " tabindex=\"0\""
    });
    out
}

fn avatar_img(template: &str, size: i64, extra: &str) -> String {
    format!(
        "<img alt=\"\" width=\"{size}\" height=\"{size}\" src=\"{}\" class=\"avatar\"{extra}>",
        escape(&template.replace("{size}", &size.to_string()))
    )
}

/// `post.shareUrl`, with the viewer's username for badges when allowed.
fn share_url(cx: &PostContext, number: i64) -> String {
    let mut url = format!("{}/t/{}/{}", cx.list.base_path, cx.topic.slug, cx.topic.id);
    if number > 1 {
        url.push_str(&format!("/{number}"));
    }
    if let Some(viewer) = cx.viewer
        && cx.settings.enable_badges
        && cx.settings.allow_username_in_share_links
    {
        url.push_str(&format!("?u={}", viewer.to_lowercase()));
    }
    url
}

/// topic-title: the statuses, the title with its emoji, the category
/// badge and the tags.
pub fn topic_title(list: &ListContext, view: &Value, url: &str) -> String {
    let category = view["category_id"]
        .as_i64()
        .and_then(|id| list.categories.get(&id));
    let badge = category
        .map(|c| crate::topic_list_view::category_badge(list, c))
        .unwrap_or_default();
    let title = s(&view["title"]);
    let fancy = view["fancy_title"].as_str().unwrap_or(title);
    format!(
        "<div class=\"title-wrapper\"><h1 data-topic-id=\"{}\"><span class=\"topic-statuses\">{}</span><a class=\"fancy-title\" href=\"{}\">{}</a></h1><div class=\"topic-category\">{badge}<div class=\"topic-header-extra\"><div class=\"list-tags\">{}</div></div></div></div>",
        view["id"],
        crate::topic_list_view::topic_statuses(list, view),
        escape(url),
        crate::topic_list_view::emoji_unescape(fancy, &list.settings, list.base_path),
        crate::topic_list_view::tags_html(list, view, title)
    )
}

/// The posts of a page in order, with the time gaps between them.
pub fn stream(cx: &PostContext, posts: &[Value]) -> Vec<String> {
    let mut out = Vec::with_capacity(posts.len());
    let mut prev: Option<Prev> = None;
    for p in posts {
        out.push(post(cx, p, prev));
        if let Some(at) = date(&p["created_at"]) {
            prev = Some(Prev {
                post_number: p["post_number"].as_i64().unwrap_or(0),
                created_at: at,
            });
        }
    }
    out
}

/// One post, with the time gap before it.
pub fn post(cx: &PostContext, p: &Value, prev: Option<Prev>) -> String {
    time_gap(cx, p, prev) + &post_body(cx, p, prev)
}

/// One post without its time gap: a small action or a regular post.
pub fn post_body(cx: &PostContext, p: &Value, prev: Option<Prev>) -> String {
    if p["post_type"].as_i64() == Some(SMALL_ACTION) || p["action_code"] == "split_topic" {
        small_action(cx, p)
    } else {
        regular(cx, p, prev)
    }
}

/// post-stream's time gap: more than show_time_gap_days since the post
/// above.
fn time_gap(cx: &PostContext, p: &Value, prev: Option<Prev>) -> String {
    let (Some(prev), Some(at)) = (prev, date(&p["created_at"])) else {
        return String::new();
    };
    let days = (at - prev.created_at).num_milliseconds() as f64 / 86_400_000.0;
    let days = days.floor() as i64;
    if days <= cx.settings.show_time_gap_days {
        return String::new();
    }
    let text = if days < 30 {
        t_count(cx.list, "dates.later.x_days", days, &[])
    } else if days < 365 {
        t_count(
            cx.list,
            "dates.later.x_months",
            (days as f64 / 30.0).round() as i64,
            &[],
        )
    } else {
        t_count(
            cx.list,
            "dates.later.x_years",
            (days as f64 / 365.0).round() as i64,
            &[],
        )
    };
    format!(
        "<div class=\"time-gap small-action\"><div class=\"topic-avatar\"></div><div class=\"small-action-desc timegap\">{}</div></div>",
        escape(&text)
    )
}

fn regular(cx: &PostContext, p: &Value, prev: Option<Prev>) -> String {
    let number = p["post_number"].as_i64().unwrap_or(0);
    let username = s(&p["username"]);
    let user_id = p["user_id"].as_i64();
    let post_type = p["post_type"].as_i64().unwrap_or(1);
    let flag = |key: &str| p[key] == true;

    let mut classes = vec!["topic-post clearfix", "post--sticky-avatar sticky-avatar"];
    if user_id.is_some() && cx.topic.created_by_id == user_id {
        classes.push("post--topic-owner topic-owner");
    }
    if flag("yours") {
        classes.push("post--current-user current-user-post");
    }
    if flag("group_moderator") {
        classes.push("post--category-moderator category-moderator");
    }
    if flag("hidden") {
        classes.push("post--hidden post-hidden");
    }
    if !p["deleted_at"].is_null() {
        classes.push("post--deleted deleted");
    }
    let group = p["primary_group_name"]
        .as_str()
        .filter(|g| !g.is_empty())
        .map(|g| format!("post--group-{g} group-{g}"));
    if let Some(g) = &group {
        classes.push(g);
    }
    if flag("wiki") {
        classes.push("post--wiki wiki");
    }
    if post_type == WHISPER {
        classes.push("post--whisper whisper");
    }
    classes.push(if post_type == MODERATOR_ACTION {
        "post--moderator moderator"
    } else {
        "post--regular regular"
    });
    if flag("user_suspended") {
        classes.push("post--user-suspended user-suspended");
    }

    let created = date(&p["created_at"]);
    let heading = t_with(
        cx.list,
        "post.accessible_heading",
        &[
            ("username", username),
            (
                "date",
                &created
                    .map(|at| relative_age_medium(cx, at, true))
                    .unwrap_or_default(),
            ),
        ],
    );
    let mut article_classes = String::from("boxed onscreen-post");
    if flag("via_email") {
        article_classes.push_str(" post--via-email via-email");
    }
    if flag("is_auto_generated") {
        article_classes.push_str(" post--auto-generated is-auto-generated");
    }

    let reply_tab = reply_to_tab(cx, p, prev);
    let contents_class = if reply_tab.is_empty() {
        "post__regular regular post__contents contents"
    } else {
        "post__regular regular post__contents contents post__contents--avoid-tab avoid-tab"
    };
    format!(
        "<div class=\"{}\" data-post-number=\"{number}\"><h2 aria-hidden=\"false\" class=\"sr-only\" id=\"post-heading-{number}\">{}</h2><article aria-labelledby=\"post-heading-{number}\" class=\"{article_classes}\" data-post-id=\"{}\" data-user-id=\"{}\" id=\"post_{number}\">{}<div class=\"post__row row\">{}<div class=\"post__body topic-body clearfix\">{}<div class=\"{contents_class}\"><div class=\"cooked\">{}<div class=\"cooked-selection-barrier\" aria-hidden=\"true\"><br></div></div><section aria-label=\"{}\" class=\"post__menu-area post-menu-area clearfix\" role=\"group\">{}</section></div><section class=\"post__actions post-actions\">{}</section></div></div></article></div>",
        classes.join(" "),
        escape(&heading),
        p["id"],
        user_id.map(|id| id.to_string()).unwrap_or_default(),
        notice(cx, p),
        avatar(cx, p),
        meta_data(cx, p, &reply_tab),
        s(&p["cooked"]),
        escape(&t(cx.list, "post.controls.menu_label")),
        menu(cx, p),
        actions_summary(cx, p),
    )
}

/// post/notice: a new or returning user's first post here, or a staff
/// note.
fn notice(cx: &PostContext, p: &Value) -> String {
    let kind = s(&p["notice"]["type"]);
    if kind.is_empty() || !p["deleted_at"].is_null() {
        return String::new();
    }
    if kind != "custom" {
        let fresh = date(&p["created_at"])
            .is_some_and(|at| (cx.list.now - at).num_days() <= cx.settings.old_post_notice_days);
        if !fresh {
            return String::new();
        }
    }
    let username = s(&p["username"]);
    let user = display_name(cx, username, p["name"].as_str());
    let emoji = |name: &str| {
        crate::topic_list_view::emoji_unescape(
            &format!(":{name}:"),
            &cx.list.settings,
            cx.list.base_path,
        )
    };
    let body = match kind {
        "new_user" => format!(
            "{}<p>{}</p>",
            emoji("tada"),
            escape(&t_with(cx.list, "post.notice.new_user", &[("user", user)]))
        ),
        "returning_user" => {
            let when = date(&p["notice"]["last_posted_at"])
                .map(|at| relative_age_medium(cx, at, false))
                .unwrap_or_default();
            format!(
                "{}<p>{}</p>",
                emoji("wave"),
                escape(&t_with(
                    cx.list,
                    "post.notice.returning_user",
                    &[("user", user), ("time", &when)]
                ))
            )
        }
        "custom" => format!(
            "{}<div class=\"post-notice-message\">{}</div>",
            icon("user-shield", None),
            s(&p["notice"]["cooked"])
        ),
        _ => return String::new(),
    };
    format!(
        "<div class=\"post__row row\"><div class=\"post-notice {}\">{body}</div></div>",
        kind.replace('_', "-")
    )
}

fn avatar(cx: &PostContext, p: &Value) -> String {
    let username = s(&p["username"]);
    let inner = if p["user_id"].is_null() || username.is_empty() {
        format!(
            "<div class=\"deleted-user-avatar\">{}</div>",
            icon("trash-can", None)
        )
    } else {
        let hidden = cx.settings.hide_user_profiles_from_public && cx.viewer.is_none();
        let (class, href) = if hidden {
            ("non-clickable main-avatar", String::new())
        } else {
            (
                "main-avatar",
                format!(" href=\"{}\"", escape(&user_path(cx, username))),
            )
        };
        format!(
            "<a class=\"{class}\" aria-hidden=\"true\" data-user-card=\"{}\"{href} tabindex=\"-1\">{}</a>",
            escape(username),
            avatar_img(
                s(&p["avatar_template"]),
                cx.settings.avatar_size_48,
                " loading=\"lazy\""
            )
        )
    };
    format!("<div class=\"topic-avatar\"><div class=\"post-avatar\">{inner}</div></div>")
}

fn meta_data(cx: &PostContext, p: &Value, reply_tab: &str) -> String {
    let mut infos = String::new();
    let l = cx.list;
    if p["post_type"].as_i64() == Some(WHISPER) {
        infos.push_str(&format!(
            "<div class=\"post-info whisper\" title=\"{}\">{}</div>",
            escape(&t(l, "post.whisper")),
            icon("far-eye-slash", None)
        ));
    }
    if p["via_email"] == true {
        let auto = p["is_auto_generated"] == true;
        infos.push_str(&format!(
            "<div class=\"post-info via-email\" title=\"{}\">{}</div>",
            escape(&t(
                l,
                if auto {
                    "post.via_auto_generated_email"
                } else {
                    "post.via_email"
                }
            )),
            icon(if auto { "envelope" } else { "far-envelope" }, None)
        ));
    }
    if p["locked"] == true {
        infos.push_str(&format!(
            "<div class=\"post-info post-locked\" title=\"{}\">{}</div>",
            escape(&t(l, "post.locked")),
            icon("lock", None)
        ));
    }
    let version = p["version"].as_i64().unwrap_or(1);
    let wiki = p["wiki"] == true;
    if version > 1 || wiki {
        infos.push_str(&edits_indicator(cx, p, version, wiki));
    }
    infos.push_str(reply_tab);
    let number = p["post_number"].as_i64().unwrap_or(0);
    let shown_at = if wiki && !p["last_wiki_edit"].is_null() {
        date(&p["last_wiki_edit"])
    } else {
        date(&p["created_at"])
    };
    if let Some(at) = shown_at {
        infos.push_str(&format!(
            "<div class=\"post-info post-date\"><a aria-label=\"{}\" class=\"post-date{}\" href=\"{}\" title=\"{}\"><span aria-hidden=\"true\">{}</span></a></div>",
            escape(&relative_age_medium(cx, at, false)),
            if wiki && !p["last_wiki_edit"].is_null() {
                " last-wiki-edit"
            } else {
                ""
            },
            escape(&share_url(cx, number)),
            escape(&t(l, "post.sr_date")),
            tiny_date(cx, at)
        ));
    }
    infos.push_str(&format!(
        "<div class=\"read-state{}\" title=\"{}\">{}</div>",
        if p["read"] == true { " read" } else { "" },
        escape(&t(l, "post.unread")),
        icon("circle", None)
    ));
    format!(
        "<div class=\"topic-meta-data\">{}<div class=\"post-infos\">{infos}</div></div>",
        poster_name(cx, p)
    )
}

fn edits_indicator(cx: &PostContext, p: &Value, version: i64, wiki: bool) -> String {
    let l = cx.list;
    let updated = date(&p["updated_at"]);
    let when = updated.map(|at| long_date(l, at)).unwrap_or_default();
    let title = if !wiki {
        t_with(l, "post.last_edited_on", &[("dateTime", &when)])
    } else if version > 1 {
        t_with(l, "post.wiki_last_edited_on", &[("dateTime", &when)])
    } else {
        t(l, "post.wiki.about")
    };
    // historyHeat: in units of 50 minutes, as core computes them.
    let heat = updated.and_then(|at| {
        let minutes = (l.now - at).num_minutes() as f64;
        let units = minutes / 50.0;
        if units < 12.0 {
            Some("heatmap-high")
        } else if units < 24.0 {
            Some("heatmap-med")
        } else if units < 48.0 {
            Some("heatmap-low")
        } else {
            None
        }
    });
    let mut class = if version > 1 {
        String::from("btn btn-icon-text btn-flat")
    } else {
        String::from("btn no-text btn-icon btn-flat")
    };
    if let Some(h) = heat {
        class.push(' ');
        class.push_str(h);
    }
    if wiki {
        class.push_str(" wiki");
    }
    let label = if version > 1 {
        format!("<span class=\"d-button-label\">{}</span>", version - 1)
    } else {
        "<span aria-hidden=\"true\">&#8203;</span>".to_string()
    };
    format!(
        "<div class=\"post-info edits\"><button class=\"{class}\" aria-label=\"{}\" title=\"{}\" type=\"button\">{}{label}</button></div>",
        escape(&t(l, "post.edit_history")),
        escape(&title),
        icon(if wiki { "far-pen-to-square" } else { "pencil" }, None)
    )
}

/// post/meta-data/reply-to-tab: who this replies to, unless that post is
/// right above it.
fn reply_to_tab(cx: &PostContext, p: &Value, prev: Option<Prev>) -> String {
    let to = &p["reply_to_user"];
    let Some(username) = to["username"].as_str().filter(|u| !u.is_empty()) else {
        return String::new();
    };
    let directly_below =
        prev.is_some_and(|prev| p["reply_to_post_number"].as_i64() == Some(prev.post_number));
    if directly_below && cx.settings.suppress_reply_directly_above {
        return String::new();
    }
    let name = display_name(cx, username, to["name"].as_str());
    format!(
        "<a class=\"reply-to-tab\" href=\"\" role=\"button\" title=\"{}\">{}{}<span>{}</span></a>",
        escape(&t(cx.list, "post.in_reply_to")),
        icon("share", None),
        avatar_img(
            s(&to["avatar_template"]),
            cx.settings.avatar_size_24,
            &format!(" title=\"{}\"", escape(name))
        ),
        escape(name)
    )
}

fn poster_name(cx: &PostContext, p: &Value) -> String {
    let username = s(&p["username"]);
    if p["user_id"].is_null() || username.is_empty() {
        return String::new();
    }
    let post_name = p["name"].as_str();
    let name = display_name(cx, username, post_name);
    let name_first = post_name == Some(name);
    let mut classes = vec![if name_first { "full-name" } else { "username" }];
    let flag = |key: &str| p[key] == true;
    if flag("admin") || flag("moderator") {
        classes.push("staff");
    }
    if flag("admin") {
        classes.push("admin");
    }
    if flag("moderator") {
        classes.push("moderator");
    }
    if flag("group_moderator") {
        classes.push("category-moderator");
    }
    if flag("new_user") {
        classes.push("new-user");
    }
    let group = p["primary_group_name"].as_str().filter(|g| !g.is_empty());
    let group_class = group.map(|g| format!("group--{g}"));
    if let Some(g) = &group_class {
        classes.push(g);
    }
    let glyph = if flag("moderator") || flag("group_moderator") {
        format!(
            "<span class=\"svg-icon-title\" title=\"{}\">{}</span>",
            escape(&t(cx.list, "user.moderator_tooltip")),
            icon("shield-halved", None)
        )
    } else {
        String::new()
    };
    let mut out = format!(
        "<div class=\"names trigger-user-card\"><span class=\"first {}\"><a{}>{}{glyph}</a></span>",
        classes.join(" "),
        user_link_attrs(cx, username, false).replacen(
            &format!(
                " aria-label=\"{}\"",
                escape(&t_with(
                    cx.list,
                    "user.profile_possessive",
                    &[("username", username)]
                ))
            ),
            &format!(
                " aria-label=\"{}\"",
                escape(&t_with(
                    cx.list,
                    "user.profile_possessive",
                    &[("username", name)]
                ))
            ),
            1
        ),
        escape(name)
    );
    // The other name, unless they read alike.
    if let Some(full) = post_name.filter(|n| !n.trim().is_empty())
        && cx.settings.display_name_on_posts
    {
        let normalize = |s: &str| {
            s.to_lowercase()
                .chars()
                .filter(|c| !c.is_whitespace() && !matches!(c, '.' | '_' | '-'))
                .collect::<String>()
        };
        if normalize(full) != normalize(username) {
            let (class, other) = if name_first {
                ("username", username)
            } else {
                ("full-name", full)
            };
            out.push_str(&format!(
                "<span class=\"second {class}\"><a{}>{}</a></span>",
                user_link_attrs(cx, username, true),
                escape(other)
            ));
        }
    }
    if let Some(title) = p["user_title"].as_str().filter(|t| !t.is_empty()) {
        let title_class: String = title
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-");
        let is_group_title = flag("title_is_group") && group.is_some();
        let mut class = format!("user-title user-title--{title_class}");
        if is_group_title && let Some(g) = group {
            class.push_str(&format!(" user-title--{}", g.to_lowercase()));
        }
        let content = match group {
            Some(g) if is_group_title => format!(
                "<a class=\"user-group trigger-group-card\" data-group-card=\"{g}\" href=\"{}/g/{g}\">{}</a>",
                cx.list.base_path,
                escape(title)
            ),
            _ => escape(title),
        };
        out.push_str(&format!("<span class=\"{class}\">{content}</span>"));
    }
    out.push_str("</div>");
    out
}

/// post/menu with the ported buttons in post_menu order.
fn menu(cx: &PostContext, p: &Value) -> String {
    let mut actions = String::new();
    actions.push_str(&like_button(cx, p));
    actions.push_str(&button(
        "btn no-text btn-icon post-action-menu__copy-link btn-flat",
        &t(cx.list, "post.controls.copy_title"),
        &t(cx.list, "post.controls.copy_title"),
        "link",
        &format!(" data-share-url=\"{}\"", escape(&share_url(cx, number(p)))),
    ));
    if cx.viewer.is_some() {
        actions.push_str(&bookmark_button(cx, p));
    }
    if cx.topic.can_create_post && cx.viewer.is_some() {
        let username = s(&p["username"]);
        actions.push_str(&format!(
            "<button aria-label=\"{}\" class=\"btn btn-icon-text post-action-menu__reply reply create fade-out btn-flat\" data-post-number=\"{}\" title=\"{}\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
            escape(&t_with(
                cx.list,
                "post.sr_reply_to",
                &[
                    ("post_number", &number(p).to_string()),
                    ("username", username)
                ]
            )),
            number(p),
            escape(&t(cx.list, "post.controls.reply")),
            icon("reply", None),
            escape(&t(cx.list, "topic.reply.title"))
        ));
    }
    format!(
        "<nav class=\"post-controls expanded\" role=\"none\"><div class=\"actions\">{actions}</div></nav><div class=\"small-user-list  who-read\"><span aria-atomic=\"true\" aria-live=\"polite\" class=\"small-user-list-content\" role=\"list\"></span></div>"
    )
}

fn number(p: &Value) -> i64 {
    p["post_number"].as_i64().unwrap_or(0)
}

/// A DButton with an icon and no label.
fn button(class: &str, label: &str, title: &str, icon_name: &str, attrs: &str) -> String {
    format!(
        "<button aria-label=\"{}\" class=\"{class}\"{attrs} title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
        escape(label),
        escape(title),
        d_icon(icon_name, None)
    )
}

/// post/menu/buttons/like: the like count and the toggle. A member's
/// like posts to /post_actions and comes back over the live stream.
fn like_button(cx: &PostContext, p: &Value) -> String {
    let like = p["actions_summary"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["id"].as_i64() == Some(LIKE)));
    let count = like.and_then(|l| l["count"].as_i64()).unwrap_or(0);
    let acted = like.is_some_and(|l| l["acted"] == true);
    let can_act = like.is_some_and(|l| l["can_act"] == true);
    let can_undo = like.is_some_and(|l| l["can_undo"] == true);
    let yours = p["yours"] == true;
    let show_like =
        cx.viewer.is_none() || (cx.topic.archived && !yours) || acted || can_act || can_undo;
    if !show_like && count == 0 {
        return String::new();
    }
    let l = cx.list;
    let count_button = if count > 0 {
        format!(
            "<button aria-expanded=\"false\" aria-haspopup=\"dialog\" aria-label=\"{}\" class=\"btn btn-flat no-text post-action-menu__like-count like-count button-count highlight-action{} {} btn-flat\" id=\"post-liked-users-list-{}\" type=\"button\">{}{count}</button>",
            escape(&t_count(l, "post.sr_post_like_count_button", count, &[])),
            if acted { " has-liked" } else { "" },
            if yours { "my-likes" } else { "regular-likes" },
            p["id"],
            if yours {
                d_icon("d-liked", None)
            } else {
                String::new()
            }
        )
    } else {
        String::new()
    };
    if !show_like {
        return format!("<div class=\"double-button\">{count_button}</div>");
    }
    let title = if acted {
        if can_undo {
            t(l, "post.controls.undo_like")
        } else {
            t(l, "post.controls.has_liked")
        }
    } else {
        t(l, "post.controls.like")
    };
    let disabled = if cx.viewer.is_none() {
        cx.topic.archived
    } else {
        !(can_act || can_undo)
    };
    let base = cx.list.base_path;
    let id = &p["id"];
    let htmx = if cx.viewer.is_none() || disabled {
        String::new()
    } else if acted {
        format!(
            " hx-delete=\"{base}/post_actions/{id}\" hx-vals='{{\"post_action_type_id\": {LIKE}}}' hx-swap=\"none\""
        )
    } else {
        format!(
            " hx-post=\"{base}/post_actions\" hx-vals='{{\"id\": {id}, \"post_action_type_id\": {LIKE}}}' hx-swap=\"none\""
        )
    };
    format!(
        "<div class=\"double-button{} post-action-menu__double-button\">{count_button}<button aria-label=\"{}\" class=\"btn no-text btn-icon post-action-menu__like toggle-like btn-icon {} btn-flat\" data-post-id=\"{id}\"{htmx} title=\"{}\" type=\"button\"{}>{}<span aria-hidden=\"true\">&#8203;</span></button></div>",
        if acted { " has-liked" } else { "" },
        escape(&title),
        if acted { "has-like" } else { "like" },
        escape(&title),
        if disabled { " disabled" } else { "" },
        d_icon(if acted { "d-liked" } else { "d-unliked" }, None)
    )
}

/// post/menu/buttons/bookmark: here a toggle (Ember's menu of reminders
/// is not ported). Nothing publishes a bookmark, so the page fetches the
/// post again after.
fn bookmark_button(cx: &PostContext, p: &Value) -> String {
    let base = cx.list.base_path;
    let id = &p["id"];
    let bookmark_id = p["bookmark_id"].as_i64();
    let reminder = !p["bookmark_reminder_at"].is_null();
    let (icon_name, title, extra, htmx) = match bookmark_id {
        Some(b) => (
            if reminder {
                "discourse-bookmark-clock"
            } else {
                "bookmark"
            },
            t_with(cx.list, "bookmarks.created_generic", &[("name", "")]),
            if reminder {
                " bookmarked with-reminder"
            } else {
                " bookmarked"
            },
            format!(" hx-delete=\"{base}/bookmarks/{b}\""),
        ),
        None => (
            "far-bookmark",
            t(cx.list, "bookmarks.not_bookmarked"),
            "",
            format!(
                " hx-post=\"{base}/bookmarks\" hx-vals='{{\"bookmarkable_id\": {id}, \"bookmarkable_type\": \"Post\"}}'"
            ),
        ),
    };
    format!(
        "<button aria-expanded=\"false\" class=\"btn no-text btn-icon fk-d-menu__trigger bookmark-menu-trigger post-action-menu__bookmark btn-flat bookmark widget-button bookmark-menu__trigger btn-icon no-text{extra}\" data-identifier=\"bookmark-menu\" data-trigger=\"\"{htmx} hx-swap=\"none\" hx-on::after-request=\"refreshPost(event, '{base}/live/post/{id}')\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
        escape(title.trim()),
        icon(icon_name, None)
    )
}

/// post/actions-summary: the viewer's own flags, and who deleted it.
fn actions_summary(cx: &PostContext, p: &Value) -> String {
    let mut out = String::new();
    for action in p["actions_summary"].as_array().into_iter().flatten() {
        if action["acted"] != true || action["id"].as_i64() == Some(LIKE) {
            continue;
        }
        let key = match action["id"].as_i64() {
            Some(3) => "post.actions.by_you.off_topic",
            Some(4) => "post.actions.by_you.inappropriate",
            Some(7) => "post.actions.by_you.notify_moderators",
            Some(8) => "post.actions.by_you.spam",
            Some(6) => "post.actions.by_you.notify_user",
            Some(10) => "post.actions.by_you.illegal",
            _ => continue,
        };
        out.push_str(&format!(
            "<div class=\"post-action\">{}</div><div class=\"clearfix\"></div>",
            escape(&t(cx.list, key))
        ));
    }
    if let (Some(at), Some(by)) = (date(&p["deleted_at"]), p["deleted_by"]["username"].as_str()) {
        out.push_str(&format!(
            "<div class=\"post-action deleted-post\">{}<a{}>{}</a>{}</div>",
            icon("trash-can", None),
            user_link_attrs(cx, by, false),
            avatar_img(
                s(&p["deleted_by"]["avatar_template"]),
                cx.settings.avatar_size_24,
                &format!(" title=\"{}\"", escape(by))
            ),
            tiny_date(cx, at)
        ));
    }
    out
}

/// The icon a small action shows (post/small-action's ICONS).
fn small_action_icon(code: &str) -> &'static str {
    match code {
        "closed.enabled" | "autoclosed.enabled" => "topic.closed",
        "closed.disabled" | "autoclosed.disabled" => "topic.opened",
        "archived.enabled" => "folder",
        "archived.disabled" => "folder-open",
        "pinned.enabled" | "pinned_globally.enabled" | "banner.enabled" => "thumbtack",
        "pinned.disabled" | "pinned_globally.disabled" | "banner.disabled" => "thumbtack unpinned",
        "visible.enabled" => "far-eye",
        "visible.disabled" => "far-eye-slash",
        "split_topic" => "right-from-bracket",
        "invited_user" | "invited_group" => "circle-plus",
        "user_left" | "removed_user" | "removed_group" => "circle-minus",
        "public_topic" | "open_topic" => "comment",
        "private_topic" => "envelope",
        "autobumped" => "hand-point-right",
        _ => "exclamation",
    }
}

/// post/small-action: an icon, the actor and what they did.
fn small_action(cx: &PostContext, p: &Value) -> String {
    let number = number(p);
    let code = s(&p["action_code"]);
    let who = s(&p["action_code_who"]);
    let username = s(&p["username"]);
    let created = date(&p["created_at"]);
    let key = format!("action_codes.{code}");
    let plain_when = created
        .map(|at| relative_age_medium(cx, at, true))
        .unwrap_or_default();
    let at_who = if who.is_empty() {
        String::new()
    } else {
        format!("@{who}")
    };
    let heading = t_with(
        cx.list,
        &key,
        &[("who", &at_who), ("when", &plain_when), ("path", "")],
    );
    let when_html = match created {
        Some(at) => format!(
            "<span class=\"relative-date\" data-time=\"{}\" data-format=\"medium-with-ago-and-on\"><span class=\"date\" title=\"{}\">{}</span></span>",
            at.timestamp_millis(),
            escape(&long_date(cx.list, at)),
            escape(&plain_when)
        ),
        None => String::new(),
    };
    let who_html = if who.is_empty() {
        String::new()
    } else if code == "invited_group" || code == "removed_group" {
        format!(
            "<a class=\"mention-group\" href=\"{}/g/{who}\">@{}</a>",
            cx.list.base_path,
            escape(who)
        )
    } else {
        format!(
            "<a class=\"mention\" href=\"{}/u/{who}\">@{}</a>",
            cx.list.base_path,
            escape(who)
        )
    };
    // The description with its placeholders as HTML: translate with
    // markers, escape, then put the markup in.
    let desc = escape(&t_with(
        cx.list,
        &key,
        &[("who", "\u{1}"), ("when", "\u{2}"), ("path", "")],
    ))
    .replace('\u{1}', &who_html)
    .replace('\u{2}', &when_html);
    let title = display_name(cx, username, p["name"].as_str());
    let title = p["user_title"]
        .as_str()
        .filter(|t| !t.is_empty())
        .unwrap_or(title);
    let custom = s(&p["cooked"]);
    let custom = if custom.is_empty() {
        String::new()
    } else {
        format!(
            "<div class=\"small-action-custom-message\"><div class=\"cooked\">{custom}<div class=\"cooked-selection-barrier\" aria-hidden=\"true\"><br></div></div></div>"
        )
    };
    format!(
        "<div data-post-number=\"{number}\"><h2 aria-hidden=\"false\" class=\"sr-only\" id=\"post-heading-{number}\">{}</h2><article aria-labelledby=\"post-heading-{number}\" class=\"small-action onscreen-post{}\" data-post-id=\"{}\" data-user-id=\"{}\" id=\"post_{number}\"><div class=\"topic-avatar\">{}</div><div class=\"small-action-desc\"><div class=\"small-action-contents\"><a{}>{}</a><p aria-hidden=\"true\">{desc}</p></div><div class=\"small-action-buttons\"></div>{custom}</div></article></div>",
        escape(&heading),
        if p["deleted_at"].is_null() {
            ""
        } else {
            " deleted"
        },
        p["id"],
        p["user_id"]
            .as_i64()
            .map(|i| i.to_string())
            .unwrap_or_default(),
        d_icon(small_action_icon(code), None),
        user_link_attrs(cx, username, false),
        avatar_img(
            s(&p["avatar_template"]),
            cx.settings.avatar_size_24,
            &format!(" title=\"{}\"", escape(title))
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaced_icons_keep_their_names_as_classes() {
        let liked = d_icon("d-unliked", None);
        assert!(liked.contains("d-icon-d-unliked "), "{liked}");
        assert!(liked.contains("href=\"#far-heart\""), "{liked}");
        let closed = d_icon("topic.closed", None);
        assert!(closed.contains("d-icon-lock "), "{closed}");
        assert!(closed.contains("href=\"#lock\""), "{closed}");
        let unpinned = d_icon("thumbtack unpinned", None);
        assert!(
            unpinned.contains("d-icon-thumbtack unpinned "),
            "{unpinned}"
        );
        assert!(unpinned.contains("href=\"#thumbtack\""), "{unpinned}");
    }

    fn render(posts: &[Value], now: &str, check: impl Fn(&[String])) {
        let i18n = crate::i18n::I18n::vendored().unwrap();
        let categories = std::collections::HashMap::new();
        let list = ListContext {
            i18n: &i18n,
            base_path: "",
            now: DateTime::parse_from_rfc3339(now)
                .unwrap()
                .with_timezone(&Utc),
            categories: &categories,
            expand_all_pinned: false,
            member_trust_level: None,
            settings: crate::topic_list_view::ListSettings {
                show_pinned_excerpt_desktop: true,
                suppress_uncategorized_badge: true,
                uncategorized_category_id: 1,
                tag_style: "simple".into(),
                suppress_overlapping_tags_in_list: false,
                topic_views_heat: [1000, 2000, 5000],
                topic_post_like_heat: [0.5, 1.0, 2.0],
                cold_age_days: [14.0, 30.0, 60.0],
                relative_date_duration: 30,
                avatar_size_24: 24,
                prioritize_name: false,
                inline_emoji: false,
                emoji_set: "twitter".into(),
                support_mixed_text_direction: false,
            },
        };
        let settings = PostSettings {
            prioritize_username_in_ux: true,
            display_name_on_posts: false,
            hide_user_profiles_from_public: false,
            avatar_size_24: 24,
            avatar_size_48: 48,
            enable_badges: true,
            allow_username_in_share_links: true,
            suppress_reply_directly_above: true,
            show_time_gap_days: 7,
            old_post_notice_days: 14,
        };
        let topic = TopicInfo {
            id: 9,
            slug: "t".into(),
            created_by_id: Some(1),
            archived: false,
            can_create_post: false,
        };
        let cx = PostContext {
            list: &list,
            settings: &settings,
            topic: &topic,
            viewer: None,
        };
        check(&stream(&cx, posts));
    }

    fn post_json(number: i64, created_at: &str, reply_to: Option<i64>) -> Value {
        let mut p = serde_json::json!({
            "id": number + 100, "post_number": number, "post_type": 1,
            "user_id": 1, "username": "alice", "avatar_template": "/a/{size}.png",
            "created_at": created_at, "cooked": "<p>hi</p>", "read": true,
            "actions_summary": [],
        });
        if let Some(n) = reply_to {
            p["reply_to_post_number"] = n.into();
            p["reply_to_user"] =
                serde_json::json!({"username": "bob", "avatar_template": "/b/{size}.png"});
        }
        p
    }

    #[test]
    fn posts_far_apart_get_a_time_gap() {
        let posts = [
            post_json(1, "2026-09-01T10:00:00Z", None),
            post_json(2, "2026-09-11T11:00:00Z", None),
            post_json(3, "2026-09-12T11:00:00Z", None),
        ];
        render(&posts, "2026-10-06T12:00:00Z", |html| {
            assert!(!html[0].contains("time-gap"));
            assert!(
                html[1].starts_with(
                    r#"<div class="time-gap small-action"><div class="topic-avatar"></div><div class="small-action-desc timegap">10 days later</div></div>"#
                ),
                "{}",
                html[1]
            );
            assert!(!html[2].contains("time-gap"));
        });
    }

    #[test]
    fn the_reply_to_tab_hides_under_its_parent() {
        let posts = [
            post_json(1, "2026-10-06T08:00:00Z", None),
            post_json(2, "2026-10-06T09:00:00Z", Some(1)),
            post_json(3, "2026-10-06T10:00:00Z", Some(1)),
        ];
        render(&posts, "2026-10-06T12:00:00Z", |html| {
            assert!(!html[1].contains("reply-to-tab"), "{}", html[1]);
            assert!(html[2].contains(
                r#"<a class="reply-to-tab" href="" role="button" title="Load parent post">"#
            ));
            assert!(html[2].contains("avoid-tab"));
            // The date, tiny and medium.
            assert!(
                html[2].contains(r#"aria-label="2 hours ago""#),
                "{}",
                html[2]
            );
            assert!(html[2].contains(r#"data-format="tiny">2h</span>"#));
            assert!(html[2].contains("post by alice 2 hours ago"));
        });
    }

    #[test]
    fn old_posts_are_dated_on_their_day() {
        render(
            &[post_json(1, "2026-09-30T09:28:00Z", None)],
            "2026-10-06T12:00:00Z",
            |html| {
                assert!(
                    html[0].contains(">post by alice on Sep 30</h2>"),
                    "{}",
                    html[0]
                );
                assert!(html[0].contains(r#"aria-label="Sep 30" class="post-date" href="/t/t/9""#));
            },
        );
    }

    /// Every client string the posts use is in the vendored locale.
    #[test]
    fn the_strings_exist() {
        let i18n = crate::i18n::I18n::vendored().unwrap();
        let plain = [
            "now",
            "dates.tiny.date_month",
            "dates.medium.date_year",
            "dates.wrap_on",
            "dates.long_with_year",
            "post.accessible_heading",
            "post.controls.menu_label",
            "post.notice.new_user",
            "post.notice.returning_user",
            "user.profile_possessive",
            "user.moderator_tooltip",
            "post.whisper",
            "post.via_email",
            "post.via_auto_generated_email",
            "post.locked",
            "post.sr_date",
            "post.unread",
            "post.last_edited_on",
            "post.wiki_last_edited_on",
            "post.wiki.about",
            "post.edit_history",
            "post.in_reply_to",
            "post.controls.copy_title",
            "post.controls.like",
            "post.controls.undo_like",
            "post.controls.has_liked",
            "post.controls.reply",
            "post.sr_reply_to",
            "topic.reply.title",
            "bookmarks.created_generic",
            "bookmarks.not_bookmarked",
            "post.actions.by_you.off_topic",
            "post.actions.by_you.inappropriate",
            "post.actions.by_you.notify_moderators",
            "post.actions.by_you.spam",
            "post.actions.by_you.notify_user",
            "post.actions.by_you.illegal",
            "action_codes.closed.enabled",
            "action_codes.split_topic",
        ];
        for key in plain {
            assert!(i18n.t(&format!("js.{key}")).is_some(), "js.{key}");
        }
        for key in [
            "dates.medium_with_ago.x_minutes",
            "dates.later.x_days",
            "post.sr_post_like_count_button",
        ] {
            assert!(
                i18n.t_count(&format!("js.{key}"), 2, &[]).is_some(),
                "js.{key}"
            );
        }
    }
}
