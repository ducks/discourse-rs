//! SvgSpriteController: `/svg-sprite/:hostname/svg-:theme_id-:version.js`
//! (the site's sprite as a script), `/svg-sprite/picker-search` (the icon
//! picker's pages), `/svg-sprite/search/:keyword` (one symbol) and
//! `/svg-sprite/:hostname/icon(/:color)/:name.svg` (one icon as an SVG).

use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::svg_sprite;
use crate::{AppError, AppState};

/// The `[\w\.-]+` the routes take a hostname as.
fn valid_hostname(hostname: &str) -> bool {
    !hostname.is_empty()
        && hostname
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// `[-a-z0-9\s\%]+`: an icon name as the routes take it.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| {
            c.is_ascii_lowercase()
                || c.is_ascii_digit()
                || c == '-'
                || c == '%'
                || c.is_whitespace()
        })
}

async fn site_settings(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// GET /svg-sprite/:hostname/svg-:theme_id-:version.js: the site the
/// hostname names (the dispatcher picks it); a version other than the
/// current one redirects to the current one.
pub async fn show(
    State(state): State<AppState>,
    Path((hostname, file)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // theme_id: /(-?[0-9]+)?/, version: /\h{40}/.
    let Some((theme, version)) = file
        .strip_prefix("svg-")
        .and_then(|f| f.strip_suffix(".js"))
        .and_then(|f| f.rsplit_once('-'))
    else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let theme_id = match theme {
        "" => None,
        t => match t.parse::<i64>() {
            Ok(id) => Some(id),
            Err(_) => return Ok(StatusCode::NOT_FOUND.into_response()),
        },
    };
    if version.len() != 40
        || !version.chars().all(|c| c.is_ascii_hexdigit())
        || !valid_hostname(&hostname)
    {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let mut conn = state.pool.acquire().await?;
    let settings = site_settings(&state, &mut conn).await?;
    let built = svg_sprite::for_site(&mut conn, &state, &settings, theme_id).await?;
    if version != built.version {
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
            svg_sprite::path(&urls.current_hostname()?, theme_id, &built.version)
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
    let mut response = svg_sprite::script(&built).into_response();
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

/// GET /svg-sprite/picker-search: a page of the icon picker's icons for
/// the site's theme, filtered by `filter`; `only_available` (default
/// true) keeps to the sprite's icons, otherwise the rest follow with
/// their symbols. Members only; a weak ETag answers a repeat with 304.
pub async fn picker_search(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if guardian.user().is_none() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let query: Vec<(String, String)> = form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
        .into_owned()
        .collect();
    let param = |k: &str| query.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    let filter = param("filter").unwrap_or_default();
    // ActiveModel's boolean cast of `only_available.presence || true`.
    let only_available = match param("only_available").filter(|v| !v.is_empty()) {
        None => true,
        Some(v) => !matches!(v.to_ascii_lowercase().as_str(), "0" | "f" | "false" | "off"),
    };
    // fetch_page_from_params(max: 1000)
    let page = match param("page") {
        None => 0,
        Some(p) => match crate::ruby::integer(p) {
            Some(n) if (0..=1000).contains(&n) => n as usize,
            _ => return Ok(super::search::invalid_parameters(&state, "page")),
        },
    };
    let mut conn = state.pool.acquire().await?;
    let settings = site_settings(&state, &mut conn).await?;
    let theme_id = Some(settings.get("default_theme_id")?.to_i());
    let built = svg_sprite::for_site(&mut conn, &state, &settings, theme_id).await?;
    let etag = {
        use sha1::{Digest, Sha1};
        format!(
            "W/\"{:x}\"",
            Sha1::digest(format!(
                "{}|{filter}|{only_available}|{page}",
                built.version
            ))
        )
    };
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag))
    {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response());
    }
    let body = svg_sprite::picker_search(&built, filter, only_available, page);
    // `render json:` escapes the symbols' markup as ActiveSupport does.
    Ok((
        [
            (header::ETAG, etag),
            (
                header::CACHE_CONTROL,
                "max-age=0, private, must-revalidate".into(),
            ),
            (
                header::CONTENT_TYPE,
                "application/json; charset=utf-8".into(),
            ),
        ],
        crate::web_hooks::to_json(&body),
    )
        .into_response())
}

/// GET /svg-sprite/search/:keyword: the symbol, inspected, as text; 404
/// for none. Members only.
pub async fn search(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(keyword): Path<String>,
) -> Result<Response, AppError> {
    if !valid_name(&keyword) {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    if guardian.user().is_none() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    Ok(match svg_sprite::search(&keyword) {
        Some(symbol) => (
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            crate::ruby::inspect_string(symbol),
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    })
}

/// GET /svg-sprite/:hostname/icon/:name.svg and with a color: the icon as
/// an SVG document, filled with the color.
pub async fn svg_icon(Path(params): Path<Vec<(String, String)>>) -> Result<Response, AppError> {
    let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    let Some(name) = get("name").and_then(|n| n.strip_suffix(".svg")) else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let color = get("color");
    // color: /(\h{3}{1,2})/
    if !valid_hostname(get("hostname").unwrap_or_default())
        || !valid_name(name)
        || color
            .is_some_and(|c| !matches!(c.len(), 3 | 6) || !c.chars().all(|c| c.is_ascii_hexdigit()))
    {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let Some(symbol) = svg_sprite::search(name) else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    Ok((
        [
            (header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
            (header::CACHE_CONTROL, "max-age=86400, public, immutable"),
        ],
        svg_sprite::icon_document(symbol, color),
    )
        .into_response())
}
