//! Port of ReviewablesController#perform: PUT
//! /review/:reviewable_id/perform/:action_id, answering with
//! ReviewablePerformResultSerializer.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::review::{self, Outcome};
use crate::review_list::Listed;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn json_error(state: &AppState, status: StatusCode, key: &str) -> Response {
    let text = state.i18n.t(key).unwrap_or(key);
    (status, Json(json!({ "errors": [text] }))).into_response()
}

/// PUT /review/:reviewable_id/perform/:action_id
pub async fn perform(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((reviewable_id, action_id)): Path<(String, String)>,
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
    // :reviewable_id is constrained to digits.
    let Some(reviewable_id) = reviewable_id
        .parse::<i64>()
        .ok()
        .filter(|_| reviewable_id.bytes().all(|b| b.is_ascii_digit()))
    else {
        return Ok(super::topics::not_found_response(&state, false));
    };
    // version_required
    let Some(version) = p
        .get("version")
        .and_then(params::scalar)
        .filter(|v| !v.trim().is_empty())
    else {
        return Ok(json_error(
            &state,
            StatusCode::UNPROCESSABLE_ENTITY,
            "reviewables.missing_version",
        ));
    };
    let version = crate::ruby::to_i(&version);

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
    let outcome =
        review::perform(&mut tx, &ctx, &guardian, reviewable_id, &action_id, version).await?;
    Ok(match outcome {
        Outcome::Done(result) => {
            tx.commit().await?;
            (StatusCode::OK, Json(result)).into_response()
        }
        Outcome::NotFound => super::topics::not_found_response(&state, false),
        Outcome::Forbidden => super::search::invalid_access(&state),
        Outcome::Conflict => json_error(&state, StatusCode::CONFLICT, "reviewables.conflict"),
    })
}

/// GET /review.json: ReviewablesController#index.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    // requires_login
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state, uri.path()));
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // ensure_can_see: can_see_review_queue?
    if !guardian.is_staff() {
        if settings.get("enable_category_group_moderation")?.truthy() {
            return Err(Unsupported("the review queue for category group moderators").into());
        }
        return Ok(super::search::invalid_access(&state));
    }
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
    };
    let p = params::parse(uri.query(), &headers, &body);
    Ok(
        match crate::review_list::index(&mut tx, &ctx, &guardian, &p).await? {
            Listed::Page(doc) => (StatusCode::OK, Json(doc)).into_response(),
            Listed::InvalidParameter(name) => super::search::invalid_parameters(&state, name),
        },
    )
}

/// GET /review: the Ember app's page.
pub async fn page() -> Result<Response, AppError> {
    Err(Unsupported("the review page (HTML)").into())
}
