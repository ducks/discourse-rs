//! Port of PresenceController: GET /presence/get, the state of the named
//! channels the viewer may see, and POST /presence/update, the member's
//! client entering and leaving channels.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::presence;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

/// `PresenceController::MAX_CHANNELS_PER_REQUEST`
const MAX_CHANNELS_PER_REQUEST: usize = 50;

/// A list of strings, as the controller requires one; None for anything
/// else.
fn strings(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(str::to_string))
        .collect()
}

/// GET /presence/get
pub async fn get(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = params::parse_query(uri.query().unwrap_or_default());
    // params.require(:channels)
    let names = match p.get("channels") {
        None => return Ok(super::accounts::param_missing("channels")),
        Some(Value::String(s)) if s.is_empty() => {
            return Ok(super::accounts::param_missing("channels"));
        }
        Some(Value::Array(a)) if a.is_empty() => {
            return Ok(super::accounts::param_missing("channels"));
        }
        Some(value) => match strings(value) {
            Some(names) => names,
            None => return Ok(super::search::invalid_parameters(&state, "channels")),
        },
    };
    let mut unique: Vec<String> = Vec::new();
    for name in names {
        if !unique.contains(&name) {
            unique.push(name);
        }
    }
    if unique.len() > MAX_CHANNELS_PER_REQUEST {
        return Ok(super::search::invalid_parameters(
            &state,
            "Too many channels",
        ));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let group_ids: Vec<i32> = match guardian.user_id() {
        Some(id) => {
            sqlx::query_scalar("SELECT group_id FROM group_users WHERE user_id = $1")
                .bind(id)
                .fetch_all(&mut *conn)
                .await?
        }
        None => Vec::new(),
    };
    let mut out = Map::new();
    for name in unique {
        let value = match presence::config(&mut conn, &settings, &name).await? {
            Some(config) if config.can_view(guardian.user_id(), &group_ids) => {
                let found = presence::state(&mut conn, &state.bus, &name).await?;
                presence::state_json(&mut conn, &urls, &config, &found).await?
            }
            _ => Value::Null,
        };
        out.insert(name, value);
    }
    Ok(Json(Value::Object(out)).into_response())
}

/// POST /presence/update
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let form: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !csrf_ok(&state, &headers, &form, uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let client_id = match p.get("client_id") {
        Some(Value::String(id)) if !crate::ruby::is_blank(id) => id.clone(),
        _ => return Ok(super::search::invalid_parameters(&state, "client_id")),
    };
    let list = |key: &str| -> Result<Option<Vec<String>>, ()> {
        match p.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => strings(value).map(Some).ok_or(()),
        }
    };
    let Ok(present) = list("present_channels") else {
        return Ok(super::search::invalid_parameters(
            &state,
            "present_channels",
        ));
    };
    let Ok(leave) = list("leave_channels") else {
        return Ok(super::search::invalid_parameters(&state, "leave_channels"));
    };
    if present
        .as_ref()
        .is_some_and(|p| p.len() > MAX_CHANNELS_PER_REQUEST)
    {
        return Ok(super::search::invalid_parameters(
            &state,
            "Too many present_channels",
        ));
    }
    let mut out = Map::new();
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    drop(conn);
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    for name in present.into_iter().flatten() {
        let mut tx = state.pool.begin().await?;
        // can_enter?: a member allowed to view (their groups looked up).
        let entered = match presence::config(&mut tx, &settings, &name).await? {
            Some(config) => {
                let group_ids: Vec<i32> =
                    sqlx::query_scalar("SELECT group_id FROM group_users WHERE user_id = $1")
                        .bind(user_id)
                        .fetch_all(&mut *tx)
                        .await?;
                if config.can_view(Some(user_id), &group_ids) {
                    presence::present(
                        &mut tx, &state.bus, &urls, &name, &config, user_id, &client_id,
                    )
                    .await?;
                    true
                } else {
                    false
                }
            }
            None => false,
        };
        tx.commit().await?;
        out.insert(name, Value::Bool(entered));
    }
    for name in leave.into_iter().flatten() {
        let mut tx = state.pool.begin().await?;
        // A channel that doesn't exist is left quietly, not revealed.
        if let Some(config) = presence::config(&mut tx, &settings, &name).await? {
            presence::leave(
                &mut tx, &state.bus, &urls, &name, &config, user_id, &client_id,
            )
            .await?;
        }
        tx.commit().await?;
    }
    Ok(Json(Value::Object(out)).into_response())
}
