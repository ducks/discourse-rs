//! Port of app/controllers/topics_controller.rb#show for anonymous users,
//! JSON only for now.

use askama::Template;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Html;
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
) -> Result<Response, AppError> {
    let (id, json) = split_format(&id);
    show(&headers, &state, None, &id, None, &params, json).await
}

/// `/t/:slug/:topic_id`
pub async fn show_with_slug(
    State(state): State<AppState>,
    Path((slug, id)): Path<(String, String)>,
    Query(params): Query<ShowParams>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (id, json) = split_format(&id);
    show(&headers, &state, Some(&slug), &id, None, &params, json).await
}

/// `/t/:slug/:topic_id/:post_number`
pub async fn show_post(
    State(state): State<AppState>,
    Path((slug, id, post_number)): Path<(String, String, String)>,
    Query(params): Query<ShowParams>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (post_number, json) = split_format(&post_number);
    show(
        &headers,
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

fn not_found(state: &AppState) -> Response {
    let i18n = &state.i18n;
    let body = json!({
        "errors": [i18n.t("not_found").unwrap_or("The requested URL or resource could not be found.")],
        "error_type": "not_found",
        // build_not_found_page's HTML isn't ported.
        "extras": {
            "title": i18n.t("page_not_found.page_title").unwrap_or("Page Not Found"),
            "html": "",
            "group": null,
        },
    });
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

async fn show(
    headers: &HeaderMap,
    state: &AppState,
    slug: Option<&str>,
    id: &str,
    post_number: Option<&str>,
    params: &ShowParams,
    json: bool,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let base_path = state.config.globals.relative_url_root().to_string();
    let format = if json { ".json" } else { "" };

    // Discourse::InvalidParameters for non-scalar ids can't happen here.
    let page = crate::ruby::to_i(params.page.as_deref().unwrap_or(""));
    if page < 0 {
        return Ok(not_found(state));
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
                None => not_found(state),
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
        Err(TopicViewError::NotFound) => return Ok(not_found(state)),
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
    let page = crate::html::topic_page(&mut conn, &state.i18n, site, &rendered.json, page).await?;
    Ok(Html(page.render().map_err(crate::html::HtmlError::from)?).into_response())
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
