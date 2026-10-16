//! /admin/api/keys: Admin::ApiController (admins; everyone else 404s, as
//! AdminConstraint has it). Listing, showing, creating (global, for one
//! user, read only or granular with allowed parameters), describing,
//! revoking, restoring and destroying admin API keys, each logged; and the
//! scopes a key can be given (crate::api_key_scopes).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::api_key_scopes;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// UserHistory.actions
const API_KEY_CREATE: i32 = 80;
const API_KEY_UPDATE: i32 = 81;
const API_KEY_DESTROY: i32 = 82;
/// `INDEX_LIMIT`
const INDEX_LIMIT: i64 = 50;

/// `ApiKey.scope_modes`
fn scope_mode_name(mode: Option<i32>) -> Value {
    match mode {
        Some(0) => json!("global"),
        Some(1) => json!("read_only"),
        Some(2) => json!("granular"),
        _ => Value::Null,
    }
}

fn admin_only(state: &AppState, guardian: &crate::guardian::Guardian) -> Option<Response> {
    (!guardian.is_admin()).then(|| super::topics::not_found_response(state))
}

/// check_xhr: the admin pages themselves are Ember's.
fn refuse_html(uri: &Uri, headers: &HeaderMap) -> Result<(), AppError> {
    if super::wants_json(uri.query(), headers) || uri.path().ends_with(".json") {
        return Ok(());
    }
    Err(Unsupported("the admin API keys pages (Ember)").into())
}

fn strip_json(id: &str) -> &str {
    id.strip_suffix(".json").unwrap_or(id)
}

fn pairs(p: &Map<String, Value>) -> Vec<(String, String)> {
    p.iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

async fn settings(state: &AppState, conn: &mut PgConnection) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// BasicUserSerializer, or null.
async fn basic_user(
    conn: &mut PgConnection,
    urls: &Urls<'_>,
    id: Option<i32>,
) -> Result<Value, AppError> {
    let Some(id) = id else {
        return Ok(Value::Null);
    };
    if urls.settings.get("enable_user_status")?.truthy() {
        return Err(Unsupported("user status on BasicUserSerializer").into());
    }
    let row: Option<(String, Option<String>, Option<i32>)> =
        sqlx::query_as("SELECT username, name, uploaded_avatar_id FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((username, name, avatar)) = row else {
        return Ok(Value::Null);
    };
    let logo = crate::admin_users::logo_small_url(&mut *conn, urls.settings).await?;
    let mut out = Map::new();
    out.insert("id".into(), json!(id));
    out.insert("username".into(), json!(username));
    if urls.settings.get("enable_names")?.truthy() {
        out.insert("name".into(), json!(name));
    }
    out.insert(
        "avatar_template".into(),
        json!(crate::avatar::avatar_template(
            urls,
            id,
            &username,
            avatar,
            logo.as_deref()
        )?),
    );
    Ok(Value::Object(out))
}

#[derive(sqlx::FromRow)]
struct KeyRow {
    id: i32,
    user_id: Option<i32>,
    created_by_id: Option<i32>,
    truncated_key: String,
    description: Option<String>,
    scope_mode: Option<i32>,
    last_used_at: Option<chrono::NaiveDateTime>,
    created_at: chrono::NaiveDateTime,
    updated_at: chrono::NaiveDateTime,
    revoked_at: Option<chrono::NaiveDateTime>,
}

const KEY_COLUMNS: &str = "id, user_id, created_by_id, truncated_key, description, scope_mode, \
    last_used_at, created_at, updated_at, revoked_at";

async fn find(conn: &mut PgConnection, id: i32) -> Result<Option<KeyRow>, AppError> {
    Ok(
        sqlx::query_as(&format!("SELECT {KEY_COLUMNS} FROM api_keys WHERE id = $1"))
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?,
    )
}

fn time(t: chrono::NaiveDateTime) -> Value {
    json!(crate::topic_list::time_json(t))
}

/// ApiKeyScopeSerializer
fn scope_json(resource: &str, action: &str, allowed: Option<Value>) -> Value {
    let mapping = api_key_scopes::mapping(resource, action);
    json!({
        "resource": resource,
        "action": action.replace('_', " "),
        "parameters": mapping.and_then(|m| m.restrictable_parameters()).unwrap_or_default(),
        "urls": mapping.map(|m| m.urls.clone()).unwrap_or_default(),
        "allowed_parameters": allowed.unwrap_or(Value::Null),
        "key": action,
    })
}

/// ApiKeySerializer (`key` only when just created).
async fn key_json(
    conn: &mut PgConnection,
    urls: &Urls<'_>,
    k: &KeyRow,
    plain_key: Option<&str>,
) -> Result<Value, AppError> {
    let scopes: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT resource, action, allowed_parameters::text FROM api_key_scopes \
         WHERE api_key_id = $1 ORDER BY id",
    )
    .bind(k.id)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Map::new();
    out.insert("id".into(), json!(k.id));
    if let Some(key) = plain_key {
        out.insert("key".into(), json!(key));
    }
    out.insert("truncated_key".into(), json!(k.truncated_key));
    out.insert("description".into(), json!(k.description));
    out.insert("scope_mode".into(), scope_mode_name(k.scope_mode));
    out.insert(
        "last_used_at".into(),
        json!(k.last_used_at.map(crate::topic_list::time_json)),
    );
    out.insert("created_at".into(), time(k.created_at));
    out.insert("updated_at".into(), time(k.updated_at));
    out.insert(
        "revoked_at".into(),
        json!(k.revoked_at.map(crate::topic_list::time_json)),
    );
    out.insert("user".into(), basic_user(conn, urls, k.user_id).await?);
    out.insert(
        "created_by".into(),
        basic_user(conn, urls, k.created_by_id).await?,
    );
    out.insert(
        "api_key_scopes".into(),
        Value::Array(
            scopes
                .into_iter()
                .map(|(r, a, allowed)| {
                    let allowed = allowed.and_then(|t| serde_json::from_str(&t).ok());
                    scope_json(&r, &a, allowed)
                })
                .collect(),
        ),
    );
    Ok(Value::Object(out))
}

/// GET /admin/api/keys(.json) and /admin/api(.json): the keys that are
/// not hidden, revoked last, newest first.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    if !wants_json(&uri, &headers) {
        // adminApi's index route goes on to the keys.
        if !uri.path().trim_end_matches('/').ends_with("/keys") {
            let base = state.config.globals.relative_url_root();
            return Ok(
                axum::response::Redirect::to(&format!("{base}/admin/api/keys")).into_response(),
            );
        }
        return page(&state, &guardian, &headers, &uri, Page::Index).await;
    }
    let q = crate::params::parse_query(uri.query().unwrap_or_default());
    let offset = q
        .get("offset")
        .and_then(crate::params::scalar)
        .map_or(0, |o| crate::ruby::to_i(&o));
    // fetch_limit_from_params(default: INDEX_LIMIT, max: INDEX_LIMIT)
    let limit = match q.get("limit") {
        None => INDEX_LIMIT,
        Some(v) => match crate::params::scalar(v).and_then(|s| s.trim().parse::<i64>().ok()) {
            Some(n) if (0..=INDEX_LIMIT).contains(&n) => n,
            _ => return Ok(super::search::invalid_parameters(&state, "limit")),
        },
    };
    let mut conn = state.pool.acquire().await?;
    let settings = settings(&state, &mut conn).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let keys = keys_doc(&mut conn, &urls, offset, limit).await?;
    Ok(Json(json!({"keys": keys, "offset": offset, "limit": limit})).into_response())
}

/// The index's keys (BasicApiKeySerializer).
async fn keys_doc(
    conn: &mut PgConnection,
    urls: &Urls<'_>,
    offset: i64,
    limit: i64,
) -> Result<Vec<Value>, AppError> {
    let rows: Vec<KeyRow> = sqlx::query_as(&format!(
        "SELECT {KEY_COLUMNS} FROM api_keys WHERE NOT hidden \
         ORDER BY revoked_at DESC NULLS FIRST, created_at DESC OFFSET $1 LIMIT $2"
    ))
    .bind(offset)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    let mut keys = Vec::with_capacity(rows.len());
    for k in &rows {
        keys.push(json!({
            "id": k.id,
            "truncated_key": k.truncated_key,
            "scope_mode": scope_mode_name(k.scope_mode),
            "description": k.description,
            "created_at": time(k.created_at),
            "last_used_at": k.last_used_at.map(crate::topic_list::time_json),
            "revoked_at": k.revoked_at.map(crate::topic_list::time_json),
            "user": basic_user(&mut *conn, urls, k.user_id).await?,
            "created_by": basic_user(&mut *conn, urls, k.created_by_id).await?,
        }));
    }
    Ok(keys)
}

fn wants_json(uri: &Uri, headers: &HeaderMap) -> bool {
    super::wants_json(uri.query(), headers) || uri.path().ends_with(".json")
}

/// Which API keys page.
enum Page {
    Index,
    Settings,
    New,
    Show(i32),
}

/// The API keys pages: the index's keys (its first page, as
/// store.findAll loads them), a key, the scopes for the new key's form,
/// or the user_api settings.
async fn page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    page: Page,
) -> Result<Response, AppError> {
    use crate::admin_api_keys_view::{self as view, Tab};
    let mut conn = state.pool.acquire().await?;
    let shell =
        super::admin_pages::Shell::load(&mut conn, state, guardian, headers, "admin_api_keys")
            .await?;
    let urls = Urls {
        config: &state.config,
        settings: &shell.settings,
    };
    let cx = shell.cx(state);
    let (tab, content) = match page {
        Page::Index => {
            let keys = keys_doc(&mut conn, &urls, 0, INDEX_LIMIT).await?;
            (Tab::Index, view::list(&cx, &keys))
        }
        Page::Show(id) => {
            let Some(k) = find(&mut conn, id).await? else {
                return Ok(super::topics::not_found_response(state));
            };
            let key = key_json(&mut conn, &urls, &k, None).await?;
            (Tab::Show, view::show(&cx, &key))
        }
        Page::New => (Tab::New, view::new(&cx, &scopes_doc())),
        Page::Settings => (
            Tab::Settings,
            super::admin_advanced::area_settings(
                &mut conn,
                state,
                &shell,
                uri,
                json!({ "categories": "user_api" }),
            )
            .await?,
        ),
    };
    let main = format!("{}{}", view::strings(&cx), view::render(&cx, tab, &content));
    let (classes, title, script) = match tab {
        Tab::Index => (
            "admin-api admin-api-keys",
            Some("admin.config.api_keys.title"),
            "admin-api-keys.js",
        ),
        Tab::Show => ("admin-api admin-api-keys", None, "admin-api-keys.js"),
        Tab::New => ("admin-api admin-api-keys new", None, "admin-api-keys.js"),
        Tab::Settings => (
            "admin-api admin-api-keys settings",
            Some("settings"),
            "site-settings.js",
        ),
    };
    let main = crate::admin_view::contents(classes, &main);
    // The key and new routes have no title token: the page is Admin's.
    let title = title
        .map(|k| crate::admin_view::t(&state.i18n, k))
        .unwrap_or_default();
    shell.render(state, uri, &title, main, script, StatusCode::OK)
}

/// ApiKeysNew's scopes: /admin/api/keys/scopes.json's.
fn scopes_doc() -> Value {
    let mut resources: Vec<&(String, Vec<(String, api_key_scopes::Mapping)>)> =
        api_key_scopes::scope_mappings().iter().collect();
    resources.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = Map::new();
    for (resource, actions) in resources {
        out.insert(
            resource.clone(),
            Value::Array(
                actions
                    .iter()
                    .map(|(name, m)| {
                        json!({
                            "scope_id": format!("{resource}:{name}"),
                            "key": name,
                            "name": name.replace('_', " "),
                            "params": m.restrictable_parameters(),
                            "urls": m.urls,
                        })
                    })
                    .collect(),
            ),
        );
    }
    Value::Object(out)
}

/// GET /admin/api/keys/scopes(.json): every resource's scopes, by
/// resource name.
pub async fn scopes(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    refuse_html(&uri, &headers)?;
    Ok(Json(json!({"scopes": scopes_doc()})).into_response())
}

/// GET /admin/api/keys/:id(.json): the key; as a page the key's, or the
/// new key's form and the settings tab, whose paths this matches too.
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    if !wants_json(&uri, &headers) {
        let page_for = match id.as_str() {
            "new" => Page::New,
            "settings" => Page::Settings,
            id => match id.parse::<i32>() {
                Ok(id) => Page::Show(id),
                Err(_) => return Ok(super::topics::not_found_response(&state)),
            },
        };
        return page(&state, &guardian, &headers, &uri, page_for).await;
    }
    let mut conn = state.pool.acquire().await?;
    let settings = settings(&state, &mut conn).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let id = i32::try_from(crate::ruby::to_i(strip_json(&id))).unwrap_or(0);
    // ApiKey.find: a missing key is RecordNotFound, which Rails renders
    // as its debug page here.
    let Some(k) = find(&mut conn, id).await? else {
        return Err(Unsupported("showing an API key that does not exist").into());
    };
    Ok(Json(json!({"key": key_json(&mut conn, &urls, &k, None).await?})).into_response())
}

/// `update_params`: `params.permit(key: [:description])[:key]`.
fn update_params(p: &Map<String, Value>) -> Option<Option<String>> {
    match p.get("key") {
        Some(Value::Object(k)) => Some(k.get("description").and_then(crate::params::scalar)),
        _ => None,
    }
}

/// `validates :description, length: { maximum: 255 }`
fn description_error(state: &AppState, description: Option<&str>) -> Option<String> {
    let d = description?;
    (d.chars().count() > 255).then(|| {
        let message = state
            .i18n
            .t_count("errors.messages.too_long", 255, &[("count", "255")])
            .unwrap_or_else(|| "is too long (maximum is 255 characters)".to_string());
        format!("Description {message}")
    })
}

fn record_invalid(errors: Vec<String>) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"errors": errors, "error_type": "record_invalid"})),
    )
        .into_response()
}

fn invalid_parameters(state: &AppState) -> Response {
    super::search::invalid_parameters(state, "Discourse::InvalidParameters")
}

/// StaffActionLogger#log_api_key and friends: subject the truncated key.
#[allow(clippy::too_many_arguments)]
async fn log(
    conn: &mut PgConnection,
    acting: i32,
    action: i32,
    truncated: &str,
    details: Option<&str>,
    previous_value: Option<&str>,
    new_value: Option<&str>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, subject, details, previous_value, \
                                     new_value, admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(action)
    .bind(acting)
    .bind(truncated)
    .bind(details)
    .bind(previous_value)
    .bind(new_value)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `TimeWithZone#to_s`
fn ruby_time(t: chrono::NaiveDateTime) -> String {
    t.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

/// POST /admin/api/keys(.json)
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    // AdminConstraint routes before the controller checks the token.
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "POST") {
        return Ok(super::session::bad_csrf());
    }
    let acting = guardian.user_id().unwrap_or_default();
    let Some(description) = update_params(&p) else {
        return Ok(invalid_parameters(&state));
    };
    let key_params = p
        .get("key")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let scope_mode = key_params.get("scope_mode").and_then(crate::params::scalar);
    let scope_list: Vec<Value> = match key_params.get("scopes") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.clone(),
        Some(Value::Object(o)) => o.values().cloned().collect(),
        Some(_) => return Err(Unsupported("API key scopes not given as a list").into()),
    };
    // validate_read_only_scopes!
    if scope_mode.as_deref() == Some("read_only") && scope_list.is_empty() {
        return Ok(invalid_parameters(&state));
    }
    // build_scopes
    let mut scopes: Vec<(String, String, Option<Value>)> = Vec::new();
    for s in &scope_list {
        let Some(scope_id) = s.get("scope_id").and_then(crate::params::scalar) else {
            return Err(Unsupported("an API key scope without a scope_id").into());
        };
        let Some((resource, action)) = scope_id.split_once(':') else {
            return Err(Unsupported("an API key scope_id without an action").into());
        };
        let action = action.split(':').next().unwrap_or_default();
        let Some(mapping) = api_key_scopes::mapping(resource, action) else {
            return Ok(invalid_parameters(&state));
        };
        let allowed = mapping.restrictable_parameters().map(|names| {
            let mut out = Map::new();
            for name in names {
                // Blank values dropped; Ruby's split(",") drops trailing
                // empty pieces.
                if let Some(v) = s.get(&name).and_then(crate::params::scalar)
                    && !v.trim().is_empty()
                {
                    let mut pieces: Vec<&str> = v.split(',').collect();
                    while pieces.last().is_some_and(|x| x.is_empty()) {
                        pieces.pop();
                    }
                    out.insert(
                        name,
                        Value::Array(pieces.into_iter().map(|x| json!(x)).collect()),
                    );
                }
            }
            Value::Object(out)
        });
        scopes.push((resource.to_string(), action.to_string(), allowed));
    }
    let mode = match scope_mode.as_deref() {
        None | Some("") => None,
        Some("global") => Some(0),
        Some("read_only") => Some(1),
        Some("granular") => Some(2),
        // ArgumentError on the enum: a 500 in Rails.
        Some(_) => return Err(Unsupported("an unknown API key scope_mode").into()),
    };
    let mut conn = state.pool.acquire().await?;
    let settings = settings(&state, &mut conn).await?;
    let user_id = match key_params
        .get("username")
        .and_then(crate::params::scalar)
        .filter(|u| !u.trim().is_empty())
    {
        Some(username) => {
            let id: Option<i32> =
                sqlx::query_scalar("SELECT id FROM users WHERE username_lower = $1")
                    .bind(username.to_lowercase())
                    .fetch_optional(&mut *conn)
                    .await?;
            match id {
                Some(id) => Some(id),
                None => return Ok(super::topics::not_found_response(&state)),
            }
        }
        None => None,
    };
    // save!'s validations: the description, then the scope mode's scopes.
    let mut errors = Vec::new();
    if let Some(e) = description_error(&state, description.as_deref()) {
        errors.push(e);
    }
    match mode {
        Some(2) if scopes.is_empty() => errors.push(format!(
            "Api key scopes {}",
            state
                .i18n
                .t("activerecord.errors.models.api_key.base.at_least_one_granular_scope")
                .unwrap_or("at least one must be selected")
        )),
        Some(1) if !(scopes.len() == 1 && scopes[0].0 == "global" && scopes[0].1 == "read") => {
            errors.push(format!(
                "Api key scopes {}",
                state
                    .i18n
                    .t("activerecord.errors.models.api_key.base.read_only_scope")
                    .unwrap_or("must be the global read scope")
            ))
        }
        _ => {}
    }
    if !errors.is_empty() {
        return Ok(record_invalid(errors));
    }
    let key = format!(
        "{}{}",
        crate::accounts::random_hex(),
        crate::accounts::random_hex()
    );
    let truncated = key[..4].to_string();
    let hash = crate::session::user_api_key::hash_key(&key);
    let mut tx = state.pool.begin().await?;
    let k: KeyRow = sqlx::query_as(&format!(
        "INSERT INTO api_keys (user_id, created_by_id, created_at, updated_at, description, key_hash, \
                               truncated_key, scope_mode) \
         VALUES ($1, $2, now(), now(), $3, $4, $5, $6) RETURNING {KEY_COLUMNS}"
    ))
    .bind(user_id)
    .bind(acting)
    .bind(&description)
    .bind(&hash)
    .bind(&truncated)
    .bind(mode)
    .fetch_one(&mut *tx)
    .await?;
    for (resource, action, allowed) in &scopes {
        sqlx::query(
            "INSERT INTO api_key_scopes (api_key_id, resource, action, allowed_parameters, created_at, updated_at) \
             VALUES ($1, $2, $3, $4::json, now(), now())",
        )
        .bind(k.id)
        .bind(resource)
        .bind(action)
        .bind(allowed.as_ref().map(Value::to_string))
        .execute(&mut *tx)
        .await?;
    }
    // saved_changes, sorted, without updated_at.
    let mut changes: Vec<(String, String)> = vec![
        ("created_at".into(), ruby_time(k.created_at)),
        ("created_by_id".into(), acting.to_string()),
        ("id".into(), k.id.to_string()),
        ("key_hash".into(), hash.clone()),
        ("truncated_key".into(), truncated.clone()),
    ];
    if let Some(d) = &description {
        changes.push(("description".into(), d.clone()));
    }
    if let Value::String(m) = scope_mode_name(mode) {
        changes.push(("scope_mode".into(), m));
    }
    if let Some(u) = user_id {
        changes.push(("user_id".into(), u.to_string()));
    }
    changes.sort();
    let new_value = changes
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join(", ");
    log(
        &mut tx,
        acting,
        API_KEY_CREATE,
        &truncated,
        None,
        None,
        Some(&new_value),
    )
    .await?;
    tx.commit().await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    Ok(Json(json!({"key": key_json(&mut conn, &urls, &k, Some(&key)).await?})).into_response())
}

/// PUT /admin/api/keys/:id(.json): the description, logged (even when
/// unchanged).
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    let acting = guardian.user_id().unwrap_or_default();
    let mut conn = state.pool.acquire().await?;
    let id = i32::try_from(crate::ruby::to_i(strip_json(&id))).unwrap_or(0);
    let Some(k) = find(&mut conn, id).await? else {
        return Err(Unsupported("updating an API key that does not exist").into());
    };
    let Some(description) = update_params(&p) else {
        return Ok(invalid_parameters(&state));
    };
    let key_params = p.get("key").and_then(Value::as_object);
    // Only a description the request names is assigned.
    let assigned = key_params.is_some_and(|k| k.contains_key("description"));
    if assigned && let Some(e) = description_error(&state, description.as_deref()) {
        return Ok(record_invalid(vec![e]));
    }
    let changed = assigned && description != k.description;
    let mut tx = state.pool.begin().await?;
    let (previous, new) = if changed {
        sqlx::query("UPDATE api_keys SET description = $2, updated_at = now() WHERE id = $1")
            .bind(k.id)
            .bind(&description)
            .execute(&mut *tx)
            .await?;
        (
            format!("description: {}", k.description.clone().unwrap_or_default()),
            format!("description: {}", description.clone().unwrap_or_default()),
        )
    } else {
        (String::new(), String::new())
    };
    log(
        &mut tx,
        acting,
        API_KEY_UPDATE,
        &k.truncated_key,
        None,
        Some(&previous),
        Some(&new),
    )
    .await?;
    tx.commit().await?;
    let settings = settings(&state, &mut conn).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let k = find(&mut conn, id)
        .await?
        .ok_or(Unsupported("an API key that vanished"))?;
    Ok(Json(json!({"key": key_json(&mut conn, &urls, &k, None).await?})).into_response())
}

/// DELETE /admin/api/keys/:id(.json): the key (its scopes stay, as
/// `has_many :api_key_scopes` has no `dependent:`), logged.
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "DELETE") {
        return Ok(super::session::bad_csrf());
    }
    let acting = guardian.user_id().unwrap_or_default();
    let mut conn = state.pool.acquire().await?;
    let id = i32::try_from(crate::ruby::to_i(strip_json(&id))).unwrap_or(0);
    let Some(k) = find(&mut conn, id).await? else {
        return Err(Unsupported("destroying an API key that does not exist").into());
    };
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM api_keys WHERE id = $1")
        .bind(k.id)
        .execute(&mut *tx)
        .await?;
    log(
        &mut tx,
        acting,
        API_KEY_DESTROY,
        &k.truncated_key,
        None,
        None,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"success": "OK"})).into_response())
}

/// POST /admin/api/keys/:id/revoke and /undo-revoke: revoked_at set (or
/// cleared), logged.
async fn set_revoked(
    state: AppState,
    guardian: crate::guardian::Guardian,
    id: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    revoke: bool,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    // AdminConstraint routes before the controller checks the token.
    if let Some(r) = admin_only(&state, &guardian) {
        return Ok(r);
    }
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "POST") {
        return Ok(super::session::bad_csrf());
    }
    let acting = guardian.user_id().unwrap_or_default();
    let mut conn = state.pool.acquire().await?;
    let id = i32::try_from(crate::ruby::to_i(&id)).unwrap_or(0);
    let Some(k) = find(&mut conn, id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let mut tx = state.pool.begin().await?;
    if revoke {
        sqlx::query(
            "UPDATE api_keys SET revoked_at = now, updated_at = now FROM clock_timestamp() AS now WHERE id = $1",
        )
        .bind(k.id)
        .execute(&mut *tx)
        .await?;
    } else if k.revoked_at.is_some() {
        sqlx::query("UPDATE api_keys SET revoked_at = NULL, updated_at = now() WHERE id = $1")
            .bind(k.id)
            .execute(&mut *tx)
            .await?;
    }
    let details = state
        .i18n
        .t(if revoke {
            "staff_action_logs.api_key.revoked"
        } else {
            "staff_action_logs.api_key.restored"
        })
        .unwrap_or_default()
        .to_string();
    log(
        &mut tx,
        acting,
        API_KEY_UPDATE,
        &k.truncated_key,
        Some(&details),
        None,
        None,
    )
    .await?;
    tx.commit().await?;
    let settings = settings(&state, &mut conn).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let k = find(&mut conn, id)
        .await?
        .ok_or(Unsupported("an API key that vanished"))?;
    Ok(Json(json!({"api_key": key_json(&mut conn, &urls, &k, None).await?})).into_response())
}

/// POST /admin/api/keys/:id/revoke(.json)
pub async fn revoke(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    set_revoked(state, guardian, id, headers, uri, body, true).await
}

/// POST /admin/api/keys/:id/undo-revoke(.json)
pub async fn undo_revoke(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    set_revoked(state, guardian, id, headers, uri, body, false).await
}
