//! Port of TopicsController#status: PUT /t/:topic_id/status and
//! PUT /t/:slug/:topic_id/status.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_status::{self, Outcome, Status};
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// `ActiveModel::Type::Boolean.new.cast`: everything but the false values
/// is true.
fn cast_bool(value: &str) -> bool {
    !matches!(
        value,
        "" | "0" | "f" | "F" | "false" | "FALSE" | "off" | "OFF"
    )
}

/// PUT /t/:topic_id/status
pub async fn status(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path(topic_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    update(state, guardian, topic_id, headers, uri, body).await
}

/// PUT /t/:slug/:topic_id/status
pub async fn status_with_slug(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path((_slug, topic_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    update(state, guardian, topic_id, headers, uri, body).await
}

async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    topic_id: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    // requires_login
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state, uri.path()));
    }
    // :topic_id is constrained to digits.
    if topic_id.is_empty() || !topic_id.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(super::topics::not_found_response(&state, false));
    }
    let present = |key: &str| {
        p.get(key)
            .and_then(params::scalar)
            .filter(|v| !v.is_empty())
    };
    let Some(status) = present("status") else {
        return Ok(super::accounts::param_missing("status"));
    };
    let Some(enabled) = present("enabled") else {
        return Ok(super::accounts::param_missing("enabled"));
    };
    let Some(status) = Status::parse(&status) else {
        return Ok(super::search::invalid_parameters(&state, "status"));
    };
    let until = present("until");
    let category_id = present("category_id").map(|c| crate::ruby::to_i(&c) as i32);

    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
    };
    let outcome = topic_status::update(
        &mut tx,
        &ctx,
        &guardian,
        crate::ruby::to_i(&topic_id) as i32,
        category_id,
        status,
        cast_bool(&enabled),
        until.as_deref(),
    )
    .await?;
    Ok(match outcome {
        Outcome::Done => {
            tx.commit().await?;
            (StatusCode::OK, Json(topic_status::response())).into_response()
        }
        Outcome::Forbidden => super::search::invalid_access(&state),
    })
}
