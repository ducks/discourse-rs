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

/// What every page's layout needs.
pub struct Site {
    pub site_title: String,
    pub site_description: String,
    pub lang: String,
    pub base_path: String,
}

impl Site {
    pub fn from_settings(settings: &SiteSettings, base_path: &str) -> Result<Site, SettingError> {
        Ok(Site {
            site_title: settings.get("title")?.to_s(),
            site_description: settings.get("site_description")?.to_s(),
            lang: settings.get("default_locale")?.to_s().replace('_', "-"),
            base_path: base_path.to_string(),
        })
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
    pub topics: Vec<TopicItem>,
    pub more_url: Option<String>,
}

pub struct PostItem {
    pub number: i64,
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
}

#[derive(Template)]
#[template(path = "topic.html")]
pub struct TopicPage {
    pub site_title: String,
    pub site_description: String,
    pub lang: String,
    pub base_path: String,
    pub title: String,
    pub title_unicode: String,
    pub canonical_url: String,
    pub breadcrumbs: Vec<CategoryBadge>,
    pub tags: Vec<TagLink>,
    pub posts: Vec<PostItem>,
    pub prev_url: Option<String>,
    pub next_url: Option<String>,
}

/// `categories` rows the pages link to.
#[derive(sqlx::FromRow)]
struct CategoryRow {
    id: i32,
    name: String,
    color: String,
    slug: String,
    parent_category_id: Option<i32>,
}

async fn categories(conn: &mut PgConnection) -> Result<Vec<CategoryRow>, sqlx::Error> {
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

fn badge(base_path: &str, cats: &[CategoryRow], id: Option<i64>) -> Option<CategoryBadge> {
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
        site_description: site.site_description,
        lang: site.lang,
        base_path: site.base_path,
        topics,
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

    let posts = view["post_stream"]["posts"]
        .as_array()
        .map(|posts| {
            posts
                .iter()
                .map(|p| {
                    let number = p["post_number"].as_i64().unwrap_or(0);
                    let username = s(&p["username"]);
                    let action = p["action_code"].as_str();
                    PostItem {
                        number,
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
                        likes: p["actions_summary"]
                            .as_array()
                            .and_then(|a| a.iter().find(|x| x["id"] == 2))
                            .and_then(|x| x["count"].as_i64())
                            .unwrap_or(0),
                        small_action: action.is_some(),
                        action_text: action.map(|code| {
                            action_text(i18n, code, p["action_code_who"].as_str().unwrap_or(""))
                        }),
                    }
                })
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
        site_description: site.site_description,
        lang: site.lang,
        base_path: site.base_path,
        title: s(&view["title"]),
        title_unicode: crate::emoji::gsub_emoji_to_unicode(&s(&view["title"])),
        canonical_url: page_url(page),
        breadcrumbs,
        tags: tags(&base, &view["tags"]),
        posts,
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
