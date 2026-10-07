//! `rescue_discourse_actions(:not_found)` for a browser: where a handler
//! answers the JSON not_found 404 to a request that does not show JSON
//! errors (not `.json`, not XHR, no JSON Accept), Rails renders the
//! not-found page instead. It goes in the port's own layout, where Rails
//! uses the no_ember one, as the port's other pages do.
//!
//! The page searches for the topic route's slug or id; elsewhere for
//! nothing (Rails takes `params[:slug] || params[:id]` of any route).
//! Permalinks, which Rails checks first, are not ported.

use askama::Template;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::html::{Chrome, Crawler};
use crate::session::current::Incoming;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

#[derive(Template)]
#[template(path = "not_found.html")]
struct NotFoundPage {
    site_title: String,
    lang: String,
    base_path: String,
    crawler: Crawler,
    viewer: Option<crate::html::Viewer>,
    bus_position: String,
    chrome: Chrome,
    page_title: String,
    html: String,
}

/// `show_json_errors`: format json or XHR.
fn shows_json_errors(path: &str, headers: &HeaderMap) -> bool {
    let has = |name: &str, needle: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains(needle))
    };
    path.ends_with(".json")
        || has("x-requested-with", "XMLHttpRequest")
        || has(header::ACCEPT.as_str(), "application/json")
}

pub async fn html_errors(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let wants_page = request.method() == Method::GET
        && !shows_json_errors(request.uri().path(), request.headers());
    if !wants_page {
        return Ok(next.run(request).await);
    }
    let headers = request.headers().clone();
    let uri = request.uri().clone();
    let guardian = request
        .extensions()
        .get::<Incoming>()
        .map(|i| i.guardian.clone())
        .unwrap_or_else(crate::guardian::Guardian::anonymous);
    let response = next.run(request).await;
    let json_not_found = response.status() == StatusCode::NOT_FOUND
        && response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|v| v.as_bytes().starts_with(b"application/json"));
    if !json_not_found {
        return Ok(response);
    }
    let base_path = state.config.globals.relative_url_root().to_string();
    let path = uri
        .path()
        .strip_prefix(base_path.as_str())
        .unwrap_or(uri.path());
    let slug = crate::not_found_page::topic_show_slug(path).unwrap_or_default();

    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let page = crate::not_found_page::NotFound {
        forbidden: false,
        custom_message: None,
        slug: &slug,
        guardian: &guardian,
    };
    let html =
        crate::not_found_page::build(&mut conn, &settings, &state.i18n, &urls, &page).await?;
    drop(conn);

    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site = crate::html::Site::from_settings(&settings, &base_path)?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(&state, &settings, &guardian, crate::sidebar::Active::None)
        .await?;
    let mut crawler = Crawler::for_request(&urls, &uri, None)?;
    crawler.description = String::new();
    let body = NotFoundPage {
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        crawler,
        viewer: site.viewer,
        bus_position: String::new(),
        chrome: site.chrome,
        page_title: state
            .i18n
            .t("page_not_found.page_title")
            .unwrap_or("Page Not Found")
            .to_string(),
        html,
    }
    .render()
    .map_err(crate::html::HtmlError::from)?;
    let response = (StatusCode::NOT_FOUND, axum::response::Html(body)).into_response();
    Ok(crate::html::with_viewer_headers(response, &vs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_errors_for_json_and_xhr() {
        let mut headers = HeaderMap::new();
        assert!(!shows_json_errors("/t/nope", &headers));
        assert!(shows_json_errors("/t/nope.json", &headers));
        headers.insert("x-requested-with", "XMLHttpRequest".parse().unwrap());
        assert!(shows_json_errors("/t/nope", &headers));
    }
}
