//! Live updates over HTTP, where Discourse serves MessageBus's
//! /message-bus/:client_id/poll: GET /bus/events streams server-sent
//! events, GET /bus/poll long-polls for networks that break streaming.
//!
//! `channels` is a comma-separated list. The stream starts after the
//! Last-Event-ID header (a browser reconnecting), else `position` (from the
//! page the client loaded), else now. What a viewer hears is decided by the
//! audience tags their session holds (see `crate::bus::tags`), checked
//! again on every reconnect.

use std::time::Duration;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use pg_bus::{Filter, Position};
use serde::Deserialize;

use crate::session::current::AuthGuardian;
use crate::{AppError, AppState};

/// How long a poll waits for a message before answering empty.
const POLL_WAIT: Duration = Duration::from_secs(25);

#[derive(Deserialize, Default)]
pub struct Params {
    channels: Option<String>,
    position: Option<String>,
}

/// The filter and starting position, or the 400 for a malformed position.
async fn subscription(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    params: &Params,
) -> Result<Result<(Filter, Position), Response>, AppError> {
    let channels = params
        .channels
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter(|c| !c.is_empty())
        .map(str::to_string)
        .collect();
    let mut conn = state.pool.acquire().await?;
    let tags = crate::bus::tags(&mut conn, guardian.user_id()).await?;
    drop(conn);
    let from = match pg_bus::sse::last_event_id(headers) {
        Some(position) => position,
        None => match params.position.as_deref().filter(|p| !p.is_empty()) {
            Some(raw) => match raw.parse() {
                Ok(position) => position,
                Err(_) => {
                    return Ok(Err(
                        (StatusCode::BAD_REQUEST, "invalid position").into_response()
                    ));
                }
            },
            None => state.bus.now().await?,
        },
    };
    Ok(Ok((Filter { channels, tags }, from)))
}

/// GET /bus/events
pub async fn events(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    Query(params): Query<Params>,
) -> Result<Response, AppError> {
    let (filter, from) = match subscription(&state, &guardian, &headers, &params).await? {
        Ok(s) => s,
        Err(response) => return Ok(response),
    };
    Ok(pg_bus::sse::events(
        state.bus.subscribe(from, filter),
        pg_bus::sse::Options::default(),
    ))
}

/// GET /bus/poll
pub async fn poll(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    Query(params): Query<Params>,
) -> Result<Response, AppError> {
    let (filter, from) = match subscription(&state, &guardian, &headers, &params).await? {
        Ok(s) => s,
        Err(response) => return Ok(response),
    };
    let poll = pg_bus::sse::poll(&state.bus, from, filter, POLL_WAIT).await?;
    Ok(Json(poll).into_response())
}
