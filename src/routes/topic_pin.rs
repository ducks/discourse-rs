//! Port of TopicsController#clear_pin and #re_pin: PUT
//! /t/:topic_id/clear-pin and /re-pin, a member's own pin on a topic
//! (TopicStatus' pin toggle, static/js/topic-status.js).

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::posting::{TopicUserAttr, change_topic_user};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// PUT /t/:topic_id/clear-pin (Topic#clear_pin_for)
pub async fn clear_pin(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(topic_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    change(state, guardian, topic_id, headers, uri, body, true).await
}

/// PUT /t/:topic_id/re-pin (Topic#re_pin_for)
pub async fn re_pin(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(topic_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    change(state, guardian, topic_id, headers, uri, body, false).await
}

async fn change(
    state: AppState,
    guardian: crate::guardian::Guardian,
    topic_id: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    clear: bool,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
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
    let topic_id = i32::try_from(crate::ruby::to_i(&topic_id)).unwrap_or(0);

    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // Topic.find_by (a deleted topic is out of its default scope), then
    // guardian.ensure_can_see!(topic): InvalidAccess for none as well.
    let topic = crate::topic_guardian::TopicCtx::load(&mut tx, &settings, &guardian, topic_id)
        .await?
        .filter(|t| !t.trashed());
    let Some(topic) = topic else {
        return Ok(super::search::invalid_access(&state));
    };
    let secure = guardian.secure_category_ids(&mut tx, &settings).await?;
    if !guardian.can_see_topic(&settings, &topic, true, &secure)? {
        return Ok(super::search::invalid_access(&state));
    }
    // TopicUser.change(cleared_pinned_at: Time.now), or nil; a topic
    // that isn't pinned takes it all the same.
    let at = clear.then(crate::clock::now_naive);
    change_topic_user(
        &state.bus,
        &mut tx,
        user_id,
        topic_id,
        &[TopicUserAttr::ClearedPinnedAt(at)],
    )
    .await?;
    tx.commit().await?;
    // render body: nil
    Ok(StatusCode::OK.into_response())
}
