//! Jobs::EmitWebHookEvent and WebHookEmitter: an event's delivery to a
//! hook, its row with what was sent and what came back, the hook's last
//! delivery status, the retries and the deactivation after them.

use std::time::Duration;

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::admin::WebHook;
use crate::final_destination::{Fetcher, Post, Posted};
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

/// The job's name.
pub const EMIT_WEB_HOOK_EVENT: &str = "emit_web_hook_event";
const PING_EVENT: &str = "ping";
const MAX_RETRY_COUNT: i64 = 4;
const RETRY_BACKOFF: i64 = 5;
/// `WebHookEmitter::REQUEST_TIMEOUT`
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// `WebHook.last_delivery_statuses`
const FAILED: i32 = 2;
const SUCCESSFUL: i32 = 3;
/// `Group::AUTO_GROUPS[:staff]`
const STAFF: i32 = 3;
/// `Discourse::SYSTEM_USER_ID`
const SYSTEM_USER_ID: i32 = -1;

/// Discourse::InvalidParameters, which fails the job.
#[derive(Debug)]
struct InvalidParameters(&'static str);

impl std::fmt::Display for InvalidParameters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Discourse::InvalidParameters: {}", self.0)
    }
}

impl std::error::Error for InvalidParameters {}

/// `@arguments[key].blank?`
fn blank(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        Value::Bool(b) => !b,
        Value::Number(_) => false,
    }
}

/// The hook's ids in a habtm join table.
async fn hook_ids(
    conn: &mut PgConnection,
    join: &str,
    key: &str,
    hook_id: i32,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(&format!(
        "SELECT {key}::bigint FROM {join} WHERE web_hook_id = $1"
    ))
    .bind(hook_id)
    .fetch_all(conn)
    .await
}

/// `(@web_hook.<x>_ids & @arguments[...]).blank?` with Ruby's equality:
/// an integer never equals a string.
fn intersects(hook: &[i64], args: &Value) -> bool {
    args.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_i64)
        .any(|id| hook.contains(&id))
}

/// `Jobs::EmitWebHookEvent#execute`
pub async fn emit_web_hook_event(state: &AppState, args: &Value) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let retry_count = args["retry_count"].as_i64().unwrap_or(0);
    let hook = match args["web_hook_id"].as_i64() {
        Some(id) => WebHook::find(&mut conn, id).await?,
        None => None,
    };
    if blank(&args["web_hook_id"]) {
        return Err(InvalidParameters("web_hook_id").into());
    }
    if blank(&args["event_type"]) {
        return Err(InvalidParameters("event_type").into());
    }
    // The hook was deleted.
    let Some(hook) = hook else {
        return Ok(());
    };
    let event_type = crate::params::scalar(&args["event_type"]).unwrap_or_default();
    let ping = event_type == PING_EVENT;
    if !ping {
        if blank(&args["payload"]) {
            return Err(InvalidParameters("payload").into());
        }
        if !hook.active {
            return Ok(());
        }
        let groups = hook_ids(&mut conn, "groups_web_hooks", "group_id", hook.id).await?;
        if !groups.is_empty() && !intersects(&groups, &args["group_ids"]) {
            return Ok(());
        }
        let categories =
            hook_ids(&mut conn, "categories_web_hooks", "category_id", hook.id).await?;
        if !categories.is_empty()
            && !args["category_id"]
                .as_i64()
                .is_some_and(|id| categories.contains(&id))
        {
            return Ok(());
        }
        let tags = hook_ids(&mut conn, "tags_web_hooks", "tag_id", hook.id).await?;
        if !tags.is_empty() && !intersects(&tags, &args["tag_ids"]) {
            return Ok(());
        }
    }

    // build_webhook_body
    let body = if ping {
        json!({ "ping": "OK" })
    } else {
        let payload = crate::params::scalar(&args["payload"]).unwrap_or_default();
        let parsed: Value = serde_json::from_str(&payload)?;
        let mut body = Map::new();
        body.insert(event_type.clone(), parsed);
        Value::Object(body)
    };
    let body = serde_json::to_string(&body)?;

    // create_webhook_event: its after_save marks the hook failed (the new
    // row's status is 0).
    let event_id: i64 = sqlx::query_scalar(
        "INSERT INTO web_hook_events (web_hook_id, payload, created_at, updated_at) \
         VALUES ($1, $2, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(hook.id)
    .bind(&body)
    .fetch_one(&mut *conn)
    .await?;
    let hook = update_delivery_status(&mut conn, &hook, 0).await?;

    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let url = hook.payload_url.trim().to_string();
    if !crate::final_destination::ruby_uri_valid(&url) {
        return Err(std::io::Error::other(format!("URI::InvalidURIError: {url}")).into());
    }
    // build_webhook_headers
    let content_type = if hook.content_type == 2 {
        "application/x-www-form-urlencoded"
    } else {
        "application/json"
    };
    let mut headers: Vec<(String, String)> = vec![
        ("Accept".into(), "*/*".into()),
        ("Connection".into(), "close".into()),
        ("Content-Length".into(), body.len().to_string()),
        ("Content-Type".into(), content_type.into()),
        ("Host".into(), uri_host(&url).unwrap_or_default()),
        ("User-Agent".into(), crate::schema::user_agent()),
        ("X-Discourse-Instance".into(), urls.base_url()?),
        ("X-Discourse-Event-Id".into(), event_id.to_string()),
        ("X-Discourse-Event-Type".into(), event_type.clone()),
    ];
    if let Some(name) = crate::params::scalar(&args["event_name"]).filter(|n| !n.trim().is_empty())
    {
        headers.push(("X-Discourse-Event".into(), name));
    }
    if let Some(secret) = hook.secret.as_deref().filter(|s| !s.trim().is_empty()) {
        headers.push((
            "X-Discourse-Event-Signature".into(),
            format!("sha256={}", signature(secret, body.as_bytes())),
        ));
    }

    let status = emit(
        &mut conn, state, &settings, &hook, event_id, &headers, &body,
    )
    .await?;

    // publish_webhook_event
    state
        .bus
        .publish(
            &mut conn,
            &format!("/web_hook_events/{}", hook.id),
            &json!({ "web_hook_event_id": event_id, "event_type": event_type }),
            Some(&[crate::bus::group_tag(STAFF)]),
        )
        .await?;

    // process_webhook_response: nothing when the request failed.
    let Some(status) = status else {
        return Ok(());
    };
    match status {
        200..=299 => {}
        404 | 410 => {
            if retry_count >= MAX_RETRY_COUNT {
                deactivate(&mut conn, state, &hook, status).await?;
            }
        }
        _ => {
            if settings.get("retry_web_hook_events")?.truthy() {
                let retry_count = retry_count + 1;
                if retry_count <= MAX_RETRY_COUNT {
                    let delay = RETRY_BACKOFF.pow((retry_count - 1) as u32) * 60;
                    let mut args = args.clone();
                    args["retry_count"] = json!(retry_count);
                    crate::jobs::enqueue_in(&mut conn, delay, EMIT_WEB_HOOK_EVENT, args).await?;
                }
            }
        }
    }
    Ok(())
}

/// `URI(url).host`, as written (an IPv6 address keeps its brackets).
fn uri_host(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = rest[..end].rsplit('@').next()?;
    if authority.starts_with('[') {
        let close = authority.find(']')?;
        return Some(authority[..=close].to_string());
    }
    Some(authority.split(':').next()?.to_string())
}

/// `OpenSSL::HMAC.hexdigest("sha256", secret, body)`
pub fn signature(secret: &str, body: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    let mut mac =
        Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes any key");
    mac.update(body);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// WebHookEvent's after_save: the hook's last delivery status from the
/// event's, saved (`save!`, its before_save stripping the url) when that
/// changes the row.
async fn update_delivery_status(
    conn: &mut PgConnection,
    hook: &WebHook,
    status: i32,
) -> Result<WebHook, AppError> {
    let last = if (200..=299).contains(&status) {
        SUCCESSFUL
    } else {
        FAILED
    };
    let url = hook.payload_url.trim().to_string();
    if last == hook.last_delivery_status && url == hook.payload_url {
        return Ok(hook.clone());
    }
    let mut saved = hook.clone();
    saved.last_delivery_status = last;
    saved.payload_url = url;
    sqlx::query(
        "UPDATE web_hooks SET last_delivery_status = $2, payload_url = $3, \
                updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(saved.id)
    .bind(saved.last_delivery_status)
    .bind(&saved.payload_url)
    .execute(&mut *conn)
    .await?;
    Ok(saved)
}

/// `WebHookEmitter#emit!`: the POST, then the event row updated with what
/// was sent and what came back (and so the hook's delivery status). The
/// response's status, None when there was none.
pub async fn emit(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    hook: &WebHook,
    event_id: i64,
    headers: &[(String, String)],
    body: &str,
) -> Result<Option<u16>, AppError> {
    let fetcher = Fetcher {
        settings,
        urls: Urls {
            config: &state.config,
            settings,
        },
    };
    let user_agent = crate::schema::user_agent();
    let url = hook.payload_url.trim();
    let started = std::time::Instant::now();
    let posted = fetcher
        .post(&Post {
            url,
            headers,
            body: body.as_bytes(),
            verify_certificate: hook.verify_certificate,
            timeout: REQUEST_TIMEOUT,
            user_agent: &user_agent,
        })
        .await?;
    let duration = started.elapsed().as_millis() as i32;
    let sent: Map<String, Value> = headers
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    let sent = serde_json::to_string(&sent)?;
    let status = match posted {
        Posted::Response {
            status,
            headers,
            body,
        } => {
            let received: Map<String, Value> = headers
                .into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect();
            sqlx::query(
                "UPDATE web_hook_events SET headers = $2, duration = $3, response_headers = $4, \
                        response_body = $5, status = $6, updated_at = clock_timestamp() \
                 WHERE id = $1",
            )
            .bind(event_id)
            .bind(&sent)
            .bind(duration)
            .bind(serde_json::to_string(&received)?)
            .bind(&body)
            .bind(i32::from(status))
            .execute(&mut *conn)
            .await?;
            Some(status)
        }
        Posted::Blocked | Posted::Failed(_) => {
            let error = match posted {
                Posted::Failed(message) => message,
                _ => state
                    .i18n
                    .t("webhooks.payload_url.blocked_or_internal")
                    .unwrap_or("Payload URL cannot be used because it resolves to a blocked or internal IP")
                    .to_string(),
            };
            sqlx::query(
                "UPDATE web_hook_events SET headers = $2, duration = $3, response_headers = $4, \
                        status = -1, updated_at = clock_timestamp() \
                 WHERE id = $1",
            )
            .bind(event_id)
            .bind(&sent)
            .bind(duration)
            .bind(serde_json::to_string(&json!({ "error": error }))?)
            .execute(&mut *conn)
            .await?;
            None
        }
    };
    update_delivery_status(conn, hook, status.map_or(-1, i32::from)).await?;
    Ok(status)
}

/// `@web_hook.update!(active: false)` and `log_web_hook_deactivate`.
async fn deactivate(
    conn: &mut PgConnection,
    state: &AppState,
    hook: &WebHook,
    status: u16,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE web_hooks SET active = FALSE, updated_at = clock_timestamp() \
         WHERE id = $1 AND active",
    )
    .bind(hook.id)
    .execute(&mut *conn)
    .await?;
    let action = crate::staff_action_logs::action_id("web_hook_deactivate")
        .ok_or(crate::Unsupported("an unknown staff action"))?;
    let status = status.to_string();
    // The context is an Array, stored as Ruby writes one out.
    let context = format!(
        "[\"webhook_id: {}\", \"webhook_response_status: {status}\"]",
        hook.id
    );
    let details = state
        .i18n
        .t_with(
            "staff_action_logs.webhook_deactivation_reason",
            &[("status", &status)],
        )
        .unwrap_or_default();
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, context, details, admin_only, \
                                     created_at, updated_at) \
         VALUES ($1, $2, $3, $4, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(action)
    .bind(SYSTEM_USER_ID)
    .bind(context)
    .bind(details)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The stored headers of an event to deliver again (`MultiJson.load`).
pub fn stored_headers(headers: Option<&str>) -> Result<Vec<(String, String)>, AppError> {
    let parsed: Value = serde_json::from_str(headers.unwrap_or("null"))?;
    let Value::Object(map) = parsed else {
        return Err(std::io::Error::other("a web hook event without headers").into());
    };
    Ok(map
        .into_iter()
        .map(|(k, v)| {
            let v = match v {
                Value::String(s) => s,
                other => other.to_string(),
            };
            (k, v)
        })
        .collect())
}

/// The redelivery of a stored event: its headers and body sent again.
pub async fn redeliver(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    hook: &WebHook,
    event: &super::admin::Event,
) -> Result<(), AppError> {
    let headers = stored_headers(event.headers.as_deref())?;
    emit(
        conn,
        state,
        settings,
        hook,
        event.id,
        &headers,
        event.payload.as_deref().unwrap_or(""),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The signature Rails sent for the body in
    /// parity/writes/web_hooks/deliver_success.
    #[test]
    fn signs_like_openssl_hmac() {
        let body = "{\"post\":{\"id\":7,\"raw\":\"héllo ☃\",\"nested\":{\"a\":[1,2]}}}";
        assert_eq!(
            signature("abcdefghijklmnop", body.as_bytes()),
            "59fc0b4fbb498b4ec01e385a54b28b256bf0998107d47bfc0161d6561fb940d0"
        );
    }

    #[test]
    fn hosts_as_uri_gives_them() {
        assert_eq!(
            uri_host("http://fixtures.localhost:8099/hook?x=1").as_deref(),
            Some("fixtures.localhost")
        );
        assert_eq!(uri_host("http://[::1]:80/").as_deref(), Some("[::1]"));
    }
}
