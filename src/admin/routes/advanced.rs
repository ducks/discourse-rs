//! What the advanced and customize admin pages share: an area's
//! AdminAreaSettings tab, loaded as /admin/config/site_settings.json loads
//! it for the tab's params.

use axum::http::Uri;

use super::pages::Shell;
use crate::{AppError, AppState};

/// AdminAreaSettings for `params` (`categories`, `filter_area` or
/// `plugin`, as the tab passes them), filtered by the URL's query.
pub(crate) async fn area_settings(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    shell: &Shell,
    uri: &Uri,
    params: serde_json::Value,
) -> Result<String, AppError> {
    let filters = super::pages::settings_filters(uri);
    let params = params
        .as_object()
        .ok_or_else(|| std::io::Error::other("the area's params"))?;
    let area = super::site_settings::config_filters(state, params)
        .ok_or_else(|| std::io::Error::other("the area settings' filters"))?;
    let (site_settings, default_theme) =
        super::site_settings::document(&mut *conn, state, &shell.settings, &area).await?;
    let (groups, site_categories) = super::pages::site_lists(&mut *conn, state, shell).await?;
    let cx = shell.cx(state);
    let page = crate::admin::site_settings_view::Page {
        settings: &site_settings,
        default_theme: &default_theme,
        groups: &groups,
        categories: &site_categories,
        filters: &filters,
        category: "all_results",
    };
    Ok(crate::admin::site_settings_view::area_settings(&cx, &page))
}
