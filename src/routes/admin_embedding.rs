//! Admin::EmbeddableHostsController (/admin/embeddable_hosts) and
//! Admin::EmbeddingController (/admin/customize/embedding), both under
//! AdminConstraint: a 404 for anyone else.
//!
//! Refused: the Ember admin page (an HTML request), an unknown embedding
//! type (Rails raises a NameError there), a missing host (NoMethodError).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::embedding::{self, HostParams};
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
        Err(Unsupported("the admin embedding page (Ember)").into())
    }
}

fn pairs(p: &Map<String, Value>) -> Vec<(String, String)> {
    p.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn id_of(raw: &str) -> Option<i32> {
    let s = raw.strip_suffix(".json").unwrap_or(raw).trim();
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    digits
        .starts_with(|c: char| c.is_ascii_digit())
        .then(|| crate::ruby::to_i(s) as i32)
}

/// EmbeddingSerializer for the current (or just written) settings, with
/// the hosts sideloaded (rest_serializer).
async fn embedding_json(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    written: &[(&'static str, crate::site_settings::Value)],
) -> Result<Response, AppError> {
    let mut hosts = Vec::new();
    let mut ids = Vec::new();
    for h in embedding::all(&mut *conn).await? {
        ids.push(h.id);
        hosts.push(embedding::serialize(&mut *conn, &h).await?);
    }
    let mut values = Vec::new();
    for name in embedding::settings() {
        let value = match written.iter().find(|(n, _)| *n == name) {
            Some((_, v)) => v.clone(),
            None => settings.get(name)?.clone(),
        };
        values.push((name, value));
    }
    let mut e = Map::new();
    e.insert("id".into(), json!("default"));
    e.insert("fields".into(), json!(embedding::settings().collect::<Vec<_>>()));
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    e.insert("base_url".into(), json!(urls.base_url()?));
    e.extend(embedding::settings_json(&values));
    e.insert("embeddable_host_ids".into(), json!(ids));
    Ok(Json(json!({
        "embeddable_hosts": hosts,
        "embedding": e,
        "__rest_serializer": "1",
    }))
    .into_response())
}

/// GET /admin/customize/embedding(.json)
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    refuse_html(&headers, &uri)?;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    embedding_json(&state, &mut conn, &settings, &[]).await
}

/// GET /admin/customize/embedding/:id(.json): the empty edit action.
pub async fn edit(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    refuse_html(&headers, &uri)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// PUT /admin/customize/embedding(.json): the type's settings from the
/// params (missing ones blank), then every embedding setting set in turn
/// (no transaction, no log: an invalid one stops the rest, the earlier
/// ones stay written).
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let p = params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    refuse_html(&headers, &uri)?;
    let Some(given) = p.get("embedding").and_then(Value::as_object) else {
        return Err(Unsupported("an embedding update without embedding (NoMethodError)").into());
    };
    let group: &[&str] = match given.get("type").and_then(params::scalar).as_deref() {
        Some("posts_and_topics") => &embedding::POSTS_AND_TOPICS,
        Some("crawlers") => &embedding::CRAWLERS,
        _ => return Err(Unsupported("an unknown embedding type (NameError in Rails)").into()),
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let mut written = Vec::new();
    for name in embedding::settings() {
        // `public_send(s) || ""`: the group's from the params, the others
        // as they are.
        let raw = if group.contains(&name) {
            match given.get(name) {
                None | Some(Value::Null) => json!(""),
                Some(v) => v.clone(),
            }
        } else {
            match settings.get(name)? {
                crate::site_settings::Value::Bool(b) => json!(b),
                crate::site_settings::Value::Int(i) => json!(i),
                other => json!(other.to_s()),
            }
        };
        match crate::site_setting_update::set(
            &mut conn,
            &state.bus,
            &settings,
            &state.site_setting_defs,
            &state.i18n,
            name,
            &raw,
        )
        .await?
        {
            Ok(value) => written.push((name, value)),
            Err(message) => {
                return Ok((
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({ "errors": [message] })),
                )
                    .into_response());
            }
        }
    }
    embedding_json(&state, &mut conn, &settings, &written).await
}

/// `save_host`'s params.
async fn host_params(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    p: &Map<String, Value>,
) -> Result<HostParams, AppError> {
    let Some(h) = p.get("embeddable_host").and_then(Value::as_object) else {
        return Err(Unsupported("an embeddable host without embeddable_host (NoMethodError)").into());
    };
    let s = |k: &str| h.get(k).and_then(params::scalar);
    let host = s("host").ok_or(Unsupported("an embeddable host without a host (NoMethodError)"))?;
    // category_id, or the uncategorized category when it names none.
    let category: Option<i32> = match s("category_id").filter(|c| !c.trim().is_empty()) {
        Some(c) => {
            sqlx::query_scalar("SELECT id FROM categories WHERE id = $1")
                .bind(crate::ruby::to_i(&c) as i32)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    let category_id = Some(category.unwrap_or(
        settings.get("uncategorized_category_id")?.to_i() as i32,
    ));
    let user_id = match s("user").filter(|u| !crate::ruby::is_blank(u)) {
        Some(name) => {
            sqlx::query_scalar("SELECT id FROM users WHERE username_lower = $1")
                .bind(name.to_lowercase())
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    let tag_ids = match h.get("tags") {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) if crate::ruby::is_blank(t) => None,
        Some(Value::Array(a)) if a.is_empty() => None,
        Some(Value::Array(a)) => Some(
            a.iter()
                .filter_map(params::scalar)
                .map(|t| crate::ruby::to_i(&t) as i32)
                .collect(),
        ),
        Some(v) => Some(
            params::scalar(v)
                .map(|t| vec![crate::ruby::to_i(&t) as i32])
                .unwrap_or_default(),
        ),
    };
    Ok(HostParams {
        host,
        allowed_paths: s("allowed_paths"),
        category_id,
        user_id,
        tag_ids,
    })
}

async fn save_host(
    state: AppState,
    guardian: crate::guardian::Guardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    id: Option<String>,
    method: &str,
) -> Result<Response, AppError> {
    let Some(actor) = guardian.user_id().filter(|_| guardian.is_admin()) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let p = params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), method) {
        return Ok(super::session::bad_csrf());
    }
    refuse_html(&headers, &uri)?;
    let mut tx = state.pool.begin().await?;
    let before = match &id {
        None => None,
        Some(raw) => match id_of(raw) {
            Some(id) => Some(embedding::find(&mut tx, id).await?.ok_or(Unsupported(
                "updating an embeddable host that doesn't exist (NoMethodError)",
            ))?),
            None => {
                return Err(Unsupported(
                    "updating an embeddable host that doesn't exist (NoMethodError)",
                )
                .into());
            }
        },
    };
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let hp = host_params(&mut tx, &settings, &p).await?;
    let saved = match embedding::save(&mut tx, actor, before.as_ref(), &hp).await? {
        Ok(h) => h,
        Err(errors) => {
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "errors": errors })),
            )
                .into_response());
        }
    };
    let body = embedding::serialize(&mut tx, &saved).await?;
    tx.commit().await?;
    Ok(Json(json!({ "embeddable_host": body, "__rest_serializer": "1" })).into_response())
}

/// POST /admin/embeddable_hosts(.json)
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    save_host(state, guardian, headers, uri, body, None, "POST").await
}

/// PUT /admin/embeddable_hosts/:id(.json)
pub async fn update_host(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    save_host(state, guardian, headers, uri, body, Some(id), "PUT").await
}

/// DELETE /admin/embeddable_hosts/:id(.json): the host (its tag rows
/// stay, no dependent destroy), the embedding settings back to their
/// defaults once no host is left, the log.
pub async fn destroy_host(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Some(actor) = guardian.user_id().filter(|_| guardian.is_admin()) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let p = params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "DELETE") {
        return Ok(super::session::bad_csrf());
    }
    refuse_html(&headers, &uri)?;
    let mut tx = state.pool.begin().await?;
    let host = match id_of(&id) {
        Some(id) => embedding::find(&mut tx, id).await?,
        None => None,
    }
    .ok_or(Unsupported(
        "destroying an embeddable host that doesn't exist (NoMethodError)",
    ))?;
    sqlx::query("DELETE FROM embeddable_hosts WHERE id = $1")
        .bind(host.id)
        .execute(&mut *tx)
        .await?;
    // after_destroy :reset_embedding_settings
    let left: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM embeddable_hosts)")
        .fetch_one(&mut *tx)
        .await?;
    if !left {
        let settings =
            SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
        for name in embedding::settings() {
            let def = state
                .site_setting_defs
                .get(name)
                .ok_or(Unsupported("an unknown embedding setting"))?;
            let default = match &def.default {
                crate::site_settings::Value::Bool(b) => json!(b),
                crate::site_settings::Value::Int(i) => json!(i),
                other => json!(other.to_s()),
            };
            if let Err(_message) = crate::site_setting_update::set(
                &mut tx,
                &state.bus,
                &settings,
                &state.site_setting_defs,
                &state.i18n,
                name,
                &default,
            )
            .await?
            {
                return Err(Unsupported("an embedding setting default that fails validation").into());
            }
        }
    }
    embedding::log(
        &mut tx,
        actor,
        embedding::EMBEDDABLE_HOST_DESTROY,
        &host.host,
        None,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}
