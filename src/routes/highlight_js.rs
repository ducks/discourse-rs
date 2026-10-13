//! HighlightJsController: `/highlight-js/:hostname/:version.js`, the site's
//! highlight.js language bundle (crate::highlight_js).

use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Months;

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, highlight_js};

/// GET /highlight-js/:hostname/:version.js. A single site serves its own
/// bundle whatever the hostname (`with_hostname(raise_on_missing: false)`);
/// a version other than the current one redirects to the current one.
pub async fn show(
    State(state): State<AppState>,
    Path((hostname, file)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // The route's constraints: a hostname of word characters, dots and
    // dashes (axum's segment already excludes `/`), and a `.js` version
    // without dots.
    let Some(requested) = file.strip_suffix(".js").filter(|v| !v.contains('.')) else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    if !hostname
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let languages = settings.get("highlighted_languages")?.to_s().to_string();
    let salt = state.config.globals.get("asset_url_salt").unwrap_or("");
    if highlight_js::version(&languages, salt) != requested {
        let urls = crate::url::Urls {
            config: &state.config,
            settings: &settings,
        };
        let host = headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("localhost");
        let location = format!(
            "http://{host}{}{}",
            state.config.globals.relative_url_root(),
            highlight_js::path(&urls.current_hostname()?, &languages, salt)
        );
        return Ok((
            StatusCode::FOUND,
            [
                (header::LOCATION, location),
                (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".into()),
                (header::CACHE_CONTROL, "no-cache, no-store".into()),
            ],
        )
            .into_response());
    }
    let body = highlight_js::bundle(languages.split('|'));
    let last_modified = crate::clock::now()
        .checked_sub_months(Months::new(120))
        .unwrap_or_default()
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string();
    let mut response = body.into_response();
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/javascript; charset=utf-8"),
    );
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("max-age=31556952, public, immutable"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    if let Ok(value) = HeaderValue::from_str(&last_modified) {
        h.insert(header::LAST_MODIFIED, value);
    }
    Ok(response)
}
