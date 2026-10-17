//! Admin::McpController, Admin::McpAuthorizationsController and
//! Admin::McpActivityController under /admin/mcp (AdminConstraint): the
//! overview, the group access rules, the primitives' policies and
//! emergency blocks, the OAuth authorizations with their revocation, and
//! the activity report (crate::mcp).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::bad_csrf;
use crate::mcp::{self, Refusal};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported, params};

/// `params.require(key)`'s 400.
fn param_missing(key: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "errors": [format!("param is missing or the value is empty or invalid: {key}")]
        })),
    )
        .into_response()
}

fn refused(state: &AppState, refusal: Refusal) -> Response {
    match refusal {
        Refusal::InvalidParameters(name) => super::search::invalid_parameters(state, name),
        Refusal::NotFound => super::topics::not_found_response(state),
    }
}

/// `params.require(key)`: present and not blank (an empty list is blank).
fn present(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !crate::ruby::is_blank(s),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(m)) => !m.is_empty(),
        Some(_) => true,
    }
}

/// A list param's strings (a scalar becomes a list of one).
fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.iter().filter_map(params::scalar).collect(),
        Some(Value::Object(map)) => map.values().filter_map(params::scalar).collect(),
        Some(other) => params::scalar(other).into_iter().collect(),
    }
}

/// `fetch_int_from_params(key, default:, min:, max:)`: Err for the
/// InvalidParameters it raises.
fn int_param(
    p: &Map<String, Value>,
    key: &str,
    default: Option<i64>,
    min: i64,
    max: Option<i64>,
) -> Result<Option<i64>, ()> {
    let Some(v) = p.get(key) else {
        return Ok(default);
    };
    let n = params::scalar(v)
        .and_then(|s| crate::ruby::integer(&s))
        .ok_or(())?;
    if n < min || max.is_some_and(|m| n > m) {
        return Err(());
    }
    Ok(Some(n))
}

/// ActiveModel's boolean cast: blank is nil, then "false", "f", "0",
/// "off" (any case) are false and everything else true.
fn cast_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Null => None,
        Value::Number(n) => Some(n.as_f64() != Some(0.0)),
        other => {
            let s = params::scalar(other)?;
            if s.is_empty() {
                return None;
            }
            Some(!matches!(
                s.to_lowercase().as_str(),
                "false" | "f" | "0" | "off"
            ))
        }
    }
}

/// The request's params, the CSRF token checked for writes; Err is the
/// response to give.
fn request(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
    method: &str,
) -> Result<Map<String, Value>, Box<Response>> {
    let p = if method == "GET" {
        params::parse(uri.query(), headers, body)
    } else {
        super::admin_config_about::params_of(state, headers, uri, body, method)
            .ok_or_else(|| Box::new(bad_csrf()))?
    };
    // AdminConstraint: anyone else is not found.
    if !guardian.is_admin() {
        return Err(Box::new(super::topics::not_found_response(state)));
    }
    Ok(p)
}

fn acting(guardian: &crate::guardian::Guardian) -> Result<i32, AppError> {
    Ok(guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?)
}

/// `"123.json"` → 123, the path's id.
fn path_id(raw: &str) -> String {
    raw.strip_suffix(".json").unwrap_or(raw).to_string()
}

async fn settings(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// GET /admin/mcp/overview
pub async fn overview(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Err(r) = request(&state, &guardian, &headers, &uri, &body, "GET") {
        return Ok(*r);
    }
    let mut conn = state.pool.acquire().await?;
    let settings = settings(&mut conn, &state).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let base_url = urls.base_url()?;
    Ok(Json(mcp::overview(&mut conn, &settings, &state.i18n, &base_url).await?).into_response())
}

async fn access_payload(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
) -> Result<Value, AppError> {
    let settings = settings(&mut *conn, state).await?;
    let mut out = mcp::primitives_payload(&mut *conn, &settings).await?;
    let rules = mcp::access_rules(&mut *conn).await?;
    if let Value::Object(map) = &mut out {
        map.insert("access_rules".into(), json!(rules));
    }
    Ok(out)
}

/// GET /admin/mcp/access
pub async fn access(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Err(r) = request(&state, &guardian, &headers, &uri, &body, "GET") {
        return Ok(*r);
    }
    let mut conn = state.pool.acquire().await?;
    Ok(Json(access_payload(&mut conn, &state).await?).into_response())
}

/// PUT and DELETE /admin/mcp/access/:group_id
pub async fn group_access(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: axum::http::Method,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let delete = method == axum::http::Method::DELETE;
    let p = match request(
        &state,
        &guardian,
        &headers,
        &uri,
        &body,
        if delete { "DELETE" } else { "PUT" },
    ) {
        Ok(p) => p,
        Err(r) => return Ok(*r),
    };
    let actor = acting(&guardian)?;
    // McpAccessRuleUpdater's `group_id.to_i`.
    let group_id = crate::ruby::to_i(&path_id(&raw));
    let mut tx = state.pool.begin().await?;
    let outcome = if delete {
        mcp::delete_access(&mut tx, actor, group_id).await?
    } else {
        if !present(p.get("scopes")) {
            return Ok(param_missing("scopes"));
        }
        mcp::replace_access(&mut tx, actor, group_id, &strings(p.get("scopes"))).await?
    };
    if let Err(refusal) = outcome {
        return Ok(refused(&state, refusal));
    }
    let response = if delete {
        StatusCode::NO_CONTENT.into_response()
    } else {
        Json(access_payload(&mut tx, &state).await?).into_response()
    };
    tx.commit().await?;
    Ok(response)
}

/// GET and PUT /admin/mcp/capabilities: the primitives, and their
/// policies updated.
pub async fn capabilities(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: axum::http::Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let write = method == axum::http::Method::PUT;
    let p = match request(
        &state,
        &guardian,
        &headers,
        &uri,
        &body,
        if write { "PUT" } else { "GET" },
    ) {
        Ok(p) => p,
        Err(r) => return Ok(*r),
    };
    let mut tx = state.pool.begin().await?;
    if write {
        let actor = acting(&guardian)?;
        mcp::update_primitives(&mut tx, actor, &strings(p.get("primitive_ids"))).await?;
    }
    let settings = settings(&mut tx, &state).await?;
    let payload = mcp::primitives_payload(&mut tx, &settings).await?;
    tx.commit().await?;
    Ok(Json(payload).into_response())
}

/// PUT /admin/mcp/capabilities/emergency-block
pub async fn emergency_block(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match request(&state, &guardian, &headers, &uri, &body, "PUT") {
        Ok(p) => p,
        Err(r) => return Ok(*r),
    };
    if !present(p.get("primitive_id")) {
        return Ok(param_missing("primitive_id"));
    }
    let primitive_id = params::string(&p, "primitive_id").unwrap_or_default();
    // `params.fetch(:blocked, true)`, cast by the column.
    let blocked = match p.get("blocked") {
        None => true,
        Some(v) => match cast_bool(v) {
            Some(b) => b,
            None => return Err(Unsupported("a blank emergency block (a NULL column)").into()),
        },
    };
    let actor = acting(&guardian)?;
    let mut tx = state.pool.begin().await?;
    if let Err(refusal) = mcp::set_emergency_block(&mut tx, actor, &primitive_id, blocked).await? {
        return Ok(refused(&state, refusal));
    }
    tx.commit().await?;
    Ok(Json(json!({ "success": true })).into_response())
}

/// The page params the reports share: `limit` (default 50, max 100),
/// `cursor` (min 1) and a filter of at most 200 characters.
fn page_params(
    state: &AppState,
    p: &Map<String, Value>,
) -> Result<(i64, Option<i64>, String), Box<Response>> {
    let limit = int_param(p, "limit", Some(50), 0, Some(100))
        .map_err(|_| Box::new(super::search::invalid_parameters(state, "limit")))?
        .unwrap_or(50);
    let cursor = int_param(p, "cursor", None, 1, None)
        .map_err(|_| Box::new(super::search::invalid_parameters(state, "cursor")))?;
    let filter = params::string(p, "filter")
        .unwrap_or_default()
        .trim()
        .to_string();
    if filter.chars().count() > 200 {
        return Err(Box::new(super::search::invalid_parameters(state, "filter")));
    }
    Ok((limit, cursor, filter))
}

/// GET /admin/mcp/authorizations
pub async fn authorizations(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match request(&state, &guardian, &headers, &uri, &body, "GET") {
        Ok(p) => p,
        Err(r) => return Ok(*r),
    };
    let (limit, cursor, filter) = match page_params(&state, &p) {
        Ok(v) => v,
        Err(r) => return Ok(*r),
    };
    let mut conn = state.pool.acquire().await?;
    let settings = settings(&mut conn, &state).await?;
    Ok(
        Json(mcp::authorizations(&mut conn, &settings, limit, cursor, &filter).await?)
            .into_response(),
    )
}

/// DELETE /admin/mcp/authorizations/:id
pub async fn revoke_authorization(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Err(r) = request(&state, &guardian, &headers, &uri, &body, "DELETE") {
        return Ok(*r);
    }
    let actor = acting(&guardian)?;
    let Some(id) = crate::ruby::integer(&path_id(&raw)) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let mut tx = state.pool.begin().await?;
    if mcp::revoke_authorization(&mut tx, actor, id)
        .await?
        .is_none()
    {
        return Ok(super::topics::not_found_response(&state));
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// GET /admin/mcp/activity
pub async fn activity(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match request(&state, &guardian, &headers, &uri, &body, "GET") {
        Ok(p) => p,
        Err(r) => return Ok(*r),
    };
    let limit = match int_param(&p, "limit", Some(50), 0, Some(100)) {
        Ok(v) => v.unwrap_or(50),
        Err(()) => return Ok(super::search::invalid_parameters(&state, "limit")),
    };
    let cursor = match int_param(&p, "cursor", None, 1, None) {
        Ok(v) => v,
        Err(()) => return Ok(super::search::invalid_parameters(&state, "cursor")),
    };
    let query = mcp::ActivityQuery {
        limit,
        cursor,
        start_date: p.get("start_date"),
        end_date: p.get("end_date"),
        filter: params::string(&p, "filter")
            .unwrap_or_default()
            .trim()
            .to_string(),
        outcome: params::string(&p, "outcome").filter(|o| !crate::ruby::is_blank(o)),
    };
    let mut conn = state.pool.acquire().await?;
    Ok(match mcp::activity(&mut conn, &query).await? {
        Ok(report) => Json(report).into_response(),
        Err(name) => super::search::invalid_parameters(&state, name),
    })
}

pub(super) fn routes(mut router: axum::Router<AppState>) -> axum::Router<AppState> {
    use axum::routing::{delete, get, put};
    for suffix in ["", ".json"] {
        router = router
            .route(&format!("/admin/mcp/overview{suffix}"), get(overview))
            .route(&format!("/admin/mcp/access{suffix}"), get(access))
            .route(
                &format!("/admin/mcp/capabilities{suffix}"),
                get(capabilities).put(capabilities),
            )
            .route(
                &format!("/admin/mcp/capabilities/emergency-block{suffix}"),
                put(emergency_block),
            )
            .route(
                &format!("/admin/mcp/authorizations{suffix}"),
                get(authorizations),
            )
            .route(&format!("/admin/mcp/activity{suffix}"), get(activity));
    }
    router
        .route(
            "/admin/mcp/access/{group_id}",
            put(group_access).delete(group_access),
        )
        .route(
            "/admin/mcp/authorizations/{id}",
            delete(revoke_authorization),
        )
}
