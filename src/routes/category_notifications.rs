//! CategoriesController#set_notifications: POST
//! /category/:category_id/notifications (`notification_level`), the
//! member's level in a category (CategoryUser.
//! set_notification_level_for_category), answered with their indirectly
//! muted categories.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

pub async fn set_notifications(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(category_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    // params[:category_id].to_i, params[:notification_level].to_i
    let category_id = crate::ruby::to_i(category_id.trim_end_matches(".json")) as i32;
    let level = p
        .get("notification_level")
        .and_then(params::scalar)
        .map_or(0, |v| crate::ruby::to_i(&v)) as i32;
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let current: Option<i32> = sqlx::query_scalar(
        "SELECT notification_level FROM category_users WHERE user_id = $1 AND category_id = $2",
    )
    .bind(user_id)
    .bind(category_id)
    .fetch_optional(&mut *tx)
    .await?;
    if current != Some(level) {
        match current {
            Some(_) => {
                sqlx::query(
                    "UPDATE category_users SET notification_level = $3 \
                     WHERE user_id = $1 AND category_id = $2",
                )
                .bind(user_id)
                .bind(category_id)
                .bind(level)
                .execute(&mut *tx)
                .await?;
            }
            None => {
                // RecordNotUnique "does not matter".
                sqlx::query(
                    "INSERT INTO category_users (user_id, category_id, notification_level) \
                     VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
                )
                .bind(user_id)
                .bind(category_id)
                .bind(level)
                .execute(&mut *tx)
                .await?;
            }
        }
        crate::user_updater::auto_watch(&mut tx, user_id).await?;
        crate::user_updater::auto_track(&mut tx, user_id).await?;
    }
    let muted =
        crate::current_user::indirectly_muted_category_ids(&mut tx, &settings, user_id).await?;
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK", "indirectly_muted_category_ids": muted })).into_response())
}
