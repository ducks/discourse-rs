//! Admin API keys: the `Api-Key` header half of
//! Auth::DefaultCurrentUserProvider (`lookup_api_user`) and
//! ApiKey#request_allowed?.
//!
//! The key's own user, or for an all-users key the one `Api-Username` or
//! `Api-User-Id` names. Granular scopes are matched for the one route
//! this port serves API clients on purpose, handle_mail (the email
//! resource's receive_emails); a scoped key anywhere else is refused.
//! User API keys, keys in query parameters, external ids and the admin
//! API rate limit are not ported.

use axum::http::{HeaderMap, Method};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;

use super::current::{SESSION_USER_COLUMNS, SessionUser};
use crate::{AppError, Unsupported};

pub const HEADER_API_KEY: &str = "api-key";

/// What the key resolved to.
pub enum Resolved {
    /// The user the request acts as.
    User(SessionUser),
    /// `invalid_api_credentials`: an unknown or revoked key, an address or
    /// scope it does not allow, or no user.
    Invalid,
    /// The user is suspended or inactive (`Discourse::InvalidAccess`).
    Refused,
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
}

/// `ApiKey#request_allowed?`'s scopes: does one permit this request?
fn scopes_permit(
    scopes: &[(String, String)],
    method: &Method,
    path: &str,
) -> Result<bool, Unsupported> {
    if scopes.is_empty() {
        return Ok(true);
    }
    let handle_mail = method == Method::POST
        && matches!(
            path,
            "/admin/email/handle_mail" | "/admin/email/handle_mail.json"
        );
    if !handle_mail {
        return Err(Unsupported("granular API key scopes on this route"));
    }
    // Only email:receive_emails maps to admin/email#handle_mail.
    Ok(scopes
        .iter()
        .any(|(resource, action)| resource == "email" && action == "receive_emails"))
}

/// `lookup_api_user`, then the suspended and inactive check.
pub async fn resolve(
    conn: &mut PgConnection,
    headers: &HeaderMap,
    ip: &str,
    method: &Method,
    path: &str,
) -> Result<Resolved, AppError> {
    let Some(key) = header(headers, HEADER_API_KEY) else {
        return Ok(Resolved::Invalid);
    };
    let hash = format!("{:x}", Sha256::digest(key.as_bytes()));
    let found: Option<(i32, Option<i32>, bool, Option<chrono::NaiveDateTime>)> = sqlx::query_as(
        "SELECT id, user_id, \
                (allowed_ips IS NULL OR cardinality(allowed_ips) = 0 OR $2::inet <<= ANY(allowed_ips)), \
                last_used_at \
         FROM api_keys WHERE revoked_at IS NULL AND key_hash = $1 ORDER BY id LIMIT 1",
    )
    .bind(&hash)
    .bind(ip)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((key_id, key_user_id, ip_allowed, _)) = found else {
        return Ok(Resolved::Invalid);
    };
    if !ip_allowed {
        return Ok(Resolved::Invalid);
    }
    let scopes: Vec<(String, String)> =
        sqlx::query_as("SELECT resource, action FROM api_key_scopes WHERE api_key_id = $1")
            .bind(key_id)
            .fetch_all(&mut *conn)
            .await?;
    if !scopes_permit(&scopes, method, path)? {
        return Ok(Resolved::Invalid);
    }
    let username = header(headers, "api-username");
    let select = format!("SELECT {SESSION_USER_COLUMNS} FROM users");
    let user: Option<SessionUser> = match key_user_id {
        Some(id) => {
            let user: Option<SessionUser> = sqlx::query_as(&format!("{select} WHERE id = $1"))
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?;
            user.filter(|u| username.is_none_or(|n| u.username.to_lowercase() == n.to_lowercase()))
        }
        None => match (username, header(headers, "api-user-id")) {
            (Some(name), _) => {
                sqlx::query_as(&format!("{select} WHERE username_lower = $1"))
                    .bind(name.to_lowercase())
                    .fetch_optional(&mut *conn)
                    .await?
            }
            (None, Some(id)) => {
                sqlx::query_as(&format!("{select} WHERE id = $1"))
                    .bind(crate::ruby::to_i(id) as i32)
                    .fetch_optional(&mut *conn)
                    .await?
            }
            (None, None) => {
                if header(headers, "api-user-external-id").is_some() {
                    return Err(
                        Unsupported("API requests by external id (DiscourseConnect)").into(),
                    );
                }
                None
            }
        },
    };
    let Some(user) = user else {
        return Ok(Resolved::Invalid);
    };
    // update_last_used!: at most once a minute.
    sqlx::query(
        "UPDATE api_keys SET last_used_at = clock_timestamp() \
         WHERE id = $1 AND (last_used_at IS NULL OR last_used_at <= clock_timestamp() - interval '1 minute')",
    )
    .bind(key_id)
    .execute(&mut *conn)
    .await?;
    if user.suspended() || !user.active {
        return Ok(Resolved::Refused);
    }
    Ok(Resolved::User(user))
}
