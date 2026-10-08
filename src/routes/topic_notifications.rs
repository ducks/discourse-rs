//! Port of TopicsController#set_notifications: POST
//! /t/:topic_id/notifications, the topic notification level menu.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::posting::{TopicUserAttr, change_topic_user, notification_reasons};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// POST /t/:topic_id/notifications
pub async fn set_notifications(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(topic_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    // requires_login
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    // :topic_id is constrained to digits.
    if topic_id.is_empty() || !topic_id.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(super::topics::not_found_response(&state));
    }
    // fetch_target_user: another user's level, for admins over the API.
    let present = |key: &str| {
        p.get(key)
            .and_then(params::scalar)
            .is_some_and(|v| !v.trim().is_empty())
    };
    if present("username") || present("external_id") {
        let api = headers.contains_key(crate::session::api_key::HEADER_API_KEY);
        if !api || !guardian.is_admin() {
            return Ok(super::search::invalid_access(&state));
        }
        return Err(Unsupported("setting another user's topic notification level").into());
    }
    let topic_id = crate::ruby::to_i(&topic_id) as i32;
    let level = p
        .get("notification_level")
        .and_then(params::scalar)
        .map(|v| crate::ruby::to_i(&v))
        .unwrap_or(0) as i32;

    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // Topic.find: a deleted topic is out of its default scope.
    let topic = match crate::topic_guardian::TopicCtx::load(&mut tx, &settings, &guardian, topic_id)
        .await?
    {
        Some(topic) if !topic.trashed() => topic,
        _ => return Ok(super::topics::not_found_response(&state)),
    };
    // guardian.ensure_can_see!(topic): a topic the user can't see is not
    // found (the recording's 404).
    let secure = guardian.secure_category_ids(&mut tx, &settings).await?;
    if !guardian.can_see_topic(&settings, &topic, true, &secure)? {
        return Ok(super::topics::not_found_response(&state));
    }
    // TopicUser.change: notifications_reason_id ||= user_changed.
    let reason = notification_reasons::USER_CHANGED;
    change_topic_user(
        &mut tx,
        user_id,
        topic_id,
        &[TopicUserAttr::NotificationLevel(level, reason)],
    )
    .await?;
    crate::bus::publish_notification_level_change(
        &state.bus,
        &mut tx,
        user_id,
        topic_id,
        level,
        Some(reason),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::OK, Json(json!({ "success": "OK" }))).into_response())
}
