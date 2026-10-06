//! The anonymous sidebar, rendered as Ember's components render it
//! (components/sidebar/anonymous/*, lib/sidebar/*) from the same Site
//! document they read: the public custom sections, the categories section
//! and the tags section, then the footer.

use serde_json::Value;

use crate::i18n::I18n;
use crate::site_settings::{SettingError, SiteSettings};
use crate::topic_list_view::{escape, icon};

/// The page being shown, for the links' active state (Ember's
/// `router.isActive` against each link's route and current-when).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Active {
    None,
    /// discovery.latest, .top, .hot and the other unscoped lists: the
    /// community section's Topics link.
    Discovery,
    /// discovery.category and its filters for this category id.
    Category(i32),
    /// discovery.categories
    Categories,
    /// tag.show for this tag name.
    Tag(String),
    /// tags (the tag index)
    Tags,
}

/// `TOP_SITE_CATEGORIES_TO_SHOW`
const TOP_SITE_CATEGORIES_TO_SHOW: usize = 5;

pub struct Context<'a> {
    pub i18n: &'a I18n,
    pub settings: &'a SiteSettings,
    pub base_path: &'a str,
    pub active: &'a Active,
}

impl Context<'_> {
    fn t(&self, key: &str) -> String {
        self.i18n.t(&format!("js.{key}")).unwrap_or(key).to_string()
    }

    /// `i18n(key, { defaultValue })`
    fn t_or(&self, key: &str, default: &str) -> String {
        self.i18n
            .t(&format!("js.{key}"))
            .unwrap_or(default)
            .to_string()
    }

    fn flag(&self, name: &str) -> Result<bool, SettingError> {
        Ok(self.settings.get(name)?.truthy())
    }
}

/// Sidebar.gjs with the anonymous sections, inside SidebarWrapper.
pub fn render(site: &Value, cx: &Context) -> Result<String, SettingError> {
    let mut out = String::new();
    out.push_str(&format!(
        "<div class=\"sidebar-wrapper\"><nav aria-label=\"{}\" class=\"sidebar-container\" id=\"d-sidebar\"><div class=\"sidebar-sections sidebar-sections-anonymous\">",
        escape(&cx.t("sidebar.title"))
    ));
    out.push_str("<div class=\"sidebar-custom-sections\">");
    for section in site["anonymous_sidebar_sections"]
        .as_array()
        .into_iter()
        .flatten()
    {
        out.push_str(&custom_section(section, cx)?);
    }
    out.push_str("</div>");
    out.push_str(&categories_section(site, cx)?);
    if cx.flag("tagging_enabled")? {
        out.push_str(&tags_section(site, cx));
    }
    out.push_str("</div>");
    // The footer's only anonymous action, keyboard shortcuts, opens a modal
    // that is not ported, so its bar is empty.
    out.push_str("<div class=\"sidebar-footer-wrapper\"><div class=\"sidebar-footer-container\"><div class=\"sidebar-footer-actions\"></div></div></div>");
    out.push_str("</nav></div>");
    Ok(out)
}

/// A rendered link (SectionLink's arguments).
#[derive(Default)]
struct Link {
    /// `@linkName`: data-list-item-name and data-link-name.
    link_name: Option<String>,
    /// Extra attributes on the li (`...attributes`).
    attributes: String,
    /// The plain `<a>` of an `@href` link, rather than a LinkTo.
    plain: bool,
    href: String,
    title: Option<String>,
    content: String,
    prefix: Prefix,
    active: bool,
}

#[derive(Default)]
enum Prefix {
    #[default]
    None,
    Icon {
        name: String,
        color: Option<String>,
    },
    Emoji {
        name: String,
        color: Option<String>,
    },
    Square {
        colors: Vec<String>,
        color: Option<String>,
        badge: Option<&'static str>,
    },
}

fn link_html(link: &Link, cx: &Context) -> String {
    let mut li = String::from("<li class=\"sidebar-section-link-wrapper\"");
    if let Some(name) = &link.link_name {
        li.push_str(&format!(" data-list-item-name=\"{}\"", escape(name)));
    }
    li.push_str(&link.attributes);
    li.push('>');

    let class = if link.active {
        "active sidebar-section-link sidebar-row"
    } else {
        "sidebar-section-link sidebar-row"
    };
    let mut a = format!("<a class=\"{class}\"");
    if let Some(title) = &link.title {
        a.push_str(&format!(" title=\"{}\"", escape(title)));
    }
    if let Some(name) = &link.link_name {
        a.push_str(&format!(" data-link-name=\"{}\"", escape(name)));
    }
    a.push_str(&format!(" href=\"{}\"", escape(&link.href)));
    if link.plain {
        a.push_str(" rel=\"noopener noreferrer\" target=\"_self\"");
    }
    a.push('>');
    format!(
        "{li}{a}{}<span class=\"sidebar-section-link-content-text\">{}</span></a></li>",
        prefix_html(&link.prefix, cx),
        link.content
    )
}

/// SectionLink#prefixColor: a hex color, with its `#`.
fn hex_color(input: &str) -> Option<String> {
    let hex = input.strip_prefix('#').unwrap_or(input);
    ((hex.len() == 6 || hex.len() == 3) && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("#{hex}"))
}

fn style(color: &Option<String>) -> String {
    match color {
        Some(c) => format!(" style=\"color: {c}\""),
        None => String::new(),
    }
}

/// SectionLinkPrefix
fn prefix_html(prefix: &Prefix, cx: &Context) -> String {
    match prefix {
        Prefix::None => String::new(),
        Prefix::Icon { name, color } => format!(
            "<span class=\"sidebar-section-link-prefix icon\"{}>{}</span>",
            style(color),
            icon(name, Some("prefix-icon"))
        ),
        Prefix::Emoji { name, color } => format!(
            "<span class=\"sidebar-section-link-prefix emoji\"{}>{}</span>",
            style(color),
            emoji_html(name, cx)
        ),
        Prefix::Square {
            colors,
            color,
            badge,
        } => {
            let mut stops: Vec<String> = colors
                .iter()
                .filter_map(|c| hex_color(c).map(|c| format!("{c} 50%")))
                .collect();
            if stops.len() == 1 {
                stops.push(stops[0].clone());
            }
            let badge = badge
                .map(|b| icon(b, Some("prefix-badge")))
                .unwrap_or_default();
            format!(
                "<span class=\"sidebar-section-link-prefix square\"{}><span class=\"prefix-square\" style=\"background: linear-gradient(90deg, {})\"></span>{badge}</span>",
                style(color),
                stops.join(", ")
            )
        }
    }
}

/// dReplaceEmoji(`:name:`, class "prefix-emoji"): a known emoji's image,
/// else the text as written.
fn emoji_html(name: &str, cx: &Context) -> String {
    if !crate::emoji::DATA.exists(name) {
        return escape(&format!(":{name}:"));
    }
    let set = cx
        .settings
        .get("emoji_set")
        .map(|v| v.to_s().to_string())
        .unwrap_or_default();
    let url = format!(
        "{}/images/emoji/{set}/{name}.png?v={}",
        cx.base_path,
        crate::emoji::image_version()
    );
    format!(
        "<img width=\"20\" height=\"20\" src=\"{}\" title=\"{n}\" alt=\"{n}\" class=\"emoji prefix-emoji\">",
        escape(&url),
        n = escape(name)
    )
}

/// Section.gjs with SectionHeader, collapsable as the anonymous sections
/// are.
fn section_html(name: &str, header: Option<&str>, links: &str, cx: &Context) -> String {
    let mut out = format!(
        "<div class=\"sidebar-section sidebar-section-wrapper sidebar-section--expanded\" data-section-name=\"{}\">",
        escape(name)
    );
    let content_id = format!("sidebar-section-content-{}", escape(name));
    if let Some(header) = header {
        out.push_str(&format!(
            "<div class=\"sidebar-section-header-wrapper sidebar-row\"><button aria-controls=\"{content_id}\" aria-expanded=\"true\" class=\"btn no-text sidebar-section-header sidebar-section-header-collapsable btn-transparent\" title=\"{}\" type=\"button\"><span class=\"sidebar-section-header-caret\">{}</span><span class=\"sidebar-section-header-text\">{}</span></button></div>",
            escape(&cx.t("sidebar.toggle_section")),
            icon("angle-down", None),
            escape(header)
        ));
    }
    out.push_str(&format!(
        "<ul class=\"sidebar-section-content\" id=\"{content_id}\">{links}</ul></div>"
    ));
    out
}

/// common/custom-section: the community section through CommunitySection,
/// any other public section through Section.
fn custom_section(section: &Value, cx: &Context) -> Result<String, SettingError> {
    let slug = section["slug"].as_str().unwrap_or("");
    let links = section["links"].as_array().cloned().unwrap_or_default();
    if section["section_type"] != "community" {
        let mut html = String::new();
        for link in &links {
            html.push_str(&link_html(&plain_link(link), cx));
        }
        let title = section["title"].as_str().unwrap_or("");
        return Ok(section_html(slug, Some(title), &html, cx));
    }

    let mut primary = Vec::new();
    let mut more = Vec::new();
    for link in &links {
        let generated = match community_link(link, cx)? {
            Some(generated) => generated,
            None => continue,
        };
        if link["segment"] == "primary" {
            primary.push(generated);
        } else {
            more.push(generated);
        }
    }
    let mut html = String::new();
    for link in &primary {
        html.push_str(&link_html(link, cx));
    }
    if !more.is_empty() {
        html.push_str(&more_links(&more, cx));
    }
    // CommunitySection#hideSectionHeader
    Ok(section_html(slug, None, &html, cx))
}

/// lib/sidebar/section-link.js: a link to its stored URL.
fn plain_link(link: &Value) -> Link {
    let name = link["name"].as_str().unwrap_or("");
    Link {
        link_name: Some(name.to_string()),
        attributes: " data-sidebar-custom-link=\"true\"".into(),
        plain: true,
        href: link["value"].as_str().unwrap_or("").to_string(),
        content: escape(name),
        prefix: Prefix::Icon {
            name: link["icon"].as_str().unwrap_or("link").to_string(),
            color: None,
        },
        ..Default::default()
    }
}

/// CommunitySection#generateLink: the special links of SPECIAL_LINKS_MAP by
/// their URL, else a plain one; None when the link is not for anonymous
/// visitors (shouldDisplay).
fn community_link(link: &Value, cx: &Context) -> Result<Option<Link>, SettingError> {
    let value = link["value"].as_str().unwrap_or("");
    let overridden_name = link["name"].as_str().unwrap_or("");
    let overridden_icon = link["icon"].as_str().filter(|s| !s.is_empty());
    let base = cx.base_path;
    let links_key = "sidebar.sections.community.links";
    let text = |name: &str| {
        cx.t_or(
            &format!("{links_key}.{}.content", name.to_lowercase()),
            name,
        )
    };
    let title = |key: &str| Some(cx.t(&format!("{links_key}.{key}.title")));

    // (name, href, title, text, default icon, plain href)
    let (name, href, title, content, default_icon, plain) = match value {
        "/latest" => (
            "everything",
            format!("{base}/latest"),
            title("topics"),
            text(overridden_name),
            "layer-group",
            false,
        ),
        "/about" => (
            "about",
            format!("{base}/about"),
            title("about"),
            text(overridden_name),
            "circle-info",
            false,
        ),
        "/u" => {
            if !cx.flag("enable_user_directory")? || cx.flag("hide_user_profiles_from_public")? {
                return Ok(None);
            }
            (
                "users",
                format!("{base}/u"),
                title("users"),
                text(overridden_name),
                "users",
                false,
            )
        }
        "/faq" => {
            let faq_url = cx.settings.get("faq_url")?.to_s().to_string();
            let rename = cx.flag("rename_faq_to_guidelines")? && faq_url.is_empty();
            let (name, title_key, text_name) = if rename {
                ("guidelines", "guidelines", "Guidelines")
            } else {
                ("faq", "faq", overridden_name)
            };
            let (href, plain) = if !faq_url.is_empty() {
                (faq_url, true)
            } else {
                (format!("{base}/{name}"), false)
            };
            (
                name,
                href,
                title(title_key),
                text(text_name),
                "circle-question",
                plain,
            )
        }
        "/badges" => {
            if !cx.flag("enable_badges")? {
                return Ok(None);
            }
            (
                "badges",
                format!("{base}/badges"),
                title("badges"),
                text(overridden_name),
                "certificate",
                false,
            )
        }
        "/filter" => (
            "filter",
            format!("{base}/filter"),
            title("filter"),
            text(overridden_name),
            "filter",
            false,
        ),
        "/g" => {
            if !cx.flag("enable_group_directory")? {
                return Ok(None);
            }
            (
                "groups",
                format!("{base}/g"),
                title("groups"),
                text(overridden_name),
                "user-group",
                false,
            )
        }
        // A member's links: my posts, my messages, review, admin, invite.
        "/my/activity" | "/my/messages" | "/review" | "/admin" | "/new-invite" => {
            return Ok(None);
        }
        _ => return Ok(Some(plain_link(link))),
    };
    Ok(Some(Link {
        link_name: Some(name.to_string()),
        attributes: " data-sidebar-custom-link=\"true\"".into(),
        plain,
        href,
        title,
        content: escape(&content),
        prefix: Prefix::Icon {
            name: overridden_icon.unwrap_or(default_icon).to_string(),
            color: None,
        },
        active: name == "everything" && *cx.active == Active::Discovery,
    }))
}

/// MoreSectionLinks: the secondary links behind a More button, in an
/// inline DMenu that sidebar.js opens.
fn more_links(links: &[Link], cx: &Context) -> String {
    let mut items = String::new();
    for link in links {
        let mut link_html = link_html(link, cx);
        link_html = link_html.replacen(
            "<li class=\"sidebar-section-link-wrapper\"",
            "<li class=\"sidebar-section-link-wrapper dropdown-menu__item\"",
            1,
        );
        // MoreSectionLink passes no ...attributes beyond its class.
        link_html = link_html.replacen(" data-sidebar-custom-link=\"true\"", "", 1);
        items.push_str(&link_html);
    }
    format!(
        "<li class=\"sidebar-section-link-wrapper\"><button aria-expanded=\"false\" class=\"fk-d-menu__trigger sidebar-more-section-trigger sidebar-section-link sidebar-more-section-links-details-summary sidebar-row --link-button\" data-identifier=\"sidebar-more-section\" data-trigger=\"\" type=\"button\">{}<span class=\"sidebar-section-link-content-text\">{}</span></button><div class=\"fk-d-menu sidebar-more-section-content\" data-content=\"\" data-identifier=\"sidebar-more-section\" role=\"dialog\" hidden><div class=\"fk-d-menu__inner-content\"><ul class=\"dropdown-menu\">{items}</ul></div></div></li>",
        prefix_html(
            &Prefix::Icon {
                name: "ellipsis-vertical".into(),
                color: None
            },
            cx
        ),
        escape(&cx.t("sidebar.more"))
    )
}

/// `Category.slugFor`: the parent's slug path first, three deep.
fn slug_for(categories: &[Value], category: &Value, depth: u32) -> String {
    let mut result = String::new();
    if depth > 1
        && let Some(parent) = category["parent_category_id"]
            .as_i64()
            .and_then(|id| categories.iter().find(|c| c["id"].as_i64() == Some(id)))
    {
        result = slug_for(categories, parent, depth - 1) + "/";
    }
    match category["slug"].as_str().map(str::trim) {
        Some(slug) if !slug.is_empty() => result + category["slug"].as_str().unwrap_or(""),
        _ => format!("{result}{}-category", category["id"]),
    }
}

/// `Category.sortCategories`: each parent followed by its children.
fn sort_categories(categories: &[Value]) -> Vec<Value> {
    fn add(out: &mut Vec<Value>, all: &[Value], parent: Option<i64>) {
        for c in all
            .iter()
            .filter(|c| c["parent_category_id"].as_i64() == parent)
        {
            out.push(c.clone());
            if let Some(id) = c["id"].as_i64() {
                add(out, all, Some(id));
            }
        }
    }
    let mut out = Vec::new();
    add(&mut out, categories, None);
    out
}

/// anonymous/categories-section
fn categories_section(site: &Value, cx: &Context) -> Result<String, SettingError> {
    let all: Vec<Value> = site["categories"].as_array().cloned().unwrap_or_default();
    let uncategorized = site["uncategorized_category_id"].as_i64();
    let allow_uncategorized = cx.flag("allow_uncategorized_topics")?;
    let can_display = |c: &Value| allow_uncategorized || c["id"].as_i64() != uncategorized;
    let fixed_positions = cx.flag("fixed_category_positions")?;

    let default_ids: Vec<i64> = cx
        .settings
        .get("default_navigation_menu_categories")?
        .to_s()
        .split('|')
        .filter_map(|id| id.trim().parse().ok())
        .collect();

    let shown: Vec<Value> = if default_ids.is_empty() {
        // topSiteCategories over Site#categoriesList.
        let list = if fixed_positions {
            all.clone()
        } else {
            // categoriesByCount: a stable sort by topic_count, descending.
            let mut by_count = all.clone();
            by_count.sort_by_key(|c| std::cmp::Reverse(c["topic_count"].as_i64().unwrap_or(0)));
            sort_categories(&by_count)
        };
        list.into_iter()
            .filter(|c| c["parent_category_id"].is_null() && can_display(c))
            .take(TOP_SITE_CATEGORIES_TO_SHOW)
            .collect()
    } else {
        // sortedCategories: by name unless positions are fixed, parents
        // before children, only the default ones.
        let mut sorted = all.clone();
        if !fixed_positions {
            sorted.sort_by(|a, b| {
                let name = |c: &Value| c["name"].as_str().unwrap_or("").to_lowercase();
                name(a).cmp(&name(b))
            });
        }
        sort_categories(&sorted)
            .into_iter()
            .filter(|c| {
                c["id"].as_i64().is_some_and(|id| default_ids.contains(&id)) && can_display(c)
            })
            .collect()
    };

    let mut links = String::new();
    for category in &shown {
        let id = category["id"].as_i64().unwrap_or(0);
        let color = category["color"].as_str().unwrap_or("").to_string();
        let prefix_color = hex_color(&color);
        let prefix = match category["style_type"].as_str() {
            Some("icon") => Prefix::Icon {
                name: category["icon"].as_str().unwrap_or("").to_string(),
                color: prefix_color,
            },
            Some("emoji") => Prefix::Emoji {
                name: category["emoji"].as_str().unwrap_or("").to_string(),
                color: prefix_color,
            },
            _ => {
                let parent_color = category["parent_category_id"]
                    .as_i64()
                    .and_then(|pid| all.iter().find(|c| c["id"].as_i64() == Some(pid)))
                    .and_then(|p| p["color"].as_str())
                    .filter(|c| !c.is_empty());
                Prefix::Square {
                    colors: match parent_color {
                        Some(p) => vec![p.to_string(), color.clone()],
                        None => vec![color.clone()],
                    },
                    color: prefix_color,
                    badge: category["read_restricted"]
                        .as_bool()
                        .unwrap_or(false)
                        .then_some("lock"),
                }
            }
        };
        links.push_str(&link_html(
            &Link {
                attributes: format!(" data-category-id=\"{id}\""),
                href: format!("{}/c/{}/{id}", cx.base_path, slug_for(&all, category, 3)),
                content: escape(category["name"].as_str().unwrap_or("")),
                prefix,
                active: *cx.active == Active::Category(id as i32),
                ..Default::default()
            },
            cx,
        ));
    }
    links.push_str(&link_html(
        &Link {
            link_name: Some("all-categories".into()),
            href: format!("{}/categories", cx.base_path),
            content: escape(&cx.t("sidebar.all_categories")),
            prefix: Prefix::Icon {
                name: "list".into(),
                color: None,
            },
            active: *cx.active == Active::Categories,
            ..Default::default()
        },
        cx,
    ));
    let header = cx.t("sidebar.sections.categories.header_link_text");
    Ok(section_html("categories", Some(&header), &links, cx))
}

/// anonymous/tags-section: the default tags, else the site's top tags;
/// nothing when there are neither.
fn tags_section(site: &Value, cx: &Context) -> String {
    let defaults = site["anonymous_default_navigation_menu_tags"].as_array();
    let top = site["navigation_menu_site_top_tags"].as_array();
    let present = |tags: Option<&Vec<Value>>| tags.is_some_and(|t| !t.is_empty());
    if !present(defaults) && !present(top) {
        return String::new();
    }
    // `defaults || top`: an empty default list still wins.
    let tags = defaults.or(top).cloned().unwrap_or_default();
    let mut links = String::new();
    for tag in &tags {
        let name = tag["name"].as_str().unwrap_or("");
        links.push_str(&link_html(
            &Link {
                attributes: format!(" data-tag-name=\"{}\"", escape(name)),
                href: format!(
                    "{}/tag/{}/{}",
                    cx.base_path,
                    tag["slug"].as_str().unwrap_or(""),
                    tag["id"]
                ),
                content: escape(name),
                prefix: Prefix::Icon {
                    name: "tag".into(),
                    color: None,
                },
                active: matches!(cx.active, Active::Tag(t) if t.eq_ignore_ascii_case(name)),
                ..Default::default()
            },
            cx,
        ));
    }
    links.push_str(&link_html(
        &Link {
            link_name: Some("all-tags".into()),
            href: format!("{}/tags", cx.base_path),
            content: escape(&cx.t("sidebar.all_tags")),
            prefix: Prefix::Icon {
                name: "list".into(),
                color: None,
            },
            active: *cx.active == Active::Tags,
            ..Default::default()
        },
        cx,
    ));
    let header = cx.t("sidebar.sections.tags.header_link_text");
    section_html("tags", Some(&header), &links, cx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn categories_sort_parents_before_children() {
        let cats = vec![
            json!({"id": 3, "parent_category_id": 1}),
            json!({"id": 1}),
            json!({"id": 2}),
            json!({"id": 4, "parent_category_id": 3}),
        ];
        let ids: Vec<i64> = sort_categories(&cats)
            .iter()
            .map(|c| c["id"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, [1, 3, 4, 2]);
    }

    #[test]
    fn slugs_include_parents_and_fall_back_to_the_id() {
        let cats = vec![
            json!({"id": 4, "slug": "general"}),
            json!({"id": 34, "slug": "sub-general", "parent_category_id": 4}),
            json!({"id": 9, "slug": " "}),
        ];
        assert_eq!(slug_for(&cats, &cats[1], 3), "general/sub-general");
        assert_eq!(slug_for(&cats, &cats[2], 3), "9-category");
    }

    #[test]
    fn prefix_colors_must_be_hex() {
        assert_eq!(hex_color("25AAE2").as_deref(), Some("#25AAE2"));
        assert_eq!(hex_color("#fff").as_deref(), Some("#fff"));
        assert_eq!(hex_color("red"), None);
    }
}
