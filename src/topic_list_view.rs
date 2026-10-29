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

use crate::emojis::Emojis;
use crate::i18n::I18n;
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
    /// TopicList#hideCategory: the rows go without their category badge
    /// (hideUniformCategory, a list of a category with no subcategories).
    pub hide_category: bool,
    /// The viewer's trust level, for a member.
    pub member_trust_level: Option<i32>,
    pub settings: ListSettings,
}

/// The site settings the rows read.
pub struct ListSettings {
    pub show_pinned_excerpt_desktop: bool,
    pub show_pinned_excerpt_mobile: bool,
    pub suppress_uncategorized_badge: bool,
    pub uncategorized_category_id: i64,
    pub tag_style: String,
    pub suppress_overlapping_tags_in_list: bool,
    pub topic_views_heat: [i64; 3],
    pub topic_post_like_heat: [f64; 3],
    pub cold_age_days: [f64; 3],
    pub relative_date_duration: i64,
    pub avatar_size_24: i64,
    /// getRawAvatarSize(48), the mobile rows' "large" avatar.
    pub avatar_size_48: i64,
    pub prioritize_name: bool,
    /// The emoji settings, custom emoji and deny list `emojiUnescape` reads.
    pub emojis: std::sync::Arc<crate::emojis::Emojis>,
    pub support_mixed_text_direction: bool,
    /// discourse-assign's settings, when it is on (its tags callback).
    pub assign: Option<crate::plugins::assign::view::AssignUi>,
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
        let raw_size = |size: i64| {
            sizes
                .iter()
                .copied()
                .find(|s| *s >= size)
                .or(sizes.last().copied())
                .unwrap_or(size)
        };
        let avatar_size_24 = raw_size(24);
        let avatar_size_48 = raw_size(48);
        Ok(ListSettings {
            show_pinned_excerpt_desktop: flag("show_pinned_excerpt_desktop")?,
            show_pinned_excerpt_mobile: flag("show_pinned_excerpt_mobile")?,
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
            avatar_size_48,
            prioritize_name: flag("enable_names")? && !flag("prioritize_username_in_ux")?,
            emojis: settings.emojis().clone(),
            support_mixed_text_direction: flag("support_mixed_text_direction")?,
            assign: crate::plugins::assign::view::AssignUi::load(settings)?,
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
pub(crate) const CLOSED_ICON: &str = "lock";

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

/// `emojiUnescape` (lib/text.js) for the `:code:` form and emoticons: a
/// known or custom emoji written with its closing colon, after space or
/// punctuation (or anywhere with inline emoji), becomes its image, and a
/// denied one disappears. The text is returned as is while emoji are off.
/// Unicode emoji (emojiReplacementRegex) are left as they are.
pub fn emoji_unescape(text: &str, emojis: &Emojis, base_path: &str) -> String {
    if !emojis.enabled {
        return text.to_string();
    }
    perform_emoji_unescape(text, emojis, base_path, true)
}

/// `PrettyText.unescape_emoji`: the server's emojiUnescape, which passes
/// the custom emoji but not the deny list.
pub fn unescape_emoji(text: &str, emojis: &Emojis, base_path: &str) -> String {
    if !emojis.enabled {
        return text.to_string();
    }
    perform_emoji_unescape(text, emojis, base_path, false)
}

/// `performEmojiUnescape` over the text regex.
fn perform_emoji_unescape(text: &str, emojis: &Emojis, base_path: &str, deny: bool) -> String {
    let regex: &Regex = if emojis.inline {
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
        let emoticon = if emojis.shortcuts {
            crate::emoji::DATA.translations.get(found)
        } else {
            None
        };
        // emojiVal: the emoticon's emoji, else the text between the colons
        // (the last character dropped, a colon or not).
        let value = match emoticon {
            Some(name) => name.as_str(),
            None => {
                let end = found.char_indices().last().map_or(0, |(i, _)| i);
                &found[1.min(end)..end]
            }
        };
        if deny && emojis.is_denied(value) {
            out.push_str(&text[last..m.start()]);
            last = m.end();
            continue;
        }
        if emoticon.is_none() && !found.ends_with(':') {
            continue;
        }
        let before = &text[..m.start()];
        let replaceable = emojis.inline
            || before.is_empty()
            || before
                .chars()
                .next_back()
                .is_some_and(|c| c.is_whitespace() || ">.,/#!$%^&*;:{}=-_`~()".contains(c));
        let Some(url) = emojis.url(value, base_path).filter(|_| replaceable) else {
            continue;
        };
        let class = if emojis.is_custom(value) {
            "emoji emoji-custom"
        } else {
            "emoji"
        };
        out.push_str(&text[last..m.start()]);
        out.push_str(&format!(
            "<img width=\"20\" height=\"20\" src='{url}' title='{value}' alt='{value}' class='{class}'>"
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

/// Category#fullSlug: the slug path, parents first, joined by dashes.
pub fn category_full_slug(cx: &ListContext, category: &ListCategory) -> String {
    slug_for(cx, category, 3).replace('/', "-")
}

/// `categoryBadgeHTML` with the default renderer, linked.
pub fn category_badge(cx: &ListContext, category: &ListCategory) -> String {
    category_badge_html(cx, category, true, false)
}

/// `Category#url`: the parent's slug first.
pub(crate) fn category_url(cx: &ListContext, category: &ListCategory) -> String {
    format!(
        "{}/c/{}/{}",
        cx.base_path,
        slug_for(cx, category, 3),
        category.id
    )
}

/// `categoryBadgeHTML`, linked or (`link: false`) a plain span; with
/// `hide_parent` the parent keeps its colors but not its class and id.
pub fn category_badge_html(
    cx: &ListContext,
    category: &ListCategory,
    link: bool,
    hide_parent: bool,
) -> String {
    if category.id == cx.settings.uncategorized_category_id
        && cx.settings.suppress_uncategorized_badge
    {
        return String::new();
    }
    let parent = category.parent_id.and_then(|id| cx.categories.get(&id));
    let url = category_url(cx, category);
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
    if let Some(p) = parent.filter(|_| !hide_parent) {
        classes.push_str(" --has-parent");
        data.push_str(&format!(" data-parent-category-id=\"{}\"", p.id));
    }
    classes.push_str(&format!(" --style-{}", category.style_type));
    let mut inner = String::new();
    match (category.style_type, &category.icon, &category.emoji) {
        ("icon", Some(name), _) => inner.push_str(&icon(name, None)),
        ("emoji", _, Some(emoji)) => inner.push_str(&emoji_unescape(
            &format!(":{emoji}:"),
            &cx.settings.emojis,
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
    let badge = format!(
        "<span {data} data-drop-close=\"true\" class=\"{classes}\">{inner}<span class=\"badge-category__name\"{dir}>{}</span></span>",
        escape(&category.name)
    );
    if link {
        format!(
            "<a class=\"badge-category__wrapper \" style=\"{style}\" href=\"{url}\">{badge}</a>"
        )
    } else {
        format!("<span class=\"badge-category__wrapper\" style=\"{style}\">{badge}</span>")
    }
}

/// `renderTags` in list mode, with `defaultRenderTag`.
/// `renderTags`: the topic's tags, then what the plugins' tags callbacks
/// return (`custom`), each an item after a separator.
pub(crate) fn tags_html(cx: &ListContext, topic: &Value, title: &str, custom: &[String]) -> String {
    let tags: Vec<&Value> = topic["tags"]
        .as_array()
        .map(|t| t.iter().collect())
        .unwrap_or_default();
    let title = title.to_lowercase();
    let tags: Vec<&Value> = if cx.settings.suppress_overlapping_tags_in_list {
        tags.into_iter()
            .filter(|t| !title.contains(&tag_name(t).to_lowercase()))
            .collect()
    } else {
        tags
    };
    tags_list(cx, &tags, custom)
}

/// `renderTags(topic)` without params, as the docked header calls it:
/// every tag, and no plugin callback (topic voting's draws nothing on a
/// topic route).
pub(crate) fn render_tags(cx: &ListContext, topic: &Value) -> String {
    let tags: Vec<&Value> = topic["tags"]
        .as_array()
        .map(|t| t.iter().collect())
        .unwrap_or_default();
    tags_list(cx, &tags, &[])
}

/// A tag is its name, or an object with one (tags_descriptions on).
fn tag_name(t: &Value) -> String {
    t.as_str()
        .map(str::to_string)
        .unwrap_or_else(|| t["name"].as_str().unwrap_or_default().to_string())
}

fn tags_list(cx: &ListContext, tags: &[&Value], custom: &[String]) -> String {
    let custom: Vec<&String> = custom.iter().filter(|c| !c.is_empty()).collect();
    if tags.is_empty() && custom.is_empty() {
        return String::new();
    }
    let separator = "<span class=\"discourse-tags__tag-separator\">,</span>";
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
        if i < tags.len() - 1 || !custom.is_empty() {
            out.push_str(separator);
        }
        out.push_str("</li>");
    }
    for (i, html) in custom.iter().enumerate() {
        out.push_str("<li>");
        out.push_str(html);
        if i < custom.len() - 1 {
            out.push_str(separator);
        }
        out.push_str("</li>");
    }
    out.push_str("</ul>");
    out
}

/// The plugins' tags callbacks (addTagsHtmlCallback) for a topic outside
/// the topic page: discourse-assign's assignees and discourse-topic-
/// voting's vote count.
pub(crate) fn tags_callbacks(cx: &ListContext, topic: &Value) -> Vec<String> {
    let url = format!("{}/t/{}/{}", cx.base_path, s(&topic["slug"]), topic["id"]);
    // discourse-assign's assignees, registered before voting's.
    let mut out = cx
        .settings
        .assign
        .as_ref()
        .map(|ui| crate::plugins::assign::view::tags_html(ui, cx.base_path, topic))
        .unwrap_or_default();
    out.push(crate::plugins::topic_voting::view::list_vote_count(
        cx.i18n, topic, &url,
    ));
    out
}

fn date(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

/// `relativeAgeTiny`
pub(crate) fn relative_age_tiny(cx: &ListContext, at: DateTime<Utc>) -> String {
    crate::viewer_dates::relative_age_tiny(cx, at)
}

/// `Topic#bumpedAtTitle` as the activity cell's title attribute, its
/// dates in the viewer's zone (crate::viewer_dates).
pub(crate) fn bumped_at_title(
    cx: &ListContext,
    created: DateTime<Utc>,
    bumped: DateTime<Utc>,
) -> String {
    let seconds = |d: DateTime<Utc>| d.format("%Y-%m-%dT%H:%M:%S").to_string();
    let created_at = t_with(
        cx,
        "topic.created_at",
        &[("date", &crate::viewer_dates::long_marker(cx, created))],
    );
    let template = if seconds(bumped) != seconds(created) {
        format!(
            "{created_at}\n{}",
            t_with(
                cx,
                "topic.bumped_at",
                &[("date", &crate::viewer_dates::long_marker(cx, bumped))]
            )
        )
    } else {
        created_at
    };
    crate::viewer_dates::attr(cx, "title", &template)
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
pub(crate) fn number(cx: &ListContext, n: i64) -> String {
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

/// The topic's url (Topic#url), and with a post number past 0 (urlForPostNumber).
fn topic_url(cx: &ListContext, topic: &Value, post_number: Option<i64>) -> String {
    let id = topic["id"].as_i64().unwrap_or(0);
    let slug = match s(&topic["slug"]).trim() {
        "" => "topic".to_string(),
        slug => slug.to_string(),
    };
    match post_number {
        Some(n) if n > 0 => format!("{}/t/{slug}/{id}/{n}", cx.base_path),
        _ => format!("{}/t/{slug}/{id}", cx.base_path),
    }
}

/// Topic#lastUnreadUrl, where TopicLink goes: the first unread post.
pub(crate) fn last_unread_url(cx: &ListContext, topic: &Value) -> String {
    let highest = topic["highest_post_number"].as_i64().unwrap_or(0);
    let category = topic["category_id"]
        .as_i64()
        .and_then(|id| cx.categories.get(&id));
    // Without a last read post, Ember's `undefined + 1` is NaN, which no
    // post number follows.
    match topic["last_read_post_number"].as_i64() {
        Some(last_read)
            if last_read >= highest
                && category.is_some_and(|c| c.navigate_to_first_post_after_read) =>
        {
            topic_url(cx, topic, Some(1))
        }
        Some(last_read) => topic_url(cx, topic, Some((last_read + 1).min(highest))),
        // A null last read post (the message lists send one) is 0 to
        // Ember's `null + 1`.
        None if topic["last_read_post_number"].is_null()
            && topic.get("last_read_post_number").is_some() =>
        {
            topic_url(cx, topic, Some(1.min(highest)))
        }
        None => topic_url(cx, topic, None),
    }
}

/// TopicPostBadges: the unread count and the new dot, each to the first
/// unread post; empty without either.
pub(crate) fn topic_post_badges(cx: &ListContext, topic: &Value, url: &str) -> String {
    let unread_posts = topic["unread_posts"].as_i64().unwrap_or(0);
    let unseen = topic["unseen"] == true;
    let mut badges = String::from("<span class=\"topic-post-badges\">");
    if unread_posts > 0 {
        let text = t_count(cx, "topic.unread_posts", unread_posts, &[]);
        // The template's comment leaves a space before the badge.
        badges.push_str(&format!(
            " &nbsp;<a aria-description=\"{text}\" class=\"badge badge-notification unread-posts\" href=\"{url}\" title=\"{text}\">{unread_posts}</a>"
        ));
    }
    if unseen {
        let dot = if cx.member_trust_level.unwrap_or(0) > 0 {
            " ".to_string()
        } else {
            t(cx, "filters.new.lower_title")
        };
        badges.push_str(&format!(
            "&nbsp;<a aria-label=\"{0}\" class=\"badge badge-notification new-topic\" href=\"{url}\" title=\"{0}\">{dot}</a>",
            t(cx, "topic.new")
        ));
    }
    badges.push_str("</span>");
    badges
}

/// ItemRepliesCell as a div (PostCountOrBadges without badges).
pub(crate) fn replies_div(cx: &ListContext, topic: &Value) -> String {
    let reply_count = topic["posts_count"].as_i64().unwrap_or(0) - 1;
    format!(
        "<div class=\"num posts-map posts{} topic-list-data\"><a aria-label=\"{}\" class=\"badge-posts\" href=\"{}\"><span class='number'>{}</span></a></div>",
        likes_heat(cx, topic),
        t_count(cx, "topic.reply_count_link", reply_count, &[]),
        topic_url(cx, topic, Some(1)),
        number(cx, reply_count)
    )
}

/// One topic's row.
pub fn row(cx: &ListContext, topic: &Value, users: &[Value]) -> String {
    row_for(cx, topic, users, false, false, true)
}

/// A row without the posters column (`@showPosters={{false}}`, the user
/// topics lists).
pub fn row_without_posters(cx: &ListContext, topic: &Value, users: &[Value]) -> String {
    row_for(cx, topic, users, false, false, false)
}

/// TopicList#lastVisitedTopic: scanning back from the end, past the pinned
/// topics at the top, the last topic bumped since the member's previous
/// visit, when an older one follows it.
pub fn last_visited_topic(topics: &[Value], previous_visit_at: DateTime<Utc>) -> Option<usize> {
    if topics.len() == 1 {
        return None;
    }
    let start = topics.iter().position(|t| t["pinned"] != true).unwrap_or(0);
    let bumped = |t: &Value| date(&t["bumped_at"]);
    let mut older = None;
    for i in (start..topics.len()).rev() {
        if bumped(&topics[i]).is_some_and(|at| at > previous_visit_at) {
            let older: &Value = older?;
            // The end of the scanned list is newer too: nothing to mark.
            if bumped(older).is_some_and(|at| at > previous_visit_at) {
                return None;
            }
            return Some(i);
        }
        older = Some(&topics[i]);
    }
    None
}

/// The last visited topic's row (class last-visit), then the separator
/// that says so.
pub fn last_visited_row(cx: &ListContext, topic: &Value, users: &[Value]) -> String {
    format!(
        "{}<tr class=\"topic-list-item-separator\"><td class=\"topic-list-data\" colspan=\"6\"><span>{}</span></td></tr>",
        row_for(cx, topic, users, false, true, true),
        escape(&t(cx, "topics.new_messages_marker"))
    )
}

/// A suggested topic's row (more-topics): no posters column, and no
/// pinned excerpt.
pub fn suggested_row(cx: &ListContext, topic: &Value) -> String {
    row_for(cx, topic, &[], true, false, false)
}

/// A suggested message's row (SuggestedTopics for a message: showPosters,
/// hideCategory, which a message has none for): the posters' users come
/// inside them (SuggestedPosterSerializer's `user`).
pub fn suggested_message_row(cx: &ListContext, topic: &Value) -> String {
    let mut topic = topic.clone();
    let mut users = Vec::new();
    if let Some(posters) = topic["posters"].as_array_mut() {
        for poster in posters {
            let user = poster["user"].clone();
            poster["user_id"] = user["id"].clone();
            users.push(user);
        }
    }
    row_for(cx, &topic, &users, true, false, true)
}

fn row_for(
    cx: &ListContext,
    topic: &Value,
    users: &[Value],
    suggested: bool,
    last_visit: bool,
    show_posters: bool,
) -> String {
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
    // TopicListItem#useMobileLayout
    let mobile = crate::mobile::mobile_view();
    let show_pinned_excerpt = if mobile {
        cx.settings.show_pinned_excerpt_mobile
    } else {
        cx.settings.show_pinned_excerpt_desktop
    };
    let expand_pinned = !suggested
        && flag("pinned")
        && show_pinned_excerpt
        && ((!cx.expand_all_pinned && flag("pinned_globally")) || cx.expand_all_pinned);

    // The row's classes.
    let mut classes = vec!["topic-list-item".to_string()];
    if let Some(c) = category {
        classes.push(format!("category-{}", category_full_slug(cx, c)));
    }
    if last_visit {
        classes.push("last-visit".to_string());
    }
    let unread_posts = topic["unread_posts"].as_i64().unwrap_or(0);
    // Topic#visited: >= for deleted posts at the end of the topic.
    let visited = topic["last_read_post_number"]
        .as_i64()
        .is_some_and(|read| read >= highest);
    for (on, class) in [
        (visited, "visited"),
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
    // discourse-solved's topic-list-item-class transformer.
    if flag("has_accepted_answer") {
        classes.push("status-solved".to_string());
    }

    let statuses = topic_statuses(cx, topic);

    let last_unread_url = last_unread_url(cx, topic);
    let title_html = emoji_unescape(s(&topic["fancy_title"]), &cx.settings.emojis, base);
    let title_html = if cx.settings.support_mixed_text_direction {
        format!("<span dir=\"auto\">{title_html}</span>")
    } else {
        title_html
    };

    // TopicLink: a visited topic says so to screen readers.
    let read_suffix = if visited {
        format!(
            "<span class=\"sr-only\">&nbsp;{}</span>",
            escape(&t(cx, "topic.sr_read"))
        )
    } else {
        String::new()
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
    // TopicCell: no badge under @hideCategory, nor for a pinned topic in
    // uncategorized.
    let badge = match category {
        Some(c) if !cx.hide_category && !pinned_uncategorized => category_badge(cx, c),
        _ => String::new(),
    };
    let tags = tags_html(cx, topic, s(&topic["title"]), &tags_callbacks(cx, topic));

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
                emoji_unescape(e, &cx.settings.emojis, base)
            )
        }
        _ => String::new(),
    };

    // ParticipantGroups: a message's groups, each to its page.
    let participant_groups = match topic["participant_groups"].as_array() {
        Some(groups) if !groups.is_empty() => {
            let items: String = groups
                .iter()
                .filter_map(Value::as_str)
                .map(|name| {
                    format!(
                        "<div class=\"participant-group\"><a class=\"user-group trigger-group-card\" data-group-card=\"{0}\" href=\"{base}/g/{0}\">{1}{0}</a></div>",
                        escape(name),
                        icon("users", None)
                    )
                })
                .collect();
            format!(
                "<div aria-label=\"{}\" class=\"participant-group-wrapper\" role=\"list\">{items}</div>",
                escape(&t(cx, "topic.participant_groups"))
            )
        }
        _ => String::new(),
    };
    let topic_cell = format!(
        "<td class=\"main-link topic-list-data\" colspan=\"1\"><span aria-level=\"2\" class=\"link-top-line\" role=\"heading\"><span class=\"topic-statuses\">{statuses}</span><a class=\"title raw-link raw-topic-link\" data-topic-id=\"{id}\" href=\"{last_unread_url}\">{title_html}{read_suffix}</a>{badges}</span><div class=\"link-bottom-line\">{badge}{tags}{participant_groups}</div>{excerpt_html}</td>"
    );

    // PostersCell: each poster's avatar, linked.
    let mut posters = String::from("<td class=\"posters topic-list-data\">");
    for poster in featured_users(topic) {
        if let Some(more) = poster["more_count"].as_str() {
            posters.push_str(&format!("<a class=\"posters-more-count\">{more}</a>"));
            continue;
        }
        let poster = &poster;
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
    if !show_posters {
        posters.clear();
    }

    if mobile {
        // TopicListItem's mobile stats: the badge unless @hideCategory.
        let category_link = category
            .filter(|_| !cx.hide_category)
            .map(|c| category_badge(cx, c))
            .unwrap_or_default();
        return mobile_row(
            cx,
            topic,
            users,
            MobileRow {
                classes: &classes.join(" "),
                id,
                statuses: &statuses,
                title: &format!(
                    "<a class=\"title raw-link raw-topic-link\" data-topic-id=\"{id}\" href=\"{last_unread_url}\">{title_html}{read_suffix}</a>"
                ),
                excerpt: &excerpt_html,
                last_unread_url: &last_unread_url,
                last_post_url: &url_for(highest),
                category_link: &category_link,
                tags: &tags,
            },
        );
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

    let activity = activity_cell(cx, topic, "", &url_for(highest));

    format!(
        "<tr class=\"{}\" data-topic-id=\"{id}\">{topic_cell}{posters}{replies}{views}{activity}</tr>",
        classes.join(" ")
    )
}

/// ActivityCell: the bump age, cold by the time since creation.
pub(crate) fn activity_cell(
    cx: &ListContext,
    topic: &Value,
    extra_class: &str,
    last_post_url: &str,
) -> String {
    let created = date(&topic["created_at"]);
    let bumped = date(&topic["bumped_at"]).or(created);
    match (created, bumped) {
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
            let title = bumped_at_title(cx, created, bumped);
            format!(
                "<td class=\"activity num topic-list-data age{cold}{extra_class}\"{title}><a class=\"post-activity\" href=\"{last_post_url}\">{}</a></td>",
                relative_date(cx, bumped)
            )
        }
        _ => format!("<td class=\"activity num topic-list-data age{extra_class}\"></td>"),
    }
}

/// What a mobile row takes from the desktop one's parts.
struct MobileRow<'a> {
    classes: &'a str,
    id: i64,
    statuses: &'a str,
    /// TopicLink
    title: &'a str,
    /// TopicExcerpt when the pinned excerpt is expanded, else empty.
    excerpt: &'a str,
    last_unread_url: &'a str,
    last_post_url: &'a str,
    /// categoryLink, empty without a category.
    category_link: &'a str,
    /// discourseTags in the list mode.
    tags: &'a str,
}

/// RepliesCell's heat: likes per post past the topic_post_like_heat
/// settings, from ten posts on.
fn likes_heat(cx: &ListContext, topic: &Value) -> &'static str {
    let posts_count = topic["posts_count"].as_i64().unwrap_or(0);
    let ratio = if posts_count < 10 {
        0.0
    } else {
        topic["like_count"].as_f64().unwrap_or(0.0) / posts_count as f64
    };
    let [low, medium, high] = cx.settings.topic_post_like_heat;
    if ratio > high {
        " heatmap-high"
    } else if ratio > medium {
        " heatmap-med"
    } else if ratio > low {
        " heatmap-low"
    } else {
        ""
    }
}

/// TopicListItem's mobile layout (useMobileLayout): one cell with the
/// latest poster's avatar, the title, the reply count (or the unread
/// badge) and a line with the category, the tags and the activity.
fn mobile_row(cx: &ListContext, topic: &Value, users: &[Value], row: MobileRow) -> String {
    let base = cx.base_path;
    let flag = |name: &str| topic[name] == true;
    // Topic#lastPoster: the poster marked latest, else the first.
    let posters = topic["posters"].as_array().cloned().unwrap_or_default();
    let last_poster = posters
        .iter()
        .find(|p| s(&p["extras"]).split(' ').any(|e| e == "latest"))
        .or(posters.first());
    // A list's posters name their user in `users`; suggested topics'
    // carry theirs inline.
    let avatar = last_poster
        .and_then(|p| {
            users
                .iter()
                .find(|u| u["id"] == p["user_id"])
                .or_else(|| p.get("user").filter(|u| u.is_object()))
        })
        .map(|user| {
            let username = s(&user["username"]);
            let name = s(&user["name"]);
            let display = if cx.settings.prioritize_name && !name.is_empty() {
                name
            } else {
                username
            };
            let size = cx.settings.avatar_size_48;
            let src = s(&user["avatar_template"]).replace("{size}", &size.to_string());
            let src = if src.starts_with('/') && !src.starts_with("//") && !src.starts_with(base) {
                format!("{base}{src}")
            } else {
                src
            };
            format!(
                "<a aria-label=\"{}\" data-user-card=\"{username}\" href=\"{base}/u/{username}\" tabindex=\"0\"><img alt='' width='48' height='48' src='{src}' class='avatar' title='{}'></a>",
                escape(&t_with(cx, "latest_poster_link", &[("username", username)])),
                escape(display)
            )
        })
        .unwrap_or_default();
    // The new topic dot; TopicPostBadges' unread count goes on the right.
    let unseen = if flag("unseen") {
        "<span class=\"topic-post-badges\">&nbsp;<span class=\"badge-notification new-topic\"></span></span>"
    } else {
        ""
    };
    // PostCountOrBadges
    let unread_posts = topic["unread_posts"].as_i64().unwrap_or(0);
    let count = if cx.member_trust_level.is_some() && unread_posts > 0 {
        topic_post_badges(cx, topic, row.last_unread_url)
    } else {
        replies_div(cx, topic)
    };
    let created = date(&topic["created_at"]);
    let bumped = date(&topic["bumped_at"]).or(created);
    let activity = match (created, bumped) {
        (Some(created), Some(bumped)) => {
            let title = bumped_at_title(cx, created, bumped);
            format!(
                "<span class=\"age activity\"{title}><a href=\"{}\">{}</a></span>",
                row.last_post_url,
                relative_date(cx, bumped)
            )
        }
        _ => String::new(),
    };
    format!(
        "<tr class=\"{classes}\" data-topic-id=\"{id}\"><td class=\"topic-list-data\"><div class=\"pull-left\">{avatar}</div><div class=\"topic-item-metadata right\"><div class=\"main-link\">{statuses}{title}{unseen}{excerpt}</div><div class=\"pull-right\">{count}</div><div class=\"topic-item-stats clearfix\"><span class=\"topic-item-stats__category-tags\">{category}{tags}</span><div class=\"num activity last\">{activity}</div></div></div></td></tr>",
        classes = row.classes,
        id = row.id,
        statuses = format_args!("<span class=\"topic-statuses\">{}</span>", row.statuses),
        title = row.title,
        excerpt = row.excerpt,
        category = row.category_link,
        tags = row.tags,
    )
}

/// Topic#featuredUsers: the posters; on a message, the participants who
/// did not post go in too (before the latest poster), up to five, and past
/// five allowed users the second-last gives way to a `+N` count
/// (`more_count`). Participants carry no extras (TopicList hydrates only
/// the posters' extraClasses) and no description.
fn featured_users(topic: &Value) -> Vec<Value> {
    const MAX_USER_COUNT: usize = 5;
    let mut users: Vec<Value> = topic["posters"].as_array().cloned().unwrap_or_default();
    let private_message = topic["archetype"] == "private_message";
    if private_message
        && let Some(participants) = topic["participants"].as_array()
        && users.len() < MAX_USER_COUNT
    {
        let poster_count = users.len();
        let push_offset = usize::from(
            poster_count > 1
                && users[poster_count - 1]["extras"]
                    .as_str()
                    .is_some_and(|e| e.contains("latest")),
        );
        let poster_ids: Vec<Value> = users.iter().map(|p| p["user_id"].clone()).collect();
        for p in participants {
            if !poster_ids.contains(&p["user_id"]) {
                let at = users.len() - push_offset;
                users.insert(
                    at,
                    serde_json::json!({"user_id": p["user_id"], "extras": null, "description": null}),
                );
                if users.len() == MAX_USER_COUNT {
                    break;
                }
            }
        }
    }
    let allowed = topic["allowed_user_count"].as_i64().unwrap_or(0);
    if private_message && allowed > MAX_USER_COUNT as i64 {
        if users.len() > MAX_USER_COUNT - 2 {
            users.remove(MAX_USER_COUNT - 2);
        }
        users.push(
            serde_json::json!({"more_count": format!("+{}", allowed - MAX_USER_COUNT as i64 + 1)}),
        );
    }
    users
}

/// TopicStatus, for a visitor (a member's pin toggles are links).
pub fn topic_statuses(cx: &ListContext, topic: &Value) -> String {
    topic_statuses_with(cx, topic, false)
}

/// TopicStatus; `disable_actions` keeps a member's statuses spans, as
/// inside another link (a search result's).
pub fn topic_statuses_with(cx: &ListContext, topic: &Value, disable_actions: bool) -> String {
    let flag = |name: &str| topic[name] == true;
    let mut statuses = String::new();
    if flag("bookmarked") {
        // canAct (a member): a link to the topic.
        let (open, close) = if cx.member_trust_level.is_some() && !disable_actions {
            let slug = match s(&topic["slug"]).trim() {
                "" => "topic",
                slug => slug,
            };
            (
                format!(
                    "<a class=\"topic-status --bookmarked\" href=\"{}/t/{slug}/{}\"",
                    cx.base_path,
                    topic["id"].as_i64().unwrap_or(0)
                ),
                "</a>",
            )
        } else {
            (
                "<span class=\"topic-status --bookmarked\"".to_string(),
                "</span>",
            )
        };
        statuses.push_str(&format!(
            "{open} title=\"{}\">{}{close}",
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
    // Topic#invisible, with visibilityReasonTranslated (a reason id of 0
    // is falsy there, so it goes unexplained).
    if topic["visible"] == false {
        const REASONS: [&str; 6] = [
            "op_flag_threshold_reached",
            "op_unhidden",
            "embedded_topic",
            "manually_unlisted",
            "manually_relisted",
            "bulk_action",
        ];
        let reason = topic["visibility_reason_id"]
            .as_i64()
            .filter(|id| *id != 0)
            .and_then(|id| REASONS.get(usize::try_from(id).ok()?))
            .map(|key| t(cx, &format!("topic_statuses.visibility_reasons.{key}")))
            .unwrap_or_default();
        statuses.push_str(&format!(
            "<span class=\"topic-status --invisible\" title=\"{}\">{}</span>",
            escape(&t_with(
                cx,
                "topic_statuses.unlisted.help",
                &[("unlistedReason", &reason)]
            )),
            icon("far-eye-slash", None)
        ));
    }
    // discourse-solved's after-topic-status outlet (SolvedStatus). The
    // unsolved status is the topic list's, and only lists carry
    // can_have_answer.
    if flag("has_accepted_answer") || flag("accepted_answer") {
        statuses.push_str(&format!(
            "<span class=\"topic-status --solved\" title=\"{}\">{}</span>",
            escape(&t(cx, "topic_statuses.solved.help")),
            icon("far-square-check", None)
        ));
    } else if flag("can_have_answer") {
        statuses.push_str(&format!(
            "<span class=\"topic-status --unsolved\" title=\"{}\">{}</span>",
            escape(&t(cx, "solved.has_no_accepted_answer")),
            icon("far-square", None)
        ));
    }
    statuses
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn featured_users_add_message_participants() {
        use serde_json::json;
        // A message's participants who did not post go before the latest
        // poster, without extras.
        let topic = serde_json::json!({
            "archetype": "private_message",
            "allowed_user_count": 3,
            "posters": [
                {"user_id": 1, "extras": null, "description": "Original Poster"},
                {"user_id": 2, "extras": "latest", "description": "Most Recent Poster"},
            ],
            "participants": [
                {"user_id": 2, "extras": "latest"},
                {"user_id": 4, "extras": "latest"},
            ],
        });
        let ids: Vec<Value> = featured_users(&topic)
            .iter()
            .map(|u| u["user_id"].clone())
            .collect();
        assert_eq!(ids, vec![json!(1), json!(4), json!(2)]);
        assert!(featured_users(&topic)[1]["extras"].is_null());
        // Past five allowed users, the second-last gives way to a count.
        let mut many = topic.clone();
        many["allowed_user_count"] = json!(8);
        let users = featured_users(&many);
        assert_eq!(users.last().unwrap()["more_count"], "+4");
        // A regular topic keeps its posters.
        let mut regular = topic;
        regular["archetype"] = json!("regular");
        assert_eq!(featured_users(&regular).len(), 2);
    }

    #[test]
    fn escapes_as_ember_does() {
        assert_eq!(
            escape("a<b>&\"'`="),
            "a&lt;b&gt;&amp;&quot;&#x27;&#x60;&#x3D;"
        );
    }

    fn settings(inline: bool) -> ListSettings {
        ListSettings {
            show_pinned_excerpt_desktop: true,
            show_pinned_excerpt_mobile: true,
            suppress_uncategorized_badge: true,
            uncategorized_category_id: 1,
            tag_style: "simple".into(),
            suppress_overlapping_tags_in_list: false,
            topic_views_heat: [1000, 2000, 5000],
            topic_post_like_heat: [0.5, 1.0, 2.0],
            cold_age_days: [14.0, 30.0, 60.0],
            relative_date_duration: 30,
            avatar_size_24: 24,
            avatar_size_48: 48,
            prioritize_name: false,
            emojis: std::sync::Arc::new(emojis(inline)),
            support_mixed_text_direction: false,
            assign: None,
        }
    }

    fn emojis(inline: bool) -> Emojis {
        Emojis {
            enabled: true,
            shortcuts: true,
            inline,
            set: "twitter".into(),
            ..Default::default()
        }
    }

    #[test]
    fn marks_the_last_topic_bumped_since_the_previous_visit() {
        use serde_json::json;
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        let topic = |bumped: &str, pinned: bool| json!({"bumped_at": bumped, "pinned": pinned});
        let visit = at("2026-10-10T00:00:00Z");
        let topics = vec![
            topic("2026-01-01T00:00:00Z", true),
            topic("2026-10-12T00:00:00Z", false),
            topic("2026-10-11T00:00:00Z", false),
            topic("2026-10-01T00:00:00Z", false),
        ];
        assert_eq!(last_visited_topic(&topics, visit), Some(2));
        // Everything newer, or everything older: no marker.
        assert_eq!(last_visited_topic(&topics[..3], visit), None);
        assert_eq!(
            last_visited_topic(&topics, at("2026-10-13T00:00:00Z")),
            None
        );
        assert_eq!(last_visited_topic(&topics[1..2], visit), None);
    }

    #[tokio::test]
    async fn the_mobile_view_lays_rows_out_as_ember_does() {
        use serde_json::json;
        let i18n = crate::i18n::I18n::vendored().unwrap();
        let categories = HashMap::new();
        let cx = ListContext {
            i18n: &i18n,
            base_path: "",
            now: Utc::now(),
            categories: &categories,
            expand_all_pinned: false,
            hide_category: false,
            member_trust_level: None,
            settings: settings(false),
        };
        let topic = json!({
            "id": 7, "slug": "hello", "fancy_title": "Hello", "title": "Hello",
            "posts_count": 3, "highest_post_number": 3,
            "created_at": "2026-10-01T00:00:00.000Z", "bumped_at": "2026-10-02T00:00:00.000Z",
            "posters": [
                {"user_id": 1, "extras": null},
                {"user_id": 2, "extras": "latest"},
            ],
        });
        let users = vec![
            json!({"id": 1, "username": "alice", "avatar_template": "/a/{size}.png"}),
            json!({"id": 2, "username": "bob", "avatar_template": "/b/{size}.png"}),
        ];
        let desktop = row(&cx, &topic, &users);
        assert!(desktop.contains("<td class=\"posters topic-list-data\">"));
        let mobile = crate::mobile::scope(true, async { row(&cx, &topic, &users) }).await;
        assert!(mobile.starts_with(
            "<tr class=\"topic-list-item\" data-topic-id=\"7\"><td class=\"topic-list-data\"><div class=\"pull-left\"><a aria-label=\"bob&#x27;s profile, latest poster\" data-user-card=\"bob\" href=\"/u/bob\" tabindex=\"0\"><img alt='' width='48' height='48' src='/b/48.png' class='avatar' title='bob'></a></div>"
        ));
        assert!(mobile.contains("<div class=\"pull-right\"><div class=\"num posts-map posts topic-list-data\"><a aria-label=\"2 replies, jump to first post\" class=\"badge-posts\" href=\"/t/hello/7/1\"><span class='number'>2</span></a></div></div>"));
        assert!(
            mobile
                .contains("<div class=\"num activity last\"><span class=\"age activity\" title=\"")
        );
        assert!(!mobile.contains("posters"));
    }

    fn emoji_codes(text: &str, inline_emoji: bool) -> Vec<String> {
        let html = emoji_unescape(text, &emojis(inline_emoji), "");
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
        // Emoticons, and the title as written.
        assert_eq!(emoji_codes("hi :) there", false), ["slight_smile"]);
        assert_eq!(emoji_codes(":Smile:", false), ["Smile"]);
    }

    #[test]
    fn unescapes_custom_and_drops_denied_emoji() {
        let mut e = emojis(false);
        e.custom.push(crate::emojis::Custom {
            name: "party_parrot".into(),
            url: Some("/uploads/default/original/1X/abc.gif".into()),
            cdn_url: Some("/uploads/default/original/1X/abc.gif".into()),
            group: "default".into(),
            created_by: None,
        });
        e.denied = vec!["heart".into(), "red_heart".into()];
        assert_eq!(
            emoji_unescape("a :party_parrot: b :heart: c", &e, "/forum"),
            format!(
                "a <img width=\"20\" height=\"20\" src='/uploads/default/original/1X/abc.gif?v={}' title='party_parrot' alt='party_parrot' class='emoji emoji-custom'> b  c",
                crate::emoji::image_version()
            )
        );
        // The server's unescape_emoji keeps denied emoji: a topic onebox's
        // title on the reference with heart denied.
        assert_eq!(
            unescape_emoji("Liked :party_parrot: and :heart: archived", &e, ""),
            format!(
                "Liked <img width=\"20\" height=\"20\" src='/uploads/default/original/1X/abc.gif?v={v}' title='party_parrot' alt='party_parrot' class='emoji emoji-custom'> and <img width=\"20\" height=\"20\" src='/images/emoji/twitter/heart.png?v={v}' title='heart' alt='heart' class='emoji'> archived",
                v = crate::emoji::image_version()
            )
        );
        e.enabled = false;
        assert_eq!(emoji_unescape(":heart: :smile:", &e, ""), ":heart: :smile:");
    }
}
