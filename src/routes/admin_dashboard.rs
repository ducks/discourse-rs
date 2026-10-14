//! The admin dashboard's problem checks and notices:
//! Admin::DashboardController#problems, Admin::ProblemChecksController and
//! Admin::AdminNoticesController.
//!
//! Every route is under StaffConstraint (others 404); `problem_checks`
//! and `admin_notices` are also under AdminConstraint (moderators 404),
//! while `/admin/problem-checks` reaches ProblemChecksController's
//! ensure_admin (moderators 403).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::guardian::Guardian;
use crate::problem_checks::{self, Env};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

fn not_found(state: &AppState) -> Response {
    super::topics::not_found_response(state)
}

fn strip_json(id: &str) -> &str {
    id.strip_suffix(".json").unwrap_or(id)
}

fn pairs(map: &serde_json::Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// The CSRF check on a write, as ApplicationController verifies it.
fn csrf(state: &AppState, headers: &HeaderMap, uri: &Uri, body: &Bytes, method: &str) -> bool {
    let p = crate::params::parse(uri.query(), headers, body);
    super::session::csrf_ok(state, headers, &pairs(&p), uri.path(), method)
}

async fn settings(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

fn env<'a>(state: &'a AppState, settings: &'a SiteSettings) -> Env<'a> {
    Env {
        settings,
        defs: &state.site_setting_defs,
        config: &state.config,
        i18n: &state.i18n,
    }
}

/// POST /admin/dashboard/problems(.json): the realtime checks, then the
/// problem notices.
pub async fn problems(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(not_found(&state));
    }
    if !csrf(&state, &headers, &uri, &body, "POST") {
        return Ok(super::session::bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let s = settings(&mut tx, &state).await?;
    let env = env(&state, &s);
    problem_checks::run_realtime(&mut tx, &env).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &s,
    };
    let problems = problem_checks::serialized_problems(&mut tx, &env, &urls).await?;
    tx.commit().await?;
    Ok(Json(json!({ "problems": problems })).into_response())
}

/// GET /admin/problem-checks(.json) and /admin/problem_checks(.json):
/// every tracker, as a bare array.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: Uri,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(not_found(&state));
    }
    if !guardian.is_admin() {
        // AdminConstraint on /admin/problem_checks; ensure_admin on the other.
        if uri.path().starts_with("/admin/problem_checks") {
            return Ok(not_found(&state));
        }
        return Ok(super::search::invalid_access(&state));
    }
    let mut conn = state.pool.acquire().await?;
    let trackers = problem_checks::all_trackers(&mut conn).await?;
    Ok(Json(Value::Array(
        trackers.iter().map(problem_checks::tracker_json).collect(),
    ))
    .into_response())
}

fn admin_only(state: &AppState, guardian: &Guardian) -> Option<Response> {
    (!guardian.is_admin()).then(|| not_found(state))
}

/// PUT /admin/problem_checks/:id/ignore(.json)
pub async fn ignore(
    state: State<AppState>,
    guardian: AuthGuardian,
    path: Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    toggle(state, guardian, path, headers, uri, body, true).await
}

/// PUT /admin/problem_checks/:id/watch(.json)
pub async fn watch(
    state: State<AppState>,
    guardian: AuthGuardian,
    path: Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    toggle(state, guardian, path, headers, uri, body, false).await
}

async fn toggle(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    ignore: bool,
) -> Result<Response, AppError> {
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    if !csrf(&state, &headers, &uri, &body, "PUT") {
        return Ok(super::session::bad_csrf());
    }
    // ProblemCheckTracker.find: a non-numeric id finds nothing either.
    let id = crate::ruby::integer(strip_json(&id));
    let mut tx = state.pool.begin().await?;
    let tracker = match id {
        Some(id) => problem_checks::tracker_by_id(&mut tx, id).await?,
        None => None,
    };
    let Some(tracker) = tracker else {
        return Ok(not_found(&state));
    };
    if ignore {
        problem_checks::ignore(&mut tx, &tracker).await?;
    } else {
        problem_checks::watch(&mut tx, tracker).await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// DELETE /admin/admin_notices/:id(.json): AdminNotices::Dismiss, the
/// notice gone and its tracker ignored; a missing notice succeeds too.
pub async fn dismiss(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    if !csrf(&state, &headers, &uri, &body, "DELETE") {
        return Ok(super::session::bad_csrf());
    }
    // The contract's integer id: `to_i` of the param.
    let id = crate::ruby::to_i(strip_json(&id));
    let mut tx = state.pool.begin().await?;
    let notice: Option<(String, Option<Value>)> =
        sqlx::query_as("SELECT identifier, details::jsonb FROM admin_notices WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some((identifier, details)) = notice {
        sqlx::query("DELETE FROM admin_notices WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let target = details
            .as_ref()
            .and_then(|d| d.get("target"))
            .and_then(Value::as_str)
            .unwrap_or(problem_checks::NO_TARGET)
            .to_string();
        let tracker: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM problem_check_trackers WHERE identifier = $1 AND target = $2 LIMIT 1",
        )
        .bind(&identifier)
        .bind(&target)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(tracker_id) = tracker
            && let Some(t) = problem_checks::tracker_by_id(&mut tx, tracker_id).await?
        {
            problem_checks::ignore(&mut tx, &t).await?;
        }
    }
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}
