//! Port of Admin::EmailController: #handle_mail, where mail-receiver (or
//! anything else holding the mail) hands Discourse an incoming email,
//! queued for Jobs::ProcessEmail; #preview_digest and #send_digest, a
//! user's digest since a date shown to the admin or sent elsewhere; and
//! the settings page's actions: #index, #server_settings, #test,
//! #advanced_test and #smtp_should_reject.

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
                &crate::email::notification::email_template(&s)?,
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

/// The admin email pages are Ember's; their routes answer JSON only.
fn refuse_html(uri: &Uri, headers: &HeaderMap) -> Result<(), AppError> {
    if super::wants_json(uri.query(), headers) || uri.path().ends_with(".json") {
        return Ok(());
    }
    Err(Unsupported("the admin email pages (Ember)").into())
}

fn post_pairs(p: &serde_json::Map<String, serde_json::Value>) -> Vec<(String, String)> {
    p.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// GET /admin/email(.json): an empty action, rendered for JSON as no
/// content.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    refuse_html(&uri, &headers)?;
    Ok(axum::http::StatusCode::NO_CONTENT.into_response())
}

/// GET /admin/email/server-settings(.json): ActionMailer's delivery
/// method and its settings, the password left out.
pub async fn server_settings(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let (method, settings) = match &state.mailer {
        // ActionMailer's :test delivery, which has no settings.
        crate::email::Mailer::Memory(_) => ("test", Vec::new()),
        crate::email::Mailer::Sendmail => ("sendmail", vec![("arguments", json!(["-i"]))]),
        crate::email::Mailer::Smtp(_) => ("smtp", smtp_settings(&state.config.globals)),
    };
    let settings: Vec<serde_json::Value> = settings
        .into_iter()
        .filter(|(name, _)| *name != "password")
        .map(|(name, value)| json!({"name": name, "value": value}))
        .collect();
    Ok(axum::Json(json!({"delivery_method": method, "settings": settings})).into_response())
}

/// `GlobalSetting.smtp_settings`, as production hands it to ActionMailer:
/// values coerced as GlobalSetting reads them, unset ones null (the
/// `compact` result is discarded).
fn smtp_settings(
    globals: &crate::config::GlobalSettings,
) -> Vec<(&'static str, serde_json::Value)> {
    use serde_json::Value;
    let coerce = |key: &str, default: Option<&str>| -> Value {
        match globals.get(key).or(default) {
            None => Value::Null,
            Some("true") => json!(true),
            Some("false") => json!(false),
            Some(v) => match v.trim().parse::<i64>() {
                Ok(i) => json!(i),
                Err(_) => json!(v),
            },
        }
    };
    let truthy = |v: &Value| !matches!(v, Value::Null | Value::Bool(false));
    // `to_f` of the setting as read.
    let float = |v: Value| -> Value {
        match v {
            Value::Number(n) => json!(n.as_f64().unwrap_or(0.0)),
            Value::String(s) => json!(s.trim().parse::<f64>().unwrap_or(0.0)),
            _ => json!(0.0),
        }
    };
    let user_name = coerce("smtp_user_name", None);
    let password = coerce("smtp_password", None);
    let force_tls = coerce("smtp_force_tls", Some("false"));
    let start_tls = coerce("smtp_enable_start_tls", Some("true"));
    let mut settings = vec![
        ("address", coerce("smtp_address", None)),
        ("port", coerce("smtp_port", Some("25"))),
        ("domain", coerce("smtp_domain", None)),
        ("user_name", user_name.clone()),
        ("password", password.clone()),
        (
            "enable_starttls_auto",
            if truthy(&force_tls) {
                json!(false)
            } else {
                start_tls
            },
        ),
        (
            "open_timeout",
            float(coerce("smtp_open_timeout", Some("5"))),
        ),
        (
            "read_timeout",
            float(coerce("smtp_read_timeout", Some("30"))),
        ),
    ];
    if truthy(&password) || truthy(&user_name) {
        settings.push((
            "authentication",
            coerce("smtp_authentication", Some("plain")),
        ));
    }
    let verify = coerce("smtp_openssl_verify_mode", None);
    if truthy(&verify) {
        settings.push(("openssl_verify_mode", verify));
    }
    if truthy(&force_tls) {
        settings.push(("tls", json!(true)));
    }
    settings
}

/// GET /admin/email/smtp_should_reject(.json)
pub async fn smtp_should_reject(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    Ok(axum::Json(json!({"reject": false})).into_response())
}

/// POST /admin/email/test(.json): TestMailer's message to the address,
/// sent as `test_message` (even with email disabled).
pub async fn test(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    if !csrf_ok(&state, &headers, &post_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let Some(address) = required(&p, "email_address") else {
        return Ok(super::accounts::param_missing("email_address"));
    };
    let mut tx = state.pool.begin().await?;
    let s = crate::site_settings::SiteSettings::load(
        &mut tx,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await?;
    let host = crate::pretty_text::Host::from_state(&state);
    let ctx = sender_ctx(&state, &host, &s);
    let built =
        crate::email::account::build_template(&ctx, "test_mailer", &address, &[], None).await?;
    // Rails answers any error with a 422 and Ruby's message, which only
    // an SMTP server's refusal would give the same: the port's errors
    // stay errors.
    crate::email::sender::send(&mut tx, &ctx, built, "test_message", None).await?;
    tx.commit().await?;
    let message = state.i18n.t("admin.email.sent_test").unwrap_or("sent!");
    Ok(axum::Json(json!({"sent_test_email_message": message})).into_response())
}

/// GET /admin/email/advanced-test(.json): the same action, for JSON.
pub async fn advanced_test_get(
    state: State<AppState>,
    guardian: AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if guardian.0.is_admin() {
        refuse_html(&uri, &headers)?;
    }
    advanced_test(state, guardian, headers, uri, body).await
}

/// POST /admin/email/advanced-test(.json): how Email::Receiver reads the
/// body of the raw email given, without processing it.
pub async fn advanced_test(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    if !csrf_ok(&state, &headers, &post_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let Some(raw) = required(&p, "email") else {
        return Ok(super::accounts::param_missing("email"));
    };
    let mail = crate::email::incoming::parse(&raw)?;
    let mut conn = state.pool.acquire().await?;
    let s = crate::site_settings::SiteSettings::load(
        &mut conn,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await?;
    // sent_to_mailinglist_mirror? would turn plain text into markdown.
    if s.get("email_in")?.truthy() {
        let mirrors: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE mailinglist_mirror)")
                .fetch_one(&mut *conn)
                .await?;
        if mirrors {
            return Err(Unsupported("advanced-test with mailing list mirror categories").into());
        }
    }
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &s,
    };
    let (text, elided, format) =
        match crate::email::receiver::select_body(&mail, &s, &urls.base_url()?)? {
            Some((t, e, f)) => (json!(t), json!(e), json!(f)),
            None => (
                serde_json::Value::Null,
                serde_json::Value::Null,
                serde_json::Value::Null,
            ),
        };
    Ok(axum::Json(json!({
        "success": "OK",
        "text": text,
        "elided": elided,
        "format": format,
    }))
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smtp_settings_as_production_reads_them() {
        let globals = crate::config::GlobalSettings::from_vars([
            ("smtp_address", "smtp.example.com"),
            ("smtp_port", "587"),
            ("smtp_user_name", "u"),
            ("smtp_password", "secret"),
        ]);
        let settings: Vec<(&str, serde_json::Value)> = smtp_settings(&globals);
        assert_eq!(
            serde_json::to_string(&settings).unwrap(),
            r#"[["address","smtp.example.com"],["port",587],["domain",null],["user_name","u"],["password","secret"],["enable_starttls_auto",true],["open_timeout",5.0],["read_timeout",30.0],["authentication","plain"]]"#
        );
        let globals = crate::config::GlobalSettings::from_vars([
            ("smtp_address", "smtp.example.com"),
            ("smtp_force_tls", "true"),
            ("smtp_openssl_verify_mode", "none"),
        ]);
        let names: Vec<&str> = smtp_settings(&globals).iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            [
                "address",
                "port",
                "domain",
                "user_name",
                "password",
                "enable_starttls_auto",
                "open_timeout",
                "read_timeout",
                "openssl_verify_mode",
                "tls"
            ]
        );
    }
}
