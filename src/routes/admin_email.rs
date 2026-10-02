//! Port of Admin::EmailController#handle_mail: where mail-receiver (or
//! anything else holding the mail) hands Discourse an incoming email,
//! queued for Jobs::ProcessEmail.

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
        return Ok(super::topics::not_found_response(&state, false));
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
