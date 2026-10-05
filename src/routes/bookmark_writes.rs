//! Port of BookmarksController#create, #update, #destroy and #toggle_pin:
//! POST /bookmarks, PUT /bookmarks/:id, DELETE /bookmarks/:id and
//! PUT /bookmarks/:bookmark_id/toggle_pin (each also with .json).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::bookmark_manager::{self, Fields, Outcome};
use crate::params;
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

#[derive(Clone, Copy)]
enum Action {
    Create,
    Update,
    Destroy,
    TogglePin,
}

async fn handle(
    state: AppState,
    guardian: crate::guardian::Guardian,
    id: Option<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    action: Action,
) -> Result<Response, AppError> {
    let method = match action {
        Action::Create => "POST",
        Action::Update | Action::TogglePin => "PUT",
        Action::Destroy => "DELETE",
    };
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), method) {
        return Ok(bad_csrf());
    }
    // requires_login
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state, uri.path()));
    };
    let text = |k: &str| p.get(k).and_then(params::scalar);
    let (name, reminder_at, preference) = (
        text("name"),
        text("reminder_at"),
        text("auto_delete_preference"),
    );
    let fields = Fields {
        name: name.as_deref(),
        reminder_at: reminder_at.as_deref(),
        auto_delete_preference: preference.as_deref(),
    };
    let bookmark_id = id
        .as_deref()
        .map(|i| crate::ruby::to_i(i.strip_suffix(".json").unwrap_or(i)) as i32)
        .unwrap_or(0);

    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let outcome = match action {
        Action::Create => {
            // params.require(:bookmarkable_id), (:bookmarkable_type)
            let Some(bookmarkable_id) = text("bookmarkable_id").filter(|v| !v.is_empty()) else {
                return Ok(super::accounts::param_missing("bookmarkable_id"));
            };
            let Some(bookmarkable_type) = text("bookmarkable_type").filter(|v| !v.is_empty())
            else {
                return Ok(super::accounts::param_missing("bookmarkable_type"));
            };
            bookmark_manager::create(
                &mut tx,
                &ctx,
                &guardian,
                &bookmarkable_id,
                &bookmarkable_type,
                &fields,
            )
            .await?
        }
        Action::Update => {
            bookmark_manager::update(&mut tx, &ctx, user_id, bookmark_id, &fields).await?
        }
        Action::TogglePin => bookmark_manager::toggle_pin(&mut tx, user_id, bookmark_id).await?,
        Action::Destroy => bookmark_manager::destroy(&mut tx, user_id, bookmark_id).await?,
    };
    let ok = |body: Value| (StatusCode::OK, Json(body)).into_response();
    Ok(match outcome {
        Outcome::Created(id) => {
            tx.commit().await?;
            ok(json!({ "success": "OK", "id": id }))
        }
        Outcome::Done => {
            tx.commit().await?;
            ok(json!({ "success": "OK" }))
        }
        Outcome::Destroyed { topic_bookmarked } => {
            tx.commit().await?;
            ok(json!({ "success": "OK", "topic_bookmarked": topic_bookmarked }))
        }
        Outcome::Invalid(errors) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "failed": "FAILED", "errors": errors })),
        )
            .into_response(),
        Outcome::NotFound => super::topics::not_found_response(&state, false),
        Outcome::Forbidden => super::search::invalid_access(&state),
    })
}

/// POST /bookmarks
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    handle(state, guardian, None, headers, uri, body, Action::Create).await
}

/// PUT /bookmarks/:id
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    handle(
        state,
        guardian,
        Some(id),
        headers,
        uri,
        body,
        Action::Update,
    )
    .await
}

/// DELETE /bookmarks/:id
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    handle(
        state,
        guardian,
        Some(id),
        headers,
        uri,
        body,
        Action::Destroy,
    )
    .await
}

/// PUT /bookmarks/:bookmark_id/toggle_pin
pub async fn toggle_pin(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    handle(
        state,
        guardian,
        Some(id),
        headers,
        uri,
        body,
        Action::TogglePin,
    )
    .await
}
