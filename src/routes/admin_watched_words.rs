//! /admin/customize/watched_words: Admin::WatchedWordsController (staff;
//! everyone else 404s, as StaffConstraint has it).
//!
//! Refused: the CSV upload (its writes run deferred and report over
//! MessageBus to the uploading client).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::watched_words::{self, Created};
use crate::word_watcher as ww;
use crate::{AppError, AppState, Unsupported};

fn staff_only(state: &AppState, guardian: &crate::guardian::Guardian) -> Option<Response> {
    (!guardian.is_staff()).then(|| super::topics::not_found_response(state))
}

fn strip_json(id: &str) -> &str {
    id.strip_suffix(".json").unwrap_or(id)
}

/// GET /admin/customize/watched_words(.json) and .../action/:id(.json)
/// (the id ignored): every word, by action and word.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = staff_only(&state, &guardian) {
        return Ok(r);
    }
    if !super::admin_pages::wants_json(&uri, &headers) {
        return super::admin_customize::watched_words_page(&state, &guardian, &headers, &uri, None)
            .await;
    }
    index_json(&state).await
}

async fn index_json(state: &AppState) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let tagging = s.get("tagging_enabled")?.truthy();
    let words = watched_words::all(&mut conn, tagging).await?;
    Ok(Json(watched_words::list_json(&mut conn, &s, &words).await?).into_response())
}

pub async fn index_action(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if let Some(r) = staff_only(&state, &guardian) {
        return Ok(r);
    }
    if !super::admin_pages::wants_json(&uri, &headers) {
        return super::admin_customize::watched_words_page(
            &state,
            &guardian,
            &headers,
            &uri,
            Some(&id),
        )
        .await;
    }
    index_json(&state).await
}

/// POST /admin/customize/watched_words(.json)
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "POST") {
        return Ok(super::session::bad_csrf());
    }
    if let Some(r) = staff_only(&state, &guardian) {
        return Ok(r);
    }
    // `opts.delete(:words)`, then `words.each`: no list is a NoMethodError.
    let words: Vec<String> = match p.get("words") {
        Some(Value::Array(list)) => list.iter().filter_map(crate::params::scalar).collect(),
        _ => return Err(Unsupported("creating watched words without a word list").into()),
    };
    let Some(action_key) = crate::params::string(&p, "action_key") else {
        return Err(Unsupported("creating watched words without an action_key").into());
    };
    let mut replacement = crate::params::string(&p, "replacement");
    // The tag action's replacement_tags: the names of the tags by id, then
    // the new ones found or created (find_or_create_tags!), joined.
    if crate::word_watcher::action_id(&action_key) == Some(crate::word_watcher::TAG)
        && let Some(tags) = p.get("replacement_tags").filter(|v| !v.is_null())
    {
        let mut conn = state.pool.acquire().await?;
        let s =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
        match resolve_replacement_tags(&mut conn, &s, &state.i18n, &guardian, tags).await? {
            Ok(names) => replacement = names,
            Err(errors) => {
                return Ok((
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({ "errors": errors, "error_type": "record_invalid" })),
                )
                    .into_response());
            }
        }
    }
    // case_sensitive / html: Rails' boolean casting of the param.
    let case_sensitive = crate::params::string(&p, "case_sensitive").map(|v| truthy(&v));
    let html = crate::params::string(&p, "html").is_some_and(|v| truthy(&v));
    let user_id = guardian.user_id().ok_or(Unsupported("anonymous staff"))?;
    let mut tx = state.pool.begin().await?;
    let s = SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let base_url = crate::url::Urls {
        config: &state.config,
        settings: &s,
    }
    .base_url()?;
    match watched_words::create_group(
        &mut tx,
        &s,
        &state.i18n,
        &base_url,
        user_id,
        &words,
        &action_key,
        replacement.as_deref(),
        case_sensitive,
        html,
    )
    .await?
    {
        Created::Words(rows) => {
            let body = watched_words::list_json(&mut tx, &s, &rows).await?;
            tx.commit().await?;
            Ok(Json(body).into_response())
        }
        // save! raised RecordInvalid; the transaction rolled back.
        Created::Invalid(errors) => Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "errors": errors, "error_type": "record_invalid" })),
        )
            .into_response()),
    }
}

/// `resolve_replacement_tags`: the tag objects' names (those with ids
/// looked up, the others found or created as the staff member), joined
/// with commas; None for none. Err with the errors of a tag Tag.create!
/// refused.
async fn resolve_replacement_tags(
    conn: &mut sqlx::PgConnection,
    s: &SiteSettings,
    i18n: &crate::i18n::I18n,
    guardian: &crate::guardian::Guardian,
    tags: &Value,
) -> Result<Result<Option<String>, Vec<String>>, AppError> {
    // A form's numbered hash is read by its values.
    let items: Vec<Value> = match tags {
        Value::Array(items) => items.clone(),
        Value::Object(map) => map.values().cloned().collect(),
        _ => return Err(Unsupported("replacement_tags of another shape").into()),
    };
    if items.is_empty() {
        return Ok(Ok(None));
    }
    let mut ids: Vec<i64> = Vec::new();
    let mut new_names: Vec<String> = Vec::new();
    for item in &items {
        let Value::Object(t) = item else {
            return Err(Unsupported("replacement_tags of another shape").into());
        };
        let id = t.get("id").and_then(crate::params::scalar);
        match id.as_deref().filter(|i| !crate::ruby::is_blank(i)) {
            Some(i) => ids.push(crate::ruby::to_i(i)),
            None => {
                if let Some(name) = t.get("name").and_then(crate::params::scalar) {
                    new_names.push(name);
                }
            }
        }
    }
    let ids: Vec<i32> = ids
        .into_iter()
        .filter_map(|i| i32::try_from(i).ok())
        .collect();
    let mut names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM tags WHERE id = ANY($1) ORDER BY ctid")
            .bind(&ids)
            .fetch_all(&mut *conn)
            .await?;
    if !new_names.is_empty() {
        match crate::tagging::find_or_create_tags(conn, s, i18n, guardian, &new_names).await? {
            Ok(found) => names.extend(found.into_iter().map(|(_, n)| n)),
            Err(errors) => return Ok(Err(errors)),
        }
    }
    Ok(Ok(Some(names.join(","))))
}

/// ActiveModel's boolean cast: false for "false", "0", "f", "off" and
/// the like, true otherwise.
fn truthy(v: &str) -> bool {
    !matches!(
        v.to_ascii_lowercase().as_str(),
        "false" | "0" | "f" | "off" | "" | "n" | "no"
    )
}

fn pairs(map: &serde_json::Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// DELETE /admin/customize/watched_words/:id(.json)
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "DELETE") {
        return Ok(super::session::bad_csrf());
    }
    if let Some(r) = staff_only(&state, &guardian) {
        return Ok(r);
    }
    let user_id = guardian.user_id().ok_or(Unsupported("anonymous staff"))?;
    let Ok(id) = strip_json(&id).parse::<i32>() else {
        return Ok(super::search::invalid_parameters(&state, "id"));
    };
    let mut tx = state.pool.begin().await?;
    if !watched_words::destroy(&mut tx, user_id, id).await? {
        return Ok(super::search::invalid_parameters(&state, "id"));
    }
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// DELETE /admin/customize/watched_words/action/:id(.json): every word of
/// the action.
pub async fn clear_all(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "DELETE") {
        return Ok(super::session::bad_csrf());
    }
    if let Some(r) = staff_only(&state, &guardian) {
        return Ok(r);
    }
    let user_id = guardian.user_id().ok_or(Unsupported("anonymous staff"))?;
    let Some(action) = ww::action_id(strip_json(&id)) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let mut tx = state.pool.begin().await?;
    watched_words::clear_all(&mut tx, user_id, action).await?;
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// GET /admin/customize/watched_words/action/:id/download: the action's
/// words as CSV (no XHR needed).
pub async fn download(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    if let Some(r) = staff_only(&state, &guardian) {
        return Ok(r);
    }
    let name = strip_json(&id);
    let Some(action) = ww::action_id(name) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let host = crate::url::Urls {
        config: &state.config,
        settings: &s,
    }
    .current_hostname()?;
    let content = watched_words::csv(&mut conn, action).await?;
    let filename = format!("{host}-watched-words-{name}.csv");
    // send_data's filename*: percent-encoded, which a plain host name
    // doesn't need.
    if !filename
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
    {
        return Err(Unsupported("a download name to percent-encode").into());
    }
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"{filename}\"; filename*=UTF-8''{}",
                    filename
                ),
            ),
            (header::CONTENT_LENGTH, content.len().to_string()),
        ],
        content,
    )
        .into_response())
}

/// POST /admin/customize/watched_words/upload
pub async fn upload(AuthGuardian(guardian): AuthGuardian) -> Result<Response, AppError> {
    let _ = guardian;
    Err(Unsupported("uploading watched words (deferred writes, MessageBus to the client)").into())
}
