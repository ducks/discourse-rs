//! Port of Admin::PluginsController (staff: StaffConstraint): the visible
//! plugins (crate::admin_plugins), and one by name. A page load draws the
//! installed plugins list or a plugin's settings
//! (crate::admin_plugins_view).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// GET /admin/plugins(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    let mut conn = state.pool.acquire().await?;
    if super::admin_email::html_request(&uri, &headers) {
        let shell = super::admin_pages::Shell::load(
            &mut conn,
            &state,
            &guardian,
            &headers,
            "admin_installed_plugins",
        )
        .await?;
        let cx = shell.cx(&state);
        let plugins = crate::admin_plugins::all(&shell.settings)?;
        let filter: String = form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
            .find(|(k, _)| k == "filter")
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default();
        let main = format!(
            "{}{}",
            crate::admin_plugins_view::strings(&cx),
            crate::admin_plugins_view::render(&crate::admin_plugins_view::index(
                &cx,
                &plugins,
                guardian.is_admin(),
                &filter
            ))
        );
        let main = crate::admin_ui::contents("admin-plugins", "", &main);
        let title = crate::admin_view::t(&state.i18n, "admin.plugins.title");
        return shell.render(
            &state,
            &uri,
            &title,
            main,
            "admin-plugins.js",
            StatusCode::OK,
        );
    }
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    Ok(Json(json!({ "plugins": crate::admin_plugins::all(&settings)? })).into_response())
}

/// GET /admin/plugins/:plugin_id(.json) and its /settings: the plugin, a
/// 404 for none. The page is the plugin's settings when they are its only
/// admin page (the show route's default); a plugin's own configuration
/// pages are its port's.
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(params): Path<Vec<(String, String)>>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    let id = params
        .iter()
        .find(|(k, _)| k == "plugin_id")
        .map(|(_, v)| v.trim_end_matches(".json").to_string())
        .unwrap_or_default();
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some(plugin) = crate::admin_plugins::find(&settings, &id)? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !super::admin_email::html_request(&uri, &headers) {
        return Ok(Json(plugin).into_response());
    }
    let plugin_id = plugin["id"].as_str().unwrap_or_default().to_string();
    // The sidebar marks the plugin's own link.
    let shell = super::admin_pages::Shell::load(
        &mut conn,
        &state,
        &guardian,
        &headers,
        &format!(
            "admin_plugin_{}",
            plugin["name"].as_str().unwrap_or_default()
        ),
    )
    .await?;
    let cx = shell.cx(&state);
    // A configuration nav's tabs, for the plugins whose pages the port
    // draws.
    let tabs = crate::plugins::admin_config_nav(&cx, &plugin_id);
    if crate::admin_plugins::has_config_nav(&plugin_id) && tabs.is_none() {
        return Err(Unsupported("a plugin's own admin pages (its configuration nav)").into());
    }
    let area = super::admin_advanced::area_settings(
        &mut conn,
        &state,
        &shell,
        &uri,
        json!({ "plugin": plugin_id }),
    )
    .await?;
    let main = format!(
        "{}{}",
        crate::admin_plugins_view::strings(&cx),
        crate::admin_plugins_view::render(&crate::admin_plugins_view::settings_with_tabs(
            &cx, &plugin, &area, tabs
        ))
    );
    let main = format!(
        "{main}<script src=\"{}\"></script>",
        crate::assets::url(&shell.site.base_path, "admin-plugins.js")
    );
    let main = crate::admin_ui::contents("admin-plugins settings", "", &main);
    let title = format!(
        "{} - {} - {}",
        crate::admin_view::t(&state.i18n, "admin.plugins.change_settings_short"),
        crate::admin_plugins_view::name_titleized(&cx, &plugin),
        crate::admin_view::t(&state.i18n, "admin.plugins.title"),
    );
    shell.render(
        &state,
        &uri,
        &title,
        main,
        "site-settings.js",
        StatusCode::OK,
    )
}
