//! The topic list's rows as the Ember client renders them on a desktop
//! (components/topic-list/item.gjs and its cells), from the list JSON:
//! statuses, the title with its emoji, the category badge, tags, the
//! pinned excerpt, the posters' avatars, replies, views and activity with
//! their heat classes, and a member's unread badges. Dates are shown in
//! UTC, as a browser in UTC shows them; a viewer's own zone is not used
//! yet.

use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use fancy_regex::Regex;
use serde_json::Value;
use sqlx::PgConnection;

use crate::i18n::I18n;
use crate::pretty_text::render::local_dates::format_utc;
use crate::site_settings::{SettingError, SiteSettings};

/// What the list needs of a category (Category's badge fields).
#[derive(Debug, Clone)]
pub struct ListCategory {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub color: String,
    pub text_color: String,
    /// `square`, `icon` or `emoji` (Category's style_type enum).
    pub style_type: &'static str,
    pub emoji: Option<String>,
    pub icon: Option<String>,
    pub read_restricted: bool,
    pub parent_id: Option<i64>,
    pub navigate_to_first_post_after_read: bool,
}

pub async fn categories(
    conn: &mut PgConnection,
) -> Result<HashMap<i64, ListCategory>, sqlx::Error> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i32,
        name: String,
        slug: String,
        color: String,
        text_color: String,
        style_type: i32,
        emoji: Option<String>,
        icon: Option<String>,
        read_restricted: bool,
        parent_category_id: Option<i32>,
        navigate_to_first_post_after_read: bool,
    }
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, name, slug, color, text_color, style_type, emoji, icon, read_restricted, \
         parent_category_id, navigate_to_first_post_after_read FROM categories",
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let category = ListCategory {
                id: i64::from(r.id),
                name: r.name,
                slug: r.slug,
                color: r.color,
                text_color: r.text_color,
                style_type: match r.style_type {
                    1 => "icon",
                    2 => "emoji",
                    _ => "square",
                },
                emoji: r.emoji.filter(|e| !e.is_empty()),
                icon: r.icon.filter(|i| !i.is_empty()),
                read_restricted: r.read_restricted,
                parent_id: r.parent_category_id.map(i64::from),
                navigate_to_first_post_after_read: r.navigate_to_first_post_after_read,
            };
            (category.id, category)
        })
        .collect())
}

/// Ember's `escapeExpression`.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            '`' => out.push_str("&#x60;"),
            '=' => out.push_str("&#x3D;"),
            c => out.push(c),
        }
    }
    out
}

/// What the rows need from the site settings and the page.
pub struct ListContext<'a> {
    pub i18n: &'a I18n,
    pub base_path: &'a str,
    pub now: DateTime<Utc>,
    pub categories: &'a HashMap<i64, ListCategory>,
    /// `expandAllPinned`: a category's or a tag's list.
    pub expand_all_pinned: bool,
    /// The viewer's trust level, for a member.
    pub member_trust_level: Option<i32>,
    pub settings: ListSettings,
}

/// The site settings the rows read.
pub struct ListSettings {
    pub show_pinned_excerpt_desktop: bool,
    pub suppress_uncategorized_badge: bool,
    pub uncategorized_category_id: i64,
    pub tag_style: String,
    pub suppress_overlapping_tags_in_list: bool,
    pub topic_views_heat: [i64; 3],
    pub topic_post_like_heat: [f64; 3],
    pub cold_age_days: [f64; 3],
    pub relative_date_duration: i64,
    pub avatar_size_24: i64,
    pub prioritize_name: bool,
    pub inline_emoji: bool,
    pub emoji_set: String,
    pub support_mixed_text_direction: bool,
}

impl ListSettings {
    pub fn load(settings: &SiteSettings) -> Result<ListSettings, SettingError> {
        let int = |name: &str| -> Result<i64, SettingError> { Ok(settings.get(name)?.to_i()) };
        let float = |name: &str| -> Result<f64, SettingError> {
            Ok(settings.get(name)?.to_s().parse().unwrap_or(0.0))
        };
        let flag = |name: &str| -> Result<bool, SettingError> { Ok(settings.get(name)?.truthy()) };
        // getRawAvatarSize(24) at a device pixel ratio of 1: the first
        // allowed size of at least 24.
        let mut sizes: Vec<i64> = settings
            .get("avatar_sizes")?
            .to_s()
            .split('|')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        sizes.sort_unstable();
        let avatar_size_24 = sizes
            .iter()
            .copied()
            .find(|s| *s >= 24)
            .or(sizes.last().copied())
            .unwrap_or(24);
        Ok(ListSettings {
            show_pinned_excerpt_desktop: flag("show_pinned_excerpt_desktop")?,
            suppress_uncategorized_badge: flag("suppress_uncategorized_badge")?,
            uncategorized_category_id: int("uncategorized_category_id")?,
            tag_style: settings.get("tag_style")?.to_s().to_string(),
            suppress_overlapping_tags_in_list: flag("suppress_overlapping_tags_in_list")?,
            topic_views_heat: [
                int("topic_views_heat_low")?,
                int("topic_views_heat_medium")?,
                int("topic_views_heat_high")?,
            ],
            topic_post_like_heat: [
                float("topic_post_like_heat_low")?,
                float("topic_post_like_heat_medium")?,
                float("topic_post_like_heat_high")?,
            ],
            cold_age_days: [
                float("cold_age_days_low")?,
                float("cold_age_days_medium")?,
                float("cold_age_days_high")?,
            ],
            relative_date_duration: int("relative_date_duration")?,
            avatar_size_24,
            prioritize_name: flag("enable_names")? && !flag("prioritize_username_in_ux")?,
            inline_emoji: flag("enable_inline_emoji_translation")?,
            emoji_set: settings.get("emoji_set")?.to_s().to_string(),
            support_mixed_text_direction: flag("support_mixed_text_direction")?,
        })
    }
}

pub(crate) fn t(cx: &ListContext, key: &str) -> String {
    cx.i18n.t(&format!("js.{key}")).unwrap_or(key).to_string()
}

pub(crate) fn t_with(cx: &ListContext, key: &str, args: &[(&str, &str)]) -> String {
    cx.i18n
        .t_with(&format!("js.{key}"), args)
        .unwrap_or_else(|| key.to_string())
}

pub(crate) fn t_count(cx: &ListContext, key: &str, count: i64, args: &[(&str, &str)]) -> String {
    cx.i18n
        .t_count(&format!("js.{key}"), count, args)
        .unwrap_or_else(|| key.to_string())
}

/// An icon from the sprite (`iconHTML`).
pub fn icon(name: &str, extra_class: Option<&str>) -> String {
    let class = extra_class.map(|c| format!(" {c}")).unwrap_or_default();
    format!(
        "<svg class=\"fa d-icon d-icon-{name} svg-icon fa-width-auto{class} svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#{name}\"></use></svg>"
    )
}

/// The icon Discourse's icon map names `topic.closed`.
const CLOSED_ICON: &str = "lock";

/// JS's `\B` is ASCII-only, which fancy_regex's `\B` is not.
const ASCII_NOT_BOUNDARY: &str =
    r"(?:(?<=[A-Za-z0-9_])(?=[A-Za-z0-9_])|(?<![A-Za-z0-9_])(?![A-Za-z0-9_]))";
static TEXT_EMOJI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"{ASCII_NOT_BOUNDARY}:[^\s:]+(?::t\d)?:?{ASCII_NOT_BOUNDARY}"
    ))
    .unwrap()
});
static TEXT_EMOJI_INLINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r":[^\s:]+(?::t\d)?:?").unwrap());

/// `emojiUnescape` for the `:code:` form: a known emoji written with its
/// closing colon, after space or punctuation (or anywhere with inline
/// emoji), becomes its image. An emoji right after a unicode one is not
/// told apart (emojiReplacementRegex), and emoticons are left as text.
pub fn emoji_unescape(text: &str, settings: &ListSettings, base_path: &str) -> String {
    let regex: &Regex = if settings.inline_emoji {
        &TEXT_EMOJI_INLINE
    } else {
        &TEXT_EMOJI
    };
    let mut out = String::new();
    let mut last = 0;
    let mut pos = 0;
    while let Ok(Some(m)) = regex.find_from_pos(text, pos) {
        pos = m.end().max(m.start() + 1);
        let found = m.as_str();
        if !found.ends_with(':') || found.len() < 3 {
            continue;
        }
        let before = &text[..m.start()];
        let replaceable = settings.inline_emoji
            || before.is_empty()
            || before
                .chars()
                .next_back()
                .is_some_and(|c| c.is_whitespace() || ">.,/#!$%^&*;:{}=-_`~()".contains(c));
        let code = found[1..found.len() - 1].to_lowercase();
        let name = code.split(':').next().unwrap_or("");
        if !replaceable || !crate::emoji::DATA.exists(name) {
            continue;
        }
        let url = format!(
            "{base_path}/images/emoji/{}/{}.png?v={}",
            settings.emoji_set,
            code.replacen(":t", "/", 1),
            crate::emoji::image_version()
        );
        out.push_str(&text[last..m.start()]);
        out.push_str(&format!(
            "<img width=\"20\" height=\"20\" src='{url}' title='{code}' alt='{code}' class='emoji'>"
        ));
        last = m.end();
    }
    out.push_str(&text[last..]);
    out
}

/// `Category.slugFor`: the parent's slug first, up to three deep.
fn slug_for(cx: &ListContext, category: &ListCategory, depth: u32) -> String {
    let mut result = String::new();
    if depth > 1
        && let Some(parent) = category.parent_id.and_then(|id| cx.categories.get(&id))
    {
        result = format!("{}/", slug_for(cx, parent, depth - 1));
    }
    if category.slug.trim().is_empty() {
        format!("{result}{}-category", category.id)
    } else {
        format!("{result}{}", category.slug)
    }
}

/// `categoryBadgeHTML` with the default renderer, linked.
pub fn category_badge(cx: &ListContext, category: &ListCategory) -> String {
    if category.id == cx.settings.uncategorized_category_id
        && cx.settings.suppress_uncategorized_badge
    {
        return String::new();
    }
    let parent = category.parent_id.and_then(|id| cx.categories.get(&id));
    let url = format!(
        "{}/c/{}/{}",
        cx.base_path,
        slug_for(cx, category, 3),
        category.id
    );
    // categoryVariables
    let mut style = format!(
        "--category-badge-color: #{};--category-badge-text-color: #{};",
        category.color, category.text_color
    );
    if let Some(p) = parent {
        style.push_str(&format!(
            "--parent-category-badge-color: #{};--parent-category-badge-text-color: #{};",
            p.color, p.text_color
        ));
    }
    let mut classes = String::from("badge-category");
    if category.read_restricted {
        classes.push_str(" restricted");
    }
    let mut data = format!("data-category-id=\"{}\"", category.id);
    if let Some(p) = parent {
        classes.push_str(" --has-parent");
        data.push_str(&format!(" data-parent-category-id=\"{}\"", p.id));
    }
    classes.push_str(&format!(" --style-{}", category.style_type));
    let mut inner = String::new();
    match (category.style_type, &category.icon, &category.emoji) {
        ("icon", Some(name), _) => inner.push_str(&icon(name, None)),
        ("emoji", _, Some(emoji)) => inner.push_str(&emoji_unescape(
            &format!(":{emoji}:"),
            &cx.settings,
            cx.base_path,
        )),
        _ => {}
    }
    if category.read_restricted {
        inner.push_str(&icon(CLOSED_ICON, None));
    }
    let dir = if cx.settings.support_mixed_text_direction {
        " dir=\"auto\""
    } else {
        ""
    };
    format!(
        "<a class=\"badge-category__wrapper \" style=\"{style}\" href=\"{url}\"><span {data} data-drop-close=\"true\" class=\"{classes}\">{inner}<span class=\"badge-category__name\"{dir}>{}</span></span></a>",
        escape(&category.name)
    )
}

/// `renderTags` in list mode, with `defaultRenderTag`.
pub(crate) fn tags_html(cx: &ListContext, topic: &Value, title: &str) -> String {
    let tags: Vec<&Value> = topic["tags"]
        .as_array()
        .map(|t| t.iter().collect())
        .unwrap_or_default();
    let tag_name = |t: &Value| -> String {
        t.as_str()
            .map(str::to_string)
            .unwrap_or_else(|| t["name"].as_str().unwrap_or_default().to_string())
    };
    let title = title.to_lowercase();
    let tags: Vec<&Value> = if cx.settings.suppress_overlapping_tags_in_list {
        tags.into_iter()
            .filter(|t| !title.contains(&tag_name(t).to_lowercase()))
            .collect()
    } else {
        tags
    };
    if tags.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "<ul class='discourse-tags' aria-label={}>",
        t(cx, "tagging.tags")
    );
    for (i, tag) in tags.iter().enumerate() {
        let name = tag_name(tag);
        let visible = escape(&name);
        let lower = visible.to_lowercase();
        let path = match tag["id"].as_i64() {
            Some(id) => {
                let slug = tag["slug"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{id}-tag"));
                format!("/tag/{slug}/{id}")
            }
            None => format!("/tag/{}", lower.replace('.', "%2E")),
        };
        let mut classes = String::from("discourse-tag");
        if !cx.settings.tag_style.is_empty() {
            classes.push(' ');
            classes.push_str(&cx.settings.tag_style);
        }
        out.push_str(&format!(
            "<li><a href='{}{path}'  data-tag-name={lower} class='{classes}'>{visible}</a>",
            cx.base_path
        ));
        if i < tags.len() - 1 {
            out.push_str("<span class=\"discourse-tags__tag-separator\">,</span>");
        }
        out.push_str("</li>");
    }
    out.push_str("</ul>");
    out
}

fn date(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

/// `longDate`
pub(crate) fn long_date(cx: &ListContext, at: DateTime<Utc>) -> String {
    format_utc(at, &t(cx, "dates.long_with_year")).unwrap_or_default()
}

/// `relativeAgeTiny`
pub(crate) fn relative_age_tiny(cx: &ListContext, at: DateTime<Utc>) -> String {
    let distance = ((cx.now - at).num_milliseconds() as f64 / 1000.0)
        .round()
        .abs();
    let minutes = (distance / 60.0).round().max(1.0) as i64;
    let duration = cx.settings.relative_date_duration;
    match minutes {
        0..=44 => t_count(cx, "dates.tiny.x_minutes", minutes, &[]),
        45..=89 => t_count(cx, "dates.tiny.about_x_hours", 1, &[]),
        90..=1409 => t_count(
            cx,
            "dates.tiny.about_x_hours",
            (minutes as f64 / 60.0).round() as i64,
            &[],
        ),
        _ if duration == 0 && minutes <= 525_599 => {
            format_utc(at, &t(cx, "dates.tiny.date_month")).unwrap_or_default()
        }
        1410..=2519 => t_count(cx, "dates.tiny.x_days", 1, &[]),
        m if m >= 2520 && m <= (if duration == 0 { 14 } else { duration }) * 1440 => t_count(
            cx,
            "dates.tiny.x_days",
            (minutes as f64 / 1440.0).round() as i64,
            &[],
        ),
        // smartShortDate
        _ => {
            use chrono::Datelike;
            let key = if at.year() == cx.now.year() {
                "dates.tiny.date_month"
            } else {
                "dates.tiny.date_year"
            };
            format_utc(at, &t(cx, key)).unwrap_or_default()
        }
    }
}

/// `autoUpdatingRelativeAge` in the tiny format, without a title.
fn relative_date(cx: &ListContext, at: DateTime<Utc>) -> String {
    let age = relative_age_tiny(cx, at);
    // relativeAgeTinyShowsYear: /'[\d]{2}$/
    let bytes = age.as_bytes();
    let with_year = bytes.len() >= 3
        && bytes[bytes.len() - 3] == b'\''
        && bytes[bytes.len() - 2..].iter().all(u8::is_ascii_digit);
    format!(
        "<span class='relative-date{}' data-time='{}' data-format='tiny'>{age}</span>",
        if with_year { " with-year" } else { "" },
        at.timestamp_millis()
    )
}

/// `number`: thousands and millions shortened.
fn number(cx: &ListContext, n: i64) -> String {
    if n > 999_999 {
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
    }
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// One topic's row.
pub fn row(cx: &ListContext, topic: &Value, users: &[Value]) -> String {
    row_for(cx, topic, users, false)
}

/// A suggested topic's row (more-topics): no posters column, and no
/// pinned excerpt.
pub fn suggested_row(cx: &ListContext, topic: &Value) -> String {
    row_for(cx, topic, &[], true)
}

fn row_for(cx: &ListContext, topic: &Value, users: &[Value], suggested: bool) -> String {
    let base = cx.base_path;
    let id = topic["id"].as_i64().unwrap_or(0);
    let slug = match s(&topic["slug"]).trim() {
        "" => "topic".to_string(),
        slug => slug.to_string(),
    };
    let url = format!("{base}/t/{slug}/{id}");
    let url_for = |n: i64| {
        if n > 0 {
            format!("{url}/{n}")
        } else {
            url.clone()
        }
    };
    let flag = |name: &str| topic[name] == true;
    let posts_count = topic["posts_count"].as_i64().unwrap_or(0);
    let highest = topic["highest_post_number"].as_i64().unwrap_or(0);
    let category = topic["category_id"]
        .as_i64()
        .and_then(|id| cx.categories.get(&id));
    let excerpt = topic["excerpt"].as_str().filter(|e| !e.is_empty());
    let expand_pinned = !suggested
        && flag("pinned")
        && cx.settings.show_pinned_excerpt_desktop
        && ((!cx.expand_all_pinned && flag("pinned_globally")) || cx.expand_all_pinned);

    // The row's classes.
    let mut classes = vec!["topic-list-item".to_string()];
    if let Some(c) = category {
        classes.push(format!("category-{}", slug_for(cx, c, 3).replace('/', "-")));
    }
    let unread_posts = topic["unread_posts"].as_i64().unwrap_or(0);
    for (on, class) in [
        (
            topic["last_read_post_number"].is_number() && cx.member_trust_level.is_some(),
            "visited",
        ),
        (excerpt.is_some(), "has-excerpt"),
        (expand_pinned && excerpt.is_some(), "excerpt-expanded"),
        (flag("unseen"), "unseen-topic"),
        (unread_posts > 0, "unread-posts"),
        (flag("liked"), "liked"),
        (flag("archived"), "archived"),
        (flag("bookmarked"), "bookmarked"),
        (flag("pinned"), "pinned"),
        (flag("closed"), "closed"),
    ] {
        if on {
            classes.push(class.to_string());
        }
    }
    if let Some(tags) = topic["tags"].as_array() {
        for tag in tags {
            let name = tag.as_str().unwrap_or_else(|| s(&tag["name"]));
            classes.push(format!("tag-{name}"));
        }
    }

    let statuses = topic_statuses(cx, topic);

    // TopicLink: to the first unread post.
    let last_read = topic["last_read_post_number"].as_i64().unwrap_or(0);
    let last_unread_url =
        if last_read >= highest && category.is_some_and(|c| c.navigate_to_first_post_after_read) {
            url_for(1)
        } else {
            url_for((last_read + 1).min(highest))
        };
    let title_html = emoji_unescape(s(&topic["fancy_title"]), &cx.settings, base);
    let title_html = if cx.settings.support_mixed_text_direction {
        format!("<span dir=\"auto\">{title_html}</span>")
    } else {
        title_html
    };

    // TopicPostBadges, for a member.
    let mut badges = String::new();
    if cx.member_trust_level.is_some() {
        badges.push_str("<span class=\"topic-post-badges\">");
        if unread_posts > 0 {
            let text = t_count(cx, "topic.unread_posts", unread_posts, &[]);
            badges.push_str(&format!(
                "&nbsp;<a aria-description=\"{text}\" class=\"badge badge-notification unread-posts\" href=\"{last_unread_url}\" title=\"{text}\">{unread_posts}</a>"
            ));
        }
        if flag("unseen") {
            let dot = if cx.member_trust_level.unwrap_or(0) > 0 {
                " ".to_string()
            } else {
                t(cx, "filters.new.lower_title")
            };
            badges.push_str(&format!(
                "&nbsp;<a aria-label=\"{0}\" class=\"badge badge-notification new-topic\" href=\"{last_unread_url}\" title=\"{0}\">{dot}</a>",
                t(cx, "topic.new")
            ));
        }
        badges.push_str("</span>");
    }

    let pinned_uncategorized =
        flag("pinned") && category.is_some_and(|c| c.id == cx.settings.uncategorized_category_id);
    let badge = match category {
        Some(c) if !pinned_uncategorized => category_badge(cx, c),
        _ => String::new(),
    };
    let tags = tags_html(cx, topic, s(&topic["title"]));

    let excerpt_html = match excerpt {
        Some(e) if expand_pinned => {
            let more = if e.ends_with("&hellip;") {
                format!(
                    " <span class=\"topic-excerpt-more\">{}</span>",
                    t(cx, "read_more")
                )
            } else {
                String::new()
            };
            format!(
                "<a class=\"topic-excerpt\" href=\"{url}\"><span>{}</span>{more}</a>",
                emoji_unescape(e, &cx.settings, base)
            )
        }
        _ => String::new(),
    };

    let topic_cell = format!(
        "<td class=\"main-link topic-list-data\" colspan=\"1\"><span aria-level=\"2\" class=\"link-top-line\" role=\"heading\"><span class=\"topic-statuses\">{statuses}</span><a class=\"title raw-link raw-topic-link\" data-topic-id=\"{id}\" href=\"{last_unread_url}\">{title_html}</a>{badges}</span><div class=\"link-bottom-line\">{badge}{tags}</div>{excerpt_html}</td>"
    );

    // PostersCell: each poster's avatar, linked.
    let mut posters = String::from("<td class=\"posters topic-list-data\">");
    for poster in topic["posters"].as_array().into_iter().flatten() {
        let Some(user) = users.iter().find(|u| u["id"] == poster["user_id"]) else {
            continue;
        };
        let username = s(&user["username"]);
        let template = s(&user["avatar_template"]);
        if username.is_empty() || template.is_empty() {
            continue;
        }
        let name = s(&user["name"]);
        let display = if cx.settings.prioritize_name && !name.is_empty() {
            name
        } else {
            username
        };
        let description = s(&poster["description"]);
        let title = if description.is_empty() {
            display.to_string()
        } else {
            t_with(
                cx,
                "user.avatar.name_and_description",
                &[("name", display), ("description", description)],
            )
        };
        let extras = s(&poster["extras"]);
        let link_class = if extras.is_empty() {
            String::new()
        } else {
            format!(" class=\"{extras}\"")
        };
        let img_class = if extras.is_empty() {
            "avatar".to_string()
        } else {
            format!("avatar {extras}")
        };
        let src = template.replace("{size}", &cx.settings.avatar_size_24.to_string());
        let src = if src.starts_with('/') && !src.starts_with("//") && !src.starts_with(base) {
            format!("{base}{src}")
        } else {
            src
        };
        posters.push_str(&format!(
            "<a aria-label=\"{}\"{link_class} data-user-card=\"{username}\" href=\"{base}/u/{username}\" tabindex=\"0\"><img alt='' width='24' height='24' src='{src}' class='{img_class}' title='{}'></a>",
            t_with(cx, "user.profile_possessive", &[("username", username)]),
            escape(&title)
        ));
    }
    posters.push_str("</td>");
    if suggested {
        posters.clear();
    }

    // RepliesCell: posts_count - 1, heat by likes per post.
    let reply_count = posts_count - 1;
    let ratio = if posts_count < 10 {
        0.0
    } else {
        topic["like_count"].as_f64().unwrap_or(0.0) / posts_count as f64
    };
    let [low, medium, high] = cx.settings.topic_post_like_heat;
    let likes_heat = if ratio > high {
        " heatmap-high"
    } else if ratio > medium {
        " heatmap-med"
    } else if ratio > low {
        " heatmap-low"
    } else {
        ""
    };
    let replies = format!(
        "<td class=\"num posts-map posts{likes_heat} topic-list-data\"><a aria-label=\"{}\" class=\"badge-posts\" href=\"{}\"><span class='number'>{}</span></a></td>",
        t_count(cx, "topic.reply_count_link", reply_count, &[]),
        url_for(1),
        number(cx, reply_count)
    );

    // ViewsCell
    let views = topic["views"].as_i64().unwrap_or(0);
    let [vlow, vmedium, vhigh] = cx.settings.topic_views_heat;
    let views_heat = if views >= vhigh {
        " heatmap-high"
    } else if views >= vmedium {
        " heatmap-med"
    } else if views >= vlow {
        " heatmap-low"
    } else {
        ""
    };
    let views_title = t_count(cx, "views_long", views, &[("number", &views.to_string())]);
    let views = format!(
        "<td class=\"num views topic-list-data{views_heat}\"><span class='number' title='{}'>{}</span></td>",
        escape(&views_title),
        number(cx, views)
    );

    // ActivityCell
    let created = date(&topic["created_at"]);
    let bumped = date(&topic["bumped_at"]).or(created);
    let activity = match (created, bumped) {
        (Some(created), Some(bumped)) => {
            // coldAgeClass(createdAt, startDate: bumpedAt)
            let days = (bumped - created).num_milliseconds() as f64 / 86_400_000.0;
            let [clow, cmedium, chigh] = cx.settings.cold_age_days;
            let cold = if days > chigh {
                " coldmap-high"
            } else if days > cmedium {
                " coldmap-med"
            } else if days > clow {
                " coldmap-low"
            } else {
                ""
            };
            let seconds = |d: DateTime<Utc>| d.format("%Y-%m-%dT%H:%M:%S").to_string();
            let title = if seconds(bumped) != seconds(created) {
                format!(
                    "{}\n{}",
                    t_with(cx, "topic.created_at", &[("date", &long_date(cx, created))]),
                    t_with(cx, "topic.bumped_at", &[("date", &long_date(cx, bumped))])
                )
            } else {
                t_with(cx, "topic.created_at", &[("date", &long_date(cx, created))])
            };
            format!(
                "<td class=\"activity num topic-list-data age{cold}\" title=\"{title}\"><a class=\"post-activity\" href=\"{}\">{}</a></td>",
                url_for(highest),
                relative_date(cx, bumped)
            )
        }
        _ => "<td class=\"activity num topic-list-data age\"></td>".to_string(),
    };

    format!(
        "<tr class=\"{}\" data-topic-id=\"{id}\">{topic_cell}{posters}{replies}{views}{activity}</tr>",
        classes.join(" ")
    )
}

/// TopicStatus, for a visitor (a member's pin toggles are links).
pub fn topic_statuses(cx: &ListContext, topic: &Value) -> String {
    let flag = |name: &str| topic[name] == true;
    let mut statuses = String::new();
    if flag("bookmarked") {
        statuses.push_str(&format!(
            "<span class=\"topic-status --bookmarked\" title=\"{}\">{}</span>",
            t(cx, "topic_statuses.bookmarked.help"),
            icon("bookmark", None)
        ));
    }
    let closed_status = match (flag("closed"), flag("archived")) {
        (true, true) => Some(("--closed --archived", "locked_and_archived")),
        (true, false) => Some(("--closed", "locked")),
        (false, true) => Some(("--archived", "archived")),
        _ => None,
    };
    if let Some((class, key)) = closed_status {
        statuses.push_str(&format!(
            "<span class=\"topic-status {class}\" title=\"{}\">{}</span>",
            t(cx, &format!("topic_statuses.{key}.help")),
            icon(CLOSED_ICON, None)
        ));
    }
    if flag("pinned") {
        statuses.push_str(&format!(
            "<span class=\"topic-status --pinned\" title=\"{}\">{}</span>",
            t(cx, "topic_statuses.pinned.help"),
            icon("thumbtack", None)
        ));
    } else if flag("unpinned") {
        statuses.push_str(&format!(
            "<span class=\"topic-status --unpinned\" title=\"{}\">{}</span>",
            t(cx, "topic_statuses.unpinned.help"),
            icon("thumbtack", Some("unpinned"))
        ));
    }
    statuses
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_as_ember_does() {
        assert_eq!(
            escape("a<b>&\"'`="),
            "a&lt;b&gt;&amp;&quot;&#x27;&#x60;&#x3D;"
        );
    }

    fn settings(inline_emoji: bool) -> ListSettings {
        ListSettings {
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
            inline_emoji,
            emoji_set: "twitter".into(),
            support_mixed_text_direction: false,
        }
    }

    fn emoji_codes(text: &str, inline_emoji: bool) -> Vec<String> {
        let html = emoji_unescape(text, &settings(inline_emoji), "");
        html.match_indices("title='")
            .map(|(i, _)| {
                let rest = &html[i + 7..];
                rest[..rest.find('\'').unwrap()].to_string()
            })
            .collect()
    }

    #[test]
    fn unescapes_emoji_on_ascii_boundaries() {
        assert_eq!(emoji_codes("hi :smile: there", false), ["smile"]);
        assert_eq!(emoji_codes(":wave:t3: hello", false), ["wave:t3"]);
        // A word character on either side is a boundary, so no emoji.
        assert!(emoji_codes("a:smile: b", false).is_empty());
        assert_eq!(emoji_codes(":smile:a :x:", false), ["x"]);
        assert_eq!(emoji_codes(":smile:a :heart:", false), ["heart"]);
        assert!(emoji_codes("é:smile:", false).is_empty());
        assert_eq!(emoji_codes("a:smile:", true), ["smile"]);
        assert!(emoji_codes(":notanemoji:", false).is_empty());
    }
}
