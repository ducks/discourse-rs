//! The color definitions stylesheets (Stylesheet::Manager's
//! `color_definitions_*`): the default theme's color scheme, or the base
//! light palette, and its dark scheme when it has one.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::color_scheme::ColorScheme;
use crate::site_settings::SiteSettings;
use crate::stylesheet::manager;
use crate::{AppError, AppState};

/// A scheme's color definitions (None: the theme's own palette, else the
/// base one) under the default theme, compiled (crate::stylesheet).
/// None for a scheme that is gone.
async fn scheme_css(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    id: Option<i32>,
) -> Result<Option<String>, AppError> {
    if let Some(id) = id
        && ColorScheme::find(conn, id).await?.is_none()
    {
        return Ok(None);
    }
    let cx = manager::Cx {
        config: &state.config,
        settings,
    };
    Ok(Some(
        manager::color_definitions(conn, &cx, None, id).await? + "\n",
    ))
}

/// The stylesheet, revalidated by its ETag on every use: it follows the
/// color scheme in the database, so it is never cached for good.
fn stylesheet(headers: &HeaderMap, body: Option<String>) -> Response {
    match body {
        Some(body) => {
            let digest = crate::assets::digest_of(body.as_bytes());
            crate::assets::cached(headers, "text/css; charset=utf-8", body, &digest, false)
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The light scheme: the default theme's, else the base palette.
pub async fn light(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let theme_id = settings.get("default_theme_id")?.to_i();
    // theme.color_scheme: a scheme that is gone (or a remote copy) is the
    // base palette, as Stylesheet::Manager falls back to it.
    let scheme_id: Option<i32> = sqlx::query_scalar(
        "SELECT cs.id FROM themes t \
         JOIN color_schemes cs ON cs.id = t.color_scheme_id AND NOT cs.remote_copy \
         WHERE t.id = $1",
    )
    .bind(theme_id as i32)
    .fetch_optional(&mut *conn)
    .await?;
    let body = scheme_css(&state, &mut conn, &settings, scheme_id).await?;
    Ok(stylesheet(&headers, body))
}

/// The default theme's dark scheme (`dark_scheme_id`), if it has one.
pub async fn dark(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let theme_id = settings.get("default_theme_id")?.to_i();
    let id: Option<i32> =
        sqlx::query_scalar("SELECT dark_color_scheme_id FROM themes WHERE id = $1")
            .bind(theme_id as i32)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
    match id.filter(|id| *id > 0) {
        Some(id) => {
            let body = scheme_css(&state, &mut conn, &settings, Some(id)).await?;
            Ok(stylesheet(&headers, body))
        }
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}

/// `/assets/color_definitions_scheme/:id.css`: any other scheme a page
/// links (a member's own light or dark palette, crate::color_mode).
pub async fn scheme(
    State(state): State<AppState>,
    Path(file): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(id) = file
        .strip_suffix(".css")
        .and_then(|id| id.parse::<i32>().ok())
    else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let body = scheme_css(&state, &mut conn, &settings, Some(id)).await?;
    Ok(stylesheet(&headers, body))
}

/// `/stylesheets/:target_:theme_id_:digest.css` (StylesheetsController#
/// show for the theme targets, `_rtl` ones too): a theme's compiled
/// stylesheet, whatever digest the link named (the port keeps no older
/// ones).
pub async fn theme(
    State(state): State<AppState>,
    Path(file): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // StylesheetsController#show_source_map: grass makes no source maps.
    if let Some(stem) = file.strip_suffix(".css.map")
        && manager::THEME_TARGETS
            .iter()
            .any(|t| stem.starts_with(&format!("{t}_")))
    {
        return Err(crate::Unsupported("theme stylesheet source maps (169b)").into());
    }
    let Some(name) = file.strip_suffix(".css") else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let Some((target, rest)) = manager::THEME_TARGETS
        .iter()
        .find_map(|t| name.strip_prefix(&format!("{t}_")).map(|rest| (*t, rest)))
    else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let (target, rest) = match rest.strip_prefix("rtl_") {
        Some(rest) => (format!("{target}_rtl"), rest),
        None => (target.to_string(), rest),
    };
    let theme_id = rest.split('_').next().and_then(|id| id.parse::<i32>().ok());
    let Some(theme_id) = theme_id else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let cx = manager::Cx {
        config: &state.config,
        settings: &settings,
    };
    let css = manager::theme_css(&mut conn, &cx, theme_id, &target).await?;
    Ok(stylesheet(&headers, css.map(|css| css + "\n")))
}

/// `/assets/wizard_fonts.css`: the wizard stylesheet's fonts
/// (manager::wizard_fonts), which Rails serves admins on every page.
pub async fn wizard_fonts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let cx = manager::Cx {
        config: &state.config,
        settings: &settings,
    };
    Ok(stylesheet(&headers, Some(manager::wizard_fonts(&cx)?)))
}

/// The discourse-fonts files this port has (static/fonts/README).
const FONT_FILES: &[(&str, &[u8])] = &[
    (
        "InterVariable.woff2",
        include_bytes!("../../static/fonts/InterVariable.woff2"),
    ),
    (
        "JetBrainsMono-Regular.woff2",
        include_bytes!("../../static/fonts/JetBrainsMono-Regular.woff2"),
    ),
    (
        "JetBrainsMono-Bold.woff2",
        include_bytes!("../../static/fonts/JetBrainsMono-Bold.woff2"),
    ),
];

/// `/fonts/:file`. The stylesheets' @font-face URLs are absolute (the base
/// URL or the CDN), and a browser loads a font from another origin only
/// with CORS: nginx.sample.conf's font location (and development.rb's
/// public_file_server headers) send `Access-Control-Allow-Origin: *`.
/// The files this port embeds, else the gem's other fonts (the admin's
/// font previews name them all) from a Discourse checkout's bundle.
pub async fn font(State(state): State<AppState>, Path(file): Path<String>) -> Response {
    let bytes: std::borrow::Cow<'static, [u8]> =
        match FONT_FILES.iter().find(|(name, _)| *name == file) {
            Some((_, bytes)) => (*bytes).into(),
            None => {
                let Some(dir) = crate::stylesheet::fonts::is_font_file(&file)
                    .then(|| state.config.fonts_dir())
                    .flatten()
                else {
                    return StatusCode::NOT_FOUND.into_response();
                };
                match tokio::fs::read(dir.join(&file)).await {
                    Ok(bytes) => bytes.into(),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        return StatusCode::NOT_FOUND.into_response();
                    }
                    Err(e) => {
                        tracing::error!("reading the font {file}: {e}");
                        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                    }
                }
            }
        };
    (
        [
            (header::CONTENT_TYPE, "font/woff2"),
            (header::CACHE_CONTROL, "max-age=31556952, public, immutable"),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        ],
        bytes,
    )
        .into_response()
}

/// `/color-scheme-stylesheet/:id(/:theme_id)` (StylesheetsController#
/// color_scheme): the stylesheet a palette preview links, as
/// `{color_scheme_id, new_href}`. A theme the viewer may use other than the
/// default one would serve its own color definitions, which the port does
/// not draw.
pub async fn color_scheme(
    State(state): State<AppState>,
    crate::session::current::AuthGuardian(guardian): crate::session::current::AuthGuardian,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
    Path(segments): Path<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let get = |name: &str| {
        segments
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    // The route's `.json` format; any other dot ends the id elsewhere.
    let id = get("id").unwrap_or_default();
    let id = id.strip_suffix(".json").unwrap_or(id);
    if id.contains('.') {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let theme_param = match get("theme_id") {
        Some(t) => {
            let t = t.strip_suffix(".json").unwrap_or(t);
            let digits = t.strip_prefix('-').unwrap_or(t);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return Ok(StatusCode::NOT_FOUND.into_response());
            }
            Some(t.to_string())
        }
        None => crate::params::parse_query(query.as_deref().unwrap_or_default())
            .get("theme_id")
            .and_then(crate::params::scalar),
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let default_theme = settings.get("default_theme_id")?.to_i();
    // `params[:theme_id]&.to_i`, nil unless guardian.allow_themes?([id],
    // include_preview: true).
    if let Some(theme_id) = theme_param.map(|t| crate::ruby::to_i(&t))
        && theme_id != default_theme
    {
        let staff_preview = guardian.is_staff()
            && sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM themes WHERE id = $1)")
                .bind(i32::try_from(theme_id).unwrap_or(0))
                .fetch_one(&mut *conn)
                .await?;
        let selectable = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM themes WHERE id = $1 AND user_selectable)",
        )
        .bind(i32::try_from(theme_id).unwrap_or(0))
        .fetch_one(&mut *conn)
        .await?;
        if staff_preview || selectable {
            return Err(crate::Unsupported(
                "color definitions of a theme other than the default (258a)",
            )
            .into());
        }
    }
    let base_path = state.config.globals.relative_url_root().to_string();
    let (scheme_id, href) = crate::color_mode::scheme_stylesheet(
        &mut conn,
        &settings,
        &base_path,
        // find_by(id:) casts as ActiveModel's integer type does (to_i).
        Some(crate::ruby::to_i(id)),
    )
    .await?;
    Ok(axum::Json(serde_json::json!({
        "color_scheme_id": scheme_id,
        "new_href": href,
    }))
    .into_response())
}
