//! The styleguide plugin's engine, mounted at /styleguide:
//! StyleguideController#index for `/` and `/:category/:section`
//! (requires_plugin, then a 404 for whoever `can_see_styleguide?` refuses),
//! with the page the client draws there (crate::plugins::styleguide::view).

use std::sync::LazyLock;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, Uri};
use axum::response::Response;
use regex::Regex;

use crate::guardian::Guardian;
use crate::plugins::styleguide::{self, view};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// GET /styleguide
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    serve(&state, &guardian, None, &headers, &uri).await
}

/// GET /styleguide/:category/:section
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((_category, section)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    serve(&state, &guardian, Some(section), &headers, &uri).await
}

async fn serve(
    state: &AppState,
    guardian: &Guardian,
    section: Option<String>,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !styleguide::can_see(guardian, &settings)? {
        return Ok(super::topics::not_found_response(state));
    }
    // routes/styleguide/show.js: an unknown section replaces itself with
    // the 404 route.
    let section = match section {
        Some(id) => match styleguide::section_by_id(&id) {
            Some(section) => Some(section),
            None => return Ok(super::topics::not_found_response(state)),
        },
        None => None,
    };
    // StyleguideIcons: the sprite's symbols.
    let theme_id = Some(settings.get("default_theme_id")?.to_i());
    let sprite = crate::svg_sprite::for_site(&mut conn, state, &settings, theme_id).await?;
    static SYMBOL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"<symbol id="([^"]+)""#).unwrap());
    let mut icons: Vec<String> = SYMBOL
        .captures_iter(&sprite.bundle)
        .map(|c| c[1].to_string())
        .collect();
    icons.sort();
    icons.dedup();

    let frame = super::groups::frame(
        state,
        &mut conn,
        &settings,
        guardian,
        headers,
        uri,
        crate::sidebar::Active::Route("styleguide".into()),
    )
    .await?;
    let main = view::page(
        &view::Cx {
            i18n: &state.i18n,
            base_path: state.config.globals.relative_url_root(),
            icons: &icons,
        },
        section,
    );
    super::groups::finish(frame, &settings, String::new(), main)
}
