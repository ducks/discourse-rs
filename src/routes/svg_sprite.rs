//! SvgSpriteController#show: `/svg-sprite/:hostname/svg-:theme_id-:version.js`,
//! the page chrome's sprite as a script (crate::svg_sprite::chrome_script).

use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::site_settings::SiteSettings;
use crate::svg_sprite;
use crate::{AppError, AppState};

/// GET /svg-sprite/:hostname/svg-:theme_id-:version.js: the site the
/// hostname names (the dispatcher picks it); a version other than the
/// current one redirects to the current one.
pub async fn show(
    State(state): State<AppState>,
    Path((hostname, file)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some((theme_id, version)) = file
        .strip_prefix("svg-")
        .and_then(|f| f.strip_suffix(".js"))
        .and_then(|f| f.rsplit_once('-'))
        .and_then(|(theme, version)| Some((theme.parse::<i64>().ok()?, version)))
    else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    if !hostname
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    if version != svg_sprite::chrome_version() {
        let mut conn = state.pool.acquire().await?;
        let settings =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
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
            svg_sprite::chrome_path(&urls.current_hostname()?, theme_id)
        );
        return Ok((
            StatusCode::FOUND,
            [
                (header::LOCATION, location),
                (header::CACHE_CONTROL, "no-cache, no-store".into()),
            ],
        )
            .into_response());
    }
    let mut response = svg_sprite::chrome_script().into_response();
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
    Ok(response)
}
