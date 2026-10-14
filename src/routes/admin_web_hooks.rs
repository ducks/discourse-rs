//! Admin::WebHooksController under /admin/api (AdminConstraint: a 404 for
//! anyone but an admin): the hooks' CRUD, their events, a ping and the
//! redeliveries.
//!
//! Refused: the Ember pages (an HTML request).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::web_hooks::admin::{self, Associations, Cx, Params, Refused, WebHook};
use crate::{AppError, AppState, Unsupported};

/// check_xhr: the admin pages themselves are Ember's.
fn json_request(headers: &HeaderMap, uri: &Uri) -> bool {
    uri.path().ends_with(".json")
        || headers
            .get("x-requested-with")
            .is_some_and(|v| v == "XMLHttpRequest")
        || super::wants_json(uri.query(), headers)
}

fn id_of(raw: &str) -> i64 {
    crate::ruby::to_i(raw.strip_suffix(".json").unwrap_or(raw))
}

fn pairs(p: &Map<String, Value>) -> Vec<(String, String)> {
    p.iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn errors(status: StatusCode, messages: Vec<String>) -> Response {
    (status, Json(json!({ "errors": messages }))).into_response()
}

/// The request as every action starts it: an admin, JSON, the CSRF token
/// for a write; else the response that ends it.
struct Admitted {
    params: Map<String, Value>,
    admin_id: i32,
}

fn admit(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    body: &[u8],
    method: Option<&str>,
) -> Result<Result<Admitted, Response>, AppError> {
    if !guardian.is_admin() {
        return Ok(Err(super::topics::not_found_response(state)));
    }
    let params = crate::params::parse(uri.query(), headers, body);
    if let Some(method) = method
        && !csrf_ok(state, headers, &pairs(&params), uri.path(), method)
    {
        return Ok(Err(bad_csrf()));
    }
    if !json_request(headers, uri) {
        return Err(Unsupported("the admin web hooks pages (Ember)").into());
    }
    let admin_id = guardian
        .user_id()
        .ok_or(Unsupported("an admin without a user"))?;
    Ok(Ok(Admitted { params, admin_id }))
}

async fn settings(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// `fetch_web_hook`: `WebHook.find(params[:id])`, a 404 when missing.
async fn fetch(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    id: &str,
) -> Result<Result<WebHook, Response>, AppError> {
    Ok(match WebHook::find(conn, id_of(id)).await? {
        Some(hook) => Ok(hook),
        None => Err(super::topics::not_found_response(state)),
    })
}

/// `render_serialized(web_hook, AdminWebHookSerializer, root: "web_hook")`
async fn rendered(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    hook: &WebHook,
    a: &Associations,
    extras_first: bool,
) -> Result<Response, AppError> {
    let cx = Cx {
        settings,
        i18n: &state.i18n,
        guardian,
        base_path: state.config.globals.relative_url_root(),
    };
    let (categories, body) = admin::rooted(conn, &cx, hook, a).await?;
    Ok(Json(if extras_first {
        json!({ "extras": { "categories": categories }, "web_hook": body })
    } else {
        json!({ "categories": categories, "web_hook": body })
    })
    .into_response())
}

/// GET /admin/api/web_hooks(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let admitted = match admit(&state, &guardian, &headers, &uri, &[], None)? {
        Ok(a) => a,
        Err(r) => return Ok(r),
    };
    let offset = crate::params::integer(&admitted.params, "offset").unwrap_or(0);
    let mut conn = state.pool.acquire().await?;
    let settings = settings(&state, &mut conn).await?;
    let cx = Cx {
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        base_path: state.config.globals.relative_url_root(),
    };
    Ok(Json(admin::index(&mut conn, &cx, offset).await?).into_response())
}

/// GET /admin/api/web_hooks/:id(.json): the categories first, as extras.
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Err(r) = admit(&state, &guardian, &headers, &uri, &[], None)? {
        return Ok(r);
    }
    let mut conn = state.pool.acquire().await?;
    let hook = match fetch(&state, &mut conn, &id).await? {
        Ok(h) => h,
        Err(r) => return Ok(r),
    };
    let settings = settings(&state, &mut conn).await?;
    let a = Associations::load(&mut conn, hook.id).await?;
    rendered(&state, &mut conn, &settings, &guardian, &hook, &a, true).await
}

/// GET /admin/api/web_hooks/:id/edit(.json): `edit` is not among
/// fetch_web_hook's actions, so it serializes nil whatever the id.
pub async fn edit(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Err(r) = admit(&state, &guardian, &headers, &uri, &[], None)? {
        return Ok(r);
    }
    Ok(Json(json!({ "web_hook": null, "extras": { "categories": null } })).into_response())
}

/// POST /admin/api/web_hooks(.json)
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let admitted = match admit(&state, &guardian, &headers, &uri, &body, Some("POST"))? {
        Ok(a) => a,
        Err(r) => return Ok(r),
    };
    let Some(params) = Params::permit(&admitted.params) else {
        return Ok(missing("web_hook"));
    };
    let mut tx = state.pool.begin().await?;
    let settings = settings(&state, &mut tx).await?;
    let cx = admin::Ctx {
        settings: &settings,
        i18n: &state.i18n,
        urls: Urls {
            config: &state.config,
            settings: &settings,
        },
        admin_id: admitted.admin_id,
    };
    match admin::create(&mut tx, &cx, &params).await? {
        Ok((hook, a)) => {
            let response =
                rendered(&state, &mut tx, &settings, &guardian, &hook, &a, false).await?;
            tx.commit().await?;
            Ok(response)
        }
        Err(refused) => Ok(refusal(&state, refused)),
    }
}

/// ParameterMissing
fn missing(name: &str) -> Response {
    errors(
        StatusCode::BAD_REQUEST,
        vec![format!(
            "param is missing or the value is empty or invalid: {name}"
        )],
    )
}

fn refusal(state: &AppState, refused: Refused) -> Response {
    match refused {
        Refused::Invalid(messages) => errors(StatusCode::UNPROCESSABLE_ENTITY, messages),
        Refused::NotFound => super::topics::not_found_response(state),
    }
}

/// PUT /admin/api/web_hooks/:id(.json)
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let admitted = match admit(&state, &guardian, &headers, &uri, &body, Some("PUT"))? {
        Ok(a) => a,
        Err(r) => return Ok(r),
    };
    let mut tx = state.pool.begin().await?;
    let hook = match fetch(&state, &mut tx, &id).await? {
        Ok(h) => h,
        Err(r) => return Ok(r),
    };
    let Some(params) = Params::permit(&admitted.params) else {
        return Ok(missing("web_hook"));
    };
    let settings = settings(&state, &mut tx).await?;
    let cx = admin::Ctx {
        settings: &settings,
        i18n: &state.i18n,
        urls: Urls {
            config: &state.config,
            settings: &settings,
        },
        admin_id: admitted.admin_id,
    };
    match admin::update(&mut tx, &cx, &hook, &params).await? {
        Ok((hook, a)) => {
            let response =
                rendered(&state, &mut tx, &settings, &guardian, &hook, &a, false).await?;
            tx.commit().await?;
            Ok(response)
        }
        // `update` rolls its transaction back.
        Err(refused) => Ok(refusal(&state, refused)),
    }
}

/// DELETE /admin/api/web_hooks/:id(.json)
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let admitted = match admit(&state, &guardian, &headers, &uri, &body, Some("DELETE"))? {
        Ok(a) => a,
        Err(r) => return Ok(r),
    };
    let mut tx = state.pool.begin().await?;
    let hook = match fetch(&state, &mut tx, &id).await? {
        Ok(h) => h,
        Err(r) => return Ok(r),
    };
    admin::destroy(&mut tx, admitted.admin_id, &hook).await?;
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// GET /admin/api/web_hook_events/:id(.json): the hook's events, a page.
pub async fn list_events(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let admitted = match admit(&state, &guardian, &headers, &uri, &[], None)? {
        Ok(a) => a,
        Err(r) => return Ok(r),
    };
    let mut conn = state.pool.acquire().await?;
    let hook = match fetch(&state, &mut conn, &id).await? {
        Ok(h) => h,
        Err(r) => return Ok(r),
    };
    let offset = crate::params::integer(&admitted.params, "offset").unwrap_or(0);
    let status = crate::params::string(&admitted.params, "status");
    let base_path = state.config.globals.relative_url_root();
    Ok(
        Json(admin::list_events(&mut conn, base_path, &hook, offset, status.as_deref()).await?)
            .into_response(),
    )
}

/// `params[:ids]` as `where(id:)` takes it: a list, or one id.
fn id_list(v: Option<&Value>) -> Vec<i64> {
    let to_id = |v: &Value| crate::params::scalar(v).map(|s| crate::ruby::to_i(&s));
    match v {
        Some(Value::Array(items)) => items.iter().filter_map(to_id).collect(),
        Some(other) => to_id(other).into_iter().collect(),
        None => Vec::new(),
    }
}

/// GET /admin/api/web_hooks/:id/events/bulk(.json)
pub async fn bulk_events(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let admitted = match admit(&state, &guardian, &headers, &uri, &[], None)? {
        Ok(a) => a,
        Err(r) => return Ok(r),
    };
    let mut conn = state.pool.acquire().await?;
    let hook = match fetch(&state, &mut conn, &id).await? {
        Ok(h) => h,
        Err(r) => return Ok(r),
    };
    // params.require(:ids)
    let ids = admitted.params.get("ids");
    let present = match ids {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(_) => true,
    };
    if !present {
        return Ok(missing("ids"));
    }
    let events = admin::events_by_id(&mut conn, &hook, &id_list(ids), false).await?;
    Ok(Json(
        events
            .iter()
            .map(|e| admin::serialize_event(e, &hook.payload_url))
            .collect::<Vec<_>>(),
    )
    .into_response())
}

/// POST /admin/api/web_hooks/:id/events/:event_id/redeliver(.json): sent
/// again now, with its stored headers.
pub async fn redeliver_event(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((id, event_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Err(r) = admit(&state, &guardian, &headers, &uri, &body, Some("POST"))? {
        return Ok(r);
    }
    let mut conn = state.pool.acquire().await?;
    let hook = match fetch(&state, &mut conn, &id).await? {
        Ok(h) => h,
        Err(r) => return Ok(r),
    };
    let event = admin::events_by_id(&mut conn, &hook, &[id_of(&event_id)], false)
        .await?
        .into_iter()
        .next();
    let Some(event) = event else {
        let text = state
            .i18n
            .t("not_found")
            .unwrap_or("The requested URL or resource could not be found.");
        return Ok(errors(StatusCode::NOT_FOUND, vec![text.to_string()]));
    };
    let settings = settings(&state, &mut conn).await?;
    crate::web_hooks::emit::redeliver(&mut conn, &state, &settings, &hook, &event).await?;
    let event = admin::Event::find(&mut conn, event.id)
        .await?
        .ok_or(Unsupported("a redelivered event that vanished"))?;
    Ok(
        Json(json!({ "web_hook_event": admin::serialize_event(&event, &hook.payload_url) }))
            .into_response(),
    )
}

/// POST /admin/api/web_hooks/:id/events/failed_redeliver(.json)
pub async fn redeliver_failed_events(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let admitted = match admit(&state, &guardian, &headers, &uri, &body, Some("POST"))? {
        Ok(a) => a,
        Err(r) => return Ok(r),
    };
    let mut tx = state.pool.begin().await?;
    let hook = match fetch(&state, &mut tx, &id).await? {
        Ok(h) => h,
        Err(r) => return Ok(r),
    };
    let ids = id_list(admitted.params.get("event_ids"));
    let Some(ids) = admin::redeliver_failed(&mut tx, &hook, &ids).await? else {
        return Ok(super::search::invalid_parameters(
            &state,
            "Discourse::InvalidParameters",
        ));
    };
    tx.commit().await?;
    Ok(Json(json!({ "event_ids": ids })).into_response())
}

/// POST /admin/api/web_hooks/:id/ping(.json): a ping event enqueued.
pub async fn ping(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Err(r) = admit(&state, &guardian, &headers, &uri, &body, Some("POST"))? {
        return Ok(r);
    }
    let mut conn = state.pool.acquire().await?;
    let hook = match fetch(&state, &mut conn, &id).await? {
        Ok(h) => h,
        Err(r) => return Ok(r),
    };
    crate::jobs::enqueue(
        &mut conn,
        crate::web_hooks::emit::EMIT_WEB_HOOK_EVENT,
        json!({ "web_hook_id": hook.id, "event_type": "ping", "event_name": "ping" }),
    )
    .await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}
