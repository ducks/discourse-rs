//! Oneboxes of the site's own pages (lib/oneboxer.rb's `local_onebox`,
//! lib/inline_oneboxer.rb's local topics) and CookedPostProcessor's
//! `post_process_oneboxes` (lib/cooked_processor_mixin.rb), which puts
//! them into a post's cooked HTML.
//!
//! What `Discourse.route_for` recognizes is ported for the controllers
//! with a local onebox (topics, users, uploads, category lists) and the
//! site's other pages, which onebox as a plain link. Other sites'
//! oneboxes are fetched from the network (FinalDestination, the onebox
//! engines) and are refused, as are own-site paths no route here knows
//! (Rails would fetch the page) and chat's local oneboxes.

use std::sync::LazyLock;

use markup5ever_rcdom::{Handle, NodeData, RcDom};
use regex::Regex;
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::pretty_text::cleanup::{
    all_elements, attr, element_name, fragment_root, has_class, new_text, parse, set_attr,
};
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, Unsupported};

const TOPIC_TEMPLATE: &str =
    include_str!("../vendor/discourse/lib/onebox/templates/discourse_topic_onebox.mustache");
const USER_TEMPLATE: &str =
    include_str!("../vendor/discourse/lib/onebox/templates/discourse_user_onebox.mustache");
const CATEGORY_TEMPLATE: &str =
    include_str!("../vendor/discourse/lib/onebox/templates/discourse_category_onebox.mustache");

/// `Oneboxer::AUDIO_REGEX` and `VIDEO_REGEX`, on the path's extension.
const AUDIO: [&str; 11] = [
    "mp3", "oga", "ogg", "opus", "wav", "m4a", "m4b", "m4p", "m4r", "aac", "flac",
];
const VIDEO: [&str; 9] = [
    "mov", "mp4", "webm", "m4v", "3gp", "ogv", "avi", "mpeg", "ogv",
];

/// `Oneboxer::HTML5_BLOCK_ELEMENTS`
const BLOCK_ELEMENTS: [&str; 37] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "canvas",
    "center",
    "dd",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "li",
    "main",
    "nav",
    "noscript",
    "ol",
    "output",
    "p",
    "pre",
    "section",
    "table",
    "tfoot",
    "ul",
    "video",
];

/// What `Discourse.route_for` gives for one of the site's URLs: the
/// controller and the parameters a local onebox reads.
#[derive(Debug, PartialEq)]
pub enum Route {
    /// topics#show (and the other topics actions a GET reaches):
    /// `route[:id] || route[:topic_id]`, `route[:post_number]`.
    Topic {
        id: String,
        post_number: Option<String>,
    },
    /// The users controller: `route[:username]`.
    User { username: String },
    /// The uploads controller.
    Upload,
    /// A list of a category: `route[:category_slug_path_with_id]`.
    Category { slug_path_with_id: String },
    /// A page whose controller has no local onebox.
    Other,
}

static USERNAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[%\w.\-]+$").unwrap());

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// The top-level paths of pages with no local onebox handler.
const OTHER_PAGES: [&str; 27] = [
    "",
    "latest",
    "new",
    "unread",
    "top",
    "hot",
    "categories",
    "tags",
    "tag",
    "about",
    "faq",
    "guidelines",
    "tos",
    "privacy",
    "badges",
    "g",
    "groups",
    "search",
    "review",
    "admin",
    "login",
    "signup",
    "filter",
    "posted",
    "bookmarks",
    "read",
    "my",
];

/// `Rails.application.routes.recognize_path` for a GET of `path`, for the
/// routes a onebox can tell apart; None for a path no route here knows.
pub fn recognize(path: &str) -> Option<Route> {
    let path = path.strip_suffix('/').unwrap_or(path);
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match segments.as_slice() {
        ["t", rest @ ..] => recognize_topic(rest),
        ["u" | "users", name, rest @ ..] => {
            // users_email#index, list#private_messages_tag and the feeds
            // are other controllers.
            let other = matches!(rest, ["preferences", "email"])
                || matches!(rest, ["messages", "tags", ..])
                || rest.last().is_some_and(|r| r.ends_with(".rss"));
            if other || !USERNAME.is_match(name) {
                Some(Route::Other)
            } else {
                Some(Route::User {
                    username: name.to_string(),
                })
            }
        }
        ["uploads" | "secure-uploads", ..] => Some(Route::Upload),
        ["c", rest @ ..] if !rest.is_empty() => Some(recognize_category(rest)),
        [first, ..] if OTHER_PAGES.contains(first) => Some(Route::Other),
        _ => None,
    }
}

fn recognize_topic(rest: &[&str]) -> Option<Route> {
    let topic = |id: &str, post_number: Option<&str>| Route::Topic {
        id: id.to_string(),
        post_number: post_number.map(str::to_string),
    };
    Some(match rest {
        // t/:id
        [id] => topic(id, None),
        ["id_for" | "external_id", _] => topic("", None),
        [id, n] if digits(id) && digits(n) => topic(id, Some(n)),
        [
            id,
            "summary" | "last" | "wordpress" | "posts" | "post_ids" | "excerpts",
        ] if digits(id) => topic(id, None),
        [_, id] if digits(id) => topic(id, None),
        [_, id, n] if digits(id) && digits(n) => topic(id, Some(n)),
        [_, id, "print" | "summary" | "last" | "wordpress"] if digits(id) => topic(id, None),
        _ => return None,
    })
}

fn recognize_category(rest: &[&str]) -> Route {
    let mut parts: Vec<&str> = rest.to_vec();
    // categories#find_by_slug, categories#index
    if parts.last() == Some(&"find_by_slug")
        || parts.contains(&"edit")
        || parts.last() == Some(&"subcategories")
    {
        return Route::Other;
    }
    // The scope's list routes: /l/:filter, /none(/l/:filter), /all.
    if parts.len() >= 2 && parts[parts.len() - 2] == "l" {
        parts.truncate(parts.len() - 2);
    }
    if matches!(parts.last(), Some(&"none") | Some(&"all")) {
        parts.pop();
    }
    Route::Category {
        slug_path_with_id: parts.join("/"),
    }
}

/// `Discourse.route_for(url)`: the route of a URL on this site (no host,
/// or the site's host under its base path), None for another site's.
fn route_for(url: &str, hostname: &str, base_path: &str) -> Result<Option<Route>, Unsupported> {
    let (host, path) = match url.split_once("://") {
        Some((_, rest)) => {
            let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            let authority = &rest[..end];
            let host = authority.rsplit('@').next().unwrap_or(authority);
            let host = host.split(':').next().unwrap_or(host);
            (Some(host.to_lowercase()), &rest[end..])
        }
        None if url.starts_with("//") => return Err(Unsupported("protocol-relative onebox URLs")),
        None => (None, url),
    };
    let path = path.split(['?', '#']).next().unwrap_or("");
    if let Some(host) = &host
        && (host != hostname || !path.starts_with(base_path))
    {
        return Ok(None);
    }
    let path = &path[base_path.len().min(path.len())..];
    match recognize(path) {
        Some(Route::Other) if path.starts_with("/chat") => {
            Err(Unsupported("oneboxes of chat channels and messages"))
        }
        Some(route) => Ok(Some(route)),
        // Rails would fetch the page as another site's.
        None => Err(Unsupported(
            "oneboxing a page of the site no route recognizes",
        )),
    }
}

/// `Onebox::Helpers.normalize_url_for_output`
fn normalize_url_for_output(url: &str) -> String {
    let url = url
        .replace(' ', "%20")
        .replace('\'', "&apos;")
        .replace('"', "&quot;");
    let url: String = url
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || "-`.~:/?#[]@!$&'()*+,;=%’".contains(*c))
        .collect();
    let has_host = url
        .split_once("://")
        .is_some_and(|(_, rest)| !rest.is_empty() && !rest.starts_with('/'));
    if has_host { url } else { String::new() }
}

/// What `Oneboxer.onebox` is asked with during post processing.
pub struct Opts {
    /// The post's last editor (`user_id:`).
    pub editor_id: Option<i32>,
    /// The post's author, for its inline oneboxes.
    pub author_id: Option<i32>,
    /// The topic's category.
    pub category_id: Option<i32>,
}

/// The site the oneboxes are made for.
pub struct Site<'a> {
    pub settings: &'a SiteSettings,
    pub i18n: &'a crate::i18n::I18n,
    pub urls: Urls<'a>,
    pub base_path: &'a str,
}

fn escape(s: &str) -> String {
    crate::discourse_diff::escape_html(s)
}

/// A mustache template's values: text, an HTML-escaped value, or a section.
enum Value {
    Text(String),
    Section(bool),
    List(Vec<Vec<(&'static str, Value)>>),
}

/// Mustache, as the onebox templates use it: `{{name}}` escaped,
/// `{{{name}}}` as is, `{{#name}}...{{/name}}` sections over a boolean, a
/// present value or a list; a line holding only a section tag is dropped
/// (the spec's standalone tags).
fn render(template: &str, values: &[(&'static str, Value)]) -> String {
    let mut out = String::new();
    let lines: Vec<&str> = template.split_inclusive('\n').collect();
    render_lines(&lines, values, &mut out);
    out
}

fn section_tag(line: &str) -> Option<(char, &str)> {
    let t = line.trim();
    let inner = t.strip_prefix("{{")?.strip_suffix("}}")?;
    let kind = inner.chars().next()?;
    if kind == '#' || kind == '/' || kind == '^' {
        Some((kind, inner[1..].trim()))
    } else {
        None
    }
}

fn render_lines(lines: &[&str], values: &[(&'static str, Value)], out: &mut String) {
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(('#', name)) = section_tag(line) {
            let end = (i + 1..lines.len())
                .find(|&j| section_tag(lines[j]) == Some(('/', name)))
                .unwrap_or(lines.len());
            let body = &lines[i + 1..end];
            match values.iter().find(|(n, _)| *n == name).map(|(_, v)| v) {
                Some(Value::Section(true)) => render_lines(body, values, out),
                Some(Value::Text(t)) if !t.is_empty() => render_lines(body, values, out),
                Some(Value::List(items)) => {
                    for item in items {
                        let mut scope: Vec<(&'static str, Value)> = Vec::new();
                        for (n, v) in item {
                            scope.push((n, clone_value(v)));
                        }
                        for (n, v) in values {
                            if !scope.iter().any(|(s, _)| s == n) {
                                scope.push((n, clone_value(v)));
                            }
                        }
                        render_lines(body, &scope, out);
                    }
                }
                _ => {}
            }
            i = end + 1;
            continue;
        }
        out.push_str(&interpolate(line, values));
        i += 1;
    }
}

fn clone_value(v: &Value) -> Value {
    match v {
        Value::Text(t) => Value::Text(t.clone()),
        Value::Section(b) => Value::Section(*b),
        Value::List(_) => Value::Section(true),
    }
}

fn interpolate(line: &str, values: &[(&'static str, Value)]) -> String {
    let lookup = |name: &str| -> String {
        match values.iter().find(|(n, _)| *n == name).map(|(_, v)| v) {
            Some(Value::Text(t)) => t.clone(),
            _ => String::new(),
        }
    };
    let mut out = String::new();
    let mut rest = line;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        if let Some(inner) = after.strip_prefix("{{{")
            && let Some(end) = inner.find("}}}")
        {
            out.push_str(&lookup(inner[..end].trim()));
            rest = &inner[end + 3..];
        } else if let Some(end) = after.find("}}") {
            out.push_str(&escape(&lookup(after[2..end].trim())));
            rest = &after[end + 2..];
        } else {
            out.push_str(after);
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

/// `PrettyText.avatar_img(template, size)`: the size's pixels, the URL at
/// the avatar size twice as large (as the server renders for 2x).
fn avatar_img(settings: &SiteSettings, template: &str, size: &str) -> Result<String, AppError> {
    let px: i64 = match size {
        "tiny" | "small" => 24,
        "medium" | "large" => 48,
        "extra_large" => 96,
        "huge" => 144,
        _ => return Err(Unsupported("an avatar size the onebox templates don't use").into()),
    };
    let sizes: Vec<i64> = settings
        .get("avatar_sizes")?
        .to_s()
        .split('|')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let raw = sizes
        .iter()
        .copied()
        .filter(|s| *s >= px * 2)
        .min()
        .or_else(|| sizes.iter().copied().max())
        .unwrap_or(px * 2);
    Ok(format!(
        "<img alt=\"\" width=\"{px}\" height=\"{px}\" src=\"{}\" class=\"avatar\">",
        escape(&template.replace("{size}", &raw.to_string()))
    ))
}

/// `Sanitize.fragment(value).strip`: the text, tags gone (block ones
/// leave a space), escaped.
fn sanitize_text(html: &str) -> String {
    let dom = parse(html);
    let mut out = String::new();
    fn walk(node: &Handle, out: &mut String) {
        match &node.data {
            NodeData::Text { contents } => out.push_str(&escape(&contents.borrow())),
            NodeData::Element { name, .. } => {
                let block = BLOCK_ELEMENTS.contains(&&*name.local) || &*name.local == "br";
                if block {
                    out.push(' ');
                }
                for child in node.children.borrow().iter() {
                    walk(child, out);
                }
                if block {
                    out.push(' ');
                }
            }
            _ => {}
        }
    }
    for child in fragment_root(&dom).children.borrow().iter() {
        walk(child, &mut out);
    }
    out.trim().to_string()
}

/// `Guardian#can_see_category?`
async fn can_see_category(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    category_id: i32,
) -> Result<bool, AppError> {
    let restricted: Option<bool> =
        sqlx::query_scalar("SELECT read_restricted FROM categories WHERE id = $1")
            .bind(category_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(restricted) = restricted else {
        return Ok(false);
    };
    if guardian.is_admin()
        && !settings
            .get("suppress_secured_categories_from_admin")?
            .truthy()
    {
        return Ok(true);
    }
    if !restricted {
        return Ok(true);
    }
    Ok(guardian
        .secure_category_ids(&mut *conn, settings)
        .await?
        .contains(&category_id))
}

async fn guardian_for(conn: &mut PgConnection, user_id: Option<i32>) -> Result<Guardian, AppError> {
    let Some(id) = user_id else {
        return Ok(Guardian::anonymous());
    };
    match crate::session::current::SessionUser::load(&mut *conn, id).await? {
        Some(user) => Ok(Guardian::for_user(&mut *conn, &user).await?),
        None => Ok(Guardian::anonymous()),
    }
}

/// `Guardian#can_see_topic?(topic)`
pub(crate) async fn can_see_topic(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    topic_id: i32,
) -> Result<bool, AppError> {
    let Some(topic) =
        crate::topic_guardian::TopicCtx::load(&mut *conn, settings, guardian, topic_id).await?
    else {
        return Ok(false);
    };
    let secure = guardian.secure_category_ids(&mut *conn, settings).await?;
    Ok(guardian.can_see_topic(settings, &topic, true, &secure)?)
}

#[derive(sqlx::FromRow)]
struct TopicRow {
    id: i32,
    title: String,
    category_id: Option<i32>,
    archetype: String,
    highest_post_number: i32,
}

/// `Oneboxer.local_topic`: the topic when it may be shown, a message
/// never.
async fn local_topic(
    conn: &mut PgConnection,
    site: &Site<'_>,
    id: &str,
    viewer_id: Option<i32>,
    category_id: Option<i32>,
) -> Result<Option<TopicRow>, AppError> {
    let s = site.settings;
    let mut current_category = None;
    if let Some(viewer) = viewer_id {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
            .bind(viewer)
            .fetch_one(&mut *conn)
            .await?;
        if exists && let Some(category) = category_id {
            let found: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE id = $1)")
                    .bind(category)
                    .fetch_one(&mut *conn)
                    .await?;
            if found {
                let guardian = guardian_for(conn, Some(viewer)).await?;
                if !can_see_category(conn, s, &guardian, category).await? {
                    return Ok(None);
                }
                current_category = Some(category);
            }
        }
    }
    // Topic.find_by(id:): the id cast to an integer.
    let id = crate::ruby::to_i(id);
    let Ok(id) = i32::try_from(id) else {
        return Ok(None);
    };
    let topic: Option<TopicRow> = sqlx::query_as(
        "SELECT id, title, category_id, archetype, highest_post_number FROM topics \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(topic) = topic else {
        return Ok(None);
    };
    if topic.archetype == "private_message" {
        return Ok(None);
    }
    let guardian = if current_category.is_some() && current_category == topic.category_id {
        guardian_for(conn, viewer_id).await?
    } else {
        Guardian::anonymous()
    };
    if !can_see_topic(conn, s, &guardian, topic.id).await? {
        return Ok(None);
    }
    Ok(Some(topic))
}

/// `Oneboxer.local_topic_html`, post processing's way (no topic of its
/// own, so never the same-topic quote).
async fn local_topic_html(
    conn: &mut PgConnection,
    site: &Site<'_>,
    url: &str,
    id: &str,
    post_number: Option<&str>,
    opts: &Opts,
) -> Result<Option<String>, AppError> {
    let Some(topic) = local_topic(conn, site, id, opts.editor_id, opts.category_id).await? else {
        return Ok(None);
    };
    if site.settings.get("content_localization_enabled")?.truthy() {
        return Err(Unsupported("localized topic oneboxes").into());
    }
    let post_number = post_number.map_or(0, crate::ruby::to_i);
    #[derive(sqlx::FromRow)]
    struct PostRow {
        post_number: i32,
        cooked: String,
        hidden: bool,
        post_type: i32,
        user_id: Option<i32>,
    }
    let post: Option<PostRow> = if post_number > 1 {
        sqlx::query_as(
            "SELECT post_number, cooked, hidden, post_type, user_id FROM posts \
             WHERE topic_id = $1 AND post_number = $2 AND deleted_at IS NULL ORDER BY id LIMIT 1",
        )
        .bind(topic.id)
        .bind(post_number as i32)
        .fetch_optional(&mut *conn)
        .await?
    } else {
        sqlx::query_as(
            "SELECT post_number, cooked, hidden, post_type, user_id FROM posts \
             WHERE topic_id = $1 AND deleted_at IS NULL ORDER BY sort_order, post_number LIMIT 1",
        )
        .bind(topic.id)
        .fetch_optional(&mut *conn)
        .await?
    };
    let Some(post) = post.filter(|p| !p.hidden && [1, 2].contains(&p.post_type)) else {
        return Ok(None);
    };
    let s = site.settings;
    let excerpt = crate::excerpt::excerpt(
        &post.cooked,
        s.get("post_onebox_maxlength")?.to_i().max(0) as usize,
        &crate::excerpt::Options {
            keep_svg: true,
            ..Default::default()
        },
    );
    // post.user.avatar_template_url: absolute, without the scheme.
    let user: Option<(i32, String, Option<i32>)> =
        sqlx::query_as("SELECT id, username, uploaded_avatar_id FROM users WHERE id = $1")
            .bind(post.user_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((user_id, username, uploaded_avatar_id)) = user else {
        return Err(Unsupported("a topic onebox of a post without its user").into());
    };
    let template =
        crate::avatar::avatar_template(&site.urls, user_id, &username, uploaded_avatar_id, None)?;
    let absolute = site.urls.absolute(&template)?;
    let schemaless = absolute
        .strip_prefix("http:")
        .or_else(|| absolute.strip_prefix("https:"))
        .unwrap_or(&absolute);
    let category_html = match topic.category_id {
        Some(id) => crate::category_badge::html_for(conn, s, site.base_path, id).await?,
        None => String::new(),
    };
    let unescape = |t: &str| -> Result<String, AppError> {
        Ok(if s.get("enable_emoji")?.truthy() {
            crate::topic_list_view::emoji_unescape_with(
                t,
                s.get("enable_inline_emoji_translation")?.truthy(),
                &s.get("emoji_set")?.to_s(),
                site.base_path,
            )
        } else {
            t.to_string()
        })
    };
    Ok(Some(render(
        TOPIC_TEMPLATE,
        &[
            ("topic_id", Value::Text(topic.id.to_string())),
            ("post_number", Value::Text(post.post_number.to_string())),
            ("avatar", Value::Text(avatar_img(s, schemaless, "tiny")?)),
            ("original_url", Value::Text(url.to_string())),
            ("title", Value::Text(unescape(&escape(&topic.title))?)),
            ("category_html", Value::Text(category_html)),
            ("quote", Value::Text(unescape(&excerpt)?)),
        ],
    )))
}

/// `Oneboxer.local_user_html`
async fn local_user_html(
    conn: &mut PgConnection,
    site: &Site<'_>,
    url: &str,
    username: &str,
    opts: &Opts,
) -> Result<Option<String>, AppError> {
    let s = site.settings;
    let Some(user) = crate::users::User::find_by_username(&mut *conn, username).await? else {
        return Ok(None);
    };
    if s.get("allow_users_to_hide_profile")?.truthy() && user.hide_profile.unwrap_or(false) {
        return Ok(None);
    }
    let viewer = guardian_for(conn, opts.editor_id).await?;
    if !user.visible_to(s, &viewer)? {
        return Ok(None);
    }
    let name = if s.get("enable_names")?.truthy() {
        user.name.clone().unwrap_or_default()
    } else {
        String::new()
    };
    let template = crate::avatar::avatar_template(
        &site.urls,
        user.id,
        &user.username,
        user.uploaded_avatar_id,
        None,
    )?;
    // UserProfile#bio_excerpt(230)
    let bio = match user.bio_cooked.as_deref().filter(|b| !b.trim().is_empty()) {
        Some(cooked) => {
            let excerpt = crate::excerpt::excerpt(cooked, 230, &Default::default());
            excerpt.strip_suffix("<br>").unwrap_or(&excerpt).to_string()
        }
        None => String::new(),
    };
    let location = user
        .location
        .as_deref()
        .filter(|l| !l.trim().is_empty())
        .map(sanitize_text)
        .unwrap_or_default();
    let website = user.website.clone().unwrap_or_default();
    let website_name = crate::users::website_name(&website).unwrap_or_default();
    let joined = site
        .i18n
        .t_with(
            "onebox.discourse.user_joined_community",
            &[("date", &user.created_at.format("%B %-d, %Y").to_string())],
        )
        .unwrap_or_default();
    Ok(Some(render(
        USER_TEMPLATE,
        &[
            ("user_id", Value::Text(user.id.to_string())),
            ("username", Value::Text(user.username.clone())),
            ("avatar", Value::Text(avatar_img(s, &template, "huge")?)),
            ("name", Value::Text(name)),
            ("bio", Value::Text(bio)),
            ("location", Value::Text(location)),
            ("joined", Value::Text(joined)),
            ("website", Value::Text(website)),
            ("website_name", Value::Text(website_name)),
            ("original_url", Value::Text(url.to_string())),
        ],
    )))
}

#[derive(sqlx::FromRow)]
struct CategoryRow {
    id: i32,
    name: String,
    color: String,
    slug: String,
    parent_category_id: Option<i32>,
    description: Option<String>,
    uploaded_logo_id: Option<i32>,
}

const CATEGORY_COLUMNS: &str =
    "id, name, color, slug, parent_category_id, description, uploaded_logo_id";

/// `Category#url`
async fn category_url(
    conn: &mut PgConnection,
    base_path: &str,
    category: &CategoryRow,
) -> Result<String, AppError> {
    let mut slugs = vec![slug_for(category)];
    let mut parent = category.parent_category_id;
    while let Some(id) = parent {
        let row: Option<CategoryRow> = sqlx::query_as(&format!(
            "SELECT {CATEGORY_COLUMNS} FROM categories WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
        let Some(row) = row else { break };
        slugs.insert(0, slug_for(&row));
        parent = row.parent_category_id;
    }
    Ok(format!("{base_path}/c/{}/{}", slugs.join("/"), category.id))
}

/// `Category#slug_for_url`
fn slug_for(category: &CategoryRow) -> String {
    if category.slug.is_empty() {
        format!("{}-category", category.id)
    } else {
        category.slug.clone()
    }
}

/// `Oneboxer.local_category_html`
async fn local_category_html(
    conn: &mut PgConnection,
    site: &Site<'_>,
    slug_path_with_id: &str,
) -> Result<Option<String>, AppError> {
    // Category.find_by_slug_path_with_id
    let mut slugs: Vec<&str> = slug_path_with_id.split('/').collect();
    let category: Option<CategoryRow> = if slugs.last().is_some_and(|s| digits(s)) {
        let id = crate::ruby::to_i(slugs.pop().unwrap_or_default());
        sqlx::query_as(&format!(
            "SELECT {CATEGORY_COLUMNS} FROM categories WHERE id = $1"
        ))
        .bind(id as i32)
        .fetch_optional(&mut *conn)
        .await?
    } else {
        // find_by_slug_path: each slug a child of the one before.
        let mut found: Option<CategoryRow> = None;
        for slug in slugs.iter().take(3) {
            let parent = found.as_ref().map(|c| c.id);
            found = sqlx::query_as(&format!(
                "SELECT {CATEGORY_COLUMNS} FROM categories WHERE lower(slug) = lower($1) \
                 AND parent_category_id IS NOT DISTINCT FROM $2"
            ))
            .bind(slug)
            .bind(parent)
            .fetch_optional(&mut *conn)
            .await?;
            if found.is_none() {
                break;
            }
        }
        found
    };
    let Some(category) = category else {
        return Ok(None);
    };
    let s = site.settings;
    let anonymous = Guardian::anonymous();
    if !can_see_category(conn, s, &anonymous, category.id).await? {
        return Ok(None);
    }
    let subcategories: Vec<CategoryRow> = sqlx::query_as(&format!(
        "SELECT {CATEGORY_COLUMNS} FROM categories WHERE parent_category_id = $1 ORDER BY id"
    ))
    .bind(category.id)
    .fetch_all(&mut *conn)
    .await?;
    let has_subcategories = !subcategories.is_empty();
    let mut visible = Vec::new();
    for sub in subcategories {
        if can_see_category(conn, s, &anonymous, sub.id).await? {
            let url = category_url(conn, site.base_path, &sub).await?;
            visible.push(vec![
                ("name", Value::Text(sub.name.clone())),
                ("color", Value::Text(sub.color.clone())),
                ("url", Value::Text(url)),
            ]);
        }
    }
    let logo_url = match category.uploaded_logo_id {
        Some(id) => {
            let url: Option<String> = sqlx::query_scalar("SELECT url FROM uploads WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?;
            url.unwrap_or_default()
        }
        None => String::new(),
    };
    let description = category
        .description
        .as_deref()
        .filter(|d| !d.trim().is_empty())
        .map(sanitize_text)
        .unwrap_or_default();
    let url = category_url(conn, site.base_path, &category).await?;
    Ok(Some(render(
        CATEGORY_TEMPLATE,
        &[
            ("url", Value::Text(url)),
            ("name", Value::Text(category.name.clone())),
            ("color", Value::Text(category.color.clone())),
            ("logo_url", Value::Text(logo_url)),
            ("description", Value::Text(description)),
            ("has_subcategories", Value::Section(has_subcategories)),
            ("subcategories", Value::List(visible)),
        ],
    )))
}

/// `Oneboxer.local_upload_html`
fn local_upload_html(settings: &SiteSettings, url: &str) -> Result<Option<String>, AppError> {
    let controls = if settings
        .get("disable_onebox_media_download_controls")?
        .truthy()
    {
        "controlslist='nodownload'"
    } else {
        ""
    };
    let normalized = normalize_url_for_output(url);
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or("");
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_lowercase())
        .unwrap_or_default();
    Ok(if VIDEO.contains(&ext.as_str()) {
        Some(format!(
            "<div class=\"onebox video-onebox\">\n  <video {controls} width=\"100%\" height=\"100%\" controls=\"\">\n    <source src='{normalized}'>\n    <a href='{normalized}'>\n      {normalized}\n    </a>\n  </video>\n</div>\n"
        ))
    } else if AUDIO.contains(&ext.as_str()) {
        Some(format!(
            "<audio {controls} controls>\n  <source src='{normalized}'>\n  <a href='{normalized}'>\n    {normalized}\n  </a>\n</audio>\n"
        ))
    } else {
        None
    })
}

/// `Oneboxer.onebox(url, user_id:, category_id:)`: the site's own pages'
/// HTML (a plain link when there is none), another site's refused.
async fn onebox(
    conn: &mut PgConnection,
    site: &Site<'_>,
    url: &str,
    opts: &Opts,
) -> Result<String, AppError> {
    if !url.is_ascii() || url.chars().any(char::is_whitespace) {
        return Err(Unsupported("onebox URLs that need encoding").into());
    }
    let hostname = site.urls.current_hostname()?;
    let Some(route) = route_for(url, &hostname, site.base_path)? else {
        return Err(Unsupported("oneboxes of other sites (fetched from the network)").into());
    };
    let html = match route {
        Route::Upload => local_upload_html(site.settings, url)?,
        Route::Topic { id, post_number } => {
            local_topic_html(conn, site, url, &id, post_number.as_deref(), opts).await?
        }
        Route::User { username } => local_user_html(conn, site, url, &username, opts).await?,
        Route::Category { slug_path_with_id } => {
            local_category_html(conn, site, &slug_path_with_id).await?
        }
        Route::Other => None,
    };
    let normalized = normalize_url_for_output(url);
    Ok(html
        .filter(|h| !h.trim().is_empty())
        .unwrap_or_else(|| format!("<a href='{normalized}'>{normalized}</a>")))
}

/// `InlineOneboxer.lookup(url, user_id:, category_id:)` for a link of the
/// site: a topic's title (with the post it points to); another site's
/// refused, since its title is fetched.
async fn inline_title(
    conn: &mut PgConnection,
    site: &Site<'_>,
    url: &str,
    opts: &Opts,
) -> Result<Option<String>, AppError> {
    let hostname = site.urls.current_hostname()?;
    let route = route_for(url, &hostname, site.base_path)?;
    match route {
        Some(Route::Topic { id, post_number }) => {
            let Some(topic) =
                local_topic(conn, site, &id, opts.author_id, opts.category_id).await?
            else {
                return Ok(None);
            };
            let post_number = post_number
                .as_deref()
                .map_or(0, crate::ruby::to_i)
                .min(i64::from(topic.highest_post_number));
            let mut title = crate::emoji::gsub_emoji_to_unicode(&topic.title);
            if post_number > 1 {
                // post_author_for_title: an anonymous reader's view.
                let author: Option<(String, i32, i32)> = sqlx::query_as(
                    "SELECT u.username, p.id, p.post_type FROM posts p JOIN users u ON u.id = p.user_id \
                     WHERE p.topic_id = $1 AND p.post_number = $2 AND p.deleted_at IS NULL LIMIT 1",
                )
                .bind(topic.id)
                .bind(post_number as i32)
                .fetch_optional(&mut *conn)
                .await?;
                let n = post_number.to_string();
                let suffix = match author.filter(|(_, _, t)| *t == 1) {
                    Some((username, ..)) => site.i18n.t_with(
                        "inline_oneboxer.topic_page_title_post_number_by_user",
                        &[("post_number", &n), ("username", &username)],
                    ),
                    None => site.i18n.t_with(
                        "inline_oneboxer.topic_page_title_post_number",
                        &[("post_number", &n)],
                    ),
                };
                title = format!("{title} - {}", suffix.unwrap_or_default());
            }
            Ok(Some(title))
        }
        // No handler: InlineOneboxer goes on as for another site's link,
        // whose title it crawls.
        Some(_) => Err(Unsupported("inline oneboxes of the site's pages other than topics").into()),
        None => {
            let has_host = url.contains("://");
            if !has_host {
                return Ok(None);
            }
            Err(
                Unsupported("inline oneboxes of other sites (titles fetched from the network)")
                    .into(),
            )
        }
    }
}

/// What the post processor would refuse to onebox, refused before the post
/// is written: another site's links, the site's paths no route knows, and
/// inline oneboxes of anything but the site's topics.
pub fn refuse_unported(
    links: &[(String, bool)],
    hostname: &str,
    base_path: &str,
) -> Result<(), Unsupported> {
    for (url, inline) in links {
        if !url.is_ascii() || url.chars().any(char::is_whitespace) {
            return Err(Unsupported("onebox URLs that need encoding"));
        }
        match route_for(url, hostname, base_path)? {
            None if *inline && !url.contains("://") => {}
            None if *inline => {
                return Err(Unsupported(
                    "inline oneboxes of other sites (titles fetched from the network)",
                ));
            }
            None => {
                return Err(Unsupported(
                    "oneboxes of other sites (fetched from the network)",
                ));
            }
            Some(Route::Topic { .. }) => {}
            Some(_) if *inline => {
                return Err(Unsupported(
                    "inline oneboxes of the site's pages other than topics",
                ));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

fn remove_class(node: &Handle, class: &str) {
    let classes: Vec<String> = attr(node, "class")
        .unwrap_or_default()
        .split_whitespace()
        .filter(|c| *c != class)
        .map(str::to_string)
        .collect();
    if let NodeData::Element { attrs, .. } = &node.data {
        let mut attrs = attrs.borrow_mut();
        if classes.is_empty() {
            attrs.retain(|a| &*a.name.local != "class");
        } else if let Some(a) = attrs.iter_mut().find(|a| &*a.name.local == "class") {
            a.value = classes.join(" ").into();
        }
    }
}

fn add_class(node: &Handle, class: &str) {
    let mut classes = attr(node, "class").unwrap_or_default();
    if !classes.split_whitespace().any(|c| c == class) {
        if !classes.is_empty() {
            classes.push(' ');
        }
        classes.push_str(class);
    }
    set_attr(node, "class", &classes);
}

fn parent_of(node: &Handle) -> Option<Handle> {
    let weak = node.parent.take();
    let parent = weak.as_ref().and_then(|w| w.upgrade());
    node.parent.set(weak);
    parent
}

fn detach(node: &Handle) {
    if let Some(parent) = parent_of(node) {
        parent
            .children
            .borrow_mut()
            .retain(|c| !std::rc::Rc::ptr_eq(c, node));
    }
    node.parent.set(None);
}

fn set_children(parent: &Handle, children: Vec<Handle>) {
    for child in parent.children.borrow().iter() {
        child.parent.set(None);
    }
    for child in &children {
        child.parent.set(Some(std::rc::Rc::downgrade(parent)));
    }
    *parent.children.borrow_mut() = children;
}

fn insert_after(anchor: &Handle, nodes: Vec<Handle>) {
    let Some(parent) = parent_of(anchor) else {
        return;
    };
    for node in &nodes {
        node.parent.set(Some(std::rc::Rc::downgrade(&parent)));
    }
    let mut children = parent.children.borrow_mut();
    let index = children
        .iter()
        .position(|c| std::rc::Rc::ptr_eq(c, anchor))
        .map_or(children.len(), |i| i + 1);
    children.splice(index..index, nodes);
}

fn replace_node(old: &Handle, nodes: Vec<Handle>) {
    let Some(parent) = parent_of(old) else {
        return;
    };
    for node in &nodes {
        node.parent.set(Some(std::rc::Rc::downgrade(&parent)));
    }
    let mut children = parent.children.borrow_mut();
    if let Some(index) = children.iter().position(|c| std::rc::Rc::ptr_eq(c, old)) {
        children.splice(index..=index, nodes);
    }
    old.parent.set(None);
}

/// A copy of an element without its children (`node.dup` keeps them; the
/// caller sets them).
fn shallow_clone(node: &Handle) -> Handle {
    match &node.data {
        NodeData::Element {
            name,
            attrs,
            template_contents,
            mathml_annotation_xml_integration_point,
        } => markup5ever_rcdom::Node::new(NodeData::Element {
            name: name.clone(),
            attrs: std::cell::RefCell::new(attrs.borrow().clone()),
            template_contents: std::cell::RefCell::new(template_contents.borrow().clone()),
            mathml_annotation_xml_integration_point: *mathml_annotation_xml_integration_point,
        }),
        _ => new_text(""),
    }
}

/// `first_significant_element_index`: past `<br>`s and blank text.
fn significant(children: &[Handle], mut index: isize, step: isize) -> isize {
    while index >= 0 && (index as usize) < children.len() {
        let c = &children[index as usize];
        let insignificant = match &c.data {
            NodeData::Element { name, .. } => &*name.local == "br",
            NodeData::Text { contents } => contents.borrow().trim().is_empty(),
            _ => false,
        };
        if !insignificant {
            break;
        }
        index += step;
    }
    index
}

/// `Oneboxer.apply`'s replacement of one link: a block onebox splits the
/// paragraph holding it.
fn apply(element: &Handle, onebox: &RcDom) {
    let nodes: Vec<Handle> = fragment_root(onebox).children.borrow().clone();
    if nodes.is_empty() {
        return;
    }
    for node in &nodes {
        node.parent.set(None);
    }
    let parent = parent_of(element);
    let block = nodes
        .iter()
        .any(|n| element_name(n).is_some_and(|name| BLOCK_ELEMENTS.contains(&name)));
    match parent {
        Some(p) if element_name(&p) == Some("p") && block => {
            let siblings: Vec<Handle> = p.children.borrow().clone();
            let idx = siblings
                .iter()
                .position(|c| std::rc::Rc::ptr_eq(c, element))
                .unwrap_or(0) as isize;
            let before = significant(&siblings, idx - 1, -1);
            let after = significant(&siblings, idx + 1, 1);
            let len = siblings.len() as isize;
            if before < 0 && after >= len {
                replace_node(&p, nodes);
            } else if before < 0 {
                set_children(&p, siblings[after as usize..].to_vec());
                // add_previous_sibling
                if let Some(grand) = parent_of(&p) {
                    for node in &nodes {
                        node.parent.set(Some(std::rc::Rc::downgrade(&grand)));
                    }
                    let mut children = grand.children.borrow_mut();
                    if let Some(i) = children.iter().position(|c| std::rc::Rc::ptr_eq(c, &p)) {
                        children.splice(i..i, nodes);
                    }
                }
            } else if after >= len {
                set_children(&p, siblings[..=before as usize].to_vec());
                insert_after(&p, nodes);
            } else {
                let rest = shallow_clone(&p);
                set_children(&p, siblings[..=before as usize].to_vec());
                set_children(&rest, siblings[after as usize..].to_vec());
                insert_after(&p, vec![rest]);
                insert_after(&p, nodes);
            }
            detach(element);
        }
        _ => replace_node(element, nodes),
    }
}

/// What one marked link becomes.
enum Outcome {
    /// Over the limit: the marking class goes.
    Unmark,
    /// The onebox's HTML in place of the link.
    Onebox(String),
    /// An inline onebox: the title, if any, then the loading class goes.
    Inline(Option<String>),
}

/// The links marked for a onebox (`a.onebox`) or an inline one
/// (`.inline-onebox-loading`), in document order, and how many oneboxes
/// the html has already.
fn marked_links(dom: &RcDom) -> (Vec<Handle>, i64) {
    let elements = all_elements(dom);
    let existing = elements
        .iter()
        .filter(|e| {
            (element_name(e) == Some("aside") && has_class(e, "onebox"))
                || (element_name(e) == Some("a") && has_class(e, "inline-onebox"))
        })
        .count() as i64;
    let links = elements
        .into_iter()
        .filter(|e| {
            (element_name(e) == Some("a") && has_class(e, "onebox"))
                || has_class(e, "inline-onebox-loading")
        })
        .collect();
    (links, existing)
}

/// Whether `post_process_oneboxes` puts a onebox in (`@has_oneboxes`, for
/// the FirstOnebox badge): a block onebox within the limit.
pub fn has_oneboxes(html: &str, max_oneboxes: i64) -> bool {
    let dom = parse(html);
    let (links, existing) = marked_links(&dom);
    let mut limit = max_oneboxes - existing;
    let mut seen: Vec<(String, bool)> = Vec::new();
    for link in links {
        let Some(url) = attr(&link, "href").filter(|h| !h.is_empty()) else {
            continue;
        };
        let is_onebox = attr(&link, "class").as_deref() == Some("onebox");
        let key = (url, is_onebox);
        if limit <= 0 && !seen.contains(&key) {
            continue;
        }
        limit -= 1;
        if is_onebox {
            return true;
        }
        seen.push(key);
    }
    false
}

/// `post_process_oneboxes` up to the image pass: each marked link,
/// within `max_oneboxes_per_post`.
pub async fn post_process(
    conn: &mut PgConnection,
    site: &Site<'_>,
    html: &str,
    opts: &Opts,
) -> Result<String, AppError> {
    // The links, read without holding the document across the lookups.
    let (links, existing): (Vec<(Option<String>, bool)>, i64) = {
        let dom = parse(html);
        let (links, existing) = marked_links(&dom);
        let links = links
            .iter()
            .map(|e| {
                (
                    attr(e, "href").filter(|h| !h.is_empty()),
                    attr(e, "class").as_deref() == Some("onebox"),
                )
            })
            .collect();
        (links, existing)
    };
    let mut limit = site.settings.get("max_oneboxes_per_post")?.to_i() - existing;
    let mut oneboxes: Vec<String> = Vec::new();
    let mut inlines: Vec<String> = Vec::new();
    let mut outcomes: Vec<Option<Outcome>> = Vec::new();
    for (href, is_onebox) in links {
        let Some(url) = href else {
            outcomes.push(None);
            continue;
        };
        let seen = if is_onebox {
            oneboxes.contains(&url)
        } else {
            inlines.contains(&url)
        };
        if limit <= 0 && !seen {
            outcomes.push(Some(Outcome::Unmark));
            continue;
        }
        limit -= 1;
        if is_onebox {
            oneboxes.push(url.clone());
            outcomes.push(Some(Outcome::Onebox(onebox(conn, site, &url, opts).await?)));
        } else {
            inlines.push(url.clone());
            outcomes.push(Some(Outcome::Inline(
                inline_title(conn, site, &url, opts).await?,
            )));
        }
    }
    let max_width = site.settings.get("max_image_width")?.to_f();
    let max_height = site.settings.get("max_image_height")?.to_f();
    let dom = parse(html);
    let (elements, _) = marked_links(&dom);
    // The onebox documents, kept until the cooked is serialized: an RcDom
    // clears its nodes' children when dropped.
    let mut fragments: Vec<RcDom> = Vec::new();
    for (element, outcome) in elements.iter().zip(outcomes) {
        let is_onebox = attr(element, "class").as_deref() == Some("onebox");
        match outcome {
            None => {}
            Some(Outcome::Unmark) => remove_class(
                element,
                if is_onebox {
                    "onebox"
                } else {
                    "inline-onebox-loading"
                },
            ),
            Some(Outcome::Onebox(html)) => {
                let fragment = parse(&html);
                apply(element, &fragment);
                fragments.push(fragment);
            }
            Some(Outcome::Inline(title)) => {
                // process_inline_onebox
                if let Some(title) = title {
                    set_children(element, vec![new_text(&title)]);
                    add_class(element, "inline-onebox");
                }
                remove_class(element, "inline-onebox-loading");
            }
        }
    }
    oneboxed_images(&dom, max_width, max_height)?;
    let out = crate::pretty_text::cleanup::to_html(&dom);
    drop(fragments);
    Ok(out)
}

/// `ImageSizer.resize`
fn resize(w: i64, h: i64, max_width: f64, max_height: f64) -> (i64, i64) {
    let (w, h) = (w as f64, h as f64);
    if w <= max_width && h <= max_height {
        return (w.floor() as i64, h.floor() as i64);
    }
    let ratio = (max_width / w).min(max_height / h);
    ((w * ratio).floor() as i64, (h * ratio).floor() as i64)
}

fn ancestors(node: &Handle) -> Vec<Handle> {
    let mut out = Vec::new();
    let mut current = parent_of(node);
    while let Some(p) = current {
        current = parent_of(&p);
        out.push(p);
    }
    out
}

/// `post_process_oneboxes`' pass over the oneboxed images (`.onebox-body
/// img, .onebox img, img.onebox`): sized (`limit_size!`), then a square
/// one in a generic or topic onebox marked an avatar, a small one shown
/// whole, any other wrapped to keep its aspect ratio.
fn oneboxed_images(dom: &RcDom, max_width: f64, max_height: f64) -> Result<(), Unsupported> {
    let images: Vec<Handle> = all_elements(dom)
        .into_iter()
        .filter(|e| {
            element_name(e) == Some("img")
                && (has_class(e, "onebox")
                    || ancestors(e)
                        .iter()
                        .any(|a| has_class(a, "onebox") || has_class(a, "onebox-body")))
        })
        .collect();
    for img in images {
        if attr(&img, "src").is_none_or(|s| s.trim().is_empty()) {
            continue;
        }
        let num = |name: &str| attr(&img, name).map_or(0, |v| crate::ruby::to_i(&v));
        let (w, h) = (num("width"), num("height"));
        if w <= 0 || h <= 0 {
            // Sized from the upload or by fetching the image.
            return Err(Unsupported("oneboxed images without their size"));
        }
        let (w, h) = resize(w, h, max_width, max_height);
        set_attr(&img, "width", &w.to_string());
        set_attr(&img, "height", &h.to_string());
        if has_class(&img, "onebox-avatar") {
            continue;
        }
        let mut parent = parent_of(&img);
        if parent.as_ref().and_then(element_name) == Some("a") {
            parent = parent.as_ref().and_then(parent_of);
        }
        let parent_class = parent
            .as_ref()
            .and_then(|p| attr(p, "class"))
            .unwrap_or_default();
        if parent_class.contains("onebox-body") {
            if w == h {
                let found = ancestors(&img).iter().any(|a| {
                    attr(a, "class").is_some_and(|c| {
                        c.split(|ch: char| !ch.is_alphanumeric())
                            .any(|word| word == "allowlistedgeneric" || word == "discoursetopic")
                    })
                });
                if found {
                    add_class(&img, "onebox-avatar");
                    continue;
                }
            }
            if w < 64 && h < 64 {
                add_class(&img, "onebox-full-image");
            } else {
                if let NodeData::Element { attrs, .. } = &img.data {
                    attrs
                        .borrow_mut()
                        .retain(|a| &*a.name.local != "width" && &*a.name.local != "height");
                }
                let wrapper = crate::pretty_text::cleanup::new_element(
                    "div",
                    &[
                        ("class", "aspect-image"),
                        ("style", &format!("--aspect-ratio:{w}/{h};")),
                    ],
                );
                insert_after(&img, vec![wrapper.clone()]);
                detach(&img);
                set_children(&wrapper, vec![img.clone()]);
            }
        } else if ["instagram-images", "tweet-images", "scale-images"]
            .iter()
            .any(|c| parent_class.contains(c))
        {
            return Err(Unsupported("oneboxed images of other sites"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_the_onebox_routes() {
        let topic = |id: &str, n: Option<&str>| {
            Some(Route::Topic {
                id: id.into(),
                post_number: n.map(Into::into),
            })
        };
        assert_eq!(recognize("/t/some-slug/41"), topic("41", None));
        assert_eq!(recognize("/t/some-slug/41/2"), topic("41", Some("2")));
        assert_eq!(recognize("/t/41"), topic("41", None));
        assert_eq!(recognize("/t/41/3"), topic("41", Some("3")));
        assert_eq!(
            recognize("/u/user2/summary"),
            Some(Route::User {
                username: "user2".into()
            })
        );
        assert_eq!(
            recognize("/c/general/sub-general/34/l/latest"),
            Some(Route::Category {
                slug_path_with_id: "general/sub-general/34".into()
            })
        );
        assert_eq!(recognize("/latest"), Some(Route::Other));
        assert_eq!(
            recognize("/uploads/default/original/1X/a.mp4"),
            Some(Route::Upload)
        );
        assert_eq!(recognize("/no-such-page"), None);
    }

    #[test]
    fn renders_sections_as_standalone_lines() {
        let t = "<div>\n  {{#name}}\n    <b>{{name}}</b>\n  {{/name}}\n  {{{raw}}}\n</div>\n";
        assert_eq!(
            render(
                t,
                &[
                    ("name", Value::Text("A & B".into())),
                    ("raw", Value::Text("<i>x</i>".into())),
                ]
            ),
            "<div>\n    <b>A &amp; B</b>\n  <i>x</i>\n</div>\n"
        );
        assert_eq!(
            render(
                t,
                &[
                    ("name", Value::Text(String::new())),
                    ("raw", Value::Text(String::new()))
                ]
            ),
            "<div>\n  \n</div>\n"
        );
    }
}
