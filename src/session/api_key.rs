//! Admin API keys: the `Api-Key` header half of
//! Auth::DefaultCurrentUserProvider (`lookup_api_user`) and
//! ApiKey#request_allowed?.
//!
//! The key's own user, or for an all-users key the one `Api-Username` or
//! `Api-User-Id` names. Granular scopes are matched as ApiKeyScope does
//! (crate::api_key_scopes, against Rails' route table). User API keys
//! are crate::session::user_api_key; keys in query parameters and
//! external ids are not ported. The admin API rate limit is the session
//! layer's.

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

/// `ApiKey#request_allowed?` past the address check: GET session#scopes
/// and about#index always; else no scopes, or one that permits the
/// request.
fn scopes_permit(
    scopes: &[(String, String, Option<String>)],
    request: &crate::api_key_scopes::Request,
) -> Result<bool, Unsupported> {
    // No scopes allow everything (the route table needn't be read).
    if scopes.is_empty() {
        return Ok(true);
    }
    if *request.method == Method::GET
        && crate::api_key_scopes::recognize(request.method, request.path)
            .ok()
            .flatten()
            .is_some_and(|(action, _)| action == "session#scopes" || action == "about#index")
    {
        return Ok(true);
    }
    for (resource, action, allowed) in scopes {
        let Some(mapping) = crate::api_key_scopes::mapping(resource, action) else {
            continue;
        };
        let allowed: Option<serde_json::Map<String, serde_json::Value>> = allowed
            .as_deref()
            .and_then(|a| serde_json::from_str(a).ok());
        if crate::api_key_scopes::matches(mapping, allowed.as_ref(), request)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `lookup_api_user`, then the suspended and inactive check.
pub async fn resolve(
    conn: &mut PgConnection,
    headers: &HeaderMap,
    ip: &str,
    request: &crate::api_key_scopes::Request<'_>,
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
    let scopes: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT resource, action, allowed_parameters::text FROM api_key_scopes \
         WHERE api_key_id = $1 ORDER BY id",
    )
    .bind(key_id)
    .fetch_all(&mut *conn)
    .await?;
    if !scopes_permit(&scopes, request)? {
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
            (None, None) => match header(headers, "api-user-external-id") {
                // SingleSignOnRecord.find_by(external_id:).user
                Some(external_id) => {
                    sqlx::query_as(&format!(
                        "{select} WHERE id = (SELECT user_id FROM single_sign_on_records \
                         WHERE external_id = $1 LIMIT 1)"
                    ))
                    .bind(external_id)
                    .fetch_optional(&mut *conn)
                    .await?
                }
                None => None,
            },
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
