//! Port of Admin::EmailController: #handle_mail, where mail-receiver (or
//! anything else holding the mail) hands Discourse an incoming email,
//! queued for Jobs::ProcessEmail; #preview_digest and #send_digest, a
//! user's digest since a date shown to the admin or sent elsewhere.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use serde_json::json;

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::session::current::AuthGuardian;
use crate::{AppError, AppState, Unsupported};

/// POST /admin/email/handle_mail(.json)
pub async fn handle_mail(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let pairs: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !csrf_ok(&state, &headers, &pairs, uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    // Admin routes are behind AdminConstraint: anyone else gets no route.
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let encoded = params::string(&p, "email_encoded").filter(|e| !e.is_empty());
    let plain = params::string(&p, "email").filter(|e| !e.is_empty());
    let (raw, deprecated): (Vec<u8>, bool) = match (encoded, plain) {
        (Some(e), _) => match base64::engine::general_purpose::STANDARD.decode(e.as_bytes()) {
            Ok(raw) => (raw, false),
            Err(_) => {
                return Err(Unsupported("handle_mail with invalid base64 (ArgumentError)").into());
            }
        },
        (None, Some(email)) => (email.into_bytes(), true),
        (None, None) => return Ok(super::accounts::param_missing("email_encoded or email")),
    };
    // Not valid UTF-8: reinterpreted as ISO-8859-1.
    let raw = match String::from_utf8(raw) {
        Ok(s) => s,
        Err(e) => e.into_bytes().iter().map(|&b| b as char).collect(),
    };
    let mut conn = state.pool.acquire().await?;
    crate::jobs::enqueue(
        &mut conn,
        "process_email",
        json!({"mail": raw, "retry_on_rate_limit": true, "source": "handle_mail"}),
    )
    .await?;
    let message = if deprecated {
        "warning: the email parameter is deprecated. all POST requests to this route should be sent with a base64 strict encoded email_encoded parameter instead. email has been received and is queued for processing"
    } else {
        "email has been received and is queued for processing"
    };
    Ok((
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )],
        message,
    )
        .into_response())
}

/// `params.require(name)`: the param, present and not blank.
fn required(p: &serde_json::Map<String, serde_json::Value>, name: &str) -> Option<String> {
    params::string(p, name).filter(|v| !crate::ruby::is_blank(v))
}

/// `CGI.unescapeHTML`: the five named entities and numeric references.
fn unescape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let decoded = after.find(';').and_then(|end| {
            let name = &after[..end];
            let c = match name {
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "lt" => Some('<'),
                "gt" => Some('>'),
                _ => {
                    let num = name.strip_prefix('#')?;
                    let code = match num.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => num.parse().ok()?,
                    };
                    char::from_u32(code)
                }
            }?;
            Some((c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The digest's user (`User.find_by_username`) with their address, and
/// the date the admin asked about as the time it means.
async fn digest_target(
    conn: &mut sqlx::PgConnection,
    last_seen_at: &str,
    username: &str,
) -> Result<Option<(i32, String, chrono::NaiveDateTime)>, AppError> {
    let user: Option<(i32, Option<String>)> = sqlx::query_as(
        "SELECT u.id, (SELECT email FROM user_emails e WHERE e.user_id = u.id AND e.primary) \
         FROM users u WHERE u.username_lower = lower($1)",
    )
    .bind(username)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((id, email)) = user else {
        return Ok(None);
    };
    // Rails compares the string with times and hands it to SQL as is.
    let at: chrono::NaiveDateTime = sqlx::query_scalar("SELECT $1::timestamp")
        .bind(last_seen_at)
        .fetch_one(&mut *conn)
        .await
        .map_err(|_| Unsupported("a digest date Postgres cannot read"))?;
    Ok(Some((id, email.unwrap_or_default(), at)))
}

fn sender_ctx<'a>(
    state: &'a AppState,
    host: &'a crate::pretty_text::Host,
    s: &'a crate::site_settings::SiteSettings,
) -> crate::email::sender::Ctx<'a> {
    crate::email::sender::Ctx {
        host,
        settings: s,
        config: &state.config,
        i18n: &state.i18n,
        mailer: &state.mailer,
    }
}

/// GET /admin/email/preview-digest(.json): the digest the user would get
/// since `last_seen_at`, as Email::Renderer renders it (its unsubscribe
/// key made all the same).
pub async fn preview_digest(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: Uri,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let p = params::parse_query(uri.query().unwrap_or(""));
    let Some(last_seen_at) = required(&p, "last_seen_at") else {
        return Ok(super::accounts::param_missing("last_seen_at"));
    };
    let Some(username) = required(&p, "username") else {
        return Ok(super::accounts::param_missing("username"));
    };
    let mut tx = state.pool.begin().await?;
    let s = crate::site_settings::SiteSettings::load(
        &mut tx,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await?;
    let Some((user_id, email, at)) = digest_target(&mut tx, &last_seen_at, &username).await? else {
        return Ok(super::search::invalid_parameters(
            &state,
            "Discourse::InvalidParameters",
        ));
    };
    let host = crate::pretty_text::Host::from_state(&state);
    let ctx = sender_ctx(&state, &host, &s);
    let built = crate::email::digest::build(
        &mut tx,
        &ctx,
        user_id,
        &email,
        &crate::email::digest::Options {
            since: Some((at, last_seen_at)),
            skip_unsubscribe_links: false,
        },
    )
    .await?;
    // Email::Renderer: a message the mailer never built has no parts, so
    // its (empty) text is cooked into the email layout.
    let (html_part, text) = match built {
        Some(built) => (built.html_part, built.message.text),
        None => (
            crate::email::notification::layout(
                "",
                &s.get("default_locale")?.to_s().replacen('_', "-", 1),
                "",
            ),
            String::new(),
        ),
    };
    let html = crate::email::sender::renderer_html(&ctx, &html_part)?;
    tx.commit().await?;
    Ok(axum::Json(json!({
        "html_content": html,
        "text_content": unescape_html(&text),
    }))
    .into_response())
}

/// POST /admin/email/send-digest(.json): that digest, without its
/// unsubscribe links, sent to `email`.
pub async fn send_digest(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let pairs: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !csrf_ok(&state, &headers, &pairs, uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let Some(last_seen_at) = required(&p, "last_seen_at") else {
        return Ok(super::accounts::param_missing("last_seen_at"));
    };
    let Some(username) = required(&p, "username") else {
        return Ok(super::accounts::param_missing("username"));
    };
    // A JSON body without `email` is wrapped into params[:email]
    // (ParamsWrapper, by the controller's name), which passes require.
    let json_body = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("json"));
    let email = required(&p, "email");
    if email.is_none() && !json_body {
        return Ok(super::accounts::param_missing("email"));
    }
    let mut tx = state.pool.begin().await?;
    let s = crate::site_settings::SiteSettings::load(
        &mut tx,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await?;
    let Some((user_id, _, at)) = digest_target(&mut tx, &last_seen_at, &username).await? else {
        return Ok(super::search::invalid_parameters(
            &state,
            "Discourse::InvalidParameters",
        ));
    };
    let host = crate::pretty_text::Host::from_state(&state);
    let ctx = sender_ctx(&state, &host, &s);
    let built = crate::email::digest::build(
        &mut tx,
        &ctx,
        user_id,
        email.as_deref().unwrap_or(""),
        &crate::email::digest::Options {
            since: Some((at, last_seen_at)),
            skip_unsubscribe_links: true,
        },
    )
    .await?;
    // Nothing built sends nothing, and still succeeds.
    if let Some(built) = built {
        if email.is_none() {
            return Err(Unsupported("send-digest to the wrapped JSON params").into());
        }
        crate::email::sender::send(&mut tx, &ctx, built, "digest", None).await?;
    }
    tx.commit().await?;
    Ok(axum::Json(json!({"success": "OK"})).into_response())
}
