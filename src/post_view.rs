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
    pub read_time_word_count: i64,
    pub show_topic_map_in_topics_without_replies: bool,
    pub show_bottom_topic_map: bool,
    /// `post_menu`: the menu's buttons, in order.
    pub post_menu: Vec<String>,
    /// `post_menu_hidden_items`: the buttons behind show more.
    pub post_menu_hidden_items: Vec<String>,
}

impl PostSettings {
    pub fn load(settings: &SiteSettings) -> Result<PostSettings, SettingError> {
        let flag = |name: &str| -> Result<bool, SettingError> { Ok(settings.get(name)?.truthy()) };
        let list = |name: &str| -> Result<Vec<String>, SettingError> {
            Ok(settings
                .get(name)?
                .to_s()
                .split('|')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect())
        };
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
            read_time_word_count: settings.get("read_time_word_count")?.to_i(),
            show_topic_map_in_topics_without_replies: flag(
                "show_topic_map_in_topics_without_replies",
            )?,
            show_bottom_topic_map: flag("show_bottom_topic_map")?,
            post_menu: list("post_menu")?,
            post_menu_hidden_items: list("post_menu_hidden_items")?,
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
    /// The topic is deleted (`topic.deleted`).
    pub deleted: bool,
    /// `details.can_delete` and `details.can_recover`, for the first
    /// post's delete button.
    pub can_delete: bool,
    pub can_recover: bool,
    /// Where Mark unread goes after (deferTopic): the home page, or for a
    /// message the viewer's inbox (User#pmPath).
    pub defer_to: String,
    /// The first post's topic map, rendered (empty when it has none).
    pub op_map: String,
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
            deleted: view["deleted_at"].is_string(),
            can_delete: view["details"]["can_delete"] == true,
            can_recover: view["details"]["can_recover"] == true,
            defer_to: "/".to_string(),
            op_map: String::new(),
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
    /// The viewer is staff.
    pub staff: bool,
    /// The viewer can send private messages (canSendPms).
    pub can_send_pms: bool,
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
pub(crate) fn d_icon(name: &str, extra: Option<&str>) -> String {
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
        "d-post-share" | "d-topic-share" => "arrow-up-from-bracket",
        "topic.closed" => "lock",
        "d-muted" => "discourse-bell-slash",
        "d-regular" => "far-bell",
        "d-tracking" => "bell",
        "d-watching" => "discourse-bell-exclamation",
        "d-watching-first" => "discourse-bell-one",
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
        crate::topic_list_view::tags_html(list, view, title, &[])
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
    // The plugins' post classes: discourse-topic-voting's on a votable
    // topic's first post.
    if number == 1 && flag("can_vote") {
        classes.push("voting-post");
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

    let map = if number == 1 {
        cx.topic.op_map.as_str()
    } else {
        ""
    };
    format!(
        "<div class=\"{}\" data-post-number=\"{number}\"><h2 aria-hidden=\"false\" class=\"sr-only\" id=\"post-heading-{number}\">{}</h2><article aria-labelledby=\"post-heading-{number}\" class=\"{article_classes}\" data-post-id=\"{}\" data-user-id=\"{}\" id=\"post_{number}\">{}{}{map}</article></div>",
        classes.join(" "),
        escape(&heading),
        p["id"],
        user_id.map(|id| id.to_string()).unwrap_or_default(),
        notice(cx, p),
        main_row(cx, p, prev),
    )
}

/// A regular post's main row: the avatar and the body.
pub fn main_row(cx: &PostContext, p: &Value, prev: Option<Prev>) -> String {
    let reply_tab = reply_to_tab(cx, p, prev);
    let contents_class = if reply_tab.is_empty() {
        "post__regular regular post__contents contents"
    } else {
        "post__regular regular post__contents contents post__contents--avoid-tab avoid-tab"
    };
    format!(
        "<div class=\"post__row row\">{}<div class=\"post__body topic-body clearfix\">{}<div class=\"{contents_class}\"><div class=\"cooked\">{}<div class=\"cooked-selection-barrier\" aria-hidden=\"true\"><br></div></div><section aria-label=\"{}\" class=\"post__menu-area post-menu-area clearfix\" role=\"group\">{}</section></div><section class=\"post__actions post-actions\">{}</section></div></div>",
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

/// post/menu.gjs: the `post_menu` buttons in order, show more before the
/// last. While collapsed, the `post_menu_hidden_items` the post would
/// draw hide behind show more when there are two or more (counting the
/// configured ones first, as availableCollapsedButtons does). The read
/// indicator, share, admin and translation buttons are not drawn; admin
/// still counts toward collapsing for those who would see it.
fn menu(cx: &PostContext, p: &Value) -> String {
    let member = cx.viewer.is_some();
    let can_edit = member && p["can_edit"] == true;
    let wiki = p["wiki"] == true && can_edit;
    let mut configured: Vec<&str> = cx
        .settings
        .post_menu
        .iter()
        .map(|key| match (wiki, key.as_str()) {
            (true, "edit") => "reply",
            (true, "reply") => "edit",
            (_, key) => key,
        })
        .collect();
    if !configured.is_empty() && !configured.contains(&"showMore") {
        configured.insert(configured.len() - 1, "showMore");
    }
    let bookmarked = !p["bookmark_id"].is_null() || p["bookmarked"] == true;
    let hidden_items: Vec<&str> = cx
        .settings
        .post_menu_hidden_items
        .iter()
        .map(String::as_str)
        .filter(|k| !(bookmarked && *k == "bookmark"))
        .collect();

    // Each configured button: whether it collapses (EditButton.hidden is
    // false for one's own editable post and wikis), and its HTML when it
    // renders.
    let mut buttons: Vec<(&str, bool, Option<String>)> = Vec::new();
    for key in &configured {
        let hideable = hidden_items.contains(key)
            && !(*key == "edit" && (wiki || (can_edit && p["yours"] == true)));
        let html = match *key {
            "like" => Some(like_button(cx, p)).filter(|h| !h.is_empty()),
            "copyLink" => Some(button(
                "btn no-text btn-icon post-action-menu__copy-link btn-flat",
                &t(cx.list, "post.controls.copy_title"),
                &t(cx.list, "post.controls.copy_title"),
                "link",
                &format!(" data-share-url=\"{}\"", escape(&share_url(cx, number(p)))),
            )),
            "flag" => (member && can_flag(cx, p) && p["hidden"] != true).then(|| flag_button(cx)),
            "edit" => can_edit.then(|| {
                button(
                    "btn no-text btn-icon post-action-menu__edit edit btn-flat",
                    &t(cx.list, "post.controls.edit"),
                    &t(cx.list, "post.controls.edit"),
                    "pencil",
                    &format!(
                        " data-post-id=\"{}\" data-post-number=\"{}\"",
                        p["id"],
                        number(p)
                    ),
                )
            }),
            "bookmark" => member.then(|| bookmark_button(cx, p)),
            "delete" => delete_button(cx, p),
            // Drawn only as a count toward collapsing (not ported).
            "admin" => (member && (cx.staff || p["can_wiki"] == true)).then(String::new),
            "reply" => (cx.topic.can_create_post && member).then(|| reply_button(cx, p)),
            _ => None,
        };
        buttons.push((key, hideable, html));
    }
    let available = buttons
        .iter()
        .filter(|(key, hideable, _)| *key != "showMore" && *hideable)
        .count();
    let collapsing = |hideable: bool, html: &Option<String>| hideable && html.is_some();
    let renderable = buttons
        .iter()
        .filter(|(_, hideable, html)| collapsing(*hideable, html))
        .count();
    let collapsed = available > 1 && renderable > 1;

    let mut actions = String::new();
    for (key, hideable, html) in &buttons {
        if *key == "showMore" {
            if collapsed {
                actions.push_str(&button(
                    "btn no-text btn-icon post-action-menu__show-more show-more-actions btn-flat",
                    &t(cx.list, "show_more"),
                    &t(cx.list, "show_more"),
                    "ellipsis",
                    "",
                ));
            }
            continue;
        }
        let Some(html) = html else { continue };
        if collapsed && *hideable {
            actions.push_str(&html.replacen("<button ", "<button hidden ", 1));
        } else {
            actions.push_str(html);
        }
    }
    format!(
        "<nav class=\"post-controls {}\" role=\"none\"><div class=\"actions\">{actions}</div></nav><div class=\"small-user-list  who-read\"><span aria-atomic=\"true\" aria-live=\"polite\" class=\"small-user-list-content\" role=\"list\"></span></div>",
        if collapsed { "collapsed" } else { "expanded" }
    )
}

/// Post#canFlag: the topic stands and some flag type (any action but a
/// like, Site#flagTypes) can act.
fn can_flag(cx: &PostContext, p: &Value) -> bool {
    !cx.topic.deleted
        && p["actions_summary"].as_array().is_some_and(|a| {
            a.iter()
                .any(|x| x["id"].as_i64() != Some(LIKE) && x["can_act"] == true)
        })
}

/// post/menu/buttons/flag (the flag modal is not ported).
fn flag_button(cx: &PostContext) -> String {
    button(
        "btn no-text btn-icon post-action-menu__flag create-flag btn-flat",
        &t(cx.list, "post.controls.flag"),
        &t(cx.list, "post.controls.flag"),
        "flag",
        "",
    )
}

/// post/menu/buttons/delete: PostMenuDeleteButton.modeFor. Deleting a
/// reply posts to DELETE /posts/:id; the topic and recover modes, and the
/// modal for one's own first post, are not wired.
fn delete_button(cx: &PostContext, p: &Value) -> Option<String> {
    cx.viewer?;
    let first = number(p) == 1;
    let deleted = !p["deleted_at"].is_null();
    let user_deleted = p["user_deleted"] == true;
    let can_recover_topic = first && (deleted || user_deleted) && cx.topic.can_recover;
    let can_delete_topic = first && !deleted && cx.topic.can_delete;
    let can_delete = p["can_delete"] == true && !deleted && (cx.staff || !user_deleted);
    let can_recover = !can_recover_topic && p["can_recover"] == true && deleted;
    let show_flag_delete = !can_delete && p["yours"] == true && can_flag(cx, p) && !cx.staff;
    let base = cx.list.base_path;
    let id = &p["id"];
    let (recover, title, attrs) = if can_recover_topic {
        (true, "topic.actions.recover", String::new())
    } else if can_delete_topic {
        (false, "post.controls.delete_topic", String::new())
    } else if can_recover {
        (true, "post.controls.undelete", String::new())
    } else if can_delete {
        (
            false,
            "post.controls.delete",
            format!(
                " hx-delete=\"{base}/posts/{id}\" hx-swap=\"none\" hx-on::after-request=\"refreshPost(event, '{base}/live/post/{id}')\""
            ),
        )
    } else if show_flag_delete {
        (
            false,
            "post.controls.delete_topic_disallowed",
            String::new(),
        )
    } else {
        return None;
    };
    let title = t(cx.list, title);
    Some(button(
        if recover {
            "btn no-text btn-icon post-action-menu__recover recover btn-flat"
        } else {
            "btn no-text btn-icon post-action-menu__delete delete btn-flat"
        },
        &title,
        &title,
        if recover {
            "arrow-rotate-left"
        } else {
            "trash-can"
        },
        &attrs,
    ))
}

/// post/menu/buttons/reply
fn reply_button(cx: &PostContext, p: &Value) -> String {
    let username = s(&p["username"]);
    format!(
        "<button aria-label=\"{}\" class=\"btn btn-icon-text post-action-menu__reply reply create fade-out btn-flat\" data-post-number=\"{}\" data-username=\"{}\" title=\"{}\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
        escape(&t_with(
            cx.list,
            "post.sr_reply_to",
            &[
                ("post_number", &number(p).to_string()),
                ("username", username)
            ]
        )),
        number(p),
        escape(username),
        escape(&t(cx.list, "post.controls.reply")),
        icon("reply", None),
        escape(&t(cx.list, "topic.reply.title"))
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

/// `number()` (lib/formatter): 1.2k, 345k, 1.2M; past `max` it shows
/// `max+`.
fn d_number(n: i64, max: Option<i64>) -> String {
    if let Some(max) = max
        && n > max
    {
        return format!("{max}+");
    }
    let n = n as f64;
    if n > 999_999.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if n > 99_999.0 {
        format!("{}k", (n / 1000.0).floor())
    } else if n > 999.0 {
        format!("{:.1}k", n / 1000.0)
    } else {
        format!("{}", n.round())
    }
}

/// A topic-map stat: the number and its label.
fn stat_body(cx: &PostContext, n: i64, max: Option<i64>, key: &str) -> String {
    format!(
        "<span class=\"number\">{}</span> <span class=\"topic-map__stat-label\">{}</span>",
        d_number(n, max),
        escape(&t_count(cx.list, key, n, &[]))
    )
}

/// components/topic-map: the topic's views, likes, links and users, and
/// its most frequent posters. `modifier` is `--op` (in the first post) or
/// `--bottom`. The stats that open menus (likes, links, users) show as
/// plain stats, as Ember shows likes where search is off; the views
/// count keeps its trigger's markup.
pub fn topic_map(cx: &PostContext, view: &Value, modifier: &str) -> String {
    let n = |key: &str| view[key].as_i64().unwrap_or(0);
    let posts_count = n("posts_count");
    let links = view["details"]["links"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0) as i64;
    let has_likes = n("like_count") > 5 && posts_count > 3;
    let has_users = n("participant_count") > 5;
    let has_links = links > 0;
    let has_summary = view["has_summary"] == true;
    let mut stats_class = String::from("topic-map__stats");
    if !has_summary && !has_likes && !has_users && !has_links {
        stats_class.push_str(" --single-stat");
    }
    if has_likes && has_users && has_links {
        stats_class.push_str(" --many-stats");
    }
    let views = n("views").max(1);
    let mut stats = format!(
        "<button aria-expanded=\"false\" class=\"btn no-text fk-d-menu__trigger topic-map__views-trigger\" data-identifier=\"topic-map__views\" data-trigger=\"\" type=\"button\">{}</button>",
        stat_body(cx, views, None, "views_lowercase")
    );
    if has_likes {
        stats.push_str(&format!(
            "<div class=\"topic-map__stat topic-map__likes\">{}</div>",
            stat_body(cx, n("like_count"), None, "likes_lowercase")
        ));
    }
    if has_links {
        stats.push_str(&format!(
            "<div class=\"topic-map__stat topic-map__links\">{}</div>",
            stat_body(cx, links, Some(50), "links_lowercase")
        ));
    }
    if has_users {
        stats.push_str(&format!(
            "<div class=\"topic-map__stat topic-map__users\">{}</div>",
            stat_body(cx, n("participant_count"), None, "users_lowercase")
        ));
    }
    let participants = view["details"]["participants"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if posts_count >= 3 && participants.len() >= 2 {
        stats.push_str("<div class=\"topic-map__users-list --users-summary\">");
        for p in participants.iter().take(5) {
            let username = s(&p["username"]);
            let group = p["primary_group_name"]
                .as_str()
                .filter(|g| !g.is_empty())
                .map(|g| format!("group-{g}"))
                .unwrap_or_default();
            let title = p["name"]
                .as_str()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or(username);
            let count = p["post_count"].as_i64().unwrap_or(0);
            stats.push_str(&format!(
                "<div class=\"{group}\"><a class=\"{}poster trigger-user-card\" title=\"{}\"{}>{}{}</a></div>",
                if cx.settings.hide_user_profiles_from_public && cx.viewer.is_none() {
                    "non-clickable "
                } else {
                    ""
                },
                escape(username),
                user_link_attrs(cx, username, false).replacen(" class=\"non-clickable\"", "", 1),
                avatar_img(
                    s(&p["avatar_template"]),
                    cx.settings.avatar_size_48,
                    &format!(" title=\"{}\"", escape(title))
                ),
                if count > 1 {
                    format!("<span class=\"post-count\">{count}</span>")
                } else {
                    String::new()
                }
            ));
        }
        stats.push_str("</div>");
    }
    // The estimated read time, past three minutes.
    let read_minutes = ((n("word_count") as f64 / cx.settings.read_time_word_count.max(1) as f64)
        .max(posts_count as f64 * 4.0 / 60.0))
    .ceil() as i64;
    let buttons = if read_minutes > 3 {
        format!(
            "<div class=\"estimated-read-time\"><span> {} </span><span> {read_minutes} {} </span></div>",
            escape(&t(cx.list, "topic_map.read")),
            escape(&t(cx.list, "topic_map.minutes"))
        )
    } else {
        String::new()
    };
    stats.push_str(&format!(
        "<div class=\"topic-map__buttons\">{buttons}</div>"
    ));
    let class = if modifier == "--op" {
        "post__topic-map topic-map --op"
    } else {
        "topic-map --bottom"
    };
    format!(
        "<div class=\"{class}\"><section class=\"topic-map__contents\"><div class=\"{stats_class}\">{stats}</div></section></div>"
    )
}

/// Whether the first post shows the topic map (post.gjs
/// shouldShowTopicMap).
pub fn shows_op_map(view: &Value, settings: &PostSettings) -> bool {
    match view["archetype"].as_str() {
        Some("private_message") => true,
        Some("regular") => {
            view["posts_count"].as_i64().unwrap_or(0) > 1
                || settings.show_topic_map_in_topics_without_replies
        }
        _ => false,
    }
}

/// Whether the bottom topic map shows (showBottomTopicMap), with every
/// post of the topic on the page.
pub fn shows_bottom_map(view: &Value, settings: &PostSettings) -> bool {
    let posts = view["post_stream"]["posts"].as_array();
    let regular = posts
        .map(|p| {
            p.iter()
                .filter(|p| p["post_type"].as_i64() != Some(SMALL_ACTION))
                .count()
        })
        .unwrap_or(0);
    let loaded = posts.map(Vec::len).unwrap_or(0);
    let stream = view["post_stream"]["stream"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    // The client means to require 200 words, but tests `isTesting` without
    // calling it, so the minimum never applies.
    settings.show_bottom_topic_map && regular > 3 && loaded >= stream
}

/// `timelineDate`: "Sep 30" this year, else "Sep 2025".
fn timeline_date(cx: &PostContext, at: DateTime<Utc>) -> String {
    use chrono::Datelike;
    let key = if at.year() == cx.list.now.year() {
        "dates.long_no_year_no_time"
    } else {
        "dates.timeline_date"
    };
    crate::pretty_text::render::local_dates::format_utc(at, &t(cx.list, key)).unwrap_or_default()
}

/// The docked timeline (components/topic-timeline) as it first renders,
/// at the first post; static/js/topic.js moves it as the page scrolls.
/// The stream's post ids and the timeline lookup ride along for that.
pub fn timeline(cx: &PostContext, view: &Value) -> String {
    let base = cx.list.base_path;
    let url = format!("{base}/t/{}/{}", cx.topic.slug, cx.topic.id);
    let stream: Vec<i64> = view["post_stream"]["stream"]
        .as_array()
        .map(|s| s.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default();
    let total = stream.len().max(1);
    let start = date(&view["created_at"])
        .map(|at| timeline_date(cx, at))
        .unwrap_or_default();
    let last = date(&view["last_posted_at"]).or_else(|| date(&view["created_at"]));
    let now_date = last
        .map(|at| {
            let tiny = relative_age_tiny(cx.list, at);
            // addAgo: only a relative age gets "ago".
            let age = if tiny.chars().next().is_some_and(|c| c.is_ascii_digit())
                && tiny.ends_with(['m', 'h', 'd'])
            {
                t_with(cx.list, "dates.wrap_ago", &[("date", &tiny)])
            } else {
                tiny
            };
            format!(
                "<span class=\"relative-date\" title=\"{}\" data-time=\"{}\" data-format=\"tiny\">{}</span>",
                escape(&t(cx.list, "topic_entrance.jump_bottom_button_title")),
                at.timestamp_millis(),
                escape(&age)
            )
        })
        .unwrap_or_default();
    // timeline-ago for the first post: the lookup's entry for index 1.
    let days_ago = view["timeline_lookup"]
        .as_array()
        .and_then(|l| l.first())
        .and_then(|e| e[1].as_i64());
    let ago = days_ago
        .map(|d| {
            format!(
                "<div class=\"timeline-ago\">{}</div>",
                escape(&timeline_date(cx, cx.list.now - chrono::Duration::days(d)))
            )
        })
        .unwrap_or_default();
    let mut footer = String::new();
    if cx.viewer.is_some() && cx.topic.can_create_post {
        footer.push_str(&format!(
            "<button class=\"btn no-text btn-icon btn-default create reply-to-post\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
            escape(&t(cx.list, "topic.reply.help")),
            icon("reply", None)
        ));
    }
    if cx.viewer.is_some() {
        footer.push_str(&notifications_button(cx, view, false));
    }
    let controls = if cx.viewer.is_some() {
        "<div class=\"timeline-controls\"></div>"
    } else {
        ""
    };
    let stream_json = serde_json::to_string(&stream).unwrap_or_default();
    // The lookup with each entry's date label, for the script.
    let labels: Vec<Value> = view["timeline_lookup"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            let label = e[1]
                .as_i64()
                .map(|d| timeline_date(cx, cx.list.now - chrono::Duration::days(d)));
            serde_json::json!([e[0], label])
        })
        .collect();
    let lookup_json = serde_json::to_string(&labels).unwrap_or_default();
    format!(
        "<div class=\"with-timeline topic-navigation\"><div class=\"timeline-container\" data-topic-url=\"{}\" data-chunk-size=\"{}\" data-stream=\"{}\" data-lookup=\"{}\" data-replies-format=\"{}\"><div class=\"topic-timeline\">{controls}<div class=\"timeline-scrollarea-wrapper\"><div class=\"timeline-date-wrapper\"><a class=\"start-date\" href=\"{}/1\" title=\"{}\"><span>{}</span></a></div><div class=\"timeline-scrollarea\" style=\"height: 300px\"><div class=\"timeline-padding\" style=\"height: 0px\"></div><div class=\"timeline-scroller\" style=\"height: 50px\"><div class=\"timeline-handle\"></div><div class=\"timeline-scroller-content\"><div class=\"timeline-replies\">{}</div>{ago}</div></div><div class=\"timeline-padding\" style=\"height: 250px\"></div></div><div class=\"timeline-date-wrapper\"><a class=\"now-date\" href=\"{}/{}\"><span>{now_date}</span></a></div></div><div class=\"timeline-footer-controls\">{footer}</div></div></div></div>",
        escape(&url),
        view["chunk_size"].as_i64().unwrap_or(20),
        escape(&stream_json),
        escape(&lookup_json),
        escape(&t(cx.list, "topic.timeline.replies_short")),
        escape(&url),
        escape(&t(cx.list, "topic_entrance.jump_top_button_title")),
        escape(&start),
        escape(&t_with(
            cx.list,
            "topic.timeline.replies_short",
            &[("current", "1"), ("total", &total.to_string())]
        )),
        escape(&url),
        view["highest_post_number"].as_i64().unwrap_or(1),
    )
}

/// The topic's footer buttons: an anonymous reader's Reply (to log in);
/// a member's share, bookmark and Reply. Flag, Mark unread, the pinned
/// and notifications buttons and the admin menu are not ported yet.
pub fn footer_buttons(cx: &PostContext, view: &Value) -> String {
    let l = cx.list;
    let reply_label = escape(&t(l, "topic.reply.title"));
    let Some(_) = cx.viewer else {
        return format!(
            "<div id=\"topic-footer-buttons\" role=\"region\"><div class=\"topic-footer-main-buttons\"><button class=\"btn btn-icon-text btn-primary\" data-login-url=\"{}/login\" type=\"button\">{}<span class=\"d-button-label\">{reply_label}</span></button></div></div>",
            l.base_path,
            icon("reply", None)
        );
    };
    let url = format!("{}/t/{}/{}", l.base_path, cx.topic.slug, cx.topic.id);
    let mut actions = format!(
        "<button class=\"btn btn-icon-text btn-default topic-footer-button share-and-invite\" aria-label=\"{share}\" data-share-url=\"{}\" id=\"topic-footer-button-share-and-invite\" title=\"{}\" type=\"button\">{}<span class=\"d-button-label\">{share}</span></button>",
        escape(&url),
        escape(&t(l, "topic.share.help")),
        d_icon("d-topic-share", None),
        share = escape(&t(l, "footer_nav.share"))
    );
    // The topic's own bookmark.
    let topic_bookmark = view["bookmarks"].as_array().and_then(|b| {
        b.iter()
            .find(|b| b["bookmarkable_type"] == "Topic")
            .and_then(|b| b["id"].as_i64())
    });
    let base = l.base_path;
    let (icon_name, label, title, extra, htmx) = match topic_bookmark {
        Some(id) => (
            "bookmark",
            t_count(l, "bookmarked.edit_bookmark", 1, &[]),
            t_with(l, "bookmarks.created_generic", &[("name", "")]),
            " bookmarked",
            format!(" hx-delete=\"{base}/bookmarks/{id}\""),
        ),
        None => (
            "far-bookmark",
            t(l, "bookmarked.title"),
            t(l, "bookmarks.not_bookmarked"),
            "",
            format!(
                " hx-post=\"{base}/bookmarks\" hx-vals='{{\"bookmarkable_id\": {}, \"bookmarkable_type\": \"Topic\"}}'",
                cx.topic.id
            ),
        ),
    };
    actions.push_str(&format!(
        "<button class=\"btn btn-icon-text fk-d-menu__trigger bookmark-menu-trigger bookmark widget-button bookmark-menu__trigger btn-icon-text btn-default topic-footer-button{extra}\" title=\"{}\" aria-expanded=\"false\" data-identifier=\"bookmark-menu\" data-trigger=\"\"{htmx} hx-swap=\"none\" hx-on::after-request=\"if (event.detail.successful) location.reload()\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
        escape(title.trim()),
        icon(icon_name, None),
        escape(&label)
    ));
    let private_message = view["archetype"] == "private_message";
    if view["details"]["can_flag_topic"] == true && !private_message {
        actions.push_str(&footer_button(
            cx,
            "flag",
            "flag-topic",
            "flag",
            "topic.flag_topic.title",
            "topic.flag_topic.help",
        ));
    }
    actions.push_str(
        &footer_button(
            cx,
            "defer",
            "defer-topic",
            "circle",
            "topic.defer.title",
            "topic.defer.help",
        )
        .replacen(
            " type=\"button\">",
            &format!(
                " data-defer-url=\"{}/t/{}/timings.json?last=1\" data-defer-to=\"{}\" type=\"button\">",
                l.base_path,
                cx.topic.id,
                escape(&cx.topic.defer_to)
            ),
            1,
        ),
    );
    let reply = if cx.topic.can_create_post {
        format!(
            "<button class=\"btn btn-icon-text btn-primary create topic-footer-button\" title=\"{}\" type=\"button\">{}<span class=\"d-button-label\">{reply_label}</span></button>",
            escape(&t(l, "topic.reply.help")),
            icon("reply", None)
        )
    } else {
        String::new()
    };
    // showNotificationsButton: PMs only for those who can send them.
    let notifications = if !private_message || cx.can_send_pms {
        notifications_button(cx, view, true).replacen(
            "class=\"topic-notifications-button\"",
            "class=\"topic-notifications-button notifications-button-footer\"",
            1,
        )
    } else {
        String::new()
    };
    format!(
        "<div aria-label=\"{}\" id=\"topic-footer-buttons\" role=\"region\"><div class=\"topic-footer-main-buttons\"><div class=\"topic-footer-main-buttons__actions\">{actions}</div>{reply}</div>{notifications}</div>{}",
        escape(&t(l, "topic.footer_buttons.region_label")),
        notifications_menu(cx, view)
    )
}

/// TopicNotificationsButton#reasonText: the level's reason with the
/// reason id when it has a translation, else the level's own.
fn reason_text(cx: &PostContext, level: i64, reason: Option<i64>) -> String {
    let l = cx.list;
    let mut key = format!("topic.notifications.reasons.{level}");
    if let Some(reason) = reason {
        let with_reason = format!("{key}_{reason}");
        if l.i18n.t(&format!("js.{with_reason}")).is_some() {
            key = with_reason;
        }
    }
    t_with(
        l,
        &key,
        &[
            ("username", &cx.viewer.unwrap_or_default().to_lowercase()),
            ("basePath", l.base_path),
        ],
    )
}

/// NotificationsTracking's menu content, once for the page: the topic
/// levels with their descriptions, the current one selected. Each posts
/// to /t/:id/notifications; static/js/topic.js floats it under the
/// trigger that opened it (Ember's DMenu portal) and, on success, sets the
/// level as TopicDetails#updateNotifications does (the reason cleared),
/// from the option's title, tooltip and reason.
fn notifications_menu(cx: &PostContext, view: &Value) -> String {
    let l = cx.list;
    let current = view["details"]["notification_level"].as_i64().unwrap_or(1);
    let suffix = if view["archetype"] == "private_message" {
        "_pm"
    } else {
        ""
    };
    let url = format!("{}/t/{}/notifications", l.base_path, cx.topic.id);
    let title = |key: &str| t(l, &format!("topic.notifications.{key}{suffix}.title"));
    let items: String = [(3, "watching"), (2, "tracking"), (1, "regular"), (0, "muted")]
        .iter()
        .map(|(level, key)| {
            format!(
                "<li class=\"dropdown-menu__item\"><button class=\"btn no-text notifications-tracking-btn{}\" data-level-id=\"{level}\" data-level-name=\"{key}\" data-title=\"{}\" data-tooltip=\"{}\" data-reason=\"{}\" hx-post=\"{url}\" hx-vals='{{\"notification_level\": {level}}}' hx-swap=\"none\" type=\"button\"><div class=\"notifications-tracking-btn__icons\">{}</div><div class=\"notifications-tracking-btn__texts\"><span class=\"notifications-tracking-btn__label\">{}</span><span class=\"notifications-tracking-btn__description\">{}</span></div></button></li>",
                if *level == current { " -selected" } else { "" },
                escape(&title(key)),
                escape(&t_with(l, "notifications_tracking.tooltip", &[("level", &title(key))])),
                escape(&reason_text(cx, *level, None)),
                d_icon(&format!("d-{key}"), None),
                escape(&title(key)),
                escape(&t(l, &format!("topic.notifications.{key}{suffix}.description"))),
            )
        })
        .collect();
    format!(
        "<div class=\"fk-d-menu notifications-tracking-content -animated\" data-content=\"\" data-identifier=\"notifications-tracking\" role=\"dialog\" data-strategy=\"absolute\" data-placement=\"bottom-start\" hidden><div class=\"fk-d-menu__inner-content\"><ul class=\"dropdown-menu\">{items}</ul></div></div>"
    )
}

/// A registered topic footer button (instance-initializers/
/// topic-footer-buttons). Flag's modal is not ported; defer is wired in
/// static/js/topic.js.
fn footer_button(
    cx: &PostContext,
    id: &str,
    class: &str,
    icon_name: &str,
    label: &str,
    title: &str,
) -> String {
    let label = escape(&t(cx.list, label));
    format!(
        "<button aria-label=\"{label}\" class=\"btn btn-icon-text btn-default topic-footer-button {class}\" id=\"topic-footer-button-{id}\" title=\"{}\" type=\"button\">{}<span class=\"d-button-label\">{label}</span></button>",
        escape(&t(cx.list, title)),
        icon(icon_name, None)
    )
}

/// TopicNotificationsButton: the notifications tracking trigger, and when
/// `expanded` its caret, full title and reason; the menu it opens is
/// notifications_menu. The reason's stale and mailing list mode variants
/// are not ported.
fn notifications_button(cx: &PostContext, view: &Value, expanded: bool) -> String {
    let l = cx.list;
    let details = &view["details"];
    let level = details["notification_level"].as_i64().unwrap_or(1);
    let key = match level {
        0 => "muted",
        2 => "tracking",
        3 => "watching",
        _ => "regular",
    };
    let suffix = if view["archetype"] == "private_message" {
        "_pm"
    } else {
        ""
    };
    let title = t(l, &format!("topic.notifications.{key}{suffix}.title"));
    let tooltip = t_with(l, "notifications_tracking.tooltip", &[("level", &title)]);
    let mut trigger = format!(
        "<button class=\"btn btn-default {} fk-d-menu__trigger notifications-tracking-trigger btn-default btn-icon notifications-tracking-trigger-btn topic-notifications-tracking\" title=\"{}\" aria-expanded=\"false\" data-identifier=\"notifications-tracking\" data-trigger=\"\" data-level-id=\"{level}\" data-level-name=\"{key}\" type=\"button\">{}",
        if expanded { "btn-icon-text" } else { "no-text" },
        escape(&tooltip),
        d_icon(&format!("d-{key}"), None)
    );
    if expanded {
        trigger.push_str(&format!(
            // The template's whitespace leaves a space after the title.
            "<span class=\"d-button-label\">{} </span>{}",
            escape(&title),
            icon("angle-down", Some("notifications-tracking-btn__caret"))
        ));
    }
    trigger.push_str("</button>");
    if !expanded {
        return format!("<div class=\"topic-notifications-button\">{trigger}</div>");
    }
    let reason = reason_text(cx, level, details["notifications_reason_id"].as_i64());
    format!(
        "<div class=\"topic-notifications-button\"><p class=\"reason\">{trigger}<span class=\"text\">{reason}</span></p></div>"
    )
}

/// more-topics: the suggested topics and BrowseMore's line, with the
/// member's new and unread counts (`topic.read_more_MF`). Private
/// messages' line (pm topic tracking) is not ported.
pub fn more_topics(
    cx: &PostContext,
    view: &Value,
    tracking: Option<&crate::topic_tracking_report::Tracking>,
) -> Result<String, crate::message_format::ParseError> {
    let l = cx.list;
    let suggested = view["suggested_topics"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if suggested.is_empty() {
        return Ok("<div class=\"more-topics__container\"></div>".to_string());
    }
    let rows: String = suggested
        .iter()
        .map(|topic| crate::topic_list_view::suggested_row(l, topic))
        .collect();
    let base = l.base_path;
    let category = view["category_id"]
        .as_i64()
        .filter(|id| *id != l.settings.uncategorized_category_id)
        .and_then(|id| l.categories.get(&id));
    use crate::topic_tracking_report::Kind;
    let (unread, new) = tracking
        .filter(|_| view["archetype"] != "private_message")
        .map(|t| {
            (
                t.count(Kind::Unread, None, None),
                t.count(Kind::New, None, None),
            )
        })
        .unwrap_or((0, 0));
    let unified_new = tracking.is_some_and(|t| t.unified_new);
    let category_link = category.map(|c| crate::topic_list_view::category_badge(l, c));
    let browse_more = if unread + new > 0 {
        use crate::message_format::{Arg, format};
        let message = l.i18n.t("js.topic.read_more_MF").unwrap_or_default();
        let (unread_url, new_url) = if unified_new {
            (
                format!("{base}/new?subset=replies"),
                format!("{base}/new?subset=topics"),
            )
        } else {
            (format!("{base}/unread"), format!("{base}/new"))
        };
        format(
            message,
            &[
                ("HAS_UNREAD_AND_NEW", Arg::Bool(unread > 0 && new > 0)),
                ("UNREAD", Arg::Num(unread)),
                ("NEW", Arg::Num(new)),
                ("HAS_CATEGORY", Arg::Bool(category.is_some())),
                (
                    "categoryLink",
                    Arg::Str(category_link.as_deref().unwrap_or("")),
                ),
                ("basePath", Arg::Str(base)),
                ("unreadUrl", Arg::Str(&unread_url)),
                ("newUrl", Arg::Str(&new_url)),
            ],
        )?
    } else {
        match category {
            Some(c) => t_with(
                l,
                "topic.read_more_in_category",
                &[
                    (
                        "categoryLink",
                        &crate::topic_list_view::category_badge(l, c),
                    ),
                    ("latestLink", &format!("{base}/latest")),
                ],
            ),
            None => t_with(
                l,
                "topic.read_more",
                &[
                    ("categoryLink", &format!("{base}/categories")),
                    ("latestLink", &format!("{base}/latest")),
                ],
            ),
        }
    };
    let th = |class: &str, label: &str| {
        format!(
            "<th class=\"topic-list-data {class}\" data-sort-order=\"{}\" scope=\"col\"><span>{label}</span></th>",
            class.split(' ').next().unwrap_or("")
        )
    };
    Ok(format!(
        "<div class=\"more-topics__container\"><div class=\"more-topics__lists single-list\"><div aria-labelledby=\"suggested-topics-title\" class=\"more-topics__list\" id=\"suggested-topics\" role=\"complementary\"><h3 class=\"more-topics__list-title\" id=\"suggested-topics-title\">{}</h3><div class=\"topics\"><table class=\"topic-list\"><caption class=\"sr-only\">{}</caption><thead class=\"topic-list-header --has-tabs\"><tr>{}{}{}{}</tr></thead><tbody class=\"topic-list-body\">{rows}</tbody></table></div></div></div><h3 class=\"more-topics__browse-more\">{browse_more}</h3></div>",
        escape(&t(l, "suggested_topics.title")),
        escape(&t(l, "sr_topic_list_caption")),
        th("default", "Topic"),
        th("posts num", "Replies"),
        th("views num", "Views"),
        th("activity num", "Activity"),
    ))
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
            read_time_word_count: 500,
            show_topic_map_in_topics_without_replies: true,
            show_bottom_topic_map: true,
            post_menu: "read|like|copyLink|flag|edit|bookmark|delete|admin|reply"
                .split('|')
                .map(str::to_string)
                .collect(),
            post_menu_hidden_items: "flag|bookmark|edit|delete|admin"
                .split('|')
                .map(str::to_string)
                .collect(),
        };
        let topic = TopicInfo {
            id: 9,
            slug: "t".into(),
            created_by_id: Some(1),
            archived: false,
            can_create_post: false,
            deleted: false,
            can_delete: false,
            can_recover: false,
            defer_to: "/".to_string(),
            op_map: String::new(),
        };
        let cx = PostContext {
            list: &list,
            settings: &settings,
            topic: &topic,
            viewer: None,
            staff: false,
            can_send_pms: false,
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

    #[test]
    fn numbers_shorten_as_the_client_does() {
        assert_eq!(d_number(0, None), "0");
        assert_eq!(d_number(999, None), "999");
        assert_eq!(d_number(1000, None), "1.0k");
        assert_eq!(d_number(1234, None), "1.2k");
        assert_eq!(d_number(123_456, None), "123k");
        assert_eq!(d_number(1_234_567, None), "1.2M");
        assert_eq!(d_number(51, Some(50)), "50+");
    }

    #[test]
    fn the_topic_map_shows_what_the_topic_has() {
        let i18n = crate::i18n::I18n::vendored().unwrap();
        let categories = std::collections::HashMap::new();
        let list = ListContext {
            i18n: &i18n,
            base_path: "",
            now: Utc::now(),
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
            read_time_word_count: 500,
            show_topic_map_in_topics_without_replies: true,
            show_bottom_topic_map: true,
            post_menu: "read|like|copyLink|flag|edit|bookmark|delete|admin|reply"
                .split('|')
                .map(str::to_string)
                .collect(),
            post_menu_hidden_items: "flag|bookmark|edit|delete|admin"
                .split('|')
                .map(str::to_string)
                .collect(),
        };
        let topic = TopicInfo {
            id: 9,
            slug: "t".into(),
            created_by_id: Some(1),
            archived: false,
            can_create_post: false,
            deleted: false,
            can_delete: false,
            can_recover: false,
            defer_to: "/".to_string(),
            op_map: String::new(),
        };
        let cx = PostContext {
            list: &list,
            settings: &settings,
            topic: &topic,
            viewer: None,
            staff: false,
            can_send_pms: false,
        };
        let participants: Vec<Value> = (0..7)
            .map(|i| {
                serde_json::json!({"username": format!("u{i}"), "avatar_template": "/a/{size}.png",
                                   "post_count": 7 - i})
            })
            .collect();
        // A quiet topic: one stat, at least one view.
        let quiet = serde_json::json!({"views": 0, "posts_count": 2, "like_count": 0,
            "participant_count": 1, "details": {"participants": []}});
        let html = topic_map(&cx, &quiet, "--op");
        assert!(html.starts_with(r#"<div class="post__topic-map topic-map --op"><section class="topic-map__contents"><div class="topic-map__stats --single-stat"><button"#), "{html}");
        assert!(html.contains(
            r#"<span class="number">1</span> <span class="topic-map__stat-label">view</span>"#
        ));
        // A busy one: likes, users and links, the five top posters, and
        // the read time.
        let busy = serde_json::json!({"views": 1234, "posts_count": 40, "like_count": 9,
            "participant_count": 7, "word_count": 3000,
            "details": {"participants": participants, "links": [{"url": "x"}]}});
        let html = topic_map(&cx, &busy, "--bottom");
        assert!(html.starts_with(r#"<div class="topic-map --bottom">"#));
        assert!(
            html.contains(r#"<div class="topic-map__stats --many-stats">"#),
            "{html}"
        );
        assert!(html.contains(
            "<span class=\"number\">1.2k</span> <span class=\"topic-map__stat-label\">views</span>"
        ));
        assert!(html.contains(
            r#"<div class="topic-map__stat topic-map__likes"><span class="number">9</span>"#
        ));
        assert!(html.contains(r#"<div class="topic-map__stat topic-map__links"><span class="number">1</span> <span class="topic-map__stat-label">link</span>"#));
        assert_eq!(
            html.matches("class=\"poster trigger-user-card\"").count(),
            5
        );
        assert!(html.contains(r#"<span class="post-count">7</span>"#));
        assert!(
            html.contains(
                "<div class=\"estimated-read-time\"><span> read </span><span> 6 min </span></div>"
            ),
            "{html}"
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
            "post.controls.edit",
            "show_more",
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
