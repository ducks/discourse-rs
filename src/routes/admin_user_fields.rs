//! Admin::UserFieldsController under /admin/config. `resources
//! :user_fields` (index, create, update, destroy) is AdminConstraint: no
//! route, a 404, for anyone else. The `user-fields` and `user_fields`
//! GET aliases (index, show, edit) are only behind the admin namespace's
//! StaffConstraint, so a moderator gets ensure_admin's InvalidAccess.
//!
//! Refused: the Ember admin pages (an HTML request).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::guardian::Guardian;
use crate::params;
use crate::session::current::AuthGuardian;
use crate::user_fields::{self, Draft};
use crate::{AppError, AppState, Unsupported};

fn json_request(headers: &HeaderMap, uri: &Uri) -> bool {
    uri.path().ends_with(".json")
        || headers
            .get("x-requested-with")
            .is_some_and(|v| v == "XMLHttpRequest")
        || super::wants_json(uri.query(), headers)
}

/// AdminConstraint: the route exists for admins only.
fn admin_route(state: &AppState, guardian: &Guardian) -> Option<Response> {
    (!guardian.is_admin()).then(|| super::topics::not_found_response(state))
}

/// StaffConstraint, then ensure_admin.
fn staff_route(state: &AppState, guardian: &Guardian) -> Option<Response> {
    if guardian.is_admin() {
        None
    } else if guardian.is_staff() {
        Some(super::search::invalid_access(state))
    } else {
        Some(super::topics::not_found_response(state))
    }
}

fn refuse_html(headers: &HeaderMap, uri: &Uri) -> Result<(), AppError> {
    if json_request(headers, uri) {
        Ok(())
    } else {
        Err(Unsupported("the admin user fields pages (Ember)").into())
    }
}

fn strip_json(s: &str) -> &str {
    s.strip_suffix(".json").unwrap_or(s)
}

/// `UserField.find(params[:id])` / `where(id:)`: the id's integer cast,
/// None when it isn't one.
fn field_id(segment: &str) -> Option<i32> {
    let s = strip_json(segment).trim();
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if !digits.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    i32::try_from(crate::ruby::to_i(s)).ok()
}

fn pairs(p: &Map<String, Value>) -> Vec<(String, String)> {
    p.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

async fn index_json(state: &AppState) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let mut out = Vec::new();
    for f in user_fields::all(&mut conn).await? {
        out.push(user_fields::serialize(&mut conn, &f).await?);
    }
    Ok(Json(json!({ "user_fields": out })).into_response())
}

/// GET /admin/config/user_fields(.json): resources' index.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = admin_route(&state, &guardian) {
        return Ok(r);
    }
    refuse_html(&headers, &uri)?;
    index_json(&state).await
}

/// GET /admin/config/user-fields(.json) and the `new` aliases: index.
pub async fn index_alias(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = staff_route(&state, &guardian) {
        return Ok(r);
    }
    if ember_page(&headers, &uri) {
        return super::admin_config::user_fields_page(
            &state,
            &guardian,
            &headers,
            &uri,
            super::admin_config::UserFieldsPage::List,
        )
        .await;
    }
    refuse_html(&headers, &uri)?;
    index_json(&state).await
}

/// An HTML request for one of the Ember pages (adminUserFields), whose
/// paths are the dashed ones.
fn ember_page(headers: &HeaderMap, uri: &Uri) -> bool {
    !json_request(headers, uri) && uri.path().starts_with("/admin/config/user-fields")
}

/// GET /admin/config/user-fields/:id and /user_fields/:id: show (`new`
/// is the index).
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = staff_route(&state, &guardian) {
        return Ok(r);
    }
    if ember_page(&headers, &uri) && id == "new" {
        return super::admin_config::user_fields_page(
            &state,
            &guardian,
            &headers,
            &uri,
            super::admin_config::UserFieldsPage::New,
        )
        .await;
    }
    refuse_html(&headers, &uri)?;
    if strip_json(&id) == "new" {
        return index_json(&state).await;
    }
    let mut conn = state.pool.acquire().await?;
    let found = match field_id(&id) {
        Some(id) => user_fields::find(&mut conn, id).await?,
        None => None,
    };
    let Some(field) = found else {
        return Ok(super::topics::not_found_response(&state));
    };
    let body = user_fields::serialize(&mut conn, &field).await?;
    Ok(Json(json!({ "user_field": body })).into_response())
}

/// GET .../:id/edit: an empty action (204 for JSON).
pub async fn edit(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = staff_route(&state, &guardian) {
        return Ok(r);
    }
    if ember_page(&headers, &uri) {
        // The route's store.find: a field that isn't there is the 404.
        let Some(id) = field_id(&id) else {
            return Ok(super::topics::not_found_response(&state));
        };
        return super::admin_config::user_fields_page(
            &state,
            &guardian,
            &headers,
            &uri,
            super::admin_config::UserFieldsPage::Edit(id),
        )
        .await;
    }
    refuse_html(&headers, &uri)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn user_field_param(p: &Map<String, Value>) -> Option<&Map<String, Value>> {
    p.get("user_field").and_then(Value::as_object)
}

fn json_errors(errors: &[String]) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "errors": errors })),
    )
        .into_response()
}

/// POST /admin/config/user_fields(.json)
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = admin_route(&state, &guardian) {
        return Ok(r);
    }
    let p = params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "POST") {
        return Ok(super::session::bad_csrf());
    }
    refuse_html(&headers, &uri)?;
    // params.require(:user_field)
    let Some(attrs) = user_field_param(&p).filter(|m| !m.is_empty()) else {
        return Ok(super::accounts::param_missing("user_field"));
    };
    let mut draft = Draft::new_record();
    draft.assign(attrs, false)?;
    let options = user_fields::options_param(attrs)?;
    let mut tx = state.pool.begin().await?;
    // position = (UserField.maximum(:position) || 0) + 1
    let max: Option<i32> = sqlx::query_scalar("SELECT MAX(position) FROM user_fields")
        .fetch_one(&mut *tx)
        .await?;
    draft.position = Some(max.unwrap_or(0) + 1);
    let saved =
        match user_fields::save(&mut tx, &state.bus, None, &draft, options.as_deref()).await? {
            Ok(f) => f,
            Err(errors) => return Ok(json_errors(&errors)),
        };
    let body = user_fields::serialize(&mut tx, &saved).await?;
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK", "user_field": body })).into_response())
}

/// PUT /admin/config/user_fields/:id(.json)
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = admin_route(&state, &guardian) {
        return Ok(r);
    }
    let p = params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    refuse_html(&headers, &uri)?;
    let Some(attrs) = user_field_param(&p) else {
        return Err(Unsupported("a user field update without user_field (NoMethodError)").into());
    };
    let mut conn = state.pool.acquire().await?;
    let found = match field_id(&id) {
        Some(id) => user_fields::find(&mut conn, id).await?,
        None => None,
    };
    let Some(before) = found else {
        return Err(Unsupported("updating a user field that doesn't exist (NoMethodError)").into());
    };
    let mut draft = Draft::from_field(&before);
    draft.assign(attrs, true)?;
    let options = user_fields::options_param(attrs)?;
    // update_options: the old options go at once, outside the save, so an
    // invalid field loses them too.
    if options.is_some() {
        sqlx::query("DELETE FROM user_field_options WHERE user_field_id = $1")
            .bind(before.id)
            .execute(&mut *conn)
            .await?;
    }
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    let saved = match user_fields::save(
        &mut tx,
        &state.bus,
        Some(&before),
        &draft,
        options.as_deref(),
    )
    .await?
    {
        Ok(f) => f,
        Err(errors) => return Ok(json_errors(&errors)),
    };
    if !saved.show_on_profile && !saved.show_on_user_card {
        sqlx::query("DELETE FROM directory_columns WHERE user_field_id = $1")
            .bind(saved.id)
            .execute(&mut *tx)
            .await?;
    }
    let body = user_fields::serialize(&mut tx, &saved).await?;
    tx.commit().await?;
    Ok(Json(json!({ "user_field": body })).into_response())
}

/// DELETE /admin/config/user_fields/:id(.json): success whether or not
/// the field exists.
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(r) = admin_route(&state, &guardian) {
        return Ok(r);
    }
    let p = params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "DELETE") {
        return Ok(super::session::bad_csrf());
    }
    refuse_html(&headers, &uri)?;
    let mut tx = state.pool.begin().await?;
    if let Some(id) = field_id(&id)
        && user_fields::find(&mut tx, id).await?.is_some()
    {
        user_fields::destroy(&mut tx, &state.bus, id).await?;
    }
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}
