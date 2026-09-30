//! Port of app/controllers/topics_controller.rb#show for anonymous users,
//! JSON only for now.

use askama::Template;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::topic_view::{CHUNK_SIZE, Options, TopicView, TopicViewError};
use crate::url::Urls;
use crate::{AppError, AppState};

#[derive(Deserialize, Default)]
pub struct ShowParams {
    page: Option<String>,
}

/// `/t/:id` and `/t/:slug` (the id route also catches a bare slug).
pub async fn show_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<ShowParams>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    let (id, json) = split_format(&id);
    show(
        Incoming {
            headers: &headers,
            uri: &uri,
        },
        &state,
        None,
        &id,
        None,
        &params,
        json,
    )
    .await
}

/// `/t/:slug/:topic_id`
pub async fn show_with_slug(
    State(state): State<AppState>,
    Path((slug, id)): Path<(String, String)>,
    Query(params): Query<ShowParams>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    let (id, json) = split_format(&id);
    show(
        Incoming {
            headers: &headers,
            uri: &uri,
        },
        &state,
        Some(&slug),
        &id,
        None,
        &params,
        json,
    )
    .await
}

/// `/t/:slug/:topic_id/:post_number`
pub async fn show_post(
    State(state): State<AppState>,
    Path((slug, id, post_number)): Path<(String, String, String)>,
    Query(params): Query<ShowParams>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    let (post_number, json) = split_format(&post_number);
    show(
        Incoming {
            headers: &headers,
            uri: &uri,
        },
        &state,
        Some(&slug),
        &id,
        Some(&post_number),
        &params,
        json,
    )
    .await
}

/// Rails' `(.:format)`: a trailing `.json` on the last segment.
fn split_format(segment: &str) -> (String, bool) {
    match segment.strip_suffix(".json") {
        Some(s) => (s.to_string(), true),
        None => (segment.to_string(), false),
    }
}

/// `render_json_error I18n.t(:not_found)`; `extras` only for topics#show.
pub(super) fn not_found_response(state: &AppState, with_extras: bool) -> Response {
    let i18n = &state.i18n;
    let mut body = serde_json::Map::new();
    body.insert(
        "errors".into(),
        json!([i18n
            .t("not_found")
            .unwrap_or("The requested URL or resource could not be found.")]),
    );
    body.insert("error_type".into(), json!("not_found"));
    if with_extras {
        // build_not_found_page's HTML isn't ported.
        body.insert(
            "extras".into(),
            json!({
                "title": i18n.t("page_not_found.page_title").unwrap_or("Page Not Found"),
                "html": "",
                "group": null,
            }),
        );
    }
    let body = serde_json::Value::Object(body);
    (StatusCode::NOT_FOUND, Json(body)).into_response()
}

/// Rails `redirect_to` a path: absolute with the request's host and scheme.
fn redirect(headers: &HeaderMap, location: String) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let location = format!("http://{host}{location}");
    (
        StatusCode::MOVED_PERMANENTLY,
        [(header::LOCATION, location)],
    )
        .into_response()
}

/// The parts of the request the topic page reads.
struct Incoming<'a> {
    headers: &'a HeaderMap,
    uri: &'a axum::http::Uri,
}

async fn show(
    incoming: Incoming<'_>,
    state: &AppState,
    slug: Option<&str>,
    id: &str,
    post_number: Option<&str>,
    params: &ShowParams,
    json: bool,
) -> Result<Response, AppError> {
    let Incoming { headers, uri } = incoming;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let base_path = state.config.globals.relative_url_root().to_string();
    let format = if json { ".json" } else { "" };

    // Discourse::InvalidParameters for non-scalar ids can't happen here.
    let page = crate::ruby::to_i(params.page.as_deref().unwrap_or(""));
    if page < 0 {
        return Ok(not_found_response(state, true));
    }

    // A slug in the id position (`/t/my-topic`, or `123abc`) redirects to
    // the canonical URL.
    let topic_id: i32 = match id.parse() {
        Ok(n) => n,
        Err(_) => {
            let found: Option<(i32, String)> = sqlx::query_as(
                "SELECT id, slug FROM topics WHERE slug = $1 AND deleted_at IS NULL ORDER BY id LIMIT 1",
            )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
            return Ok(match found {
                Some((id, slug)) => redirect(
                    headers,
                    canonical(&base_path, &slug, id, None, page, format),
                ),
                None => not_found_response(state, true),
            });
        }
    };
    let post_number: Option<i64> = match post_number {
        Some(n) => Some(
            n.parse()
                .map_err(|_| crate::Unsupported("non-numeric post_number"))?,
        ),
        None => None,
    };

    let guardian = Guardian::anonymous();
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let mut view = TopicView {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        options: Options { page, post_number },
    };
    let rendered = match view.render(topic_id).await {
        Ok(r) => r,
        Err(TopicViewError::NotFound) => return Ok(not_found_response(state, true)),
        Err(e) => return Err(e.into()),
    };

    // Pages past the end redirect to the last one.
    if page > 1 {
        let count = rendered.json["post_stream"]["stream"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(0) as i64;
        if (page - 1) * CHUNK_SIZE >= count {
            let last_page = ((count - 1).max(0) / CHUNK_SIZE) + 1;
            let mut location = format!("{base_path}/t/{}/{topic_id}{format}", rendered.slug);
            if last_page > 1 {
                location.push_str(&format!("?page={last_page}"));
            }
            return Ok(redirect(headers, location));
        }
    }

    // slugs_do_not_match, or a non-JSON request without a slug.
    let slug_mismatch = slug.is_some_and(|s| s != rendered.slug) || (slug.is_none() && !json);
    if slug_mismatch {
        return Ok(redirect(
            headers,
            canonical(
                &base_path,
                &rendered.slug,
                topic_id,
                post_number,
                page,
                format,
            ),
        ));
    }

    if json {
        return Ok(Json(rendered.json).into_response());
    }
    let site = crate::html::Site::from_settings(&settings, &base_path)?;
    let mut page =
        crate::html::topic_page(&mut conn, &state.i18n, site, &rendered.json, page).await?;
    page.crawler = topic_crawler(
        &mut conn,
        state,
        &settings,
        uri,
        &page.canonical_url,
        &rendered.json,
    )
    .await?;
    let body = page.render().map_err(crate::html::HtmlError::from)?;
    Ok(crate::html::crawler_response(
        body,
        &page.crawler,
        &settings,
    )?)
}

/// The crawler block of topics/show: canonical_url from the topic view,
/// the description from the topic's stored excerpt else the first shown
/// post's summary, the topic image else the site's.
async fn topic_crawler(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    uri: &axum::http::Uri,
    canonical_path: &str,
    view: &serde_json::Value,
) -> Result<crate::html::Crawler, AppError> {
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let canonical = format!("{}{canonical_path}", urls.base_url_no_prefix()?);
    let title = view["title"].as_str().unwrap_or("").to_string();
    // TopicView#summary: 500 characters of the first post, links stripped,
    // entities as text, newlines as spaces.
    let first_cooked = view["post_stream"]["posts"][0]["cooked"]
        .as_str()
        .unwrap_or("");
    let summary = crate::excerpt::excerpt(
        first_cooked,
        500,
        &crate::excerpt::Options {
            strip_links: true,
            text_entities: true,
            image_mode: crate::excerpt::ImageMode::Strip,
            ..Default::default()
        },
    )
    .replace('\n', " ")
    .trim()
    .to_string();
    let topic_id = view["id"].as_i64().unwrap_or(0) as i32;
    let stored: Option<Option<String>> =
        sqlx::query_scalar("SELECT excerpt FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_optional(&mut *conn)
            .await?;
    // plain_text_excerpt: the stored excerpt without tags, entities decoded.
    let plain = stored
        .flatten()
        .map(|e| html_escape::decode_html_entities(&strip_tags(&e)).into_owned())
        .filter(|e| !e.is_empty());
    let description = plain.unwrap_or_else(|| summary.clone());
    let image = match view["image_url"].as_str().filter(|u| !u.is_empty()) {
        Some(u) => Some(urls.absolute(u)?),
        None => crate::html::site_opengraph_image(&mut *conn, &urls).await?,
    };
    let mut crawler = crate::html::Crawler::for_request(&urls, uri, Some(canonical))?
        .with_meta(&title, &summary, image);
    crawler.description = description;
    Ok(crawler)
}

/// `gsub(/<[^>]*>/, "")`
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// `redirect_to_correct_topic`'s target.
fn canonical(
    base_path: &str,
    slug: &str,
    id: i32,
    post_number: Option<i64>,
    page: i64,
    format: &str,
) -> String {
    let mut url = format!("{base_path}/t/{slug}/{id}");
    if let Some(n) = post_number.filter(|n| *n > 0) {
        url.push_str(&format!("/{n}"));
    }
    url.push_str(format);
    if page > 0 {
        url.push_str(&format!("?page={page}"));
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_urls() {
        assert_eq!(canonical("", "a-b", 5, None, 0, ".json"), "/t/a-b/5.json");
        assert_eq!(
            canonical("/f", "a-b", 5, Some(3), 2, ""),
            "/f/t/a-b/5/3?page=2"
        );
        assert_eq!(split_format("35.json"), ("35".to_string(), true));
        assert_eq!(split_format("35"), ("35".to_string(), false));
    }
}
