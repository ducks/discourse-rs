//! Port of Admin::SiteSettingsController#update: PUT
//! /admin/site_settings/:id(.json) with the new value under the setting's
//! name. The route is for admins only (AdminConstraint).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_setting_update::{self, Outcome};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// PUT /admin/site_settings/:id
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state, false));
    }
    let id = id.strip_suffix(".json").unwrap_or(&id).to_string();
    if id == "bulk_update" {
        return Err(Unsupported("bulk site setting updates").into());
    }
    if p.contains_key("update_existing_user") {
        return Err(Unsupported("backfilling user preferences (update_existing_user)").into());
    }
    // params[id].to_s
    let raw = p.get(&id).and_then(params::scalar).unwrap_or_default();
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let outcome = site_setting_update::update(
        &mut tx,
        &state.site_setting_defs,
        &settings,
        &state.i18n,
        &guardian,
        &id,
        &raw,
    )
    .await?;
    Ok(match outcome {
        Outcome::Done => {
            tx.commit().await?;
            StatusCode::NO_CONTENT.into_response()
        }
        Outcome::Invalid(message) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "errors": [message] })),
        )
            .into_response(),
    })
}
