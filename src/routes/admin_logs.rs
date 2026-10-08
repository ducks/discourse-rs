//! GET /admin/logs/staff_action_logs(.json): Admin::StaffActionLogsController#index.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};

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
