//! Permalinks (app/models/permalink.rb, PermalinkGuardian): old URLs an
//! import or a slug change left behind, each sending readers to one topic,
//! post, category, tag, user or outside URL.

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::NaiveDateTime;
use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::topic_guardian::{PostCtx, TopicCtx};
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Permalink {
    pub id: i32,
    pub url: String,
    pub topic_id: Option<i32>,
    pub post_id: Option<i32>,
    pub category_id: Option<i32>,
    pub tag_id: Option<i32>,
    pub user_id: Option<i32>,
    pub external_url: Option<String>,
    pub created_at: NaiveDateTime,
}

const COLUMNS: &str =
    "id, url, topic_id, post_id, category_id, tag_id, user_id, external_url, created_at";

/// `Permalink.normalize_url`: stripped, one leading `/` dropped, then the
/// `permalink_normalizations` rules (Ruby regexes, not ported).
pub fn normalize_url(settings: &SiteSettings, url: &str) -> Result<String, AppError> {
    if !settings.get("permalink_normalizations")?.to_s().is_empty() {
        return Err(Unsupported("permalink_normalizations (Ruby regex rules)").into());
    }
    let url = crate::ruby::strip(url);
    Ok(url.strip_prefix('/').unwrap_or(url).to_string())
}

/// `Permalink.find_by_url`: by the normalized url, not encoded.
pub async fn find_by_url(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    url: &str,
) -> Result<Option<Permalink>, AppError> {
    let url = normalize_url(settings, url)?;
    Ok(
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM permalinks WHERE url = $1"))
            .bind(url)
            .fetch_optional(&mut *conn)
            .await?,
    )
}

pub async fn find(conn: &mut PgConnection, id: i32) -> Result<Option<Permalink>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {COLUMNS} FROM permalinks WHERE id = $1"))
        .bind(id)
        .fetch_optional(&mut *conn)
        .await
}

/// `Permalink.filter_by(url)`: newest first, at most 100, the filter
/// matched anywhere in the url or the outside URL.
pub async fn filter_by(
    conn: &mut PgConnection,
    filter: Option<&str>,
) -> Result<Vec<Permalink>, sqlx::Error> {
    match filter.filter(|f| !crate::ruby::is_blank(f)) {
        Some(f) => {
            sqlx::query_as(&format!(
                "SELECT {COLUMNS} FROM permalinks WHERE url ILIKE $1 OR external_url ILIKE $1 \
                 ORDER BY created_at DESC LIMIT 100"
            ))
            .bind(format!("%{f}%"))
            .fetch_all(&mut *conn)
            .await
        }
        None => {
            sqlx::query_as(&format!(
                "SELECT {COLUMNS} FROM permalinks ORDER BY created_at DESC LIMIT 100"
            ))
            .fetch_all(&mut *conn)
            .await
        }
    }
}

/// A live (not trashed) topic's slug and title.
async fn live_topic(
    conn: &mut PgConnection,
    id: i32,
) -> Result<Option<(String, String)>, sqlx::Error> {
    sqlx::query_as("SELECT slug, title FROM topics WHERE id = $1 AND deleted_at IS NULL")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await
}

/// A live post's topic id, number, and its live topic's slug and title.
async fn live_post(
    conn: &mut PgConnection,
    id: i32,
) -> Result<Option<(i32, i32, Option<(String, String)>)>, sqlx::Error> {
    let post: Option<(i32, i32)> = sqlx::query_as(
        "SELECT topic_id, post_number FROM posts WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((topic_id, number)) = post else {
        return Ok(None);
    };
    Ok(Some((topic_id, number, live_topic(conn, topic_id).await?)))
}

/// `Topic.relative_url`: `/t/<slug>/<id>`, the slug left out when blank.
fn topic_path(base_path: &str, slug: &str, id: i32) -> String {
    if slug.is_empty() {
        format!("{base_path}/t/{id}")
    } else {
        format!("{base_path}/t/{slug}/{id}")
    }
}

/// `Post#url`: `/t/<slug>/<topic id>/<number>` (`/404` without a topic).
fn post_path(topic: Option<&(String, String)>, topic_id: i32, number: i32) -> String {
    match topic {
        Some((slug, _)) => format!("/t/{slug}/{topic_id}/{number}"),
        None => "/404".to_string(),
    }
}

/// `UrlHelper.encode_component` for a username.
fn encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'.' | b'~')
            || b"!*'();:@&=+$,/?#[]".contains(&b)
        {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

async fn username(conn: &mut PgConnection, id: i32) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await
}

/// `Permalink#target_url`: the outside URL (a site path gets the base
/// path), else the first association still there.
pub async fn target_url(
    conn: &mut PgConnection,
    base_path: &str,
    p: &Permalink,
) -> Result<Option<String>, AppError> {
    if let Some(external) = &p.external_url {
        let mut chars = external.chars();
        let relative = chars.next() == Some('/') && chars.next().is_some_and(|c| c != '/');
        return Ok(Some(if relative {
            format!("{base_path}{external}")
        } else {
            external.clone()
        }));
    }
    if let Some(id) = p.post_id
        && let Some((topic_id, number, topic)) = live_post(conn, id).await?
    {
        return Ok(Some(format!(
            "{base_path}{}",
            post_path(topic.as_ref(), topic_id, number)
        )));
    }
    if let Some(id) = p.topic_id
        && let Some((slug, _)) = live_topic(conn, id).await?
    {
        return Ok(Some(topic_path(base_path, &slug, id)));
    }
    if let Some(id) = p.category_id
        && let Some(c) = crate::category::Category::find(conn, id).await?
    {
        return Ok(Some(c.url(conn, base_path).await?));
    }
    if let Some(id) = p.tag_id
        && let Some(t) = crate::tags::Tag::find(conn, id).await?
    {
        return Ok(Some(t.url(base_path)));
    }
    if let Some(id) = p.user_id
        && let Some(name) = username(conn, id).await?
    {
        return Ok(Some(format!("{base_path}/u/{}", encode_component(&name))));
    }
    Ok(None)
}

/// `can_see_permalink_target?`: by whichever target is set; a target
/// that is gone (or trashed) is not seen.
pub async fn can_see_target(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    p: &Permalink,
) -> Result<bool, AppError> {
    if p.external_url.is_some() {
        return Ok(true);
    }
    if let Some(id) = p.topic_id {
        let Some(topic) = TopicCtx::load(&mut *conn, settings, guardian, id).await? else {
            return Ok(false);
        };
        if topic.deleted_at.is_some() {
            return Ok(false);
        }
        let secure = guardian.secure_category_ids(&mut *conn, settings).await?;
        return Ok(guardian.can_see_topic(settings, &topic, true, &secure)?);
    }
    if let Some(id) = p.post_id {
        let Some(post) = PostCtx::load(&mut *conn, id).await? else {
            return Ok(false);
        };
        if post.trashed() {
            return Ok(false);
        }
        if guardian.is_admin() {
            return Ok(true);
        }
        let topic_id: i32 = sqlx::query_scalar("SELECT topic_id FROM posts WHERE id = $1")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
        let topic_can_see = match TopicCtx::load(&mut *conn, settings, guardian, topic_id).await? {
            Some(topic) if topic.deleted_at.is_none() => {
                let secure = guardian.secure_category_ids(&mut *conn, settings).await?;
                guardian.can_see_topic(settings, &topic, true, &secure)?
            }
            _ => false,
        };
        return Ok(guardian.can_see_post(settings, &post, topic_can_see)?);
    }
    if let Some(id) = p.category_id {
        return Ok(crate::onebox::can_see_category(conn, settings, guardian, id).await?);
    }
    if let Some(id) = p.tag_id {
        if !settings.get("tagging_enabled")?.truthy() {
            return Ok(false);
        }
        return Ok(match crate::tags::Tag::find(conn, id).await? {
            Some(tag) => tag.visible_to(conn, guardian).await?,
            None => false,
        });
    }
    if let Some(id) = p.user_id {
        // can_see_user? is true for any user.
        return Ok(username(conn, id).await?.is_some());
    }
    Ok(false)
}

/// The permalink's target for this viewer: found, visible and pointing
/// somewhere.
pub async fn visible_target(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    base_path: &str,
    guardian: &Guardian,
    url: &str,
) -> Result<Option<String>, AppError> {
    let Some(p) = find_by_url(conn, settings, url).await? else {
        return Ok(None);
    };
    if !can_see_target(conn, settings, guardian, &p).await? {
        return Ok(None);
    }
    target_url(conn, base_path, &p).await
}

/// `redirect_to target, status: :moved_permanently`: a site path made
/// absolute with the request's host.
pub fn moved_permanently(headers: &HeaderMap, target: &str) -> Response {
    let location = if target.starts_with('/') && !target.starts_with("//") {
        let host = headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("localhost");
        format!("http://{host}{target}")
    } else {
        target.to_string()
    };
    (
        StatusCode::MOVED_PERMANENTLY,
        [(header::LOCATION, location)],
    )
        .into_response()
}

/// `redirect_with_client_support(target, status: 301)`: an XHR gets the
/// URL as text with `Discourse-Xhr-Redirect`, anything else the 301.
pub fn redirect_with_client_support(headers: &HeaderMap, target: &str) -> Response {
    let xhr = headers
        .get("x-requested-with")
        .is_some_and(|v| v == "XMLHttpRequest");
    if xhr {
        return (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                (
                    header::HeaderName::from_static("discourse-xhr-redirect"),
                    "true",
                ),
            ],
            target.to_string(),
        )
            .into_response();
    }
    moved_permanently(headers, target)
}

/// `rescue_discourse_actions(:not_found, check_permalinks: true)`: the
/// redirect when `path` has a permalink this viewer may follow.
pub async fn check(
    state: &AppState,
    guardian: &Guardian,
    headers: &HeaderMap,
    path: &str,
) -> Result<Option<Response>, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let base_path = state.config.globals.relative_url_root();
    Ok(
        visible_target(&mut conn, &settings, base_path, guardian, path)
            .await?
            .map(|target| redirect_with_client_support(headers, &target)),
    )
}

/// The url as `before_validation :normalize_url, :encode_url` leaves it.
/// Addressable::URI.encode is not ported: urls it would change are refused.
pub fn url_for_write(settings: &SiteSettings, url: &str) -> Result<String, AppError> {
    let url = normalize_url(settings, url)?;
    let unchanged = url
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=".contains(c));
    if !unchanged {
        return Err(Unsupported("permalink urls Addressable::URI.encode would change").into());
    }
    Ok(url)
}

/// PermalinkSerializer.
pub async fn serialize(
    conn: &mut PgConnection,
    urls: &Urls<'_>,
    base_path: &str,
    p: &Permalink,
) -> Result<Value, AppError> {
    let base_url = urls.base_url()?;
    let topic = match p.topic_id {
        Some(id) => live_topic(conn, id).await?,
        None => None,
    };
    let post = match p.post_id {
        Some(id) => live_post(conn, id).await?,
        None => None,
    };
    let category = match p.category_id {
        Some(id) => crate::category::Category::find(conn, id).await?,
        None => None,
    };
    let category_url = match &category {
        Some(c) => Some(c.url(conn, base_path).await?),
        None => None,
    };
    let tag = match p.tag_id {
        Some(id) => crate::tags::Tag::find(conn, id).await?,
        None => None,
    };
    let user = match p.user_id {
        Some(id) => username(conn, id).await?,
        None => None,
    };
    Ok(json!({
        "id": p.id,
        "url": p.url,
        "topic_id": p.topic_id,
        "topic_title": topic.as_ref().map(|(_, title)| title),
        // Topic#url: the full URL.
        "topic_url": topic.as_ref().map(|(slug, _)| {
            format!("{}{}", urls.base_url_no_prefix().unwrap_or_default(), topic_path(base_path, slug, p.topic_id.unwrap_or(0)))
        }),
        "post_id": p.post_id,
        "post_url": post.as_ref().map(|(topic_id, number, topic)| {
            format!("{base_url}{}", post_path(topic.as_ref(), *topic_id, *number))
        }),
        "post_number": post.as_ref().map(|(_, number, _)| number),
        "post_topic_title": post.as_ref().and_then(|(_, _, t)| t.as_ref().map(|(_, title)| title)),
        "category_id": p.category_id,
        "category_name": category.as_ref().map(|c| &c.name),
        "category_url": category_url,
        "external_url": p.external_url,
        "tag_id": p.tag_id,
        "tag_name": tag.as_ref().map(|t| &t.name),
        "tag_url": tag.as_ref().map(|t| format!("{base_url}/tag/{}/{}", t.slug_for_url(), t.id)),
        "user_id": p.user_id,
        "user_url": user.as_ref().map(|u| format!("{base_url}/u/{}", encode_component(u))),
        "username": user,
    }))
}
