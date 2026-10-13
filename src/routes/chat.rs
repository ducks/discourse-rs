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
use crate::ruby::cast_boolean;
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
    let mut out = cx.me_channels().await?;
    // ChannelIndexSerializer#global_presence_channel_state, for members:
    // PresenceChannel.new("/chat/online").state.
    if guardian.user_id().is_some()
        && let Some(config) = crate::presence::config(&mut conn, &settings, "/chat/online").await?
    {
        let urls = crate::url::Urls {
            config: &state.config,
            settings: &settings,
        };
        let online = crate::presence::state(&mut conn, &state.bus, "/chat/online").await?;
        out["global_presence_channel_state"] =
            crate::presence::state_json(&mut conn, &urls, &config, &online).await?;
    }
    Ok(Json(out).into_response())
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

/// ActiveModel's integer cast of a param: digits first (after any sign
/// and space), else nil.
fn cast_integer(v: Option<&str>) -> Option<i64> {
    let v = v?;
    v.trim_start()
        .trim_start_matches(['+', '-'])
        .starts_with(|c: char| c.is_ascii_digit())
        .then(|| crate::ruby::to_i(v))
}

/// GET /chat/api/channels/:channel_id/messages:
/// Chat::Api::ChannelMessagesController#index (Chat::ListChannelMessages).
pub async fn messages(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    uri: Uri,
) -> Result<Response, AppError> {
    use crate::plugins::chat::messages::{ListParams, Listed};
    /// `ChannelMessagesController::MAX_PAGE_SIZE`
    const MAX_PAGE_SIZE: i64 = 50;

    let mut tx = state.pool.begin().await?;
    let settings = match begin(&state, &mut tx, &guardian).await? {
        Ok(settings) => settings,
        Err(refused) => return Ok(refused),
    };
    let p = uri.query().map(params::parse_query).unwrap_or_default();
    let scalar = |key: &str| p.get(key).and_then(params::scalar);

    // The contract.
    let channel_id = cast_integer(Some(id.strip_suffix(".json").unwrap_or(&id)));
    let page_size = cast_integer(scalar("page_size").as_deref());
    let direction = scalar("direction");
    let mut errors = Vec::new();
    if channel_id.is_none() {
        errors.push("Channel can't be blank");
    }
    if page_size.is_some_and(|n| n < 1) {
        errors.push("Page size must be greater than or equal to 1");
    }
    if direction
        .as_deref()
        .is_some_and(|d| d != "past" && d != "future")
    {
        errors.push("Direction is not included in the list");
    }
    let Some(channel_id) = channel_id.filter(|_| errors.is_empty()) else {
        return Ok((
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({"failed": "FAILED", "errors": errors})),
        )
            .into_response());
    };
    let list = ListParams {
        page_size: page_size.unwrap_or(MAX_PAGE_SIZE).min(MAX_PAGE_SIZE),
        target_message_id: cast_integer(scalar("target_message_id").as_deref()),
        direction,
        fetch_from_last_read: cast_boolean(scalar("fetch_from_last_read").as_deref()) == Some(true),
        target_date: scalar("target_date"),
    };
    let mut cx = context(&state, &mut tx, &settings, &guardian);
    let response = match cx.list_messages(channel_id, &list).await? {
        Listed::Messages(body) => Json(body).into_response(),
        Listed::NotFound => return Ok(super::topics::not_found_response(&state)),
        Listed::Forbidden => return Ok(super::search::invalid_access(&state)),
    };
    tx.commit().await?;
    Ok(response)
}

/// A write's request: the CSRF check, then the guard; the transaction,
/// settings and params, or the response refusing it.
async fn begin_write(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &axum::http::HeaderMap,
    uri: &Uri,
    method: &str,
    body: &[u8],
) -> Result<
    Result<
        (
            sqlx::Transaction<'static, sqlx::Postgres>,
            SiteSettings,
            serde_json::Map<String, serde_json::Value>,
        ),
        Response,
    >,
    AppError,
> {
    let p = params::parse(uri.query(), headers, body);
    let form: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !super::session::csrf_ok(state, headers, &form, uri.path(), method) {
        return Ok(Err(super::session::bad_csrf()));
    }
    let mut tx = state.pool.begin().await?;
    let settings = match begin(state, &mut tx, guardian).await? {
        Ok(settings) => settings,
        Err(refused) => return Ok(Err(refused)),
    };
    Ok(Ok((tx, settings, p)))
}

/// A membership write's outcome as its controller answers it.
fn membership_response(
    state: &AppState,
    outcome: crate::plugins::chat::membership::Outcome,
) -> Response {
    use crate::plugins::chat::membership::Outcome;
    match outcome {
        Outcome::Done(body) => Json(body).into_response(),
        Outcome::NotFound => super::topics::not_found_response(state),
        Outcome::Forbidden => super::search::invalid_access(state),
        Outcome::Invalid(errors) => (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({"failed": "FAILED", "errors": errors})),
        )
            .into_response(),
        Outcome::InvalidParameter(name) => super::search::invalid_parameters(state, name),
        Outcome::Unprocessable => (
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"failed": "FAILED"})),
        )
            .into_response(),
    }
}

/// The channel id a path names (an integer param cast as ActiveModel
/// casts it; anything else finds no channel).
fn path_channel_id(id: &str) -> i64 {
    cast_integer(Some(id.strip_suffix(".json").unwrap_or(id))).unwrap_or(0)
}

/// POST, PUT and DELETE /chat/api/channels/:channel_id/memberships/me:
/// ChannelsCurrentUserMembershipController (join, star, leave).
pub async fn own_membership(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: axum::http::Method,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let (mut tx, settings, p) =
        match begin_write(&state, &guardian, &headers, &uri, method.as_str(), &body).await? {
            Ok(begun) => begun,
            Err(refused) => return Ok(refused),
        };
    let channel_id = path_channel_id(&id);
    let mut cx = context(&state, &mut tx, &settings, &guardian);
    let outcome = match method.as_str() {
        "POST" => cx.join(channel_id).await?,
        "PUT" => {
            let starred = cast_boolean(p.get("starred").and_then(params::scalar).as_deref());
            cx.star(channel_id, starred).await?
        }
        _ => cx.leave(channel_id).await?,
    };
    tx.commit().await?;
    Ok(membership_response(&state, outcome))
}

/// DELETE /chat/api/channels/:channel_id/memberships/me/follows:
/// Chat::UnfollowChannel.
pub async fn unfollow(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let (mut tx, settings, _) =
        match begin_write(&state, &guardian, &headers, &uri, "DELETE", &body).await? {
            Ok(begun) => begun,
            Err(refused) => return Ok(refused),
        };
    let outcome = context(&state, &mut tx, &settings, &guardian)
        .unfollow(path_channel_id(&id))
        .await?;
    tx.commit().await?;
    Ok(membership_response(&state, outcome))
}

/// PUT /chat/api/channels/:channel_id/read: Chat::UpdateUserChannelLastRead.
pub async fn mark_read(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let (mut tx, settings, p) =
        match begin_write(&state, &guardian, &headers, &uri, "PUT", &body).await? {
            Ok(begun) => begun,
            Err(refused) => return Ok(refused),
        };
    let message_id = cast_integer(p.get("message_id").and_then(params::scalar).as_deref());
    let outcome = context(&state, &mut tx, &settings, &guardian)
        .mark_read(&state.bus, path_channel_id(&id), message_id)
        .await?;
    tx.commit().await?;
    Ok(membership_response(&state, outcome))
}

/// PUT /chat/api/channels/read: Chat::MarkAllUserChannelsRead.
pub async fn mark_all_read(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: axum::http::HeaderMap,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let (mut tx, settings, _) =
        match begin_write(&state, &guardian, &headers, &uri, "PUT", &body).await? {
            Ok(begun) => begun,
            Err(refused) => return Ok(refused),
        };
    let outcome = context(&state, &mut tx, &settings, &guardian)
        .mark_all_read(&state.bus)
        .await?;
    tx.commit().await?;
    Ok(membership_response(&state, outcome))
}

/// POST /chat/:chat_channel_id: Chat::Api::ChannelMessagesController#create
/// (Chat::CreateMessage).
pub async fn create_message(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    use crate::plugins::chat::create::{Outcome, Params};
    let (mut tx, settings, p) =
        match begin_write(&state, &guardian, &headers, &uri, "POST", &body).await? {
            Ok(begun) => begun,
            Err(refused) => return Ok(refused),
        };
    if let Some(refused) = message_rate_limiter(&state, &settings, &guardian).await? {
        return Ok(refused);
    }
    let scalar = |key: &str| p.get(key).and_then(params::scalar);
    let upload_ids = match p.get("upload_ids") {
        Some(serde_json::Value::Array(ids)) => ids.iter().filter_map(params::scalar).collect(),
        Some(v) => params::scalar(v).into_iter().collect(),
        None => Vec::new(),
    };
    let params = Params {
        chat_channel_id: id.strip_suffix(".json").unwrap_or(&id).to_string(),
        message: scalar("message"),
        in_reply_to_id: scalar("in_reply_to_id"),
        staged_id: scalar("staged_id"),
        thread_id: scalar("thread_id"),
        upload_ids,
        blocks: p.contains_key("blocks"),
        client_created_at: scalar("client_created_at"),
    };
    let host = crate::pretty_text::Host::from_state(&state);
    let outcome = context(&state, &mut tx, &settings, &guardian)
        .create_message(&host, &state.bus, params)
        .await?;
    let response = match outcome {
        Outcome::Created(id) => {
            tx.commit().await?;
            return Ok(Json(json!({"success": "OK", "message_id": id})).into_response());
        }
        Outcome::NotFound => super::topics::not_found_response(&state),
        Outcome::Forbidden => super::search::invalid_access(&state),
        Outcome::Invalid(errors) => (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({"failed": "FAILED", "errors": errors})),
        )
            .into_response(),
        Outcome::Unprocessable(error) => (
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"errors": [error]})),
        )
            .into_response(),
    };
    Ok(response)
}

/// `Chat::MessageRateLimiter.run!`, before the service (an invalid message
/// counts too): not for staff, nor when the trust level's allowance is
/// 0. The message that finds none left silences its sender
/// (chat_auto_silence_duration minutes, by the system user) and is
/// refused.
async fn message_rate_limiter(
    state: &AppState,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
) -> Result<Option<Response>, AppError> {
    let Some(user) = guardian.user() else {
        return Ok(None);
    };
    if guardian.is_staff() {
        return Ok(None);
    }
    let allowed = if user.trust_level == 0 {
        settings.get("chat_allowed_messages_for_trust_level_0")?
    } else {
        settings.get("chat_allowed_messages_for_other_trust_levels")?
    }
    .to_i();
    if allowed == 0 {
        return Ok(None);
    }
    let limiter = crate::rate_limiter::RateLimiter::new(
        Some(crate::rate_limiter::LimitUser {
            id: user.id,
            staff: false,
        }),
        "create_chat_message",
        allowed,
        30,
    );
    if limiter.remaining(&state.pool).await? == 0 {
        let minutes = settings.get("chat_auto_silence_duration")?.to_i();
        if minutes != 0 {
            let host = crate::pretty_text::Host::from_state(state);
            let ctx = crate::posting::Ctx {
                host: &host,
                settings,
                config: &state.config,
                i18n: &state.i18n,
                bus: &state.bus,
            };
            let until = crate::clock::now_naive() + chrono::Duration::minutes(minutes);
            let reason = state
                .i18n
                .t("chat.errors.rate_limit_exceeded")
                .unwrap_or_default()
                .to_string();
            crate::user_penalties::silencer_silence(&state.pool, &ctx, -1, user.id, until, &reason)
                .await?;
        }
    }
    if let Err(refused) = limiter
        .performed(&state.pool, state.config.rate_limits)
        .await?
    {
        return Ok(Some(refused.into_response(&state.i18n)));
    }
    Ok(None)
}

/// POST /chat/api/channels/:channel_id/drafts: Chat::UpsertDraft.
pub async fn draft(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let (mut tx, settings, p) =
        match begin_write(&state, &guardian, &headers, &uri, "POST", &body).await? {
            Ok(begun) => begun,
            Err(refused) => return Ok(refused),
        };
    let thread_id = p.get("thread_id").and_then(params::scalar);
    let outcome = context(&state, &mut tx, &settings, &guardian)
        .upsert_draft(path_channel_id(&id), p.get("data"), thread_id.as_deref())
        .await?;
    tx.commit().await?;
    Ok(membership_response(&state, outcome))
}

fn modify_response(state: &AppState, outcome: crate::plugins::chat::modify::Outcome) -> Response {
    use crate::plugins::chat::modify::Outcome;
    match outcome {
        Outcome::Done(body) => Json(body).into_response(),
        Outcome::NotFound => super::topics::not_found_response(state),
        Outcome::Forbidden(None) => super::search::invalid_access(state),
        Outcome::Forbidden(Some(key)) => super::search::invalid_access_with(state, &key),
        Outcome::InvalidParameters => {
            super::search::invalid_parameters(state, "Discourse::InvalidParameters")
        }
        Outcome::ParamMissing(name) => super::accounts::param_missing(name),
        Outcome::Invalid(errors) => (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({"failed": "FAILED", "errors": errors})),
        )
            .into_response(),
        Outcome::Unprocessable(error) => (
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"errors": [error]})),
        )
            .into_response(),
        Outcome::RecordInvalid(errors) => (
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"errors": errors, "error_type": "record_invalid"})),
        )
            .into_response(),
    }
}

/// Commits a change that went through, else drops it.
async fn finish(
    state: &AppState,
    tx: sqlx::Transaction<'static, sqlx::Postgres>,
    outcome: crate::plugins::chat::modify::Outcome,
) -> Result<Response, AppError> {
    if matches!(outcome, crate::plugins::chat::modify::Outcome::Done(_)) {
        tx.commit().await?;
    }
    Ok(modify_response(state, outcome))
}

fn strip_format(id: &str) -> &str {
    id.strip_suffix(".json").unwrap_or(id)
}

/// PUT /chat/api/channels/:channel_id/messages/:message_id
/// (Chat::Api::ChannelMessagesController#update).
pub async fn update_message(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: axum::http::HeaderMap,
    Path((id, message_id)): Path<(String, String)>,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let (mut tx, settings, p) =
        match begin_write(&state, &guardian, &headers, &uri, "PUT", &body).await? {
            Ok(begun) => begun,
            Err(refused) => return Ok(refused),
        };
    let upload_ids: Vec<String> = match p.get("upload_ids") {
        Some(serde_json::Value::Array(ids)) => ids.iter().filter_map(params::scalar).collect(),
        Some(v) => params::scalar(v).into_iter().collect(),
        None => Vec::new(),
    };
    let host = crate::pretty_text::Host::from_state(&state);
    let outcome = context(&state, &mut tx, &settings, &guardian)
        .update_message(
            &host,
            &state.bus,
            contract_integer(&id),
            strip_format(&message_id),
            p.get("message").and_then(params::scalar),
            &upload_ids,
        )
        .await?;
    finish(&state, tx, outcome).await
}

/// DELETE /chat/api/channels/:channel_id/messages/:message_id
/// (Chat::Api::ChannelMessagesController#destroy).
pub async fn trash_message(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: axum::http::HeaderMap,
    Path((id, message_id)): Path<(String, String)>,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let (mut tx, settings, _) =
        match begin_write(&state, &guardian, &headers, &uri, "DELETE", &body).await? {
            Ok(begun) => begun,
            Err(refused) => return Ok(refused),
        };
    let outcome = context(&state, &mut tx, &settings, &guardian)
        .trash_message(
            &state.bus,
            contract_integer(&id),
            contract_integer(&message_id),
        )
        .await?;
    finish(&state, tx, outcome).await
}

/// PUT /chat/api/channels/:channel_id/messages/:message_id/restore
/// (Chat::Api::ChannelMessagesController#restore).
pub async fn restore_message(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: axum::http::HeaderMap,
    Path((id, message_id)): Path<(String, String)>,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let (mut tx, settings, _) =
        match begin_write(&state, &guardian, &headers, &uri, "PUT", &body).await? {
            Ok(begun) => begun,
            Err(refused) => return Ok(refused),
        };
    let outcome = context(&state, &mut tx, &settings, &guardian)
        .restore_message(
            &state.bus,
            contract_integer(&id),
            contract_integer(&message_id),
        )
        .await?;
    finish(&state, tx, outcome).await
}

/// PUT /chat/:chat_channel_id/react/:message_id (Chat::ChatController#react).
pub async fn react(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: axum::http::HeaderMap,
    Path((id, message_id)): Path<(String, String)>,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let (mut tx, settings, p) =
        match begin_write(&state, &guardian, &headers, &uri, "PUT", &body).await? {
            Ok(begun) => begun,
            Err(refused) => return Ok(refused),
        };
    let emoji = p.get("emoji").and_then(params::scalar);
    let react_action = p.get("react_action").and_then(params::scalar);
    let outcome = context(&state, &mut tx, &settings, &guardian)
        .react(
            &state.bus,
            &id,
            strip_format(&message_id),
            emoji.as_deref(),
            react_action.as_deref(),
        )
        .await?;
    finish(&state, tx, outcome).await
}

/// A service contract's integer attribute (ActiveModel's cast): blank is
/// nil, anything else `to_i`.
fn contract_integer(v: &str) -> Option<i64> {
    let v = strip_format(v);
    (!v.trim().is_empty()).then(|| crate::ruby::to_i(v))
}

/// PUT /chat/:chat_channel_id/:message_id/rebake (Chat::ChatController#rebake).
pub async fn rebake_message(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: axum::http::HeaderMap,
    Path((id, message_id)): Path<(String, String)>,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let (mut tx, settings, _) =
        match begin_write(&state, &guardian, &headers, &uri, "PUT", &body).await? {
            Ok(begun) => begun,
            Err(refused) => return Ok(refused),
        };
    let outcome = context(&state, &mut tx, &settings, &guardian)
        .rebake_message(contract_integer(&id), contract_integer(&message_id))
        .await?;
    finish(&state, tx, outcome).await
}

/// POST /chat/dismiss-retention-reminder: ChatController#
/// dismiss_retention_reminder, the reminder of a category channel's or a
/// direct message's retention set dismissed for the member.
pub async fn dismiss_retention_reminder(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: axum::http::HeaderMap,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let (mut tx, _, p) = match begin_write(&state, &guardian, &headers, &uri, "POST", &body).await?
    {
        Ok(begun) => begun,
        Err(refused) => return Ok(refused),
    };
    let Some(chatable_type) = p
        .get("chatable_type")
        .and_then(params::scalar)
        .filter(|t| !t.is_empty())
    else {
        return Ok(super::accounts::param_missing("chatable_type"));
    };
    let field = match chatable_type.as_str() {
        "Category" => "dismissed_channel_retention_reminder",
        "DirectMessage" => "dismissed_dm_retention_reminder",
        _ => {
            return Ok(super::search::invalid_parameters(
                &state,
                "Discourse::InvalidParameters",
            ));
        }
    };
    sqlx::query(&format!(
        "UPDATE user_options SET {field} = TRUE WHERE user_id = $1 AND {field} IS DISTINCT FROM TRUE"
    ))
    .bind(guardian.user_id().unwrap_or(0))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"success": "OK"})).into_response())
}

/// GET /chat/direct_messages(.json): Chat::DirectMessagesController#index,
/// the one-to-one (or self) direct message of the member and the named
/// users, with the member's membership; an empty 404 when there is none.
pub async fn direct_messages(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: axum::http::HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = match begin(&state, &mut conn, &guardian).await? {
        Ok(settings) => settings,
        Err(refused) => return Ok(refused),
    };
    let p = params::parse(uri.query(), &headers, &[]);
    let usernames: Vec<String> = match p.get("usernames") {
        Some(serde_json::Value::Array(names)) => names.iter().filter_map(params::scalar).collect(),
        Some(v) => params::scalar(v)
            .unwrap_or_default()
            .split(',')
            .map(str::to_string)
            .collect(),
        None => Vec::new(),
    };
    if usernames.iter().all(|u| u.is_empty()) {
        return Ok(super::accounts::param_missing("usernames"));
    }
    let Some(me) = guardian.user() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    // users = [current_user] + User.where(username: others)
    let others: Vec<String> = usernames
        .into_iter()
        .filter(|u| *u != me.username)
        .collect();
    let mut ids: Vec<i32> = vec![me.id];
    if !others.is_empty() {
        ids.extend(
            sqlx::query_scalar::<_, i32>("SELECT id FROM users WHERE username = ANY($1)")
                .bind(&others)
                .fetch_all(&mut *conn)
                .await?,
        );
    }
    ids.sort();
    ids.dedup();
    // Chat::DirectMessage.for_user_ids(ids): the participants exactly.
    let sql = format!(
        "SELECT {} FROM chat_channels WHERE chat_channels.deleted_at IS NULL \
           AND chat_channels.chatable_type = 'DirectMessage' AND chat_channels.chatable_id = ( \
             SELECT direct_message_channels.id FROM direct_message_channels \
             JOIN direct_message_users ON direct_message_users.direct_message_channel_id = direct_message_channels.id \
             JOIN users ON users.id = direct_message_users.user_id \
             WHERE direct_message_channels.\"group\" = FALSE \
             GROUP BY direct_message_channels.id \
             HAVING ARRAY_AGG(users.id ORDER BY users.id) = $1::int[] \
             ORDER BY direct_message_channels.id LIMIT 1)",
        crate::plugins::chat::channels::CHANNEL_COLUMNS
    );
    let channel: Option<crate::plugins::chat::channels::ChannelRow> = sqlx::query_as(&sql)
        .bind(&ids)
        .fetch_optional(&mut *conn)
        .await?;
    let Some(channel) = channel else {
        return Ok(axum::http::StatusCode::NOT_FOUND.into_response());
    };
    let mut cx = context(&state, &mut conn, &settings, &guardian);
    Ok(Json(cx.show(&channel).await?).into_response())
}
