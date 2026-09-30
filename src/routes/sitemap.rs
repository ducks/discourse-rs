//! Port of app/controllers/sitemap_controller.rb and app/models/sitemap.rb.
//! Rails regenerates the `sitemaps` rows hourly in a job; the index here
//! regenerates them on request, and recent/news touch their row as the
//! controller does.

use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use chrono::NaiveDateTime;
use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

const RECENT: &str = "recent";
const NEWS: &str = "news";
/// id, slug, bumped_at, updated_at, posts_count
type UrlRow = (
    i32,
    String,
    Option<NaiveDateTime>,
    NaiveDateTime,
    Option<i32>,
);
/// `TopicView::CHUNK_SIZE`
const CHUNK_SIZE: i64 = 20;

fn xmlschema(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `Sitemap#sitemap_topics`' base scope.
const INDEXABLE: &str = "FROM topics INNER JOIN categories ON categories.id = topics.category_id \
    WHERE topics.deleted_at IS NULL AND topics.visible = TRUE AND categories.read_restricted = FALSE";

/// `Sitemap.touch(name)`: upsert the row with its newest topic time (or
/// three days ago), enabled.
async fn touch(
    conn: &mut PgConnection,
    name: &str,
    page_size: i64,
) -> Result<NaiveDateTime, sqlx::Error> {
    let last: Option<NaiveDateTime> = match name {
        RECENT => {
            sqlx::query_scalar(&format!(
                "SELECT max(topics.updated_at) {INDEXABLE} AND topics.bumped_at > now() - interval '3 days'"
            ))
            .fetch_one(&mut *conn)
            .await?
        }
        NEWS => {
            sqlx::query_scalar(&format!(
                "SELECT max(topics.updated_at) {INDEXABLE} AND topics.bumped_at > now() - interval '72 hours'"
            ))
            .fetch_one(&mut *conn)
            .await?
        }
        page => {
            let offset = (page.parse::<i64>().unwrap_or(1) - 1) * page_size;
            sqlx::query_scalar(&format!(
                "SELECT max(t.updated_at) FROM (SELECT topics.updated_at {INDEXABLE} \
                 ORDER BY topics.id ASC LIMIT $1 OFFSET $2) t"
            ))
            .bind(page_size)
            .bind(offset)
            .fetch_one(&mut *conn)
            .await?
        }
    };
    let last_posted_at: NaiveDateTime = sqlx::query_scalar(
        "INSERT INTO sitemaps (name, last_posted_at, enabled) \
         VALUES ($1, COALESCE($2, now() - interval '3 days'), TRUE) \
         ON CONFLICT (name) DO UPDATE SET last_posted_at = EXCLUDED.last_posted_at, enabled = TRUE \
         RETURNING last_posted_at",
    )
    .bind(name)
    .bind(last)
    .fetch_one(&mut *conn)
    .await?;
    Ok(last_posted_at)
}

/// `Sitemap.regenerate_sitemaps`
async fn regenerate(conn: &mut PgConnection, page_size: i64) -> Result<(), sqlx::Error> {
    let mut names = vec![RECENT.to_string(), NEWS.to_string()];
    touch(conn, RECENT, page_size).await?;
    touch(conn, NEWS, page_size).await?;
    let count: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(topic_count), 0)::bigint FROM categories WHERE read_restricted = FALSE")
            .fetch_one(&mut *conn)
            .await?;
    let mut size = count / page_size;
    if count % page_size > 0 {
        size += 1;
    }
    for index in 0..size {
        let name = (index + 1).to_string();
        touch(conn, &name, page_size).await?;
        names.push(name);
    }
    sqlx::query("UPDATE sitemaps SET enabled = FALSE WHERE NOT (name = ANY($1))")
        .bind(&names)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `build_sitemap_topic_url`
fn topic_url(
    base_url: &str,
    slug: &str,
    id: i32,
    posts_count: Option<i32>,
    chunk_size: i64,
) -> String {
    let url = format!("{base_url}/t/{slug}/{id}");
    let Some(count) = posts_count else {
        return url;
    };
    let count = i64::from(count);
    let mut page = count / chunk_size;
    if count % chunk_size > 0 {
        page += 1;
    }
    if page > 1 {
        format!("{url}?page={page}")
    } else {
        url
    }
}

struct Context {
    settings: SiteSettings,
    base_url: String,
}

async fn context(state: &AppState, conn: &mut PgConnection) -> Result<Option<Context>, AppError> {
    let settings =
        SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_sitemap")?.truthy() {
        return Ok(None);
    }
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let base_url = urls.base_url()?;
    Ok(Some(Context { settings, base_url }))
}

fn xml(body: String, content_type: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}

/// GET /sitemap.xml
pub async fn index(State(state): State<AppState>) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let Some(ctx) = context(&state, &mut conn).await? else {
        return Ok(super::topics::not_found_response(&state, false));
    };
    let page_size = ctx.settings.get("sitemap_page_size")?.to_i().max(1);
    regenerate(&mut conn, page_size).await?;
    let rows: Vec<(String, NaiveDateTime)> = sqlx::query_as(
        "SELECT name, last_posted_at FROM sitemaps WHERE enabled = TRUE AND name <> $1 ORDER BY id",
    )
    .bind(NEWS)
    .fetch_all(&mut *conn)
    .await?;
    let mut body = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<sitemapindex xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for (name, last_posted_at) in rows {
        body.push_str(&format!(
            "      <sitemap>\n         <loc>{}/sitemap_{name}.xml</loc>\n         <lastmod>{}</lastmod>\n      </sitemap>\n",
            ctx.base_url,
            xmlschema(last_posted_at)
        ));
    }
    body.push_str("</sitemapindex>\n");
    Ok(xml(body, "application/xml; charset=utf-8"))
}

fn urlset(base_url: &str, topics: &[UrlRow], chunk_size: i64) -> String {
    let mut body = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"\n    \
xmlns:image=\"http://www.google.com/schemas/sitemap-image/1.1\"\n    \
xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"\n    \
xsi:schemaLocation=\"http://www.sitemaps.org/schemas/sitemap/0.9\n        \
http://www.sitemaps.org/schemas/sitemap/0.9/sitemap.xsd\n        \
http://www.google.com/schemas/sitemap-image/1.1\n        \
http://www.google.com/schemas/sitemap-image/1.1/sitemap-image.xsd\">\n",
    );
    for (id, slug, bumped_at, updated_at, posts_count) in topics {
        body.push_str(&format!(
            "      <url>\n        <loc>{}</loc>\n        <lastmod>{}</lastmod>\n      </url>\n",
            xml_escape(&topic_url(base_url, slug, *id, *posts_count, chunk_size)),
            xmlschema(bumped_at.unwrap_or(*updated_at))
        ));
    }
    body.push_str("</urlset>\n");
    body
}

/// GET /sitemap_{page}.xml
pub async fn page(
    State(state): State<AppState>,
    Path(page): Path<String>,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let Some(ctx) = context(&state, &mut conn).await? else {
        return Ok(super::topics::not_found_response(&state, false));
    };
    let Some(page) = page.strip_suffix(".xml") else {
        return Ok(super::topics::not_found_response(&state, false));
    };
    if page == RECENT {
        return recent(state, conn, ctx).await;
    }
    let Ok(number) = page.parse::<i64>() else {
        return Ok(super::topics::not_found_response(&state, false));
    };
    if number < 1 || number.to_string() != page {
        return Ok(super::topics::not_found_response(&state, false));
    }
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sitemaps WHERE enabled = TRUE AND name = $1)",
    )
    .bind(page)
    .fetch_one(&mut *conn)
    .await?;
    if !exists {
        return Ok(super::topics::not_found_response(&state, false));
    }
    let page_size = ctx.settings.get("sitemap_page_size")?.to_i().max(1);
    let topics: Vec<UrlRow> = sqlx::query_as(&format!(
        "SELECT topics.id, topics.slug, topics.bumped_at, topics.updated_at, NULL::int {INDEXABLE} \
         ORDER BY topics.id ASC LIMIT $1 OFFSET $2"
    ))
    .bind(page_size)
    .bind((number - 1) * page_size)
    .fetch_all(&mut *conn)
    .await?;
    let chunk = CHUNK_SIZE;
    Ok(xml(
        urlset(&ctx.base_url, &topics, chunk),
        "text/xml; charset=UTF-8",
    ))
}

/// GET /sitemap_recent.xml
async fn recent(
    state: AppState,
    mut conn: sqlx::pool::PoolConnection<sqlx::Postgres>,
    ctx: Context,
) -> Result<Response, AppError> {
    let page_size = ctx.settings.get("sitemap_page_size")?.to_i().max(1);
    touch(&mut conn, RECENT, page_size).await?;
    let topics: Vec<UrlRow> = sqlx::query_as(&format!(
        "SELECT topics.id, topics.slug, topics.bumped_at, topics.updated_at, topics.posts_count {INDEXABLE} \
         AND topics.bumped_at > now() - interval '3 days' ORDER BY topics.bumped_at DESC"
    ))
    .fetch_all(&mut *conn)
    .await?;
    let chunk = CHUNK_SIZE;
    let _ = state;
    Ok(xml(
        urlset(&ctx.base_url, &topics, chunk),
        "text/xml; charset=UTF-8",
    ))
}

/// GET /news.xml
pub async fn news(State(state): State<AppState>) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let Some(ctx) = context(&state, &mut conn).await? else {
        return Ok(super::topics::not_found_response(&state, false));
    };
    let page_size = ctx.settings.get("sitemap_page_size")?.to_i().max(1);
    touch(&mut conn, NEWS, page_size).await?;
    let locale = ctx.settings.get("default_locale")?.to_s().to_lowercase();
    let locale = if locale.starts_with("zh") {
        locale.replacen('_', "-", 1)
    } else {
        locale.split('_').next().unwrap_or("").to_string()
    };
    let title = ctx.settings.get("title")?.to_s();
    let topics: Vec<(i32, String, String, NaiveDateTime)> = sqlx::query_as(&format!(
        "SELECT topics.id, topics.title, topics.slug, topics.created_at {INDEXABLE} \
         AND topics.bumped_at > now() - interval '72 hours' ORDER BY topics.bumped_at DESC"
    ))
    .fetch_all(&mut *conn)
    .await?;
    let chunk = CHUNK_SIZE;
    let mut body = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"\n    \
xmlns:news=\"http://www.google.com/schemas/sitemap-news/0.9\"\n    \
xmlns:image=\"http://www.google.com/schemas/sitemap-image/1.1\"\n    \
xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"\n    \
xsi:schemaLocation=\"http://www.sitemaps.org/schemas/sitemap/0.9\n        \
http://www.sitemaps.org/schemas/sitemap/0.9/sitemap.xsd\n        \
http://www.google.com/schemas/sitemap-news/0.9\n        \
http://www.google.com/schemas/sitemap-news/0.9/sitemap-news.xsd\n        \
http://www.google.com/schemas/sitemap-image/1.1\n        \
http://www.google.com/schemas/sitemap-image/1.1/sitemap-image.xsd\">\n",
    );
    for (id, topic_title, slug, created_at) in &topics {
        body.push_str(&format!(
            "      <url>\n          <loc>{}</loc>\n          <news:news>\n              <news:publication>\n                  <news:name>{}</news:name>\n                  <news:language>{locale}</news:language>\n              </news:publication>\n              <news:publication_date>{}</news:publication_date>\n              <news:title>{}</news:title>\n          </news:news>\n      </url>\n",
            xml_escape(&topic_url(&ctx.base_url, slug, *id, None, chunk)),
            xml_escape(&title),
            xmlschema(*created_at),
            xml_escape(topic_title)
        ));
    }
    body.push_str("</urlset>\n");
    Ok(xml(body, "text/xml; charset=UTF-8"))
}
