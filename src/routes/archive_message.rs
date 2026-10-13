//! TopicsController#archive_message and #move_to_inbox: PUT
//! /t/:id/archive-message and /t/:id/move-to-inbox take a message out of
//! (or back into) the inbox of the user and of each of their groups the
//! message is to.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::session::{bad_csrf, csrf_ok};
use crate::archived_messages;
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// PUT /t/:id/archive-message
pub async fn archive(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    toggle(state, guardian, id, headers, uri, body, true).await
}

/// PUT /t/:id/move-to-inbox
pub async fn move_to_inbox(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    toggle(state, guardian, id, headers, uri, body, false).await
}

/// `toggle_archive_message(archive)`
async fn toggle(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    id: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    archive: bool,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let pairs: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !csrf_ok(&state, &headers, &pairs, uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    // requires_login
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // Topic.find(params[:id].to_i)
    let topic_id = crate::ruby::to_i(&id);
    let topic_id = i32::try_from(topic_id).unwrap_or(0);
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND deleted_at IS NULL)",
    )
    .bind(topic_id)
    .fetch_one(&mut *tx)
    .await?;
    if !exists {
        return Ok(super::topics::not_found_response(&state));
    }
    // ensure_can_see!: a topic the user can't see is not found (the
    // recording's 404).
    let topic = crate::topic_guardian::TopicCtx::load(&mut tx, &settings, &guardian, topic_id)
        .await?
        .ok_or(Unsupported("loading a topic to archive"))?;
    let secure = guardian.secure_category_ids(&mut tx, &settings).await?;
    if !guardian.can_see_topic(&settings, &topic, true, &secure)? {
        return Ok(super::topics::not_found_response(&state));
    }
    // The user's groups the message is to.
    let groups: Vec<i32> = sqlx::query_scalar(
        "SELECT groups.id FROM groups \
         INNER JOIN topic_allowed_groups ON groups.id = topic_allowed_groups.group_id \
         WHERE topic_allowed_groups.topic_id = $1 \
           AND topic_allowed_groups.group_id IN (SELECT group_id FROM group_users WHERE user_id = $2)",
    )
    .bind(topic_id)
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut archived_for = None;
    for group_id in groups {
        if archive {
            archived_messages::group_archive(
                &state.bus,
                &mut tx,
                group_id,
                topic_id,
                Some(user_id),
            )
            .await?;
            archived_for = Some(group_id);
        } else {
            archived_messages::group_move_to_inbox(
                &state.bus,
                &mut tx,
                group_id,
                topic_id,
                Some(user_id),
            )
            .await?;
        }
    }
    let allowed: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topic_allowed_users WHERE topic_id = $1 AND user_id = $2)",
    )
    .bind(topic_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if allowed {
        if archive {
            archived_messages::user_archive(&state.bus, &mut tx, user_id, topic_id).await?;
        } else {
            archived_messages::user_move_to_inbox(&state.bus, &mut tx, user_id, topic_id).await?;
        }
    }
    let name: Option<String> = match archived_for {
        Some(group_id) => {
            sqlx::query_scalar("SELECT name FROM groups WHERE id = $1")
                .bind(group_id)
                .fetch_optional(&mut *tx)
                .await?
        }
        None => None,
    };
    tx.commit().await?;
    Ok(match archived_for {
        Some(_) => Json(json!({ "group_name": name })).into_response(),
        // render body: nil
        None => StatusCode::OK.into_response(),
    })
}
