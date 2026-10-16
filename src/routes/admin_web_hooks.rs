//! Admin::WebHooksController under /admin/api (AdminConstraint: a 404 for
//! anyone but an admin): the hooks' CRUD, their events, a ping and the
//! redeliveries.
//!
//! The pages themselves as HTML (crate::admin_web_hooks_view).

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

/// Which web hooks page.
enum Page {
    Index,
    New,
    Show(String),
    Edit(String),
}

/// The web hooks pages: the index's document (its first page and extras),
/// with a hook's for its events and edit pages.
async fn page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    which: Page,
) -> Result<Response, AppError> {
    use crate::admin_web_hooks_view as view;
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(state));
    }
    let mut conn = state.pool.acquire().await?;
    let shell =
        super::admin_pages::Shell::load(&mut conn, state, guardian, headers, "admin_webhooks")
            .await?;
    let settings = &shell.settings;
    let hook_cx = Cx {
        settings,
        i18n: &state.i18n,
        guardian,
        base_path: state.config.globals.relative_url_root(),
    };
    let index = admin::index(&mut conn, &hook_cx, 0).await?;
    let cx = shell.cx(state);
    let mut extra = String::new();
    // The hook WebhooksForm saves, for the script.
    let mut attrs = String::new();
    let (classes, content) = match which {
        Page::Index => ("admin-api admin-web-hooks", view::list(&cx, &index)),
        Page::Show(id) => {
            let hook = match fetch(state, &mut conn, &id).await? {
                Ok(h) => h,
                Err(r) => return Ok(r),
            };
            let a = Associations::load(&mut conn, hook.id).await?;
            let (_, body) = admin::rooted(&mut conn, &hook_cx, &hook, &a).await?;
            let status = form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
                .find(|(k, _)| k == "status")
                .map(|(_, v)| v.into_owned())
                .filter(|v| !v.is_empty());
            let events =
                admin::list_events(&mut conn, hook_cx.base_path, &hook, 0, status.as_deref())
                    .await?;
            (
                "admin-api admin-web-hooks",
                view::show(&cx, &body, &events, status.as_deref()),
            )
        }
        Page::New | Page::Edit(_) => {
            let (hook, categories) = match &which {
                Page::Edit(id) => {
                    let hook = match fetch(state, &mut conn, id).await? {
                        Ok(h) => h,
                        Err(r) => return Ok(r),
                    };
                    let a = Associations::load(&mut conn, hook.id).await?;
                    attrs = format!(" data-web-hook-id=\"{}\"", hook.id);
                    let (categories, body) = admin::rooted(&mut conn, &hook_cx, &hook, &a).await?;
                    (Some(body), categories)
                }
                _ => (None, Value::Array(Vec::new())),
            };
            let categories: Vec<(String, String)> = categories
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| {
                    (
                        c["id"].as_i64().unwrap_or(0).to_string(),
                        c["name"].as_str().unwrap_or_default().to_string(),
                    )
                })
                .collect();
            let group_ids: Vec<i32> = hook
                .as_ref()
                .and_then(|h| h["group_ids"].as_array())
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_i64().and_then(|i| i32::try_from(i).ok()))
                .collect();
            // updateGroupsFilter: Site's groups, in its order (by name).
            let groups: Vec<(String, String)> = sqlx::query_as::<_, (i32, String)>(
                "SELECT id, name FROM groups WHERE id = ANY($1) ORDER BY name",
            )
            .bind(&group_ids)
            .fetch_all(&mut *conn)
            .await?
            .into_iter()
            .map(|(id, name)| (id.to_string(), name))
            .collect();
            let list_categories = crate::topic_list_view::categories(&mut conn).await?;
            let list = crate::topic_list_view::ListContext {
                i18n: &state.i18n,
                base_path: &shell.site.base_path,
                now: crate::clock::now(),
                categories: &list_categories,
                expand_all_pinned: false,
                member_trust_level: None,
                settings: crate::topic_list_view::ListSettings::load(settings)?,
            };
            let form = view::form(
                &cx,
                &list,
                &view::FormData {
                    hook: hook.as_ref(),
                    extras: &index["extras"],
                    categories,
                    groups,
                    tagging_enabled: settings.get("tagging_enabled")?.truthy(),
                },
            );
            // CategorySelector's rows and the choosers' strings for
            // prefs-choosers.js.
            let allowed = guardian.allowed_category_ids(&mut conn, settings).await?;
            let rows: Vec<(i32, i32)> =
                sqlx::query_as("SELECT id, topic_count FROM categories WHERE id = ANY($1)")
                    .bind(&allowed)
                    .fetch_all(&mut *conn)
                    .await?;
            let chooser: Vec<crate::composer_view::ChooserCategory> = rows
                .into_iter()
                .map(|(id, topic_count)| crate::composer_view::ChooserCategory {
                    id: i64::from(id),
                    topic_count: i64::from(topic_count),
                    description_text: None,
                    minimum_required_tags: 0,
                })
                .collect();
            let base = &shell.site.base_path;
            let username = guardian
                .user()
                .map(|u| u.username.clone())
                .unwrap_or_default();
            extra.push_str(&crate::preferences_view::tracking::category_rows(
                &list,
                &crate::preferences_view::tracking::SiteCategories { allowed: &chooser },
            ));
            extra.push_str(&crate::preferences_view::users::choosers_strings(
                &list,
                settings,
                &username,
                None,
                (
                    &crate::assets::url(base, "moment.min.js"),
                    &crate::assets::url(base, "moment-timezone-with-data-10-year-range.min.js"),
                ),
            )?);
            extra.push_str(&format!(
                "<script src=\"{}\"></script>",
                crate::assets::url(base, "prefs-choosers.js")
            ));
            let classes = if matches!(which, Page::New) {
                "admin-api admin-web-hooks new"
            } else {
                "admin-api admin-web-hooks edit"
            };
            (classes, form)
        }
    };
    let main = format!(
        "{}{}{extra}",
        view::strings(&cx),
        view::render(
            &cx,
            &content,
            !classes.ends_with(" new") && !classes.ends_with(" edit")
        )
    );
    let main = crate::admin_ui::contents(classes, &attrs, &main);
    // No route of these has a title token.
    shell.render(state, uri, "", main, "admin-web-hooks.js", StatusCode::OK)
}

/// GET /admin/api/web_hooks(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if !json_request(&headers, &uri) {
        return page(&state, &guardian, &headers, &uri, Page::Index).await;
    }
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
    if !json_request(&headers, &uri) {
        let which = if id == "new" {
            Page::New
        } else {
            Page::Show(id)
        };
        return page(&state, &guardian, &headers, &uri, which).await;
    }
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
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if !json_request(&headers, &uri) {
        return page(&state, &guardian, &headers, &uri, Page::Edit(id)).await;
    }
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
