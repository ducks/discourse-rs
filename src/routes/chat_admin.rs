//! Chat's incoming webhooks (Chat::IncomingWebhooksController), its admin
//! webhooks (Chat::Admin::IncomingWebhooksController, JSON and the admin
//! pages) and the messages export (Chat::Admin::ExportController).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::guardian::Guardian;
use crate::params;
use crate::plugins::chat::admin_view;
use crate::plugins::chat::channels::Context;
use crate::plugins::chat::incoming_webhooks as hooks;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

async fn load_settings(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

fn context<'a>(
    state: &'a AppState,
    conn: &'a mut sqlx::PgConnection,
    settings: &'a SiteSettings,
    guardian: &'a Guardian,
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

fn strip_format(v: &str) -> &str {
    v.strip_suffix(".json").unwrap_or(v)
}

/// `params.require(key)`: present, not blank.
fn require(p: &Map<String, Value>, key: &str) -> Option<String> {
    match p.get(key)? {
        Value::String(s) if s.trim().is_empty() => None,
        Value::Array(a) if a.is_empty() => None,
        Value::Object(o) if o.is_empty() => None,
        v => Some(params::scalar(v).unwrap_or_default()),
    }
}

/// POST /chat/hooks/:key: `create_message` with the text as given.
pub async fn create_message(
    State(state): State<AppState>,
    Path(key): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    incoming(state, key, headers, uri, body, false).await
}

/// POST /chat/hooks/:key/slack: `create_message_slack_compatible`.
pub async fn create_message_slack(
    State(state): State<AppState>,
    Path(key): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    incoming(state, key, headers, uri, body, true).await
}

/// Chat::IncomingWebhooksController: requires_plugin, no CSRF or login;
/// validate_payload, debug_payload, the text (Slack's processed), then
/// process_webhook_payload: the webhook by key or a 404, its rate limit
/// (10 a minute), the system user following the channel, and
/// Chat::CreateMessage as the system user with the webhook's event.
async fn incoming(
    state: AppState,
    key: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    slack: bool,
) -> Result<Response, AppError> {
    let mut tx = state.pool.begin().await?;
    let settings = load_settings(&state, &mut tx).await?;
    if !crate::plugins::chat::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    let key = strip_format(&key).to_string();
    let p = params::parse(uri.query(), &headers, &body);
    // validate_payload: `!params[:x]`, so only an absent (or null) one.
    let given = |k: &str| p.get(k).is_some_and(|v| !v.is_null());
    if !given("text") && !given("payload") && !given("attachments") {
        return Ok(super::search::invalid_parameters(
            &state,
            "Discourse::InvalidParameters",
        ));
    }
    if settings.get("chat_debug_webhook_payloads")?.truthy() {
        tracing::warn!(
            "Debugging chat webhook payload for endpoint {key}: {}",
            json!({
                "payload": p.get("payload"),
                "attachments": p.get("attachments"),
                "text": p.get("text"),
            })
        );
    }
    let text_param = p.get("text").and_then(params::scalar);
    let text = if slack {
        if text_param.as_deref().is_some_and(|t| !t.trim().is_empty()) {
            Some(hooks::process_text(
                text_param.as_deref().unwrap_or_default(),
            ))
        } else {
            let attachments = match p.get("payload").filter(|v| {
                !matches!(v, Value::Null) && params::scalar(v).is_none_or(|s| !s.trim().is_empty())
            }) {
                Some(Value::String(raw)) => match serde_json::from_str::<Value>(raw) {
                    Ok(payload) => payload["attachments"].clone(),
                    Err(_) => {
                        return Ok(super::search::invalid_parameters(
                            &state,
                            "Discourse::InvalidParameters",
                        ));
                    }
                },
                Some(payload) => payload["attachments"].clone(),
                None => p.get("attachments").cloned().unwrap_or(Value::Null),
            };
            match hooks::legacy_attachment_text(&attachments) {
                Some(text) => Some(text),
                None => {
                    return Err(std::io::Error::other(
                        "NoMethodError: undefined method `[]` for nil (a webhook without an attachment)",
                    )
                    .into());
                }
            }
        }
    } else {
        text_param
    };
    let Some(webhook) = hooks::find_by_key(&mut tx, &key).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let limiter = crate::rate_limiter::RateLimiter::new(
        None,
        &format!("incoming_chat_webhook_{}", webhook.id),
        hooks::MESSAGES_PER_MINUTE,
        60,
    );
    if let Err(refused) = limiter
        .performed(&state.pool, state.config.rate_limits)
        .await?
    {
        return Ok(refused.into_response(&state.i18n));
    }
    // webhook.chat_channel.add(Discourse.system_user)
    let now = crate::clock::now_naive();
    sqlx::query(
        "INSERT INTO user_chat_channel_memberships (user_id, chat_channel_id, following, created_at, updated_at) \
         VALUES (-1, $1, TRUE, $2, $2) \
         ON CONFLICT (user_id, chat_channel_id) DO UPDATE SET following = TRUE, updated_at = $2 \
         WHERE NOT user_chat_channel_memberships.following",
    )
    .bind(webhook.chat_channel_id)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    let Some(system) = crate::session::current::SessionUser::load(&mut tx, -1).await? else {
        return Err(crate::Unsupported("a site without its system user").into());
    };
    let guardian = Guardian::for_user(&mut tx, &system).await?;
    use crate::plugins::chat::create::{Outcome, Params};
    let params = Params {
        chat_channel_id: webhook.chat_channel_id.to_string(),
        message: text,
        in_reply_to_id: None,
        staged_id: None,
        thread_id: None,
        upload_ids: Vec::new(),
        blocks: false,
        client_created_at: None,
        enforce_membership: false,
    };
    let host = crate::pretty_text::Host::from_state(&state);
    let outcome = context(&state, &mut tx, &settings, &guardian)
        .create_message(&host, &state.bus, params)
        .await?;
    Ok(match outcome {
        Outcome::Created(message_id) => {
            // create_webhook_event
            sqlx::query(
                "INSERT INTO chat_webhook_events (chat_message_id, incoming_chat_webhook_id, created_at, updated_at) \
                 VALUES ($1, $2, $3, $3)",
            )
            .bind(message_id)
            .bind(webhook.id)
            .bind(crate::clock::now_naive())
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Json(json!({"success": "OK"})).into_response()
        }
        Outcome::NotFound => super::topics::not_found_response(&state),
        Outcome::Forbidden => super::search::invalid_access(&state),
        // on_failed_contract: InvalidParameters with the messages.
        Outcome::Invalid(errors) => super::search::invalid_parameters(
            &state,
            &format!(
                "[{}]",
                errors
                    .iter()
                    .map(|e| format!("{e:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        Outcome::Unprocessable(error) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"errors": [error]})),
        )
            .into_response(),
    })
}

/// AdminConstraint and requires_plugin: a 404 for anyone but admins, or
/// with chat off.
async fn admin_gate(
    state: &AppState,
    guardian: &Guardian,
    settings: &SiteSettings,
) -> Result<Option<Response>, AppError> {
    if !guardian.is_admin() || !crate::plugins::chat::enabled(settings)? {
        return Ok(Some(super::topics::not_found_response(state)));
    }
    Ok(None)
}

fn base_url(state: &AppState, settings: &SiteSettings) -> Result<String, AppError> {
    Ok(crate::url::Urls {
        config: &state.config,
        settings,
    }
    .base_url()?)
}

/// The admin plugin page around a webhooks route.
async fn admin_page(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    guardian: &Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    crumb: (String, String),
    content: impl FnOnce(&crate::admin_view::AdminCx) -> String,
) -> Result<Response, AppError> {
    let settings = load_settings(state, conn).await?;
    let Some(plugin) = crate::admin_plugins::find(&settings, admin_view::PLUGIN_ID)? else {
        return Ok(super::topics::not_found_response(state));
    };
    let shell =
        super::admin_pages::Shell::load(conn, state, guardian, headers, "admin_plugin_chat")
            .await?;
    let cx = shell.cx(state);
    let page = crate::admin_plugins_view::config_page(
        &cx,
        &plugin,
        (crumb.0.clone(), &crumb.1),
        Some(admin_view::config_nav(&cx, true)),
        &content(&cx),
    );
    let main = crate::admin_plugins_view::render(&page);
    let main = crate::admin_ui::contents("admin-plugins chat", "", &main);
    let title = format!(
        "{} - {} - {}",
        crumb.0,
        crate::admin_plugins_view::name_titleized(&cx, &plugin),
        crate::admin_view::t(&state.i18n, "admin.plugins.title"),
    );
    shell.render(state, uri, &title, main, "chat-admin.js", StatusCode::OK)
}

/// The public channels' titles, as ChatChannelChooser lists them.
async fn channel_choices(cx: &mut Context<'_>, channels: &[Value]) -> Vec<(i64, String)> {
    let _ = cx;
    channels
        .iter()
        .map(|c| {
            (
                c["id"].as_i64().unwrap_or(0),
                c["title"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

/// GET /admin/plugins/chat/hooks(.json): Chat::AdminChatIndexSerializer,
/// or the webhooks page.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = load_settings(&state, &mut conn).await?;
    if let Some(r) = admin_gate(&state, &guardian, &settings).await? {
        return Ok(r);
    }
    let base = base_url(&state, &settings)?;
    let mut cx = context(&state, &mut conn, &settings, &guardian);
    let channels = cx.admin_public_channels().await?;
    let mut webhooks = Vec::new();
    for w in hooks::all(&mut *cx.conn).await? {
        webhooks.push(cx.webhook_json(&w, &base).await?);
    }
    if !super::admin_email::html_request(&uri, &headers) {
        return Ok(Json(json!({
            "chat_channels": channels,
            "incoming_chat_webhooks": webhooks,
        }))
        .into_response());
    }
    let online: Vec<i64> = Vec::new();
    let titles: Vec<String> = webhooks
        .iter()
        .map(|w| {
            crate::plugins::chat::page::channel_title(
                &w["chat_channel"],
                settings.emojis(),
                state.config.globals.relative_url_root(),
                &online,
            )
        })
        .collect();
    let crumb = crate::admin_view::t(&state.i18n, "chat.incoming_webhooks.title");
    admin_page(
        &state,
        &mut conn,
        &guardian,
        &headers,
        &uri,
        (crumb, admin_view::HOOKS_PATH.to_string()),
        |cx| admin_view::hooks_index(cx, &webhooks, &titles),
    )
    .await
}

/// GET /admin/plugins/chat/hooks/new(.json): the public channels, or the
/// new webhook page.
pub async fn new(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = load_settings(&state, &mut conn).await?;
    if let Some(r) = admin_gate(&state, &guardian, &settings).await? {
        return Ok(r);
    }
    let mut cx = context(&state, &mut conn, &settings, &guardian);
    let channels = cx.admin_public_channels().await?;
    if !super::admin_email::html_request(&uri, &headers) {
        return Ok(Json(json!({ "chat_channels": channels })).into_response());
    }
    let choices = channel_choices(&mut cx, &channels).await;
    let crumb = crate::admin_view::t(&state.i18n, "chat.incoming_webhooks.title");
    admin_page(
        &state,
        &mut conn,
        &guardian,
        &headers,
        &uri,
        (crumb, admin_view::HOOKS_PATH.to_string()),
        |cx| admin_view::hooks_form(cx, None, &choices),
    )
    .await
}

/// GET /admin/plugins/chat/hooks/:id/edit(.json):
/// Chat::AdminChatWebhookShowSerializer, or the edit page.
pub async fn edit(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = load_settings(&state, &mut conn).await?;
    if let Some(r) = admin_gate(&state, &guardian, &settings).await? {
        return Ok(r);
    }
    let Some(webhook) = hooks::find(&mut conn, crate::ruby::to_i(strip_format(&id))).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let base = base_url(&state, &settings)?;
    let mut cx = context(&state, &mut conn, &settings, &guardian);
    let channels = cx.admin_public_channels().await?;
    let webhook = cx.webhook_json(&webhook, &base).await?;
    if !super::admin_email::html_request(&uri, &headers) {
        return Ok(Json(json!({ "chat_channels": channels, "webhook": webhook })).into_response());
    }
    let choices = channel_choices(&mut cx, &channels).await;
    let crumb = crate::admin_view::t(&state.i18n, "chat.incoming_webhooks.title");
    admin_page(
        &state,
        &mut conn,
        &guardian,
        &headers,
        &uri,
        (crumb, admin_view::HOOKS_PATH.to_string()),
        |cx| admin_view::hooks_form(cx, Some(&webhook), &choices),
    )
    .await
}

/// A write's params with its CSRF token checked, after the gate.
async fn begin_write(
    state: &AppState,
    guardian: &Guardian,
    method: &Method,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
) -> Result<Result<(SiteSettings, Map<String, Value>), Response>, AppError> {
    let p = params::parse(uri.query(), headers, body);
    let pairs: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    let mut conn = state.pool.acquire().await?;
    let settings = load_settings(state, &mut conn).await?;
    if let Some(r) = admin_gate(state, guardian, &settings).await? {
        return Ok(Err(r));
    }
    if !csrf_ok(state, headers, &pairs, uri.path(), method.as_str()) {
        return Ok(Err(bad_csrf()));
    }
    Ok(Ok((settings, p)))
}

/// `Chat::Channel.find_by(id:)`, refused (404) when missing or a DM.
async fn hook_channel(conn: &mut sqlx::PgConnection, id: &str) -> Result<Option<i64>, AppError> {
    let Ok(id) = id.trim().parse::<i64>() else {
        return Ok(None);
    };
    Ok(sqlx::query_scalar(
        "SELECT id FROM chat_channels WHERE id = $1 AND deleted_at IS NULL AND chatable_type <> 'DirectMessage'",
    )
    .bind(id)
    .fetch_optional(conn)
    .await?)
}

fn record_errors(errors: Vec<String>) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "errors": errors })),
    )
        .into_response()
}

/// POST /admin/plugins/chat/hooks: a webhook with its name, channel and a
/// new key (SecureRandom.hex(12)).
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let (settings, p) = match begin_write(&state, &guardian, &method, &headers, &uri, &body).await?
    {
        Ok(begun) => begun,
        Err(r) => return Ok(r),
    };
    let Some(name) = require(&p, "name") else {
        return Ok(super::accounts::param_missing("name"));
    };
    let Some(channel_id) = require(&p, "chat_channel_id") else {
        return Ok(super::accounts::param_missing("chat_channel_id"));
    };
    let mut tx = state.pool.begin().await?;
    let Some(channel_id) = hook_channel(&mut tx, &channel_id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let errors = hooks::validate(&name, None, None, None);
    if !errors.is_empty() {
        return Ok(record_errors(errors));
    }
    let key = {
        use rand::RngCore;
        let mut b = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut b);
        b.iter().map(|x| format!("{x:02x}")).collect::<String>()
    };
    let now = crate::clock::now_naive();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO incoming_chat_webhooks (name, key, chat_channel_id, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $4) RETURNING id",
    )
    .bind(&name)
    .bind(&key)
    .bind(channel_id)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    let webhook = hooks::find(&mut tx, id)
        .await?
        .ok_or(crate::Unsupported("a webhook gone while creating it"))?;
    let base = base_url(&state, &settings)?;
    let body = context(&state, &mut tx, &settings, &guardian)
        .webhook_json(&webhook, &base)
        .await?;
    tx.commit().await?;
    Ok(Json(body).into_response())
}

/// PUT /admin/plugins/chat/hooks/:id: its name, description, emoji,
/// username and channel.
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: Method,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let (_, p) = match begin_write(&state, &guardian, &method, &headers, &uri, &body).await? {
        Ok(begun) => begun,
        Err(r) => return Ok(r),
    };
    let Some(name) = require(&p, "name") else {
        return Ok(super::accounts::param_missing("name"));
    };
    let Some(channel_id) = require(&p, "chat_channel_id") else {
        return Ok(super::accounts::param_missing("chat_channel_id"));
    };
    let mut tx = state.pool.begin().await?;
    let Some(webhook) = hooks::find(&mut tx, crate::ruby::to_i(strip_format(&id))).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let Some(channel_id) = hook_channel(&mut tx, &channel_id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let text = |k: &str| p.get(k).and_then(params::scalar);
    let (description, emoji, username) = (text("description"), text("emoji"), text("username"));
    let errors = hooks::validate(
        &name,
        username.as_deref(),
        description.as_deref(),
        emoji.as_deref(),
    );
    if !errors.is_empty() {
        return Ok(record_errors(errors));
    }
    let changed = name != webhook.name
        || description != webhook.description
        || emoji != webhook.emoji
        || username != webhook.username
        || channel_id != webhook.chat_channel_id;
    if changed {
        sqlx::query(
            "UPDATE incoming_chat_webhooks SET name = $2, description = $3, emoji = $4, username = $5, \
               chat_channel_id = $6, updated_at = $7 WHERE id = $1",
        )
        .bind(webhook.id)
        .bind(&name)
        .bind(&description)
        .bind(&emoji)
        .bind(&username)
        .bind(channel_id)
        .bind(crate::clock::now_naive())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"success": "OK"})).into_response())
}

/// DELETE /admin/plugins/chat/hooks/:id: the webhook and its events, if
/// it exists.
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: Method,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Err(r) = begin_write(&state, &guardian, &method, &headers, &uri, &body).await? {
        return Ok(r);
    }
    let id = crate::ruby::to_i(strip_format(&id));
    let mut tx = state.pool.begin().await?;
    for sql in [
        "DELETE FROM chat_webhook_events WHERE incoming_chat_webhook_id = $1",
        "DELETE FROM incoming_chat_webhooks WHERE id = $1",
    ] {
        sqlx::query(sql).bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"success": "OK"})).into_response())
}

/// POST /chat/admin/export/messages: Jobs::ExportCsvFile for chat_message
/// (the job is item 200's) and the entity_export log; no content.
pub async fn export_messages(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Err(r) = begin_write(&state, &guardian, &method, &headers, &uri, &body).await? {
        return Ok(r);
    }
    let user_id = guardian.user_id().unwrap_or(0);
    let mut tx = state.pool.begin().await?;
    crate::jobs::enqueue(
        &mut tx,
        "export_csv_file",
        json!({ "entity": "chat_message", "user_id": user_id }),
    )
    .await?;
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, subject, ip_address, admin_only, created_at, updated_at) \
         SELECT $1, $2, 'chat_message', ip_address, FALSE, clock_timestamp(), clock_timestamp() FROM users WHERE id = $2",
    )
    .bind(crate::staff_action_logs::action_id("entity_export"))
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
