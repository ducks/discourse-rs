//! Port of TopicsController#timings (POST /topics/timings and POST
//! /t/:topic_id/timings) and NotificationsController#mark_read (PUT
//! /notifications/mark-read and /notifications/read).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::read_tracking::{self, MarkRead};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// POST /topics/timings
pub async fn timings(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    record(state, guardian, None, headers, uri, body).await
}

/// POST /t/:topic_id/timings
pub async fn topic_timings(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(topic_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    record(state, guardian, Some(topic_id), headers, uri, body).await
}

async fn record(
    state: AppState,
    guardian: crate::guardian::Guardian,
    route_topic_id: Option<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    // requires_login
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state, uri.path()));
    }
    let int = |key: &str| {
        p.get(key)
            .and_then(params::scalar)
            .map(|v| crate::ruby::to_i(&v))
            .unwrap_or(0)
    };
    let topic_id = match &route_topic_id {
        Some(id) => crate::ruby::to_i(id),
        None => int("topic_id"),
    } as i32;
    let topic_time = int("topic_time");
    // timings: {post_number => msecs}, in the order sent.
    let timings: Vec<(i64, i64)> = match p.get("timings") {
        Some(Value::Object(map)) => map
            .iter()
            .map(|(n, t)| {
                (
                    crate::ruby::to_i(n),
                    params::scalar(t)
                        .map(|t| crate::ruby::to_i(&t))
                        .unwrap_or(0),
                )
            })
            .collect(),
        _ => Vec::new(),
    };

    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // find_visible_topic_from_topic_id
    let visible =
        match crate::topic_guardian::TopicCtx::load(&mut tx, &settings, &guardian, topic_id).await?
        {
            Some(topic) if !topic.trashed() => {
                let secure = guardian.secure_category_ids(&mut tx, &settings).await?;
                guardian.can_see_topic(&settings, &topic, true, &secure)?
            }
            _ => false,
        };
    if !visible {
        return Ok(super::topics::not_found_response(&state, false));
    }
    let notifications_read = read_tracking::process_timings(
        &state.bus, &mut tx, &settings, &guardian, topic_id, topic_time, timings,
    )
    .await?;
    if notifications_read && let Some(user_id) = guardian.user_id() {
        crate::bus::publish_notifications_state(&state.bus, &mut tx, &settings, user_id).await?;
    }
    tx.commit().await?;
    // render body: nil
    Ok(StatusCode::OK.into_response())
}

/// PUT /notifications/mark-read
pub async fn mark_read(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state, uri.path()));
    };
    let id = p
        .get("id")
        .and_then(params::scalar)
        .filter(|v| !v.is_empty())
        .map(|v| crate::ruby::to_i(&v));
    let dismiss_types = p.get("dismiss_types").and_then(params::scalar);
    let mut tx = state.pool.begin().await?;
    Ok(
        match read_tracking::mark_read(&mut tx, user_id, id, dismiss_types.as_deref()).await? {
            MarkRead::Done => {
                let settings =
                    SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals)
                        .await?;
                crate::bus::publish_notifications_state(&state.bus, &mut tx, &settings, user_id)
                    .await?;
                tx.commit().await?;
                (StatusCode::OK, Json(read_tracking::success())).into_response()
            }
            MarkRead::Invalid(message) => super::search::invalid_parameters(&state, &message),
        },
    )
}
