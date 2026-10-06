//! What the composer's preview loads: the markdown crate as WebAssembly
//! (build.rs) and the site's render settings for it.

use axum::Json;
use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// The markdown crate as WebAssembly, built and embedded by build.rs.
const MARKDOWN_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/discourse_markdown.wasm"));

/// Its version, for the url it is cached under.
pub const MARKDOWN_WASM_VERSION: &str = env!("MARKDOWN_WASM_VERSION");

/// GET /assets/markdown.wasm: cached for good, its url carrying the
/// version.
pub async fn markdown_wasm() -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/wasm"),
            (header::CACHE_CONTROL, "max-age=31556952, public, immutable"),
        ],
        MARKDOWN_WASM,
    )
        .into_response()
}

/// GET /assets/markdown-settings.json: the RenderSettings posts cook with,
/// for the preview. A site whose settings the renderer refuses gets a 404,
/// and its composer no preview.
pub async fn markdown_settings(State(state): State<AppState>) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    match crate::pretty_text::render_settings(&settings, &state.i18n, &state.config) {
        Ok(render) => Ok(([(header::CACHE_CONTROL, "no-cache")], Json(render)).into_response()),
        Err(crate::pretty_text::CookError::Unsupported(e)) => {
            tracing::info!(error = %e, "no composer preview");
            Ok(axum::http::StatusCode::NOT_FOUND.into_response())
        }
        Err(e) => Err(e.into()),
    }
}
