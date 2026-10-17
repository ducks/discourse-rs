//! The admin dashboard itself: Admin::DashboardController#index, general,
//! the empty moderation/security/reports actions, the section
//! configuration and settings, and the mountable reports (available,
//! layout, bulk). Staff only (StaffConstraint, others 404); the
//! configuration, settings, available reports, layout and traffic routes
//! are under AdminConstraint (moderators 404 too).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::admin_dashboard::{self as dashboard, InvalidSetting, Layout};
use crate::reports::{self, Viewer};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

fn not_found(state: &AppState) -> Response {
    super::topics::not_found_response(state)
}

fn pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn invalid(state: &AppState, key: &str) -> Response {
    super::search::invalid_parameters(state, key)
}

/// GET /admin/dashboard(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(not_found(&state));
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let cx = reports::context(
        &mut conn,
        &settings,
        &state.site_setting_defs,
        &state.i18n,
        &urls,
    )
    .await?;
    let improvements = guardian
        .upcoming_change_enabled(&mut conn, &settings, "dashboard_improvements")
        .await?;
    if !improvements {
        let mut data = dashboard::index_data(&mut conn).await?;
        if settings.get("version_checks")?.truthy() {
            data["version_check"] = dashboard::check_version(&mut conn, &cx).await?;
        }
        return Ok(Json(data).into_response());
    }
    let start = p.get("start_date").and_then(crate::params::scalar);
    let end = p.get("end_date").and_then(crate::params::scalar);
    let doc = sections_document(
        &mut conn,
        &state,
        &guardian,
        &settings,
        &cx,
        start.as_deref(),
        end.as_deref(),
    )
    .await?;
    Ok(Json(doc).into_response())
}

/// The redesigned dashboard's document: the visible sections built for the
/// range, the problems, and for admins the configuration.
async fn sections_document(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    settings: &SiteSettings,
    cx: &reports::Cx<'_>,
    start: Option<&str>,
    end: Option<&str>,
) -> Result<Value, AppError> {
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let viewer = Viewer {
        guardian: Some(guardian),
    };
    let all = dashboard::sections(&mut *conn, cx).await?;
    let visible: Vec<String> = all
        .iter()
        .filter(|(_, v)| *v)
        .map(|(id, _)| id.clone())
        .collect();
    let sections = dashboard::build_sections(&mut *conn, cx, &viewer, &visible, start, end).await?;
    let env = crate::problem_checks::Env {
        settings,
        defs: &state.site_setting_defs,
        config: &state.config,
        i18n: &state.i18n,
    };
    let problems = crate::problem_checks::serialized_problems(&mut *conn, &env, &urls).await?;
    let mut out = Map::new();
    out.insert("sections".into(), sections);
    out.insert("problems".into(), problems);
    if guardian.is_admin() {
        // The configuration as it stands after the sections were built.
        let all = dashboard::sections(&mut *conn, cx).await?;
        out.insert(
            "configuration".into(),
            json!({ "sections": dashboard::sections_json(&all) }),
        );
    }
    Ok(Value::Object(out))
}

/// What GET /admin/dashboard.json serves the redesigned dashboard for the
/// range, as the admin page asks for it.
pub async fn document(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    settings: &SiteSettings,
    start: &str,
    end: &str,
) -> Result<Value, AppError> {
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let cx = reports::context(
        &mut *conn,
        settings,
        &state.site_setting_defs,
        &state.i18n,
        &urls,
    )
    .await?;
    sections_document(conn, state, guardian, settings, &cx, Some(start), Some(end)).await
}

/// GET /admin/dashboard/general(.json)
pub async fn general(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(not_found(&state));
    }
    let mut conn = state.pool.acquire().await?;
    Ok(Json(dashboard::general_data(&mut conn).await?).into_response())
}

/// GET /admin/dashboard/{moderation,security,reports}(.json): empty
/// actions (the Ember pages' JSON).
pub async fn empty(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(not_found(&state));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// The admin routes' checks: AdminConstraint, then CSRF for writes.
fn admin_write(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    p: &Map<String, Value>,
    method: &str,
) -> Option<Response> {
    if !guardian.is_admin() {
        return Some(not_found(state));
    }
    if !super::session::csrf_ok(state, headers, &pairs(p), uri.path(), method) {
        return Some(super::session::bad_csrf());
    }
    None
}

/// PUT /admin/dashboard/configuration(.json)
pub async fn update_configuration(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if let Some(r) = admin_write(&state, &guardian, &headers, &uri, &p, "PUT") {
        return Ok(r);
    }
    let sections: Vec<Value> = match p.get("sections") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err(Unsupported("dashboard sections that aren't a list").into()),
    };
    let user_id = guardian.user_id().ok_or(Unsupported("anonymous staff"))?;
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let cx = reports::Cx {
        settings: &settings,
        defs: &state.site_setting_defs,
        i18n: &state.i18n,
        urls: &urls,
        tertiary: None,
    };
    dashboard::update_configuration(&mut tx, &cx, &sections, user_id).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// PUT /admin/dashboard/sections/:section_id/settings/:setting_key(.json)
pub async fn update_section_settings(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((section_id, key)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if let Some(r) = admin_write(&state, &guardian, &headers, &uri, &p, "PUT") {
        return Ok(r);
    }
    let key = key.strip_suffix(".json").unwrap_or(&key).to_string();
    // params.permit: lists of scalars only.
    let mut permitted = Map::new();
    for name in ["category_ids", "groups"] {
        if let Some(Value::Array(items)) = p.get(name)
            && items.iter().all(|i| {
                matches!(
                    i,
                    Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null
                )
            })
        {
            permitted.insert(name.into(), Value::Array(items.clone()));
        }
    }
    let mut tx = state.pool.begin().await?;
    match dashboard::update_setting(&mut tx, &section_id, &key, &permitted).await? {
        Ok(()) => {
            tx.commit().await?;
            Ok(StatusCode::NO_CONTENT.into_response())
        }
        Err(InvalidSetting(param)) => Ok(invalid(&state, param)),
    }
}

/// GET /admin/dashboard/reports/available(.json)
pub async fn available_reports(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(not_found(&state));
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    let search = p.get("search").and_then(crate::params::scalar);
    let cursor = match p.get("cursor") {
        Some(Value::Object(c)) => {
            let get = |k: &str| {
                c.get(k)
                    .and_then(crate::params::scalar)
                    .filter(|s| !crate::ruby::is_blank(s))
            };
            match (get("title"), get("key")) {
                (Some(t), Some(k)) => Some((t, k)),
                _ => None,
            }
        }
        _ => None,
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let cx = reports::context(
        &mut conn,
        &settings,
        &state.site_setting_defs,
        &state.i18n,
        &urls,
    )
    .await?;
    let viewer = Viewer {
        guardian: Some(&guardian),
    };
    let enabled = dashboard::reports_section(&mut conn, &cx, &viewer, search.as_deref()).await?;
    let listing = dashboard::listing(&cx, &viewer, cursor, search.as_deref())?;
    Ok(Json(json!({
        "providers": listing["providers"],
        "enabled": enabled["items"],
        "available": listing["items"],
        "has_more": listing["has_more"],
        "cursor": listing["cursor"],
    }))
    .into_response())
}

/// PUT /admin/dashboard/reports/layout(.json)
pub async fn update_reports_layout(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if let Some(r) = admin_write(&state, &guardian, &headers, &uri, &p, "PUT") {
        return Ok(r);
    }
    let Some(items) = dashboard::parse_items(p.get("items")) else {
        return Ok(invalid(&state, "items"));
    };
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let cx = reports::Cx {
        settings: &settings,
        defs: &state.site_setting_defs,
        i18n: &state.i18n,
        urls: &urls,
        tertiary: None,
    };
    let viewer = Viewer {
        guardian: Some(&guardian),
    };
    match dashboard::update_layout(&mut tx, &cx, &viewer, &items).await? {
        Layout::Saved => {
            tx.commit().await?;
            Ok(StatusCode::NO_CONTENT.into_response())
        }
        Layout::InvalidItems => Ok(invalid(&state, "items")),
        Layout::InvalidAccess => Ok(super::search::invalid_access(&state)),
    }
}

/// POST /admin/dashboard/reports/bulk(.json)
pub async fn bulk_reports(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(not_found(&state));
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "POST") {
        return Ok(super::session::bad_csrf());
    }
    let Some(items) = dashboard::parse_items(p.get("items")) else {
        return Ok(invalid(&state, "items"));
    };
    // params.permit(filters: [:start_date, :end_date])
    let mut filters = Map::new();
    if let Some(Value::Object(f)) = p.get("filters") {
        for k in ["start_date", "end_date"] {
            if let Some(v) = f.get(k).filter(|v| crate::params::scalar(v).is_some()) {
                filters.insert(k.into(), v.clone());
            }
        }
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let cx = reports::context(
        &mut conn,
        &settings,
        &state.site_setting_defs,
        &state.i18n,
        &urls,
    )
    .await?;
    let viewer = Viewer {
        guardian: Some(&guardian),
    };
    let out = dashboard::bulk_fetch(&mut conn, &cx, &viewer, &items, &filters).await?;
    Ok(Json(out).into_response())
}

/// GET /admin/dashboard/site-traffic-explorer(.json): admins, with the
/// dashboard improvements.
pub async fn traffic(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    use crate::admin_traffic_explorer as explorer;
    if !guardian.is_admin() {
        return Ok(not_found(&state));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !guardian
        .upcoming_change_enabled(&mut conn, &settings, "dashboard_improvements")
        .await?
    {
        return Ok(not_found(&state));
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    let params = match explorer::parse(&p)? {
        Ok(params) => params,
        Err(explorer::Invalid(key)) => {
            return Ok(invalid(
                &state,
                key.unwrap_or("Discourse::InvalidParameters"),
            ));
        }
    };
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let cx = explorer::Cx {
        settings: &settings,
        i18n: &state.i18n,
        hostname: urls.current_hostname()?,
    };
    Ok(match explorer::load(&mut conn, &cx, &params).await? {
        explorer::Outcome::Traffic(traffic) => Json(traffic).into_response(),
        explorer::Outcome::Timeout => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error_type": "traffic_query_timeout"})),
        )
            .into_response(),
    })
}
