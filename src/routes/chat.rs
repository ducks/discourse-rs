//! The chat plugin's API (Chat::Api::*): what Chat::BaseController
//! guards every action with, and the actions ported so far.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::Uri;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sqlx::PgConnection;

use crate::params;
use crate::plugins::chat::channels::{Context, Found, MEMBERSHIPS_LIMIT, STATUSES, Search};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// Chat::BaseController's before actions: requires_plugin (404),
/// ensure_logged_in, ensure_can_chat. None when the request may go on.
async fn guard(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
) -> Result<Option<Response>, AppError> {
    if !crate::plugins::chat::enabled(settings)? {
        return Ok(Some(super::topics::not_found_response(state)));
    }
    if guardian.is_anonymous() {
        return Ok(Some(super::login_required::not_logged_in(state)));
    }
    if !crate::plugins::chat::can_chat(conn, settings, guardian).await? {
        return Ok(Some(super::search::invalid_access(state)));
    }
    Ok(None)
}

/// The settings, and the guard's refusal if any.
async fn begin(
    state: &AppState,
    conn: &mut PgConnection,
    guardian: &crate::guardian::Guardian,
) -> Result<Result<SiteSettings, Response>, AppError> {
    let settings =
        SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
    Ok(match guard(state, conn, &settings, guardian).await? {
        Some(refused) => Err(refused),
        None => Ok(settings),
    })
}

fn context<'a>(
    state: &'a AppState,
    conn: &'a mut PgConnection,
    settings: &'a SiteSettings,
    guardian: &'a crate::guardian::Guardian,
) -> Context<'a> {
    Context {
        conn,
        settings,
        i18n: &state.i18n,
        guardian,
        base_path: state.config.globals.relative_url_root(),
        config: &state.config,
    }
}

/// GET /chat/api/me/channels: Chat::Api::CurrentUserChannelsController#index.
pub async fn me_channels(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match begin(&state, &mut conn, &guardian).await? {
        Ok(settings) => settings,
        Err(refused) => return Ok(refused),
    };
    let mut cx = context(&state, &mut conn, &settings, &guardian);
    Ok(Json(cx.me_channels().await?).into_response())
}

/// `ActiveModel::Type::Boolean.new.cast`: nil for nothing or "", false
/// for the false values, else true.
fn cast_boolean(v: Option<&str>) -> Option<bool> {
    match v? {
        "" => None,
        "0" | "f" | "F" | "false" | "FALSE" | "off" | "OFF" => Some(false),
        _ => Some(true),
    }
}

/// GET /chat/api/channels: Chat::Api::ChannelsController#index.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match begin(&state, &mut conn, &guardian).await? {
        Ok(settings) => settings,
        Err(refused) => return Ok(refused),
    };
    let p = uri.query().map(params::parse_query).unwrap_or_default();
    let scalar = |key: &str| p.get(key).and_then(params::scalar);
    let filter = scalar("filter");
    let limit = scalar("limit").map_or(25, |l| crate::ruby::to_i(&l));
    let offset = scalar("offset").map_or(0, |o| crate::ruby::to_i(&o));
    let status = scalar("status").filter(|s| STATUSES.contains(&s.as_str()));
    let chatable_id = scalar("chatable_id");
    let chatable_type = scalar("chatable_type");
    let include_subcategories = (chatable_type.as_deref() == Some("Category"))
        .then(|| cast_boolean(scalar("include_subcategories").as_deref()));
    let present = |v: &Option<String>| v.as_deref().is_some_and(|v| !v.trim().is_empty());
    let search = Search {
        filter: filter.clone(),
        status: status
            .as_deref()
            .and_then(|s| STATUSES.iter().position(|x| *x == s))
            .map(|i| i as i32),
        chatable: (present(&chatable_id) && present(&chatable_type)).then(|| {
            (
                chatable_type.clone().unwrap_or_default(),
                crate::ruby::to_i(chatable_id.as_deref().unwrap_or_default()),
            )
        }),
        include_subcategories: include_subcategories.flatten() == Some(true),
        following: false,
        starred: false,
        limit: Some(limit),
        offset,
    };
    let channels = context(&state, &mut conn, &settings, &guardian)
        .index(&search)
        .await?;

    // options.merge(offset: offset + limit).to_query: each pair escaped,
    // a nil as its bare key, sorted.
    let pair = |k: &str, v: Option<String>| match v {
        Some(v) => format!("{k}={}", crate::ruby::cgi_escape(&v)),
        None => k.to_string(),
    };
    let mut query = vec![
        pair("filter", filter),
        pair("limit", Some(limit.to_string())),
        pair("offset", Some((offset + limit).to_string())),
        pair("status", status),
        pair("chatable_id", chatable_id),
        pair("chatable_type", chatable_type),
    ];
    if let Some(include) = include_subcategories {
        query.push(pair(
            "include_subcategories",
            include.map(|b| b.to_string()),
        ));
    }
    query.sort();
    Ok(Json(json!({
        "channels": channels,
        "meta": {"load_more_url": format!("/chat/api/channels?{}", query.join("&"))},
    }))
    .into_response())
}

/// The channel the path names, or the response refusing it.
async fn channel(
    state: &AppState,
    cx: &mut Context<'_>,
    id: &str,
) -> Result<Result<crate::plugins::chat::channels::ChannelRow, Response>, AppError> {
    Ok(
        match cx
            .find_joinable(id.strip_suffix(".json").unwrap_or(id))
            .await?
        {
            Found::Channel(channel) => Ok(channel),
            Found::NotFound => Err(super::topics::not_found_response(state)),
            Found::Forbidden => Err(super::search::invalid_access(state)),
        },
    )
}

/// GET /chat/api/channels/:channel_id: Chat::Api::ChannelsController#show.
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match begin(&state, &mut conn, &guardian).await? {
        Ok(settings) => settings,
        Err(refused) => return Ok(refused),
    };
    let mut cx = context(&state, &mut conn, &settings, &guardian);
    let channel = match channel(&state, &mut cx, &id).await? {
        Ok(channel) => channel,
        Err(refused) => return Ok(refused),
    };
    Ok(Json(cx.show(&channel).await?).into_response())
}

/// GET /chat/api/channels/:channel_id/memberships:
/// Chat::Api::ChannelsMembershipsController#index.
pub async fn memberships(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match begin(&state, &mut conn, &guardian).await? {
        Ok(settings) => settings,
        Err(refused) => return Ok(refused),
    };
    let p = uri.query().map(params::parse_query).unwrap_or_default();
    let scalar = |key: &str| p.get(key).and_then(params::scalar);
    let mut cx = context(&state, &mut conn, &settings, &guardian);
    let channel = match channel(&state, &mut cx, &id).await? {
        Ok(channel) => channel,
        Err(refused) => return Ok(refused),
    };
    let offset = scalar("offset").map_or(0, |o| crate::ruby::to_i(&o));
    // fetch_limit_from_params(default: 50, max: 50): an integer within
    // 0..=50.
    let limit = match scalar("limit") {
        None => MEMBERSHIPS_LIMIT,
        Some(raw) => match raw.trim().parse::<i64>() {
            Ok(n) if (0..=MEMBERSHIPS_LIMIT).contains(&n) => n,
            _ => return Ok(super::search::invalid_parameters(&state, "limit")),
        },
    };
    let username = scalar("username");
    Ok(Json(
        cx.members(&channel, offset, limit, username.as_deref())
            .await?,
    )
    .into_response())
}
