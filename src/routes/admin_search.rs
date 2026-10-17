//! Admin::SearchController (90g9): `/admin/search/all.json` and
//! `/admin/search.json`, Admin::Search::List's settings (basic attributes),
//! themes and components, reports and upcoming changes, for admins; the
//! search page as HTML; and with `fragment` the data source's formatted
//! items (AdminSearchDataSource#buildMap), which admin-search.js scores.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::admin_pages::{Shell, wants_json};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, params};

/// Admin::Search::List::RESULT_FILTER_STATUSES
const RESULT_FILTER_STATUSES: [&str; 4] = ["experimental", "alpha", "beta", "stable"];

/// What Admin::Search::List finds.
struct Found {
    settings: Vec<Value>,
    themes_and_components: Vec<Value>,
    reports: Value,
    upcoming_changes: Vec<Value>,
}

/// Admin::Search::List: the settings for the contract's filters, every
/// theme by name, the reports and the upcoming changes (whose listing
/// saves the admin's last visit), or None when no setting is left
/// (`model :settings` fails).
async fn list(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    p: &Map<String, Value>,
) -> Result<Option<Found>, AppError> {
    let area = params::string(p, "filter_area").filter(|a| !a.is_empty());
    let filters = crate::admin_site_settings::Filters {
        categories: super::admin_site_settings::param_list(p, "categories"),
        plugin: params::string(p, "plugin").filter(|v| !v.is_empty()),
        names: super::admin_site_settings::param_list(p, "filter_names"),
        include_locale_setting: area.is_none() || area.as_deref() == Some("localization"),
        area,
        basic: true,
        ..Default::default()
    };
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let cx = crate::admin_site_settings::Context {
        defs: &state.site_setting_defs,
        settings,
        i18n: &state.i18n,
        globals: &state.config.globals,
        base_path: state.config.globals.relative_url_root(),
        urls: &urls,
    };
    let found_settings =
        crate::admin_site_settings::all_settings(&mut *conn, &cx, &filters).await?;
    if found_settings.is_empty() {
        return Ok(None);
    }
    let theme_cx = crate::themes::Ctx {
        settings,
        i18n: &state.i18n,
        urls: &urls,
        defs: &state.site_setting_defs,
    };
    let mut themes = Vec::new();
    for theme in crate::themes::Theme::where_ordered(&mut *conn, "TRUE").await? {
        themes.push(Value::Object(
            crate::themes::basic(&mut *conn, &theme_cx, &theme).await?,
        ));
    }
    let reports = super::admin_reports::list_doc(&mut *conn, state, settings, guardian).await?;
    let statuses: Vec<String> = RESULT_FILTER_STATUSES
        .iter()
        .map(|s| s.to_string())
        .collect();
    let upcoming_changes =
        super::admin_config_upcoming_changes::list_for(&mut *conn, state, settings, &statuses)
            .await?;
    if let Some(user_id) = guardian.user_id() {
        crate::upcoming_changes::update_last_visited(&mut *conn, user_id).await?;
    }
    Ok(Some(Found {
        settings: found_settings,
        themes_and_components: themes,
        reports,
        upcoming_changes,
    }))
}

/// GET /admin/search(.json) and /admin/search/all(.json): StaffConstraint,
/// then the list for an admin (InvalidAccess for a moderator), the page
/// as HTML, or the data source's items with `fragment`.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    let query = params::parse_query(uri.query().unwrap_or(""));
    let fragment = query.contains_key("fragment");
    let page = uri
        .path()
        .trim_end_matches(".json")
        .ends_with("/admin/search");
    if !fragment && !wants_json(&uri, &headers) {
        // `render body: nil` for /admin/search/all; the page otherwise.
        if !page {
            return Ok(StatusCode::OK.into_response());
        }
        let mut conn = state.pool.acquire().await?;
        let shell = Shell::load(&mut conn, &state, &guardian, &headers, "").await?;
        let cx = shell.cx(&state);
        let filter = params::string(&query, "filter").unwrap_or_default();
        let main = crate::admin_view::contents(
            "admin-search",
            &crate::admin_search_view::page(&cx, &filter),
        );
        let title = crate::admin_view::t(cx.i18n, "admin.search.title");
        return shell.render(&state, &uri, &title, main, "", StatusCode::OK);
    }
    if !guardian.is_admin() {
        return Ok(super::search::invalid_access(&state));
    }
    let p = params::parse(uri.query(), &headers, &body);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some(found) = list(&mut conn, &state, &settings, &guardian, &p).await? else {
        // An unhandled failure of the service: no response from the block.
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    if fragment {
        let shell = Shell::load(&mut conn, &state, &guardian, &headers, "").await?;
        let cx = shell.cx(&state);
        let mut items = crate::admin_search::pages(&cx)?;
        items.extend(crate::admin_search::setting_items(&cx, &found.settings));
        let (themes, components) =
            crate::admin_search::theme_items(&cx, &found.themes_and_components);
        items.extend(themes);
        items.extend(components);
        let reports = found.reports.as_array().cloned().unwrap_or_default();
        items.extend(crate::admin_search::report_items(&cx, &reports));
        items.extend(crate::admin_search::upcoming_change_items(
            &cx,
            &found.upcoming_changes,
        ));
        let items: Vec<Value> = items.iter().map(|i| i.json()).collect();
        return Ok(Json(json!({ "items": items })).into_response());
    }
    Ok(Json(json!({
        "settings": found.settings,
        "themes_and_components": found.themes_and_components,
        "reports": found.reports,
        "upcoming_changes": found.upcoming_changes,
    }))
    .into_response())
}

pub(super) fn routes(mut router: axum::Router<AppState>) -> axum::Router<AppState> {
    use axum::routing::get;
    for path in [
        "/admin/search",
        "/admin/search.json",
        "/admin/search/all",
        "/admin/search/all.json",
    ] {
        router = router.route(path, get(index));
    }
    router
}
