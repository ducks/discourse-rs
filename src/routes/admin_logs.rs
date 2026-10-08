//! GET /admin/logs/staff_action_logs(.json): Admin::StaffActionLogsController#index.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

use serde_json::json;

use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::staff_action_logs::{self, Outcome, Params, STAFF_FILTERS};
use crate::{AppError, AppState};

/// Staff only (StaffConstraint, others 404).
pub async fn staff_action_logs(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    // Query and body params, as Rails merges them; a repeated query
    // param is already its last value.
    let params = crate::params::parse(uri.query(), &headers, &body);
    let mut query: Vec<(String, String)> = Vec::new();
    for (k, v) in &params {
        match crate::params::scalar(v) {
            Some(v) => query.push((k.clone(), v)),
            None if v.is_null() => {}
            None => {
                return Err(
                    crate::Unsupported("staff action log params that are lists or hashes").into(),
                );
            }
        }
    }
    let get = |k: &str| {
        query
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.clone())
    };
    let mut permitted: Vec<(String, String)> = Vec::new();
    for (k, _) in &query {
        if STAFF_FILTERS.contains(&k.as_str()) && !permitted.iter().any(|(p, _)| p == k) {
            permitted.push((k.clone(), get(k).unwrap_or_default()));
        }
    }
    let p = Params {
        action_id: get("action_id"),
        custom_type: get("custom_type"),
        acting_user: get("acting_user"),
        target_user: get("target_user"),
        subject: get("subject"),
        action_name: get("action_name"),
        start_date: get("start_date"),
        end_date: get("end_date"),
        page: get("page"),
        limit: get("limit"),
        permitted,
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let user_cx = crate::admin_user_show::Context {
        settings: &settings,
        defs: &state.site_setting_defs,
        i18n: &state.i18n,
        urls: &urls,
        globals: &state.config.globals,
        development: state.config.rails_env == crate::config::RailsEnv::Development,
    };
    let cx = staff_action_logs::Context {
        settings: &settings,
        user_cx: &user_cx,
        base_path: state.config.globals.relative_url_root(),
    };
    Ok(
        match staff_action_logs::index(&mut conn, &cx, &guardian, &p).await? {
            Outcome::Json(body) => Json(body).into_response(),
            Outcome::InvalidParameter(key) => super::search::invalid_parameters(&state, key),
        },
    )
}

/// Admin::StaffController: staff, others 404.
fn staff_only(state: &AppState, guardian: &crate::guardian::Guardian) -> Option<Response> {
    (!guardian.is_staff()).then(|| super::topics::not_found_response(state))
}

fn can_see_ip(guardian: &crate::guardian::Guardian, s: &SiteSettings) -> Result<bool, AppError> {
    Ok(guardian.is_admin() || (guardian.is_moderator() && s.get("moderators_view_ips")?.truthy()))
}

/// GET /admin/logs/screened_emails(.json), for those who can see emails.
pub async fn screened_emails(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    if let Some(r) = staff_only(&state, &guardian) {
        return Ok(r);
    }
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !guardian.is_admin()
        && !(s.get("moderators_view_emails")?.truthy() && guardian.is_moderator())
    {
        return Ok(super::search::invalid_access(&state));
    }
    let see_ip = can_see_ip(&guardian, &s)?;
    Ok(Json(crate::screened::emails(&mut conn, see_ip).await?).into_response())
}

/// DELETE /admin/logs/screened_emails/:id(.json)
pub async fn destroy_screened_email(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "DELETE") {
        return Ok(super::session::bad_csrf());
    }
    if let Some(r) = staff_only(&state, &guardian) {
        return Ok(r);
    }
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !guardian.is_admin()
        && !(s.get("moderators_view_emails")?.truthy() && guardian.is_moderator())
    {
        return Ok(super::search::invalid_access(&state));
    }
    // ScreenedEmail.find(params[:id].to_i)
    let id = crate::ruby::to_i(id.strip_suffix(".json").unwrap_or(&id)) as i32;
    let deleted = sqlx::query("DELETE FROM screened_emails WHERE id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    if deleted.rows_affected() == 0 {
        return Ok(super::topics::not_found_response(&state));
    }
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// GET /admin/logs/screened_urls(.json)
pub async fn screened_urls(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    if let Some(r) = staff_only(&state, &guardian) {
        return Ok(r);
    }
    let mut conn = state.pool.acquire().await?;
    Ok(Json(crate::screened::urls(&mut conn).await?).into_response())
}

/// GET /admin/logs/screened_ip_addresses(.json), for those who can see
/// IPs.
pub async fn screened_ip_addresses(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = staff_only(&state, &guardian) {
        return Ok(r);
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !can_see_ip(&guardian, &s)? {
        return Ok(super::search::invalid_access(&state));
    }
    let filter = crate::params::string(&p, "filter");
    Ok(Json(crate::screened::ip_addresses(&mut conn, filter.as_deref()).await?).into_response())
}

/// POST /admin/logs/screened_ip_addresses(.json)
pub async fn create_screened_ip(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    screened_ip_write(state, guardian, None, headers, uri, body, "POST").await
}

/// PUT /admin/logs/screened_ip_addresses/:id(.json)
pub async fn update_screened_ip(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    screened_ip_write(state, guardian, Some(id), headers, uri, body, "PUT").await
}

/// DELETE /admin/logs/screened_ip_addresses/:id(.json)
pub async fn destroy_screened_ip(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    screened_ip_write(state, guardian, Some(id), headers, uri, body, "DELETE").await
}

fn pairs(map: &serde_json::Map<String, serde_json::Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// #create, #update and #destroy after the before_actions: can_see_ip,
/// the record (404), only admins touching allow_admin rules (403).
async fn screened_ip_write(
    state: AppState,
    guardian: crate::guardian::Guardian,
    id: Option<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    method: &str,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), method) {
        return Ok(super::session::bad_csrf());
    }
    if let Some(r) = staff_only(&state, &guardian) {
        return Ok(r);
    }
    let mut tx = state.pool.begin().await?;
    let s = SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    if !can_see_ip(&guardian, &s)? {
        return Ok(super::search::invalid_access(&state));
    }
    let id = match id {
        Some(id) => {
            let Ok(id) = id.strip_suffix(".json").unwrap_or(&id).parse::<i32>() else {
                return Ok(super::topics::not_found_response(&state));
            };
            let action: Option<i32> =
                sqlx::query_scalar("SELECT action_type FROM screened_ip_addresses WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await?;
            let Some(action) = action else {
                return Ok(super::topics::not_found_response(&state));
            };
            if !guardian.is_admin() && action == crate::screened::ALLOW_ADMIN {
                return Ok(super::search::invalid_access(&state));
            }
            Some(id)
        }
        None => None,
    };
    let action_name = crate::params::string(&p, "action_name");
    if !guardian.is_admin() && action_name.as_deref() == Some("allow_admin") {
        return Ok(super::search::invalid_access(&state));
    }
    if method == "DELETE" {
        sqlx::query("DELETE FROM screened_ip_addresses WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(Json(json!({ "success": "OK" })).into_response());
    }
    // allowed_params: params.require(:ip_address), a known action_name.
    let ip = crate::params::string(&p, "ip_address").filter(|v| !crate::ruby::is_blank(v));
    let Some(ip) = ip else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "errors": ["param is missing or the value is empty or invalid: ip_address"]
            })),
        )
            .into_response());
    };
    let action = match action_name.filter(|a| !crate::ruby::is_blank(a)) {
        None => None,
        Some(a) => match crate::screened::action_type(&a) {
            Some(t) => Some(t),
            None => return Ok(super::search::invalid_parameters(&state, "action_name")),
        },
    };
    Ok(
        match crate::screened::save_ip(&mut tx, id, &ip, action).await? {
            crate::screened::Saved::Ok(row) => {
                tx.commit().await?;
                Json(json!({ "screened_ip_address": row })).into_response()
            }
            crate::screened::Saved::Invalid(errors) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "errors": errors })),
            )
                .into_response(),
            crate::screened::Saved::NotFound => super::topics::not_found_response(&state),
        },
    )
}
