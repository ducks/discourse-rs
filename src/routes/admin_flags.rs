//! Admin::Config::FlagsController under /admin/config/flags: behind the
//! admin namespace's StaffConstraint (anyone else: no route), then
//! ensure_admin (moderators: InvalidAccess). The services' outcomes map
//! as the controller maps them: a failed contract 400 with its errors, a
//! failed policy 403 or its message (422), a missing model 422 (the
//! controller only maps a `:message` model), success with
//! `Discourse.request_refresh!`.
//!
//! Refused: the Ember admin pages (an HTML request).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::admin_flags::{self, Params};
use crate::guardian::Guardian;
use crate::params;
use crate::session::current::AuthGuardian;
use crate::{AppError, AppState, Unsupported};

fn json_request(headers: &HeaderMap, uri: &Uri) -> bool {
    uri.path().ends_with(".json")
        || headers
            .get("x-requested-with")
            .is_some_and(|v| v == "XMLHttpRequest")
        || super::wants_json(uri.query(), headers)
}

/// StaffConstraint, then ensure_admin: the admin's id.
fn admin(state: &AppState, guardian: &Guardian) -> Result<i32, Box<Response>> {
    match guardian.user_id() {
        Some(id) if guardian.is_admin() => Ok(id),
        _ if guardian.is_staff() => Err(Box::new(super::search::invalid_access(state))),
        _ => Err(Box::new(super::topics::not_found_response(state))),
    }
}

fn refuse_html(headers: &HeaderMap, uri: &Uri) -> Result<(), AppError> {
    if json_request(headers, uri) {
        Ok(())
    } else {
        Err(Unsupported("the admin flags pages (Ember)").into())
    }
}

fn strip_json(s: &str) -> &str {
    s.strip_suffix(".json").unwrap_or(s)
}

/// An `:integer` attribute's cast: digits, else nil.
fn int(s: &str) -> Option<i64> {
    let t = strip_json(s).trim();
    let digits = t.strip_prefix(['+', '-']).unwrap_or(t);
    if !digits.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    Some(crate::ruby::to_i(t))
}

fn pairs(p: &Map<String, Value>) -> Vec<(String, String)> {
    p.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn failed() -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "failed": "FAILED" })),
    )
        .into_response()
}

fn failed_contract(errors: Vec<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "failed": "FAILED", "errors": errors })),
    )
        .into_response()
}

fn json_error(state: &AppState, key: &str) -> Response {
    let message = state.i18n.t(key).unwrap_or(key).to_string();
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "errors": [message] })),
    )
        .into_response()
}

fn success() -> Response {
    Json(json!({ "success": "OK" })).into_response()
}

/// The contract's attributes from the params (`:string`, `:boolean`,
/// and applies_to as given).
fn contract(p: &Map<String, Value>) -> Result<Params, AppError> {
    let string = |k: &str| match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => params::scalar(v)
            .map(Some)
            .ok_or(Unsupported("a flag attribute that isn't a string")),
    };
    let boolean = |k: &str| -> Result<Option<bool>, Unsupported> {
        Ok(match p.get(k) {
            None | Some(Value::Null) => None,
            Some(Value::Bool(b)) => Some(*b),
            Some(v) => crate::ruby::cast_boolean(
                params::scalar(v)
                    .ok_or(Unsupported("a flag attribute that isn't a boolean"))?
                    .as_str()
                    .into(),
            ),
        })
    };
    let applies_to = match p.get("applies_to") {
        None | Some(Value::Null) => None,
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .map(|i| {
                    i.as_str()
                        .map(str::to_string)
                        .ok_or(Unsupported("applies_to items that aren't strings"))
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(_) => return Err(Unsupported("applies_to that isn't a list").into()),
    };
    Ok(Params {
        name: string("name")?,
        description: string("description")?,
        require_message: boolean("require_message")?,
        enabled: boolean("enabled")?,
        applies_to,
        auto_action_type: boolean("auto_action_type")?,
    })
}

/// GET /admin/config/flags(.json), /new and /:id: empty actions (204).
pub async fn empty(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Err(r) = admin(&state, &guardian) {
        return Ok(*r);
    }
    refuse_html(&headers, &uri)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn empty_member(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path(_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    empty(state, guardian, headers, uri).await
}

struct Request {
    actor: i32,
    p: Map<String, Value>,
}

/// The filters every write runs: the route's guard, CSRF, check_xhr.
fn begin(
    state: &AppState,
    guardian: &Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
    method: &str,
) -> Result<Result<Request, Response>, AppError> {
    let actor = match admin(state, guardian) {
        Ok(id) => id,
        Err(r) => return Ok(Err(*r)),
    };
    let p = params::parse(uri.query(), headers, body);
    if !super::session::csrf_ok(state, headers, &pairs(&p), uri.path(), method) {
        return Ok(Err(super::session::bad_csrf()));
    }
    refuse_html(headers, uri)?;
    Ok(Ok(Request { actor, p }))
}

fn base_path(state: &AppState) -> &str {
    state.config.globals.relative_url_root()
}

async fn rendered(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    flag: &admin_flags::Flag,
) -> Result<Response, AppError> {
    let used = admin_flags::used(&mut *conn, flag.id).await?;
    let body = admin_flags::serialize(&state.i18n, base_path(state), flag, "post_action", used);
    Ok(Json(json!({ "flag": body })).into_response())
}

/// POST /admin/config/flags(.json)
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let r = match begin(&state, &guardian, &headers, &uri, &body, "POST")? {
        Ok(r) => r,
        Err(response) => return Ok(response),
    };
    let mut tx = state.pool.begin().await?;
    let settings = crate::site_settings::SiteSettings::load(
        &mut tx,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await?;
    // policy :invalid_access (can_create_flag?), before the contract.
    let limit = settings.get("custom_flags_limit")?.to_i();
    if admin_flags::custom_count(&mut tx).await? >= limit {
        return Ok(super::search::invalid_access(&state));
    }
    let p = contract(&r.p)?;
    let errors = p.errors();
    if !errors.is_empty() {
        return Ok(failed_contract(errors));
    }
    if admin_flags::name_taken(&mut tx, p.name.as_deref().unwrap_or(""), None).await? {
        return Ok(json_error(&state, "flags.errors.unique_name"));
    }
    let flag = admin_flags::create(&mut tx, r.actor, &p).await?;
    crate::bus::request_refresh(&state.bus, &mut tx).await?;
    let response = rendered(&state, &mut tx, &flag).await?;
    tx.commit().await?;
    Ok(response)
}

/// PUT /admin/config/flags/:id(.json)
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let r = match begin(&state, &guardian, &headers, &uri, &body, "PUT")? {
        Ok(r) => r,
        Err(response) => return Ok(response),
    };
    let p = contract(&r.p)?;
    let id = int(&id);
    let mut errors = Vec::new();
    if id.is_none() {
        errors.push("Id can't be blank".to_string());
    }
    errors.extend(p.errors());
    if !errors.is_empty() {
        return Ok(failed_contract(errors));
    }
    let mut tx = state.pool.begin().await?;
    let Some(flag) = admin_flags::find(&mut tx, id.unwrap_or_default()).await? else {
        return Ok(failed());
    };
    if flag.system() {
        return Ok(json_error(&state, "flags.errors.system"));
    }
    if admin_flags::name_taken(&mut tx, p.name.as_deref().unwrap_or(""), Some(flag.id)).await? {
        return Ok(json_error(&state, "flags.errors.unique_name"));
    }
    let flag = admin_flags::update(&mut tx, r.actor, &flag, &p).await?;
    crate::bus::request_refresh(&state.bus, &mut tx).await?;
    let response = rendered(&state, &mut tx, &flag).await?;
    tx.commit().await?;
    Ok(response)
}

/// DELETE /admin/config/flags/:id(.json)
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let r = match begin(&state, &guardian, &headers, &uri, &body, "DELETE")? {
        Ok(r) => r,
        Err(response) => return Ok(response),
    };
    let Some(id) = int(&id) else {
        return Ok(failed_contract(vec!["Id can't be blank".into()]));
    };
    let mut tx = state.pool.begin().await?;
    let Some(flag) = admin_flags::find(&mut tx, id).await? else {
        return Ok(failed());
    };
    if flag.system() {
        return Ok(json_error(&state, "flags.errors.system"));
    }
    if admin_flags::used(&mut tx, flag.id).await? {
        return Ok(json_error(&state, "flags.errors.used"));
    }
    admin_flags::destroy(&mut tx, r.actor, &flag).await?;
    crate::bus::request_refresh(&state.bus, &mut tx).await?;
    tx.commit().await?;
    Ok(success())
}

/// PUT /admin/config/flags/:flag_id/toggle(.json)
pub async fn toggle(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(flag_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let r = match begin(&state, &guardian, &headers, &uri, &body, "PUT")? {
        Ok(r) => r,
        Err(response) => return Ok(response),
    };
    let Some(id) = int(&flag_id) else {
        return Ok(failed_contract(vec!["Flag can't be blank".into()]));
    };
    let mut tx = state.pool.begin().await?;
    let Some(flag) = admin_flags::find(&mut tx, id).await? else {
        return Ok(failed());
    };
    admin_flags::toggle(&mut tx, r.actor, &flag).await?;
    crate::bus::request_refresh(&state.bus, &mut tx).await?;
    tx.commit().await?;
    Ok(success())
}

/// PUT /admin/config/flags/:flag_id/reorder/:direction(.json)
pub async fn reorder(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((flag_id, direction)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let r = match begin(&state, &guardian, &headers, &uri, &body, "PUT")? {
        Ok(r) => r,
        Err(response) => return Ok(response),
    };
    let direction = strip_json(&direction).to_string();
    let id = int(&flag_id);
    let mut errors = Vec::new();
    if id.is_none() {
        errors.push("Flag can't be blank".to_string());
    }
    if direction != "up" && direction != "down" {
        errors.push("Direction is not included in the list".to_string());
    }
    if !errors.is_empty() {
        return Ok(failed_contract(errors));
    }
    let mut tx = state.pool.begin().await?;
    let Some(flag) = admin_flags::find(&mut tx, id.unwrap_or_default()).await? else {
        return Ok(failed());
    };
    // can_reorder_flag?
    if flag.name_key.as_deref() == Some("notify_user") {
        return Ok(super::search::invalid_access(&state));
    }
    if admin_flags::reorder(&mut tx, r.actor, &flag, &direction)
        .await?
        .is_none()
    {
        return Ok(json_error(&state, "flags.errors.wrong_move"));
    }
    crate::bus::request_refresh(&state.bus, &mut tx).await?;
    tx.commit().await?;
    Ok(success())
}
