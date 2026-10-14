//! /admin/customize/watched_words: Admin::WatchedWordsController (staff;
//! everyone else 404s, as StaffConstraint has it).
//!
//! Refused: the CSV upload (its writes run deferred and report over
//! MessageBus to the uploading client), and the tag action.

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
) -> Result<Response, AppError> {
    if let Some(r) = staff_only(&state, &guardian) {
        return Ok(r);
    }
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let tagging = s.get("tagging_enabled")?.truthy();
    let words = watched_words::all(&mut conn, tagging).await?;
    Ok(Json(watched_words::list_json(&mut conn, &s, &words).await?).into_response())
}

pub async fn index_action(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path(_): Path<String>,
) -> Result<Response, AppError> {
    index(state, guardian).await
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
    if p.contains_key("replacement_tags") {
        return Err(Unsupported("tag watched words (topic tagging writes)").into());
    }
    // `opts.delete(:words)`, then `words.each`: no list is a NoMethodError.
    let words: Vec<String> = match p.get("words") {
        Some(Value::Array(list)) => list.iter().filter_map(crate::params::scalar).collect(),
        _ => return Err(Unsupported("creating watched words without a word list").into()),
    };
    let Some(action_key) = crate::params::string(&p, "action_key") else {
        return Err(Unsupported("creating watched words without an action_key").into());
    };
    let replacement = crate::params::string(&p, "replacement");
    // case_sensitive / html: Rails' boolean casting of the param.
    let case_sensitive = crate::params::string(&p, "case_sensitive").map(|v| truthy(&v));
    let html = crate::params::string(&p, "html").is_some_and(|v| truthy(&v));
    let user_id = guardian.user_id().ok_or(Unsupported("anonymous staff"))?;
    let mut tx = state.pool.begin().await?;
    let s = SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    ww::refuse_unported(&s)?;
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
