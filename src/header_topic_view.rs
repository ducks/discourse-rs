//! The docked header's topic info (components/header/topic/info.gjs and
//! its Participant): once the topic's title scrolls out of view, Ember's
//! header draws the topic's title, linked back to the top, after the
//! minimized logo, with its statuses, category badges, tags and, for a
//! message, its envelope and participants. The server renders both into
//! a template on the topic page; static/js/header-topic-info.js docks
//! them as the header service decides (services/header.js
//! topicInfoVisible).

use serde_json::Value;

use crate::post_view::{PostContext, avatar_img};
use crate::topic_list_view::{ListCategory, escape, icon, t};

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// The `<template>` static/js/header-topic-info.js docks from: the
/// minimized logo (HomeLogo with `@minimized`) and Header::Topic::Info.
pub fn template(cx: &PostContext, view: &Value, logo: &MinimizedLogo) -> String {
    format!(
        "<template id=\"header-topic-info\">{}{}</template>",
        minimized_logo(cx.list.base_path, logo),
        info(cx, view)
    )
}

/// What HomeLogoContents draws minimized: the small logo
/// (`site_logo_small_url`) or, without one, the house icon.
pub struct MinimizedLogo {
    pub small_url: String,
    pub title: String,
}

/// HomeLogo's `div.title`, minimized.
pub fn minimized_logo(base_path: &str, logo: &MinimizedLogo) -> String {
    let contents = if logo.small_url.is_empty() {
        icon("house", None)
    } else {
        // Logo with the logo-small key. Its dark variant (a <picture>) is
        // 258a's, with the other dark logos.
        format!(
            "<img alt=\"{}\" class=\"logo-small\" id=\"site-logo\" src=\"{}\" width=\"36\">",
            escape(&logo.title),
            escape(&logo.small_url)
        )
    };
    format!("<div class=\"title--minimized title\"><a href=\"{base_path}/\">{contents}</a></div>")
}

/// Header::Topic::Info for the topic view document.
pub fn info(cx: &PostContext, view: &Value) -> String {
    let list = cx.list;
    let id = view["id"].as_i64().unwrap_or(0);
    let url = format!("{}/t/{}/{id}", list.base_path, s(&view["slug"]));
    // showPM: a message, not a warning.
    let show_pm = view["archetype"] == "private_message" && view["is_warning"] != true;
    let tags = crate::topic_list_view::render_tags(list, view);
    // twoRows: tags, a message, or topic_featured_link_enabled.
    let two_rows = !tags.is_empty() || show_pm || cx.settings.topic_featured_link_enabled;
    let two_rows_class = if two_rows { "two-rows " } else { "" };

    let mut title = String::new();
    if show_pm {
        title.push_str(&format!(
            "<a aria-label=\"{}\" class=\"private-message-glyph-wrapper\"{}>{}</a>",
            escape(&t(list, "user.messages.inbox")),
            cx.topic
                .pm_path
                .as_deref()
                .map(|href| format!(" href=\"{}\"", escape(href)))
                .unwrap_or_default(),
            icon("envelope", Some("private-message-glyph"))
        ));
    }
    let fancy = view["fancy_title"]
        .as_str()
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| s(&view["title"]));
    // `@topicInfo.fancyTitle` and `@topicInfo.url`: the statuses
    // (TopicStatus in the header context), the link and the title suffix
    // outlet.
    if !fancy.is_empty() {
        title.push_str(&format!(
            "<span class=\"topic-statuses\">{}</span><a class=\"topic-link\" data-topic-id=\"{id}\" href=\"{}\"><span>{}</span></a><span class=\"header-topic-title-suffix\"></span>",
            crate::topic_list_view::topic_statuses(list, view),
            escape(&url),
            crate::topic_list_view::emoji_unescape(fancy, &list.settings.emojis, list.base_path),
        ));
    }

    let categories = view["category_id"]
        .as_i64()
        .and_then(|c| list.categories.get(&c))
        .filter(|c| {
            c.id != list.settings.uncategorized_category_id
                || !list.settings.suppress_uncategorized_badge
        })
        .map(|c| categories_wrapper(cx, c))
        .unwrap_or_default();

    let mut extra = tags;
    if show_pm {
        extra.push_str(&participants(cx, view, &url));
    }
    // FeaturedLink: a topic with a featured link refuses before here, at
    // its featured_link_root_domain (topic_view.rs).

    format!(
        "<div class=\"{two_rows_class}extra-info-wrapper\"><div class=\"{two_rows_class}extra-info\"><div class=\"title-wrapper\"><h1 class=\"header-title\">{title}</h1>{categories}<div class=\"topic-header-extra\">{extra}</div></div></div></div>"
    )
}

/// The category's badges, each without its parent: the grandparent's (on
/// the desktop only), the parent's, then its own.
fn categories_wrapper(cx: &PostContext, category: &ListCategory) -> String {
    let list = cx.list;
    let badge = |c: &ListCategory| crate::topic_list_view::category_badge_html(list, c, true, true);
    let mut out = String::from("<div class=\"categories-wrapper\">");
    if let Some(parent) = category.parent_id.and_then(|p| list.categories.get(&p)) {
        if let Some(grandparent) = parent.parent_id.and_then(|p| list.categories.get(&p))
            && !crate::mobile::mobile_view()
        {
            out.push_str(&badge(grandparent));
        }
        out.push_str(&badge(parent));
    }
    out.push_str(&badge(category));
    out.push_str("</div>");
    out
}

/// `.topic-header-participants`: the message's users then its groups, up
/// to maxExtraItems (5 beside tags, else 10), and a link to the top for
/// the rest. As Ember counts them, the rest are the allowed users and the
/// topic's own `allowed_groups` (which the topic view document leaves
/// out), not the details' groups.
fn participants(cx: &PostContext, view: &Value, url: &str) -> String {
    let list = cx.list;
    let details = &view["details"];
    let empty = Vec::new();
    let users = details["allowed_users"].as_array().unwrap_or(&empty);
    let groups = details["allowed_groups"].as_array().unwrap_or(&empty);
    let has_tags = view["tags"].as_array().is_some_and(|t| !t.is_empty());
    let max_extra_items: usize = if has_tags { 5 } else { 10 };
    let total = users.len() + view["allowed_groups"].as_array().map_or(0, Vec::len);

    let mut out = String::from("<div class=\"topic-header-participants\">");
    for participant in users.iter().chain(groups.iter()).take(max_extra_items) {
        match participant["username"].as_str() {
            Some(username) => out.push_str(&format!(
                "<span class=\"trigger-user-card\"><a class=\"icon\" data-auto-route=\"true\" href=\"{}/u/{}\" title=\"{}\">{}</a></span>",
                list.base_path,
                escape(&username.to_lowercase()),
                escape(username),
                avatar_img(
                    s(&participant["avatar_template"]),
                    cx.settings.avatar_size_24,
                    &format!(" title=\"{}\"", escape(username))
                )
            )),
            None => {
                let name = s(&participant["name"]);
                out.push_str(&format!(
                    "<span class=\"trigger-group-card\"><a class=\"icon\" data-auto-route=\"true\" href=\"{}/g/{}\" title=\"{}\"><span>{} {}</span></a></span>",
                    list.base_path,
                    escape(name),
                    escape(name),
                    icon("users", None),
                    escape(name)
                ));
            }
        }
    }
    if total > max_extra_items {
        out.push_str(&format!(
            "<a class=\"more-participants\" data-topic-id=\"{}\" href=\"{}\">+{}</a>",
            view["id"].as_i64().unwrap_or(0),
            escape(url),
            total - max_extra_items
        ));
    }
    out.push_str("</div>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::post_view::{PostSettings, TopicInfo};
    use crate::topic_list_view::{ListContext, ListSettings};
    use serde_json::json;
    use std::collections::HashMap;

    fn category(id: i64, name: &str, parent: Option<i64>) -> ListCategory {
        ListCategory {
            id,
            name: name.into(),
            slug: name.to_lowercase(),
            color: "25AAE2".into(),
            text_color: "FFFFFF".into(),
            style_type: "square",
            emoji: None,
            icon: None,
            read_restricted: false,
            parent_id: parent,
            navigate_to_first_post_after_read: false,
        }
    }

    /// Header::Topic::Info for `view`, with a three-level category tree
    /// (1 uncategorized, 2 > 3 > 4) and the viewer's inbox at /my/inbox.
    fn render(view: Value, featured_link_enabled: bool) -> String {
        let i18n = crate::i18n::I18n::vendored().unwrap();
        let categories: HashMap<i64, ListCategory> = [
            category(1, "Uncategorized", None),
            category(2, "Top", None),
            category(3, "Middle", Some(2)),
            category(4, "Bottom", Some(3)),
        ]
        .into_iter()
        .map(|c| (c.id, c))
        .collect();
        let list = ListContext {
            i18n: &i18n,
            base_path: "",
            now: chrono::DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            categories: &categories,
            expand_all_pinned: false,
            hide_category: false,
            member_trust_level: Some(1),
            settings: ListSettings {
                show_pinned_excerpt_desktop: true,
                show_pinned_excerpt_mobile: true,
                suppress_uncategorized_badge: true,
                uncategorized_category_id: 1,
                tag_style: "simple".into(),
                suppress_overlapping_tags_in_list: true,
                topic_views_heat: [1000, 2000, 5000],
                topic_post_like_heat: [0.5, 1.0, 2.0],
                cold_age_days: [14.0, 30.0, 60.0],
                relative_date_duration: 30,
                avatar_size_24: 24,
                avatar_size_48: 48,
                prioritize_name: false,
                emojis: std::sync::Arc::new(crate::emojis::Emojis {
                    enabled: true,
                    shortcuts: true,
                    set: "twitter".into(),
                    ..Default::default()
                }),
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
            short_progress_text_threshold: 10000,
            tagging_enabled: true,
            topic_featured_link_enabled: featured_link_enabled,
            post_menu: Vec::new(),
            post_menu_hidden_items: Vec::new(),
            reactions: None,
            solved: None,
            presence: false,
            poll: None,
            cakeday: None,
            hashtag_tag_icon: None,
        };
        let mut topic = TopicInfo::from_view(&view);
        topic.pm_path = Some("/my/inbox".into());
        let cx = PostContext {
            list: &list,
            settings: &settings,
            topic: &topic,
            viewer: Some("me"),
            staff: false,
            can_send_pms: true,
            ignored_users: &[],
        };
        info(&cx, &view)
    }

    fn user(username: &str) -> Value {
        json!({
            "id": 1,
            "username": username,
            "avatar_template": format!("/a/{username}/{{size}}.png"),
        })
    }

    #[test]
    fn a_topic_shows_its_title_statuses_categories_and_tags() {
        let view = json!({
            "id": 7,
            "slug": "a-topic",
            "title": "A topic",
            "fancy_title": "A topic &amp; more",
            "archetype": "regular",
            "category_id": 4,
            "tags": ["topic"],
            "closed": true,
            "details": {},
        });
        let html = render(view.clone(), true);
        assert!(html.starts_with(
            "<div class=\"two-rows extra-info-wrapper\"><div class=\"two-rows extra-info\"><div class=\"title-wrapper\"><h1 class=\"header-title\"><span class=\"topic-statuses\"><span class=\"topic-status --closed\""
        ), "{html}");
        assert!(html.contains(
            "<a class=\"topic-link\" data-topic-id=\"7\" href=\"/t/a-topic/7\"><span>A topic &amp; more</span></a><span class=\"header-topic-title-suffix\"></span></h1>"
        ), "{html}");
        // Each badge without its parent's class, the grandparent's first
        // on the desktop.
        let badges: Vec<&str> = html.matches("data-category-id=\"").collect();
        assert_eq!(badges.len(), 3, "{html}");
        let top = html.find("data-category-id=\"2\"").unwrap();
        let middle = html.find("data-category-id=\"3\"").unwrap();
        let bottom = html.find("data-category-id=\"4\"").unwrap();
        assert!(top < middle && middle < bottom);
        assert!(!html.contains("--has-parent"), "{html}");
        // renderTags without the list's overlapping tag filter.
        assert!(html.contains(
            "<div class=\"topic-header-extra\"><ul class='discourse-tags' aria-label=Tags><li><a href='/tag/topic'  data-tag-name=topic class='discourse-tag simple'>topic</a></li></ul></div>"
        ), "{html}");
        assert!(!html.contains("topic-header-participants"));
        assert!(!html.contains("private-message-glyph"));
    }

    #[tokio::test]
    async fn the_mobile_view_leaves_out_the_grandparent_badge() {
        let view = json!({
            "id": 7, "slug": "a-topic", "title": "A topic", "archetype": "regular",
            "category_id": 4, "details": {},
        });
        let html = crate::mobile::scope(true, async { render(view, true) }).await;
        assert!(!html.contains("data-category-id=\"2\""), "{html}");
        assert!(html.contains("data-category-id=\"3\""), "{html}");
        assert!(html.contains("data-category-id=\"4\""), "{html}");
    }

    #[test]
    fn one_row_without_tags_a_message_or_featured_links() {
        let view = json!({
            "id": 7, "slug": "a-topic", "title": "A topic", "archetype": "regular",
            "category_id": 1, "details": {},
        });
        let html = render(view, false);
        assert!(html.starts_with(
            "<div class=\"extra-info-wrapper\"><div class=\"extra-info\"><div class=\"title-wrapper\">"
        ), "{html}");
        // suppress_uncategorized_badge: no categories at all.
        assert!(!html.contains("categories-wrapper"), "{html}");
        assert!(html.ends_with("<div class=\"topic-header-extra\"></div></div></div></div>"));
    }

    #[test]
    fn a_message_shows_its_envelope_and_participants() {
        let view = json!({
            "id": 8, "slug": "a-message", "title": "A message", "archetype": "private_message",
            "details": {
                "allowed_users": [user("Bob"), user("alice")],
                "allowed_groups": [{ "id": 3, "name": "staff" }],
            },
        });
        let html = render(view, true);
        assert!(html.contains(
            "<h1 class=\"header-title\"><a aria-label=\"Inbox\" class=\"private-message-glyph-wrapper\" href=\"/my/inbox\"><svg class=\"fa d-icon d-icon-envelope svg-icon fa-width-auto private-message-glyph svg-string\""
        ), "{html}");
        assert!(html.contains(
            "<div class=\"topic-header-participants\"><span class=\"trigger-user-card\"><a class=\"icon\" data-auto-route=\"true\" href=\"/u/bob\" title=\"Bob\"><img alt=\"\" width=\"24\" height=\"24\" src=\"/a/Bob/24.png\" class=\"avatar\" title=\"Bob\"></a></span><span class=\"trigger-user-card\"><a class=\"icon\" data-auto-route=\"true\" href=\"/u/alice\" title=\"alice\">"
        ), "{html}");
        assert!(html.contains(
            "<span class=\"trigger-group-card\"><a class=\"icon\" data-auto-route=\"true\" href=\"/g/staff\" title=\"staff\"><span><svg class=\"fa d-icon d-icon-users svg-icon fa-width-auto svg-string\""
        ), "{html}");
        assert!(
            html.contains("</svg> staff</span></a></span></div>"),
            "{html}"
        );
        assert!(!html.contains("more-participants"));
    }

    #[test]
    fn a_warning_has_no_envelope_or_participants() {
        let view = json!({
            "id": 8, "slug": "a-warning", "title": "A warning", "archetype": "private_message",
            "is_warning": true,
            "details": { "allowed_users": [user("bob")] },
        });
        let html = render(view, false);
        assert!(!html.contains("private-message-glyph-wrapper"), "{html}");
        assert!(!html.contains("topic-header-participants"), "{html}");
        assert!(
            html.starts_with("<div class=\"extra-info-wrapper\">"),
            "{html}"
        );
    }

    #[test]
    fn beside_tags_five_participants_show_and_a_link_counts_the_rest() {
        let users: Vec<Value> = (0..7).map(|i| user(&format!("u{i}"))).collect();
        let view = json!({
            "id": 8, "slug": "a-message", "title": "A message", "archetype": "private_message",
            "tags": ["t"],
            "details": { "allowed_users": users, "allowed_groups": [{ "id": 3, "name": "staff" }] },
        });
        let html = render(view, true);
        assert_eq!(
            html.matches("class=\"trigger-user-card\"").count(),
            5,
            "{html}"
        );
        assert!(!html.contains("trigger-group-card"), "{html}");
        // totalParticipants counts the details' users and the topic's own
        // allowed_groups, which the document leaves out.
        assert!(html.contains(
            "<a class=\"more-participants\" data-topic-id=\"8\" href=\"/t/a-message/8\">+2</a></div>"
        ), "{html}");
    }

    #[test]
    fn the_minimized_logo_is_the_small_logo_or_the_house() {
        let small = minimized_logo(
            "/forum",
            &MinimizedLogo {
                small_url: "http://x/small.png".into(),
                title: "Site & co".into(),
            },
        );
        assert_eq!(
            small,
            "<div class=\"title--minimized title\"><a href=\"/forum/\"><img alt=\"Site &amp; co\" class=\"logo-small\" id=\"site-logo\" src=\"http://x/small.png\" width=\"36\"></a></div>"
        );
        let house = minimized_logo(
            "",
            &MinimizedLogo {
                small_url: String::new(),
                title: "Site".into(),
            },
        );
        assert!(house.starts_with(
            "<div class=\"title--minimized title\"><a href=\"/\"><svg class=\"fa d-icon d-icon-house svg-icon"
        ), "{house}");
    }
}
