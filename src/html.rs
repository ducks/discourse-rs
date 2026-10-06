//! Server-rendered pages for anonymous readers, built from the same
//! documents the JSON endpoints return. The structure follows Discourse's
//! crawler views (app/views/list/list.erb, topics/show.html.erb); the
//! styling is ours.

use askama::Template;
use chrono::NaiveDateTime;
use serde_json::Value;
use sqlx::PgConnection;

use crate::i18n::I18n;
use crate::site_settings::{SettingError, SiteSettings};

#[derive(Debug)]
pub enum HtmlError {
    Db(sqlx::Error),
    Setting(SettingError),
    Template(askama::Error),
}

impl std::fmt::Display for HtmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HtmlError::Db(e) => write!(f, "rendering page: {e}"),
            HtmlError::Setting(e) => e.fmt(f),
            HtmlError::Template(e) => write!(f, "rendering template: {e}"),
        }
    }
}

impl std::error::Error for HtmlError {}

impl From<sqlx::Error> for HtmlError {
    fn from(e: sqlx::Error) -> Self {
        HtmlError::Db(e)
    }
}

impl From<SettingError> for HtmlError {
    fn from(e: SettingError) -> Self {
        HtmlError::Setting(e)
    }
}

impl From<askama::Error> for HtmlError {
    fn from(e: askama::Error) -> Self {
        HtmlError::Template(e)
    }
}

/// What the shell shows a logged-in viewer: their name, and the CSRF
/// token the logout form and the client need (anonymous pages carry
/// none, as they may be cached).
#[derive(Clone, Default)]
pub struct Viewer {
    pub username: String,
    pub csrf_token: String,
    /// The header's avatar (48px).
    pub avatar_url: String,
}

/// The viewer block for a page, plus the `_forum_session` cookie to set
/// when minting the CSRF token created the session.
#[derive(Clone, Default)]
pub struct ViewerState {
    pub viewer: Option<Viewer>,
    pub set_cookie: Option<String>,
}

/// The headers a logged-in page carries: the session cookie when it was
/// just created, `X-Discourse-Username`, and no caching (anonymous pages
/// may be cached, these never are).
pub fn with_viewer_headers(
    mut response: axum::response::Response,
    viewer: &ViewerState,
) -> axum::response::Response {
    use axum::http::{HeaderValue, header};
    if let Some(cookie) = &viewer.set_cookie
        && let Ok(value) = HeaderValue::from_str(cookie)
    {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    if let Some(v) = &viewer.viewer {
        if let Ok(value) = HeaderValue::from_str(&v.username) {
            response.headers_mut().insert("x-discourse-username", value);
        }
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache, no-store"),
        );
    }
    response
}
/// What every page's layout needs.
pub struct Site {
    pub site_title: String,
    pub site_description: String,
    pub lang: String,
    pub base_path: String,
    pub viewer: Option<Viewer>,
    /// Where the page's live updates start (pg-bus position as text), taken
    /// before its data was read; empty for no live updates.
    pub bus_position: String,
    /// What the layout's header and body need beyond the settings.
    pub chrome: Chrome,
}

impl Site {
    pub fn from_settings(settings: &SiteSettings, base_path: &str) -> Result<Site, SettingError> {
        Ok(Site {
            site_title: settings.get("title")?.to_s(),
            site_description: settings.get("site_description")?.to_s(),
            lang: settings.get("default_locale")?.to_s().replace('_', "-"),
            base_path: base_path.to_string(),
            viewer: None,
            bus_position: String::new(),
            chrome: Chrome::default(),
        })
    }
}

/// The page chrome around every page: the header and the body's classes.
#[derive(Clone, Default)]
pub struct Chrome {
    /// The header's logo (`SiteSetting.site_logo_url`), empty for the site
    /// title as text.
    pub logo_url: String,
    /// The body's classes: `uc-<name>` for each enabled upcoming change
    /// with CSS (ApplicationController#upcomingChangeBodyClasses).
    pub body_classes: String,
    /// `canSignUp`: the header shows a Sign Up button.
    pub can_signup: bool,
}

impl Site {
    /// Fills the page chrome. Every enabled upcoming change counts as
    /// enabled for the viewer; changes enabled for some groups only are
    /// not told apart yet, and read-only mode is not ported.
    pub async fn load_chrome(
        &mut self,
        state: &crate::AppState,
        settings: &SiteSettings,
    ) -> Result<(), crate::AppError> {
        let mut conn = state.pool.acquire().await?;
        let urls = crate::url::Urls {
            config: &state.config,
            settings,
        };
        self.chrome.logo_url = crate::site_icons::site_url(&mut conn, &urls, "logo").await?;
        let mut classes = Vec::new();
        for name in state.site_setting_defs.upcoming_changes_with_css() {
            if settings.get(name)?.truthy() {
                classes.push(format!("uc-{}", name.replace('_', "-")));
            }
        }
        self.chrome.body_classes = classes.join(" ");
        self.chrome.can_signup = !settings.get("invite_only")?.truthy()
            && settings.get("allow_new_registrations")?.truthy()
            && !settings.get("enable_discourse_connect")?.truthy();
        Ok(())
    }
}

pub struct CategoryBadge {
    pub name: String,
    pub color: String,
    pub url: String,
}

pub struct TagLink {
    pub name: String,
    pub url: String,
}

pub struct PosterItem {
    pub username: String,
    pub description: String,
}

pub struct TopicItem {
    pub title_unicode: String,
    pub url: String,
    pub title: String,
    pub category: Option<CategoryBadge>,
    pub tags: Vec<TagLink>,
    pub excerpt: Option<String>,
    pub posters: Vec<PosterItem>,
    pub replies: i64,
    pub views: i64,
    pub bumped_at: String,
    pub bumped_at_iso: String,
    pub pinned: bool,
    pub closed: bool,
}

#[derive(Template)]
#[template(path = "latest.html")]
pub struct LatestPage {
    pub site_title: String,
    pub site_description: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: Crawler,
    pub viewer: Option<Viewer>,
    pub bus_position: String,
    pub chrome: Chrome,
    pub topics: Vec<TopicItem>,
    pub more_url: Option<String>,
    /// Set on category pages.
    pub heading: Option<CategoryHeading>,
    /// Set on tag pages (list.erb's tag breadcrumb).
    pub tag: Option<TagHeading>,
    /// The list kept live (`latest`, `new`, `unread`), empty for none.
    pub live_filter: String,
    /// When the page was rendered, in milliseconds (live updates count the
    /// topics bumped after it).
    pub live_since: String,
    /// The navigation pills (`top_menu`); empty where the page has none.
    pub nav: Vec<NavItem>,
}

/// A navigation pill, as NavItem renders it.
pub struct NavItem {
    /// The filter: `latest`, `new`, `hot`, `categories`...
    pub name: String,
    pub label: String,
    /// `js.filters.<name>.help`
    pub title: String,
    pub href: String,
    pub active: bool,
    /// The id of the live count span (`new-count`, `unread-count`), for a
    /// member.
    pub count_id: Option<&'static str>,
}

/// The pills of the top-level lists: `top_menu` in order, the ones that
/// need an account only for a member, `active` marked.
pub fn nav_items(
    i18n: &I18n,
    settings: &SiteSettings,
    base_path: &str,
    active: &str,
    member: bool,
) -> Result<Vec<NavItem>, SettingError> {
    const MEMBERS_ONLY: [&str; 5] = ["new", "unread", "read", "posted", "bookmarks"];
    let top_menu = settings.get("top_menu")?.to_s();
    Ok(top_menu
        .split('|')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .filter(|name| member || !MEMBERS_ONLY.contains(name))
        .map(|name| NavItem {
            name: name.to_string(),
            label: i18n
                .t(&format!("js.filters.{name}.title"))
                .unwrap_or(name)
                .to_string(),
            title: i18n
                .t(&format!("js.filters.{name}.help"))
                .unwrap_or_default()
                .to_string(),
            href: format!("{base_path}/{name}"),
            active: name == active,
            count_id: match (member, name) {
                (true, "new") => Some("new-count"),
                (true, "unread") => Some("unread-count"),
                _ => None,
            },
        })
        .collect())
}

pub struct TagHeading {
    pub name: String,
    pub url: String,
}

pub struct PostItem {
    pub number: i64,
    /// The post's id, for its actions.
    pub id: i64,
    /// Where likes are posted (`/post_actions`).
    pub actions_url: String,
    pub base_path: String,
    pub url: String,
    pub username: String,
    pub name: Option<String>,
    pub user_url: String,
    pub created_at: String,
    pub created_at_iso: String,
    pub cooked: String,
    pub likes: i64,
    pub small_action: bool,
    pub action_text: Option<String>,
    /// The viewer may like it (`actions_summary` like `can_act`).
    pub can_like: bool,
    /// The viewer liked it (`acted`).
    pub liked: bool,
    /// The viewer may take their like back (`can_undo`).
    pub can_unlike: bool,
    /// A logged-in viewer may bookmark it.
    pub can_bookmark: bool,
    /// The viewer's bookmark of it.
    pub bookmark_id: Option<i64>,
}

#[derive(Template)]
#[template(path = "topic.html")]
pub struct TopicPage {
    pub site_title: String,
    pub site_description: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: Crawler,
    pub viewer: Option<Viewer>,
    pub bus_position: String,
    pub chrome: Chrome,
    pub title: String,
    pub title_unicode: String,
    pub canonical_url: String,
    pub breadcrumbs: Vec<CategoryBadge>,
    pub tags: Vec<TagLink>,
    pub posts: Vec<PostItem>,
    pub prev_url: Option<String>,
    pub next_url: Option<String>,
    pub topic_id: i64,
    /// The last page: live updates append new posts here.
    pub live: bool,
    pub can_reply: bool,
}

/// `categories` rows the pages link to.
#[derive(sqlx::FromRow)]
pub(crate) struct CategoryRow {
    id: i32,
    name: String,
    color: String,
    slug: String,
    parent_category_id: Option<i32>,
}

pub(crate) async fn categories(conn: &mut PgConnection) -> Result<Vec<CategoryRow>, sqlx::Error> {
    sqlx::query_as("SELECT id, name, color, slug, parent_category_id FROM categories")
        .fetch_all(conn)
        .await
}

/// `Category#url`: parent slug first for subcategories.
fn category_url(base_path: &str, cats: &[CategoryRow], c: &CategoryRow) -> String {
    match c
        .parent_category_id
        .and_then(|p| cats.iter().find(|x| x.id == p))
    {
        Some(parent) => format!("{base_path}/c/{}/{}/{}", parent.slug, c.slug, c.id),
        None => format!("{base_path}/c/{}/{}", c.slug, c.id),
    }
}

pub(crate) fn badge(
    base_path: &str,
    cats: &[CategoryRow],
    id: Option<i64>,
) -> Option<CategoryBadge> {
    let c = cats.iter().find(|c| Some(i64::from(c.id)) == id)?;
    Some(CategoryBadge {
        name: c.name.clone(),
        color: c.color.clone(),
        url: category_url(base_path, cats, c),
    })
}

fn tags(base_path: &str, v: &Value) -> Vec<TagLink> {
    v.as_array()
        .map(|tags| {
            tags.iter()
                .filter_map(|t| t["name"].as_str())
                .map(|name| TagLink {
                    name: name.to_string(),
                    url: format!("{base_path}/tag/{name}"),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// "Sep 30, 2026" from a Rails ISO timestamp; the raw string if unparsable.
fn date(iso: &str) -> String {
    NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S%.3fZ")
        .map(|t| t.format("%b %-d, %Y").to_string())
        .unwrap_or_else(|_| iso.to_string())
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

/// The latest page from the /latest.json document.
pub async fn latest_page(
    conn: &mut PgConnection,
    site: Site,
    list: &Value,
) -> Result<LatestPage, HtmlError> {
    let cats = categories(conn).await?;
    let users = list["users"].as_array().cloned().unwrap_or_default();
    let username = |id: &Value| {
        users
            .iter()
            .find(|u| u["id"] == *id)
            .and_then(|u| u["username"].as_str())
            .unwrap_or("")
            .to_string()
    };
    let base = site.base_path.clone();
    let topics = list["topic_list"]["topics"]
        .as_array()
        .map(|topics| {
            topics
                .iter()
                .map(|t| TopicItem {
                    url: format!("{base}/t/{}/{}", s(&t["slug"]), t["id"]),
                    title: s(&t["title"]),
                    title_unicode: crate::emoji::gsub_emoji_to_unicode(&s(&t["title"])),
                    category: badge(&base, &cats, t["category_id"].as_i64()),
                    tags: tags(&base, &t["tags"]),
                    excerpt: t["excerpt"].as_str().map(str::to_string),
                    posters: t["posters"]
                        .as_array()
                        .map(|ps| {
                            ps.iter()
                                .map(|p| PosterItem {
                                    username: username(&p["user_id"]),
                                    description: s(&p["description"]),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    replies: t["reply_count"].as_i64().unwrap_or(0),
                    views: t["views"].as_i64().unwrap_or(0),
                    bumped_at: date(&s(&t["bumped_at"])),
                    bumped_at_iso: s(&t["bumped_at"]),
                    pinned: t["pinned"] == true,
                    closed: t["closed"] == true,
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(LatestPage {
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        site_description: site.site_description,
        lang: site.lang,
        base_path: site.base_path,
        crawler: Crawler::default(),
        topics,
        heading: None,
        tag: None,
        live_filter: String::new(),
        live_since: String::new(),
        nav: Vec::new(),
        more_url: list["topic_list"]["more_topics_url"]
            .as_str()
            .map(str::to_string),
    })
}

/// `js.action_codes.<code>` with an empty `when`, as the crawler view does.
fn action_text(i18n: &I18n, code: &str, who: &str) -> String {
    i18n.t_with(
        &format!("js.action_codes.{code}"),
        &[("when", ""), ("who", who), ("href", "")],
    )
    .map(|t| t.trim().to_string())
    .unwrap_or_else(|| code.to_string())
}

/// The topic page from the /t/:id.json document.
pub async fn topic_page(
    conn: &mut PgConnection,
    i18n: &I18n,
    site: Site,
    view: &Value,
    page: i64,
) -> Result<TopicPage, HtmlError> {
    let cats = categories(conn).await?;
    let base = site.base_path.clone();
    let slug = s(&view["slug"]);
    let id = view["id"].as_i64().unwrap_or(0);
    let topic_url = format!("{base}/t/{slug}/{id}");

    let mut breadcrumbs = Vec::new();
    if let Some(c) = cats
        .iter()
        .find(|c| Some(i64::from(c.id)) == view["category_id"].as_i64())
    {
        if let Some(parent) = c
            .parent_category_id
            .and_then(|p| cats.iter().find(|x| x.id == p))
        {
            breadcrumbs.push(CategoryBadge {
                name: parent.name.clone(),
                color: parent.color.clone(),
                url: category_url(&base, &cats, parent),
            });
        }
        breadcrumbs.push(CategoryBadge {
            name: c.name.clone(),
            color: c.color.clone(),
            url: category_url(&base, &cats, c),
        });
    }

    let member = site.viewer.is_some();
    let posts = view["post_stream"]["posts"]
        .as_array()
        .map(|posts| {
            posts
                .iter()
                .map(|p| post_item(i18n, &base, &topic_url, p, member))
                .collect()
        })
        .unwrap_or_default();

    let stream_len = view["post_stream"]["stream"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0) as i64;
    let chunk = view["chunk_size"].as_i64().unwrap_or(20);
    let last_page = ((stream_len - 1).max(0) / chunk) + 1;
    let page = page.max(1);
    let page_url = |n: i64| {
        if n <= 1 {
            topic_url.clone()
        } else {
            format!("{topic_url}?page={n}")
        }
    };

    Ok(TopicPage {
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        site_description: site.site_description,
        lang: site.lang,
        base_path: site.base_path,
        crawler: Crawler::default(),
        title: s(&view["title"]),
        title_unicode: crate::emoji::gsub_emoji_to_unicode(&s(&view["title"])),
        canonical_url: page_url(page),
        breadcrumbs,
        tags: tags(&base, &view["tags"]),
        posts,
        topic_id: id,
        // New posts land on the last page; earlier pages stay as they are.
        live: page >= last_page,
        // details.can_create_post: a logged-in viewer who may reply.
        can_reply: view["details"]["can_create_post"] == true,
        prev_url: (page > 1).then(|| page_url(page - 1)),
        next_url: (page < last_page).then(|| page_url(page + 1)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_urls() {
        assert_eq!(date("2026-09-30T07:28:29.580Z"), "Sep 30, 2026");
        assert_eq!(date("garbage"), "garbage");
        let cats = vec![
            CategoryRow {
                id: 1,
                name: "General".into(),
                color: "0088CC".into(),
                slug: "general".into(),
                parent_category_id: None,
            },
            CategoryRow {
                id: 2,
                name: "Sub".into(),
                color: "AB9364".into(),
                slug: "sub".into(),
                parent_category_id: Some(1),
            },
        ];
        assert_eq!(category_url("", &cats, &cats[0]), "/c/general/1");
        assert_eq!(category_url("/f", &cats, &cats[1]), "/f/c/general/sub/2");
        assert!(badge("", &cats, Some(9)).is_none());
    }
}

/// A subcategory entry on a category page's first page.
pub struct SubcategoryItem {
    pub name: String,
    pub url: String,
    pub description: Option<String>,
}

/// The category page header: the category (and its parent) as links,
/// plus its visible subcategories on the first page (list.erb 13-37).
pub struct CategoryHeading {
    pub name: String,
    pub url: String,
    pub parent: Option<CategoryBadge>,
    pub subcategories: Vec<SubcategoryItem>,
}

pub async fn category_heading(
    conn: &mut PgConnection,
    base_path: &str,
    category: &crate::category::Category,
    first_page: bool,
) -> Result<CategoryHeading, HtmlError> {
    let parent = match category.parent_category_id {
        Some(id) => match crate::category::Category::find(conn, id).await? {
            Some(p) => Some(CategoryBadge {
                name: p.name.clone(),
                color: p.color.clone(),
                url: p.url(conn, base_path).await?,
            }),
            None => None,
        },
        None => None,
    };
    let mut subcategories = Vec::new();
    if first_page {
        for sub in category.visible_subcategories(conn).await? {
            subcategories.push(SubcategoryItem {
                name: sub.name.clone(),
                url: sub.url(conn, base_path).await?,
                description: sub
                    .description
                    .as_deref()
                    .map(|d| crate::categories::description_plain_text(Some(d)))
                    .transpose()
                    .ok()
                    .flatten()
                    .flatten(),
            });
        }
    }
    Ok(CategoryHeading {
        name: category.name.clone(),
        url: category.url(conn, base_path).await?,
        parent,
        subcategories,
    })
}

pub struct CategoryIndexItem {
    pub name: String,
    pub url: String,
    pub color: String,
    pub description: Option<String>,
    pub topic_count: i64,
    pub subcategories: Vec<CategoryBadge>,
    pub topics: Vec<FeaturedTopic>,
}

pub struct FeaturedTopic {
    pub title: String,
    pub url: String,
    pub bumped_at: String,
    pub bumped_at_iso: String,
}

#[derive(Template)]
#[template(path = "categories.html")]
pub struct CategoriesPage {
    pub site_title: String,
    pub site_description: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: Crawler,
    pub viewer: Option<Viewer>,
    pub bus_position: String,
    pub chrome: Chrome,
    pub categories: Vec<CategoryIndexItem>,
}

/// The categories index from the /categories.json document
/// (categories/index.html.erb's table, plus featured topics).
pub async fn categories_page(
    conn: &mut PgConnection,
    _i18n: &I18n,
    site: Site,
    doc: &Value,
) -> Result<CategoriesPage, HtmlError> {
    let cats = categories(conn).await?;
    let base = site.base_path.clone();
    let items = doc["category_list"]["categories"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|c| {
                    let subcategories = c["subcategory_ids"]
                        .as_array()
                        .map(|ids| {
                            ids.iter()
                                .filter_map(|id| badge(&base, &cats, id.as_i64()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let row = cats
                        .iter()
                        .find(|x| Some(i64::from(x.id)) == c["id"].as_i64());
                    CategoryIndexItem {
                        name: s(&c["name"]),
                        url: row
                            .map(|r| category_url(&base, &cats, r))
                            .unwrap_or_default(),
                        color: s(&c["color"]),
                        description: c["description"].as_str().map(str::to_string),
                        topic_count: c["topic_count"].as_i64().unwrap_or(0),
                        subcategories,
                        topics: c["topics"]
                            .as_array()
                            .map(|ts| {
                                ts.iter()
                                    .map(|t| FeaturedTopic {
                                        title: crate::emoji::gsub_emoji_to_unicode(&s(&t["title"])),
                                        url: format!("{base}/t/{}/{}", s(&t["slug"]), t["id"]),
                                        bumped_at: date(&s(&t["bumped_at"])),
                                        bumped_at_iso: s(&t["bumped_at"]),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(CategoriesPage {
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        site_description: site.site_description,
        lang: site.lang,
        base_path: site.base_path,
        crawler: Crawler::default(),
        categories: items,
    })
}

#[derive(Template)]
#[template(path = "tags.html")]
pub struct TagsPage {
    pub site_title: String,
    pub site_description: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: Crawler,
    pub viewer: Option<Viewer>,
    pub bus_position: String,
    pub chrome: Chrome,
    pub groups: Vec<TagGroupItem>,
}

pub struct TagGroupItem {
    pub name: Option<String>,
    pub tags: Vec<TagBoxItem>,
}

pub struct TagBoxItem {
    pub name: String,
    pub url: String,
    pub count: i64,
}

/// The tags index from the /tags.json document (tags/index.html.erb):
/// each category's tags, then the rest under "Other Tags".
pub fn tags_page(
    i18n: &I18n,
    site: Site,
    doc: &Value,
    category_names: &[(i64, String)],
) -> TagsPage {
    let base = site.base_path.clone();
    let boxes = |tags: &Value| -> Vec<TagBoxItem> {
        tags.as_array()
            .map(|list| {
                list.iter()
                    .map(|t| TagBoxItem {
                        name: s(&t["text"]),
                        url: format!("{base}/tag/{}", t["id"]),
                        count: t["count"].as_i64().unwrap_or(0),
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut groups: Vec<TagGroupItem> = doc["extras"]["categories"]
        .as_array()
        .map(|cats| {
            cats.iter()
                .map(|c| TagGroupItem {
                    name: category_names
                        .iter()
                        .find(|(id, _)| Some(*id) == c["id"].as_i64())
                        .map(|(_, name)| name.clone()),
                    tags: boxes(&c["tags"]),
                })
                .collect()
        })
        .unwrap_or_default();
    let other = boxes(&doc["tags"]);
    if !other.is_empty() {
        groups.push(TagGroupItem {
            name: Some(
                i18n.t("js.tagging.other_tags")
                    .unwrap_or("Other Tags")
                    .to_string(),
            ),
            tags: other,
        });
    }
    TagsPage {
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        site_description: site.site_description,
        lang: site.lang,
        base_path: site.base_path,
        crawler: Crawler::default(),
        groups,
    }
}

/// What the layout's `<head>` says to crawlers: the canonical link, the
/// description meta and `crawlable_meta_data`'s OpenGraph/Twitter tags.
#[derive(Debug, Clone, Default)]
pub struct Crawler {
    /// Absolute; empty when the page sets none.
    pub canonical: String,
    /// `<meta name="description">` (`description_content`)
    pub description: String,
    /// og:title; empty when the page emits no crawlable_meta_data.
    pub title: String,
    /// og:description
    pub og_description: String,
    /// og:image, absolute
    pub image: Option<String>,
    /// og:url: the request's own URL
    pub url: String,
    /// `add_noindex_header_to_non_canonical`: the canonical differs from
    /// the request URL.
    pub noindex: bool,
}

impl Crawler {
    /// `default_canonical`: the request path plus its `page` param.
    pub fn default_canonical(base_url_no_prefix: &str, path: &str, query: Option<&str>) -> String {
        let mut canonical = format!("{base_url_no_prefix}{path}");
        if let Some(q) = query {
            let page: Vec<&str> = q.split('&').filter(|p| p.starts_with("page=")).collect();
            if !page.is_empty() {
                canonical.push('?');
                canonical.push_str(&page.join("&"));
            }
        }
        canonical
    }

    pub fn new(
        base_url_no_prefix: &str,
        path: &str,
        query: Option<&str>,
        canonical: String,
    ) -> Crawler {
        let mut url = format!("{base_url_no_prefix}{path}");
        if let Some(q) = query.filter(|q| !q.is_empty()) {
            url.push('?');
            url.push_str(q);
        }
        Crawler {
            noindex: !canonical.is_empty() && canonical != url,
            canonical,
            url,
            ..Crawler::default()
        }
    }

    /// The crawler block for a request: the given canonical, else the
    /// default one.
    pub fn for_request(
        urls: &crate::url::Urls<'_>,
        uri: &axum::http::Uri,
        canonical: Option<String>,
    ) -> Result<Crawler, crate::url::UrlError> {
        let base = urls.base_url_no_prefix()?;
        let canonical =
            canonical.unwrap_or_else(|| Crawler::default_canonical(&base, uri.path(), uri.query()));
        Ok(Crawler::new(&base, uri.path(), uri.query(), canonical))
    }

    /// `crawlable_meta_data(title:, description:, image:)`: the site's
    /// OpenGraph image when the page has none.
    pub fn with_meta(mut self, title: &str, description: &str, image: Option<String>) -> Crawler {
        self.title = crate::emoji::gsub_emoji_to_unicode(title);
        self.og_description = crate::emoji::gsub_emoji_to_unicode(description);
        self.image = image.filter(|i| !i.is_empty());
        self
    }
}

/// The site's OpenGraph image (`SiteSetting.site_opengraph_image_url`),
/// None when nothing resolves.
pub async fn site_opengraph_image(
    conn: &mut PgConnection,
    urls: &crate::url::Urls<'_>,
) -> Result<Option<String>, crate::site_icons::IconError> {
    let url = crate::site_icons::site_url(conn, urls, "opengraph_image").await?;
    Ok((!url.is_empty()).then_some(url))
}

/// An HTML response with the non-canonical noindex header when the
/// setting asks for it.
pub fn crawler_response(
    body: String,
    crawler: &Crawler,
    settings: &SiteSettings,
    viewer: &ViewerState,
) -> Result<axum::response::Response, SettingError> {
    use axum::response::IntoResponse;
    let mut response = axum::response::Html(body).into_response();
    if crawler.noindex && !settings.get("allow_indexing_non_canonical_urls")?.truthy() {
        response.headers_mut().insert(
            "x-robots-tag",
            axum::http::HeaderValue::from_static("noindex"),
        );
    }
    Ok(with_viewer_headers(response, viewer))
}

/// One post of a topic as the page shows it, from its PostSerializer JSON.
/// `member`: the viewer is logged in (they may bookmark).
pub fn post_item(i18n: &I18n, base: &str, topic_url: &str, p: &Value, member: bool) -> PostItem {
    let number = p["post_number"].as_i64().unwrap_or(0);
    let username = s(&p["username"]);
    let action = p["action_code"].as_str();
    // The like entry of PostSerializer's actions_summary, as the viewer sees it.
    let like = p["actions_summary"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["id"] == 2));
    let flag = |key: &str| like.is_some_and(|l| l[key] == true);
    PostItem {
        number,
        id: p["id"].as_i64().unwrap_or(0),
        actions_url: format!("{base}/post_actions"),
        base_path: base.to_string(),
        url: format!("{topic_url}/{number}"),
        user_url: format!("{base}/u/{username}"),
        username,
        name: p["name"]
            .as_str()
            .filter(|n| !n.is_empty())
            .map(str::to_string),
        created_at: date(&s(&p["created_at"])),
        created_at_iso: s(&p["created_at"]),
        cooked: s(&p["cooked"]),
        likes: like.and_then(|x| x["count"].as_i64()).unwrap_or(0),
        small_action: action.is_some(),
        action_text: action
            .map(|code| action_text(i18n, code, p["action_code_who"].as_str().unwrap_or(""))),
        can_like: flag("can_act"),
        liked: flag("acted"),
        can_unlike: flag("can_undo"),
        can_bookmark: member,
        bookmark_id: p["bookmark_id"].as_i64(),
    }
}

/// A live update of one post for the topic page: appended to the posts
/// when new, else replacing the post in place (htmx out-of-band swaps).
#[derive(Template)]
#[template(path = "post_fragment.html")]
pub struct PostFragment {
    pub post: PostItem,
    pub append: bool,
}

/// What a notification says its actor did, by `Notification.types`, for
/// the header's alert and the user menu.
pub fn notification_verb(notification_type: i64) -> &'static str {
    match notification_type {
        1 => "mentioned you in",
        2 => "replied in",
        3 => "quoted you in",
        4 => "edited your post in",
        5 | 19 => "liked your post in",
        6 => "sent you a message,",
        7 => "invited you to a message,",
        9 | 36 => "posted in",
        11 => "linked to your post in",
        12 => "earned a badge,",
        13 => "invited you to",
        17 => "posted a new topic,",
        24 => "Reminder:",
        _ => "in",
    }
}
