//! The sidebar, rendered as Ember's components render it
//! (components/sidebar/anonymous/* and user/*, lib/sidebar/*) from the same
//! documents they read, the Site and a member's current user: the custom
//! sections, the categories section and the tags section, then the footer.
//! The Topics, category and tag links carry the member's new and unread
//! counts (topic_tracking_report); the section header actions open modals
//! that are not ported.

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
    /// userActivity.index for the current user: My posts.
    MyPosts,
    /// Full-page chat (the chat.* routes) on no channel.
    Chat,
    /// Full-page chat on this channel.
    ChatChannel(i64),
}

impl Active {
    /// The page's `Active` for its live updates' url (`parse` reads it).
    pub fn key(&self) -> String {
        match self {
            Active::None => String::new(),
            Active::Discovery => "discovery".into(),
            Active::Category(id) => format!("category:{id}"),
            Active::Categories => "categories".into(),
            Active::Tag(name) => format!("tag:{name}"),
            Active::Tags => "tags".into(),
            Active::MyPosts => "my-posts".into(),
            Active::Chat => "chat".into(),
            Active::ChatChannel(id) => format!("chat-channel:{id}"),
        }
    }

    pub fn parse(key: &str) -> Active {
        match key.split_once(':') {
            Some(("category", id)) => id.parse().map(Active::Category).unwrap_or(Active::None),
            Some(("tag", name)) => Active::Tag(name.to_string()),
            Some(("chat-channel", id)) => {
                id.parse().map(Active::ChatChannel).unwrap_or(Active::None)
            }
            _ => match key {
                "discovery" => Active::Discovery,
                "categories" => Active::Categories,
                "tags" => Active::Tags,
                "my-posts" => Active::MyPosts,
                "chat" => Active::Chat,
                _ => Active::None,
            },
        }
    }
}

/// `TOP_SITE_CATEGORIES_TO_SHOW`
const TOP_SITE_CATEGORIES_TO_SHOW: usize = 5;

/// What a member's sidebar reads from their current user
/// (`current_user::sidebar_member`).
pub struct Member {
    pub username: String,
    pub admin: bool,
    pub staff: bool,
    pub can_review: bool,
    pub can_send_private_messages: bool,
    pub can_invite_to_forum: bool,
    pub draft_count: i64,
    pub reviewable_count: i64,
    /// user_option.sidebar_show_count_of_new_items
    pub show_count: bool,
    /// unified_new_enabled
    pub unified_new: bool,
    /// user_option.sidebar_link_to_filtered_list: a link with new or
    /// unread topics goes to that list.
    pub link_to_filtered_list: bool,
    /// Their topic tracking state, for the links' counts.
    pub tracking: crate::topic_tracking_report::Tracking,
    /// sidebar_sections, sidebar_category_ids, display_sidebar_tags and
    /// sidebar_tags, as the serializer writes them.
    pub fields: serde_json::Map<String, Value>,
}

pub struct Context<'a> {
    pub i18n: &'a I18n,
    pub settings: &'a SiteSettings,
    pub base_path: &'a str,
    pub active: &'a Active,
    /// The member, or None for an anonymous visitor.
    pub member: Option<&'a Member>,
    /// SiteSetting.emoji_set, for emoji prefixes.
    pub emoji_set: &'a str,
    /// The bundled plugins' sections (chat's), after the core ones.
    pub plugin_sections: &'a str,
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

/// Sidebar.gjs with the anonymous or the user sections, inside
/// SidebarWrapper.
pub fn render(site: &Value, cx: &Context) -> Result<String, SettingError> {
    let mut out = String::new();
    out.push_str(&format!(
        "<div class=\"sidebar-wrapper\"><nav aria-label=\"{}\" class=\"sidebar-container\" id=\"d-sidebar\"><div class=\"sidebar-sections{}\">",
        escape(&cx.t("sidebar.title")),
        if cx.member.is_some() {
            ""
        } else {
            " sidebar-sections-anonymous"
        }
    ));
    out.push_str("<div class=\"sidebar-custom-sections\">");
    let sections = match cx.member {
        Some(m) => m.fields.get("sidebar_sections").unwrap_or(&Value::Null),
        None => &site["anonymous_sidebar_sections"],
    };
    for section in sections.as_array().into_iter().flatten() {
        out.push_str(&custom_section(section, cx)?);
    }
    out.push_str("</div>");
    out.push_str(&categories_section(site, cx)?);
    // The user sections show tags when the member has some to browse.
    let show_tags = match cx.member {
        Some(m) => m.fields.get("display_sidebar_tags") == Some(&Value::Bool(true)),
        None => cx.flag("tagging_enabled")?,
    };
    if show_tags {
        out.push_str(&tags_section(site, cx)?);
    }
    out.push_str(cx.plugin_sections);
    out.push_str("</div>");
    // The footer's only anonymous action, keyboard shortcuts, opens a modal
    // that is not ported, so its bar is empty.
    out.push_str("<div class=\"sidebar-footer-wrapper\"><div class=\"sidebar-footer-container\"><div class=\"sidebar-footer-actions\"></div></div></div>");
    out.push_str("</nav></div>");
    Ok(out)
}

/// The member's links that carry counts (Topics, their categories and
/// tags), each with an out-of-band swap that replaces it on the page: the
/// live updates' sidebar.
pub fn tracked_links(site: &Value, cx: &Context) -> Result<Vec<String>, SettingError> {
    let Some(m) = cx.member else {
        return Ok(Vec::new());
    };
    let oob = |link: &Link, selector: String| {
        link_html(link, cx).replacen(
            "<li ",
            &format!("<li hx-swap-oob=\"outerHTML:{}\" ", attr(&selector)),
            1,
        )
    };
    let mut out = Vec::new();
    let sections = m.fields.get("sidebar_sections").and_then(Value::as_array);
    let everything = sections
        .into_iter()
        .flatten()
        .filter(|s| s["section_type"] == "community")
        .flat_map(|s| s["links"].as_array().cloned().unwrap_or_default())
        .find(|l| l["value"] == "/latest");
    if let Some(link) = everything
        && let Some(link) = community_link(&link, cx)?
    {
        out.push(oob(
            &link,
            "#d-sidebar li[data-list-item-name='everything']".into(),
        ));
    }
    for (id, link) in category_links(site, cx)? {
        out.push(oob(
            &link,
            format!("#sidebar-section-content-categories > li[data-category-id='{id}']"),
        ));
    }
    if m.fields.get("display_sidebar_tags") == Some(&Value::Bool(true))
        && let Some(links) = tag_links(site, cx)?
    {
        for (name, link) in links {
            let name = name.replace('\\', "\\\\").replace('\'', "\\'");
            out.push(oob(
                &link,
                format!("#sidebar-section-content-tags > li[data-tag-name='{name}']"),
            ));
        }
    }
    Ok(out)
}

/// A rendered link (SectionLink's arguments).
#[derive(Default)]
pub(crate) struct Link {
    /// `@linkName`: data-list-item-name and data-link-name.
    pub(crate) link_name: Option<String>,
    /// Extra attributes on the li (`...attributes`).
    pub(crate) attributes: String,
    /// The plain `<a>` of an `@href` link, rather than a LinkTo.
    pub(crate) plain: bool,
    pub(crate) href: String,
    pub(crate) title: Option<String>,
    pub(crate) content: String,
    pub(crate) prefix: Prefix,
    /// `@prefixBadge`: an icon over the prefix (a restricted category's lock).
    pub(crate) prefix_badge: Option<&'static str>,
    pub(crate) active: bool,
    /// `@badgeText`
    pub(crate) badge: Option<String>,
    /// An unread suffix icon (`@suffixType` icon, `@suffixCSSClass` unread).
    pub(crate) suffix: Option<&'static str>,
}

#[derive(Default)]
pub(crate) enum Prefix {
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
    },
}

/// What a link counts: the countable that is active (the first with
/// topics) and its count.
#[derive(Clone, Copy)]
enum Countable {
    Unread(i64),
    New(i64),
    /// unified new: the new and the unread together.
    NewAndUnread(i64),
}

/// EverythingSectionLink and TagSectionLink: unread first, new when there
/// is nothing unread, both together with unified new.
fn unread_then_new(m: &Member, tag: Option<i32>) -> Option<Countable> {
    use crate::topic_tracking_report::Kind;
    let unread = m.tracking.count(Kind::Unread, None, tag);
    let new = if unread == 0 || m.unified_new {
        m.tracking.count(Kind::New, None, tag)
    } else {
        0
    };
    if m.unified_new && unread + new > 0 {
        Some(Countable::NewAndUnread(unread + new))
    } else if unread > 0 {
        Some(Countable::Unread(unread))
    } else if new > 0 {
        Some(Countable::New(new))
    } else {
        None
    }
}

/// CategorySectionLink's countables: new and unread together with unified
/// new, else unread, then new.
fn category_countable(m: &Member, category_id: i32) -> Option<Countable> {
    use crate::topic_tracking_report::Kind;
    let count = |kind| m.tracking.count(kind, Some(category_id), None);
    if m.unified_new {
        let n = count(Kind::NewAndUnread);
        return (n > 0).then_some(Countable::NewAndUnread(n));
    }
    let unread = count(Kind::Unread);
    if unread > 0 {
        return Some(Countable::Unread(unread));
    }
    let new = count(Kind::New);
    (new > 0).then_some(Countable::New(new))
}

/// The countable on the link: the count as its badge
/// (sidebar_show_count_of_new_items) or else a dot, and with
/// sidebar_link_to_filtered_list its href to the list, `filtered` being
/// the path the list's name is appended to.
fn show_countable(link: &mut Link, cx: &Context, countable: Option<Countable>, filtered: &str) {
    let (Some(m), Some(countable)) = (cx.member, countable) else {
        return;
    };
    if m.show_count {
        link.badge = match countable {
            Countable::NewAndUnread(n) => Some(n.to_string()),
            Countable::Unread(n) => cx.i18n.t_count("js.sidebar.unread_count", n, &[]),
            Countable::New(n) => cx.i18n.t_count("js.sidebar.new_count", n, &[]),
        };
    } else {
        link.suffix = Some("circle");
    }
    if m.link_to_filtered_list {
        let list = match countable {
            Countable::Unread(_) => "unread",
            Countable::New(_) | Countable::NewAndUnread(_) => "new",
        };
        link.href = format!("{filtered}/{list}");
    }
}

pub(crate) fn link_html(link: &Link, cx: &Context) -> String {
    link_html_with(link, &LinkExtra::default(), cx)
}

/// What a plugin's links add: classes on the `<a>`, a suffix component, and
/// a hover button (`@hoverValue`, `@hoverTitle`).
#[derive(Default)]
pub(crate) struct LinkExtra {
    pub(crate) classes: String,
    pub(crate) suffix_html: String,
    pub(crate) hover: Option<(&'static str, String)>,
}

/// SectionLink.gjs
pub(crate) fn link_html_with(link: &Link, extra: &LinkExtra, cx: &Context) -> String {
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
    let mut a = if extra.classes.is_empty() {
        format!("<a class=\"{class}\"")
    } else {
        format!("<a class=\"{class} {}\"", escape(&extra.classes))
    };
    if let Some(title) = &link.title {
        a.push_str(&format!(" title=\"{}\"", escape(title)));
    }
    if let Some(name) = &link.link_name {
        a.push_str(&format!(" data-link-name=\"{}\"", escape(name)));
    }
    a.push_str(&format!(" href=\"{}\"", attr(&link.href)));
    if link.plain {
        a.push_str(" rel=\"noopener noreferrer\" target=\"_self\"");
    }
    a.push('>');
    let badge = link
        .badge
        .as_ref()
        .map(|b| {
            format!(
                "<span class=\"sidebar-section-link-content-badge\">{}</span>",
                escape(b)
            )
        })
        .unwrap_or_default();
    let hover = extra
        .hover
        .as_ref()
        .map(|(name, title)| {
            format!(
                "<span class=\"sidebar-section-link-hover\"><button aria-label=\"{t}\" class=\"sidebar-section-hover-button btn-flat\" title=\"{t}\" type=\"button\">{}</button></span>",
                icon(name, Some("hover-icon")),
                t = escape(title)
            )
        })
        .unwrap_or_default();
    let suffix = link
        .suffix
        .map(|s| {
            format!(
                "<span class=\"sidebar-section-link-suffix icon unread\">{}</span>",
                icon(s, None)
            )
        })
        .unwrap_or_default();
    format!(
        "{li}{a}{}<span class=\"sidebar-section-link-content-text\">{}</span>{badge}{}{suffix}{}</a></li>",
        prefix_html(&link.prefix, link.prefix_badge, cx),
        link.content,
        extra.suffix_html,
        hover
    )
}

/// An attribute value escaped as the DOM serializes it: a URL keeps its
/// `=` and `'`, which escape() would write as references.
fn attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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
pub(crate) fn prefix_html(prefix: &Prefix, badge: Option<&str>, cx: &Context) -> String {
    let badge = badge
        .map(|b| icon(b, Some("prefix-badge")))
        .unwrap_or_default();
    match prefix {
        Prefix::None => String::new(),
        Prefix::Icon { name, color } => format!(
            "<span class=\"sidebar-section-link-prefix icon\"{}>{}{badge}</span>",
            style(color),
            crate::post_view::d_icon(name, Some("prefix-icon"))
        ),
        Prefix::Emoji { name, color } => format!(
            "<span class=\"sidebar-section-link-prefix emoji\"{}>{}{badge}</span>",
            style(color),
            emoji_html(name, cx)
        ),
        Prefix::Square { colors, color } => {
            let mut stops: Vec<String> = colors
                .iter()
                .filter_map(|c| hex_color(c).map(|c| format!("{c} 50%")))
                .collect();
            if stops.len() == 1 {
                stops.push(stops[0].clone());
            }
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
    let set = cx.emoji_set;
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
    section_html_with(name, header, "", links, cx)
}

/// Section.gjs with buttons after the header (a plugin section's inline
/// actions).
pub(crate) fn section_html_with(
    name: &str,
    header: Option<&str>,
    header_buttons: &str,
    links: &str,
    cx: &Context,
) -> String {
    let mut out = format!(
        "<div class=\"sidebar-section sidebar-section-wrapper sidebar-section--expanded\" data-section-name=\"{}\">",
        escape(name)
    );
    let content_id = format!("sidebar-section-content-{}", escape(name));
    if let Some(header) = header {
        out.push_str(&format!(
            "<div class=\"sidebar-section-header-wrapper sidebar-row\"><button aria-controls=\"{content_id}\" aria-expanded=\"true\" class=\"btn no-text sidebar-section-header sidebar-section-header-collapsable btn-transparent\" title=\"{}\" type=\"button\"><span class=\"sidebar-section-header-caret\">{}</span><span class=\"sidebar-section-header-text\">{}</span></button>{header_buttons}</div>",
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
        "/my/activity" | "/my/messages" | "/review" | "/admin" | "/new-invite" => {
            return Ok(member_link(value, overridden_name, overridden_icon, cx));
        }
        _ => return Ok(Some(plain_link(link))),
    };
    let mut link = Link {
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
        ..Default::default()
    };
    if name == "everything"
        && let Some(m) = cx.member
    {
        show_countable(&mut link, cx, unread_then_new(m, None), base);
    }
    Ok(Some(link))
}

/// lib/sidebar/user/community-section: a member's own links, None for an
/// anonymous visitor or a member they are not for (shouldDisplay).
fn member_link(
    value: &str,
    overridden_name: &str,
    overridden_icon: Option<&str>,
    cx: &Context,
) -> Option<Link> {
    let m = cx.member?;
    let base = cx.base_path;
    let links_key = "sidebar.sections.community.links";
    // The user route's username, lowercased.
    let user_path = format!("{base}/u/{}", escape(&m.username.to_lowercase()));
    // `overriddenName.toLowerCase().replace(" ", "_")`
    let key_name = overridden_name.to_lowercase().replacen(' ', "_", 1);
    let text = cx.t_or(&format!("{links_key}.{key_name}.content"), overridden_name);
    let title = |key: &str| Some(cx.t(&format!("{links_key}.{key}")));
    let icon = |default: &str| Prefix::Icon {
        name: overridden_icon.unwrap_or(default).to_string(),
        color: None,
    };
    let mut link = Link {
        attributes: " data-sidebar-custom-link=\"true\"".into(),
        ..Default::default()
    };
    match value {
        "/my/activity" => {
            let has_draft = m.draft_count > 0;
            link.link_name = Some("my-posts".into());
            // currentWhen: the activity index (and drafts, not drawn yet).
            link.active = *cx.active == Active::MyPosts;
            if has_draft {
                link.href = format!("{user_path}/activity/drafts");
                link.title = title("my_posts.title_drafts");
                link.content = escape(&if m.unified_new {
                    cx.t(&format!("{links_key}.my_posts.content_drafts"))
                } else {
                    text
                });
                link.prefix = if m.unified_new {
                    Prefix::Icon {
                        name: "pencil".into(),
                        color: None,
                    }
                } else {
                    icon("user")
                };
                if m.show_count {
                    link.badge = Some(if m.unified_new {
                        m.draft_count.to_string()
                    } else {
                        cx.i18n
                            .t_count(
                                &format!("js.{links_key}.my_posts.draft_count"),
                                m.draft_count,
                                &[],
                            )
                            .unwrap_or_default()
                    });
                } else {
                    link.suffix = Some("circle");
                }
            } else {
                link.href = format!("{user_path}/activity");
                link.title = title("my_posts.title");
                link.content = escape(&text);
                link.prefix = icon("user");
            }
        }
        "/my/messages" => {
            if !m.can_send_private_messages {
                return None;
            }
            link.link_name = Some("my-messages".into());
            link.href = format!("{user_path}/messages");
            link.title = title("my_messages.title");
            link.content = escape(&text);
            link.prefix = icon("link");
        }
        "/review" => {
            if !m.can_review {
                return None;
            }
            link.link_name = Some("review".into());
            link.href = format!("{base}/review");
            link.title = title("review.title");
            link.content = escape(&text);
            link.prefix = icon("flag");
            // getReviewBadgeText
            if m.reviewable_count > 0 {
                link.badge = cx.i18n.t_count(
                    &format!("js.{links_key}.review.pending_count"),
                    m.reviewable_count,
                    &[],
                );
            }
        }
        "/admin" => {
            if !m.staff {
                return None;
            }
            link.link_name = Some("admin".into());
            link.href = format!("{base}/admin");
            link.title = title("admin.content");
            link.content = escape(&text);
            link.prefix = icon("wrench");
        }
        "/new-invite" => {
            if !m.can_invite_to_forum {
                return None;
            }
            link.link_name = Some("invite".into());
            link.href = format!("{base}/new-invite");
            link.title = title("invite.title");
            link.content = escape(&text);
            link.prefix = icon("paper-plane");
        }
        _ => return None,
    }
    Some(link)
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
            None,
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

/// `default_navigation_menu_categories`
fn default_category_ids(cx: &Context) -> Result<Vec<i64>, SettingError> {
    Ok(cx
        .settings
        .get("default_navigation_menu_categories")?
        .to_s()
        .split('|')
        .filter_map(|id| id.trim().parse().ok())
        .collect())
}

/// The categories section's category links (anonymous/categories-section
/// and user/categories-section), by id.
fn category_links(site: &Value, cx: &Context) -> Result<Vec<(i64, Link)>, SettingError> {
    let all: Vec<Value> = site["categories"].as_array().cloned().unwrap_or_default();
    let uncategorized = site["uncategorized_category_id"].as_i64();
    let allow_uncategorized = cx.flag("allow_uncategorized_topics")?;
    let can_display = |c: &Value| allow_uncategorized || c["id"].as_i64() != uncategorized;
    let fixed_positions = cx.flag("fixed_category_positions")?;
    let default_ids = default_category_ids(cx)?;

    // topSiteCategories over Site#categoriesList.
    let top_site = || -> Vec<Value> {
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
    };
    // `categories`, and whether sortedCategories reorders them: a member's
    // own categories (else the top ones) always are; an anonymous
    // visitor's only when they are the site's defaults.
    let (categories, sort) = match cx.member {
        Some(m) => {
            let ids: Vec<i64> = m
                .fields
                .get("sidebar_category_ids")
                .and_then(Value::as_array)
                .map(|ids| ids.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default();
            if ids.is_empty() {
                (top_site(), true)
            } else {
                let found = all
                    .iter()
                    .filter(|c| c["id"].as_i64().is_some_and(|id| ids.contains(&id)))
                    .cloned()
                    .collect();
                (found, true)
            }
        }
        None if default_ids.is_empty() => (top_site(), false),
        None => {
            let found = all
                .iter()
                .filter(|c| c["id"].as_i64().is_some_and(|id| default_ids.contains(&id)))
                .cloned()
                .collect();
            (found, true)
        }
    };
    let shown: Vec<Value> = if sort {
        // sortedCategories: by name unless positions are fixed, parents
        // before children, only the chosen ones.
        let chosen: Vec<i64> = categories.iter().filter_map(|c| c["id"].as_i64()).collect();
        let mut sorted = all.clone();
        if !fixed_positions {
            sorted.sort_by(|a, b| {
                let name = |c: &Value| c["name"].as_str().unwrap_or("").to_lowercase();
                name(a).cmp(&name(b))
            });
        }
        sort_categories(&sorted)
            .into_iter()
            .filter(|c| c["id"].as_i64().is_some_and(|id| chosen.contains(&id)) && can_display(c))
            .collect()
    } else {
        categories
    };

    let mut links = Vec::new();
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
                }
            }
        };
        let href = format!("{}/c/{}/{id}", cx.base_path, slug_for(&all, category, 3));
        let mut link = Link {
            attributes: format!(" data-category-id=\"{id}\""),
            href: href.clone(),
            content: escape(category["name"].as_str().unwrap_or("")),
            prefix,
            // `category.restricted` is the lock icon.
            prefix_badge: category["read_restricted"]
                .as_bool()
                .unwrap_or(false)
                .then_some("lock"),
            active: *cx.active == Active::Category(id as i32),
            ..Default::default()
        };
        if let Some(m) = cx.member {
            let countable = category_countable(m, id as i32);
            show_countable(&mut link, cx, countable, &format!("{href}/l"));
        }
        links.push((id, link));
    }
    Ok(links)
}

fn categories_section(site: &Value, cx: &Context) -> Result<String, SettingError> {
    let mut links: String = category_links(site, cx)?
        .iter()
        .map(|(_, link)| link_html(link, cx))
        .collect();
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
    if cx.member.is_some_and(|m| m.admin) && default_category_ids(cx)?.is_empty() {
        links.push_str(&configure_defaults_link("categories", cx));
    }
    let header = cx.t("sidebar.sections.categories.header_link_text");
    Ok(section_html("categories", Some(&header), &links, cx))
}

/// The admin's link to set the site's default categories or tags
/// (`kind`), shown while there are none.
fn configure_defaults_link(kind: &str, cx: &Context) -> String {
    let name = format!("configure-default-navigation-menu-{kind}");
    link_html(
        &Link {
            href: format!(
                "{}/admin/site_settings/category/sidebar?filter=default_navigation_menu_{kind}",
                cx.base_path
            ),
            content: escape(&cx.t(&format!("sidebar.sections.{kind}.configure_defaults"))),
            prefix: Prefix::Icon {
                name: "wrench".into(),
                color: None,
            },
            link_name: Some(name),
            ..Default::default()
        },
        cx,
    )
}

/// anonymous/tags-section and user/tags-section. An anonymous visitor gets
/// the default tags, else the site's top tags, and no section when there
/// are neither; a member gets their own tags, else the top ones, and the
/// section either way.
fn tag_links(site: &Value, cx: &Context) -> Result<Option<Vec<(String, Link)>>, SettingError> {
    let top = site["navigation_menu_site_top_tags"].as_array();
    let tags = match cx.member {
        Some(m) => match m.fields.get("sidebar_tags").and_then(Value::as_array) {
            Some(own) if !own.is_empty() => own.clone(),
            _ => top.cloned().unwrap_or_default(),
        },
        None => {
            let defaults = site["anonymous_default_navigation_menu_tags"].as_array();
            let present = |tags: Option<&Vec<Value>>| tags.is_some_and(|t| !t.is_empty());
            if !present(defaults) && !present(top) {
                return Ok(None);
            }
            // `defaults || top`: an empty default list still wins.
            defaults.or(top).cloned().unwrap_or_default()
        }
    };
    let mut links = Vec::new();
    for tag in &tags {
        let name = tag["name"].as_str().unwrap_or("");
        // PMTagSectionLink: a tag only on messages links to the member's
        // messages with it.
        let pm_only = tag["pm_only"] == Value::Bool(true);
        let href = match cx.member {
            Some(m) if pm_only => format!(
                "{}/u/{}/messages/tags/{}",
                cx.base_path,
                m.username.to_lowercase(),
                name
            ),
            _ => format!(
                "{}/tag/{}/{}",
                cx.base_path,
                tag["slug"].as_str().unwrap_or(""),
                tag["id"]
            ),
        };
        let mut link = Link {
            attributes: format!(" data-tag-name=\"{}\"", escape(name)),
            href: href.clone(),
            content: escape(name),
            prefix: Prefix::Icon {
                name: "tag".into(),
                color: None,
            },
            active: matches!(cx.active, Active::Tag(t) if t.eq_ignore_ascii_case(name)),
            ..Default::default()
        };
        // TagSectionLink counts; PMTagSectionLink does not.
        if let Some(m) = cx.member
            && !pm_only
        {
            let countable = tag["id"]
                .as_i64()
                .and_then(|id| unread_then_new(m, Some(id as i32)));
            show_countable(&mut link, cx, countable, &format!("{href}/l"));
        }
        links.push((name.to_string(), link));
    }
    Ok(Some(links))
}

/// The tags section, when there is one (tag_links).
fn tags_section(site: &Value, cx: &Context) -> Result<String, SettingError> {
    let Some(tag_links) = tag_links(site, cx)? else {
        return Ok(String::new());
    };
    let mut links: String = tag_links
        .iter()
        .map(|(_, link)| link_html(link, cx))
        .collect();
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
    if cx.member.is_some_and(|m| m.admin)
        && cx
            .settings
            .get("default_navigation_menu_tags")?
            .to_s()
            .is_empty()
    {
        links.push_str(&configure_defaults_link("tags", cx));
    }
    let header = cx.t("sidebar.sections.tags.header_link_text");
    Ok(section_html("tags", Some(&header), &links, cx))
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
