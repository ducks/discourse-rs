//! Admin::FormTemplatesController under /admin/customize/form-templates
//! (AdminConstraint: a 404 for anyone else) and FormTemplatesController's
//! /form-templates, both behind enable_form_templates.
//!
//! A page request for the list, new or a template is the Ember page
//! (crate::routes::admin_customize::form_templates_page).
//!
//! Refused: a valid preview and the public show, which re-dump the
//! template with Psych (`process!`).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::form_templates;
use crate::guardian::Guardian;
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

fn json_request(headers: &HeaderMap, uri: &Uri) -> bool {
    uri.path().ends_with(".json")
        || headers
            .get("x-requested-with")
            .is_some_and(|v| v == "XMLHttpRequest")
        || super::wants_json(uri.query(), headers)
}

fn refuse_html(headers: &HeaderMap, uri: &Uri) -> Result<(), AppError> {
    if json_request(headers, uri) {
        Ok(())
    } else {
        Err(Unsupported("the form template pages (Ember)").into())
    }
}

fn pairs(p: &Map<String, Value>) -> Vec<(String, String)> {
    p.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn strip_json(s: &str) -> &str {
    s.strip_suffix(".json").unwrap_or(s)
}

/// `FormTemplate.find(params[:id])`'s id cast.
fn id_of(raw: &str) -> Option<i64> {
    let s = strip_json(raw).trim();
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    digits
        .starts_with(|c: char| c.is_ascii_digit())
        .then(|| crate::ruby::to_i(s))
}

/// AdminConstraint, then ensure_form_templates_enabled; the settings.
async fn admin_guard(
    state: &AppState,
    guardian: &Guardian,
    conn: &mut sqlx::PgConnection,
) -> Result<Result<SiteSettings, Response>, AppError> {
    if !guardian.is_admin() {
        return Ok(Err(super::topics::not_found_response(state)));
    }
    let settings =
        SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_form_templates")?.truthy() {
        return Ok(Err(super::search::invalid_access(state)));
    }
    Ok(Ok(settings))
}

fn record_invalid(errors: Vec<String>) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "errors": errors, "error_type": "record_invalid" })),
    )
        .into_response()
}

fn rendered(t: &form_templates::FormTemplate) -> Response {
    Json(json!({ "form_template": form_templates::serialize(t) })).into_response()
}

/// GET /admin/customize/form-templates(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    if let Err(r) = admin_guard(&state, &guardian, &mut conn).await? {
        return Ok(r);
    }
    let list: Vec<Value> = form_templates::all(&mut conn)
        .await?
        .iter()
        .map(form_templates::serialize)
        .collect();
    drop(conn);
    if !json_request(&headers, &uri) {
        return super::admin_customize::form_templates_page(
            &state, &guardian, &headers, &uri, &list, None,
        )
        .await;
    }
    Ok(Json(json!({ "form_templates": list })).into_response())
}

/// `params.require(key)` as a string.
fn required(p: &Map<String, Value>, key: &str) -> Result<Option<String>, AppError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match params::scalar(v) {
            Some(s) if s.is_empty() => Ok(None),
            Some(s) => Ok(Some(s)),
            None => Err(Unsupported("a form template param that isn't a string").into()),
        },
    }
}

/// POST /admin/customize/form-templates(.json)
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "POST") {
        return Ok(super::session::bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let settings = match admin_guard(&state, &guardian, &mut tx).await? {
        Ok(s) => s,
        Err(r) => return Ok(r),
    };
    refuse_html(&headers, &uri)?;
    let Some(name) = required(&p, "name")? else {
        return Ok(super::accounts::param_missing("name"));
    };
    let Some(template) = required(&p, "template")? else {
        return Ok(super::accounts::param_missing("template"));
    };
    let errors = form_templates::errors(
        &mut tx,
        &settings,
        &state.i18n,
        None,
        Some(&name),
        Some(&template),
    )
    .await?;
    if !errors.is_empty() {
        return Ok(record_invalid(errors));
    }
    let saved = form_templates::create(&mut tx, &name, &template).await?;
    tx.commit().await?;
    Ok(rendered(&saved))
}

/// GET /admin/customize/form-templates/:id(.json): show; `new` and
/// `preview` share the path.
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match admin_guard(&state, &guardian, &mut conn).await? {
        Ok(s) => s,
        Err(r) => return Ok(r),
    };
    if !json_request(&headers, &uri) {
        // adminCustomizeFormTemplates.new, or .edit for the template.
        let template = if id == "new" {
            None
        } else {
            let found = match id_of(&id) {
                Some(id) => form_templates::find(&mut conn, id).await?,
                None => None,
            };
            match found {
                Some(t) => Some(form_templates::serialize(&t)),
                None => return Ok(super::topics::not_found_response(&state)),
            }
        };
        drop(conn);
        return super::admin_customize::form_templates_page(
            &state,
            &guardian,
            &headers,
            &uri,
            &[],
            Some(template.as_ref()),
        )
        .await;
    }
    match strip_json(&id) {
        "new" => return Ok(StatusCode::NO_CONTENT.into_response()),
        "preview" => {
            let p = params::parse(uri.query(), &headers, &body);
            return preview(&state, &mut conn, &settings, &p).await;
        }
        _ => {}
    }
    let found = match id_of(&id) {
        Some(id) => form_templates::find(&mut conn, id).await?,
        None => None,
    };
    Ok(match found {
        Some(t) => rendered(&t),
        None => super::topics::not_found_response(&state),
    })
}

/// preview: the validations run; a valid template would be processed and
/// re-dumped with Psych, which isn't ported.
async fn preview(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    p: &Map<String, Value>,
) -> Result<Response, AppError> {
    let Some(name) = required(p, "name")? else {
        return Ok(super::accounts::param_missing("name"));
    };
    let Some(template) = required(p, "template")? else {
        return Ok(super::accounts::param_missing("template"));
    };
    let id = match p
        .get("id")
        .and_then(params::scalar)
        .filter(|s| !s.is_empty())
    {
        Some(raw) => {
            let found = match id_of(&raw) {
                Some(id) => form_templates::find(&mut *conn, id).await?,
                None => None,
            };
            match found {
                Some(t) => Some(t.id),
                None => return Ok(super::topics::not_found_response(state)),
            }
        }
        None => None,
    };
    let errors = form_templates::errors(
        &mut *conn,
        settings,
        &state.i18n,
        id,
        Some(&name),
        Some(&template),
    )
    .await?;
    if !errors.is_empty() {
        return Ok(record_invalid(errors));
    }
    Err(Unsupported("previewing a form template (Psych's YAML.dump)").into())
}

/// GET /admin/customize/form-templates/:id/edit(.json): find, then the
/// empty action.
pub async fn edit(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    if let Err(r) = admin_guard(&state, &guardian, &mut conn).await? {
        return Ok(r);
    }
    // No Ember route has this path.
    if !json_request(&headers, &uri) {
        return Ok(super::topics::not_found_response(&state));
    }
    let found = match id_of(&id) {
        Some(id) => form_templates::find(&mut conn, id).await?,
        None => None,
    };
    Ok(match found {
        Some(_) => StatusCode::NO_CONTENT.into_response(),
        None => super::topics::not_found_response(&state),
    })
}

/// PUT /admin/customize/form-templates/:id(.json)
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let settings = match admin_guard(&state, &guardian, &mut tx).await? {
        Ok(s) => s,
        Err(r) => return Ok(r),
    };
    refuse_html(&headers, &uri)?;
    let found = match id_of(&id) {
        Some(id) => form_templates::find(&mut tx, id).await?,
        None => None,
    };
    let Some(before) = found else {
        return Ok(super::topics::not_found_response(&state));
    };
    let s = |k: &str| match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => params::scalar(v)
            .map(Some)
            .ok_or(Unsupported("a form template param that isn't a string")),
    };
    let name = s("name")?;
    let template = s("template")?;
    let errors = form_templates::errors(
        &mut tx,
        &settings,
        &state.i18n,
        Some(before.id),
        name.as_deref(),
        template.as_deref(),
    )
    .await?;
    if !errors.is_empty() {
        return Ok(record_invalid(errors));
    }
    let saved = form_templates::update(
        &mut tx,
        &before,
        name.as_deref().unwrap_or(""),
        template.as_deref().unwrap_or(""),
    )
    .await?;
    tx.commit().await?;
    Ok(rendered(&saved))
}

/// DELETE /admin/customize/form-templates/:id(.json)
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "DELETE") {
        return Ok(super::session::bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    if let Err(r) = admin_guard(&state, &guardian, &mut tx).await? {
        return Ok(r);
    }
    refuse_html(&headers, &uri)?;
    let found = match id_of(&id) {
        Some(id) => form_templates::find(&mut tx, id).await?,
        None => None,
    };
    let Some(t) = found else {
        return Ok(super::topics::not_found_response(&state));
    };
    form_templates::destroy(&mut tx, t.id).await?;
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// requires_login, then ensure_form_templates_enabled; the categories
/// the viewer may see.
async fn public_guard(
    state: &AppState,
    guardian: &Guardian,
    conn: &mut sqlx::PgConnection,
) -> Result<Result<Vec<i32>, Response>, AppError> {
    if guardian.is_anonymous() {
        return Ok(Err(super::login_required::not_logged_in(state)));
    }
    let settings =
        SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_form_templates")?.truthy() {
        return Ok(Err(super::search::invalid_access(state)));
    }
    Ok(Ok(guardian
        .allowed_category_ids(&mut *conn, &settings)
        .await?))
}

/// GET /form-templates(.json): FormTemplateSerializer without process!.
pub async fn public_index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let allowed = match public_guard(&state, &guardian, &mut conn).await? {
        Ok(a) => a,
        Err(r) => return Ok(r),
    };
    refuse_html(&headers, &uri)?;
    let list: Vec<Value> = form_templates::accessible(&mut conn, &allowed)
        .await?
        .iter()
        .map(form_templates::serialize)
        .collect();
    Ok(Json(json!({ "form_templates": list })).into_response())
}

/// GET /form-templates/:id(.json): a 404 for one the viewer may not use;
/// the rest is process! (Psych), refused.
pub async fn public_show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let allowed = match public_guard(&state, &guardian, &mut conn).await? {
        Ok(a) => a,
        Err(r) => return Ok(r),
    };
    refuse_html(&headers, &uri)?;
    let id = id_of(&id);
    let found = form_templates::accessible(&mut conn, &allowed)
        .await?
        .into_iter()
        .any(|t| Some(t.id) == id);
    if !found {
        return Ok(super::topics::not_found_response(&state));
    }
    Err(Unsupported("a form template processed for the composer (Psych's YAML.dump)").into())
}
