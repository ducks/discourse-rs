//! Port of app/controllers/notifications_controller.rb#index.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::notifications::{self, Notifications};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

#[derive(Deserialize, Default)]
pub struct Params {
    recent: Option<String>,
    silent: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
    filter: Option<String>,
    username: Option<String>,
    filter_by_types: Option<String>,
}

/// GET /notifications(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<Params>,
) -> Result<Response, AppError> {
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let recent = params
        .recent
        .as_deref()
        .is_some_and(|r| !r.trim().is_empty());
    let viewer = guardian.user().expect("authenticated");

    // The target: `username` only without `recent`; admins may read
    // anyone's, exact-case lookup.
    let (user_id, username) = match (&params.username, recent) {
        (Some(name), false) => {
            let row: Option<(i32, String)> =
                sqlx::query_as("SELECT id, username FROM users WHERE username = $1")
                    .bind(name)
                    .fetch_optional(&mut *conn)
                    .await?;
            let Some((id, username)) = row else {
                return Ok(super::topics::not_found_response(&state));
            };
            if !guardian.is_me(id) && !guardian.is_admin() {
                return Ok(super::search::invalid_access(&state));
            }
            (id, username)
        }
        _ => (viewer.id, viewer.username.clone()),
    };

    // fetch_limit_from_params: an integer within 0..=60.
    let default_limit = if recent { 15 } else { 60 };
    let limit = match params.limit.as_deref() {
        None => default_limit,
        Some(raw) => match raw.trim().parse::<i64>() {
            Ok(n) if (0..=60).contains(&n) => n,
            _ => return Ok(super::search::invalid_parameters(&state, "limit")),
        },
    };
    let offset = params
        .offset
        .as_deref()
        .map(crate::ruby::to_i)
        .unwrap_or(0)
        .max(0);
    let types = match params.filter_by_types.as_deref().filter(|t| !t.is_empty()) {
        Some(raw) => match notifications::parse_types(raw) {
            Ok(types) => types,
            Err(message) => return Ok(super::search::invalid_parameters(&state, &message)),
        },
        None => Vec::new(),
    };
    let query = notifications::Query {
        recent,
        silent: params.silent.is_some(),
        limit,
        offset,
        filter: params.filter.clone(),
        types,
    };
    let (doc, bumped) = Notifications {
        conn: &mut conn,
        settings: &settings,
        guardian: &guardian,
    }
    .index(user_id, &username, &query)
    .await?;
    if bumped {
        crate::bus::publish_notifications_state(&state.bus, &mut conn, &settings, user_id).await?;
    }
    Ok((
        StatusCode::OK,
        [(header::CACHE_CONTROL, "no-cache, no-store")],
        Json(doc),
    )
        .into_response())
}
