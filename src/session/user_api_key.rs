//! User API keys: the `User-Api-Key` half of
//! Auth::DefaultCurrentUserProvider#current_user (the lookup, the two
//! limiters, `UserApiKey#ensure_allowed!`, the suspended and inactive
//! check and `update_last_used`), and UserApiKeyScope's route matchers.
//!
//! Matchers name `controller#action`s; the port has no such names, so each
//! action a scope names is the method and path pattern Rails routes to it.

use std::sync::LazyLock;

use axum::http::{HeaderMap, Method};
use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgConnection;

use super::current::{SESSION_USER_COLUMNS, SessionUser};
use crate::rate_limiter::{LimitExceeded, RateLimiter};
use crate::{AppError, AppState, Unsupported};

pub const HEADER_USER_API_KEY: &str = "user-api-key";
pub const HEADER_USER_API_CLIENT_ID: &str = "user-api-client-id";

/// What the key resolved to.
pub enum Resolved {
    /// The key's user, and the key's id.
    User(SessionUser),
    /// `Discourse::InvalidAccess`: an unknown, revoked or expired key, a
    /// request no scope allows, or a suspended or inactive user.
    Invalid,
    /// One of the two limiters refused the request.
    Limited(LimitExceeded),
}

/// `ApiKey.hash_key`
pub fn hash_key(key: &str) -> String {
    format!("{:x}", Sha256::digest(key.as_bytes()))
}

/// What a request looks like to a RouteMatcher.
pub struct Request<'a> {
    pub method: &'a Method,
    pub path: &'a str,
    /// The query string, for `request.fullpath` (message_bus).
    pub query: Option<&'a str>,
    /// `request.parameters["id"]`, for the revoke matcher: absent, or the
    /// value as JSON (a string from a query or form, any JSON value from a
    /// JSON body).
    pub id_param: Option<Value>,
}

/// A path Rails routes to an action, with the optional `(.:format)`.
fn route(pattern: &str) -> Regex {
    Regex::new(&format!(r"\A{pattern}(?:\.[^/.]+)?\z")).unwrap()
}

static NOTIFICATIONS_INDEX: LazyLock<Regex> = LazyLock::new(|| route("/notifications"));
static NOTIFICATIONS_TOTALS: LazyLock<Regex> = LazyLock::new(|| route("/notifications/totals"));
static NOTIFICATIONS_MARK_READ: LazyLock<Regex> =
    LazyLock::new(|| route("/notifications/(?:mark-read|read)"));
static SESSION_CURRENT: LazyLock<Regex> = LazyLock::new(|| route("/session/current"));
static TOPIC_TRACKING_STATE: LazyLock<Regex> =
    LazyLock::new(|| route(r"/(?:u|users)/[%\w.\-]+?/topic-tracking-state"));
static USER_STATUS: LazyLock<Regex> = LazyLock::new(|| route("/user-status"));
static REVOKE: LazyLock<Regex> = LazyLock::new(|| route("/user-api-key/revoke"));
static BOOKMARKS: LazyLock<Regex> = LazyLock::new(|| route(r"/(?:u|users)/[%\w.\-]+?/bookmarks"));
static MESSAGE_BUS_POLL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A/message-bus/.*/poll").unwrap());
static BUS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\A/bus/(?:events|poll)\z").unwrap());

impl Request<'_> {
    fn is(&self, method: Method, pattern: &Regex) -> bool {
        *self.method == method && pattern.is_match(self.path)
    }

    /// `actions: "message_bus"`: `request.fullpath =~ %r{\A/message-bus/.*/poll}`,
    /// and the port's own `GET /bus/events` and `/bus/poll`, which carry
    /// what the MessageBus poll carries in Rails.
    fn message_bus_poll(&self) -> bool {
        if *self.method == Method::GET && BUS.is_match(self.path) {
            return true;
        }
        let fullpath = match self.query {
            Some(q) if !q.is_empty() => format!("{}?{q}", self.path),
            _ => self.path.to_string(),
        };
        *self.method == Method::POST && MESSAGE_BUS_POLL.is_match(&fullpath)
    }

    fn bookmarks_ics(&self) -> bool {
        *self.method == Method::GET && BOOKMARKS.is_match(self.path) && self.path.ends_with(".ics")
    }
}

/// `UserApiKeyScope#permits?` for one scope name (UserApiKeyScope::SCOPES;
/// no ported plugin adds scopes, and an unknown name has no matchers).
fn scope_permits(name: &str, r: &Request) -> Result<bool, Unsupported> {
    Ok(match name {
        "read" => *r.method == Method::GET,
        "write" => matches!(
            *r.method,
            Method::GET | Method::POST | Method::PATCH | Method::PUT | Method::DELETE
        ),
        "message_bus" => r.message_bus_poll(),
        "notifications" => {
            r.message_bus_poll()
                || r.is(Method::GET, &NOTIFICATIONS_INDEX)
                || r.is(Method::GET, &NOTIFICATIONS_TOTALS)
                || r.is(Method::PUT, &NOTIFICATIONS_MARK_READ)
        }
        "session_info" => {
            r.is(Method::GET, &SESSION_CURRENT) || r.is(Method::GET, &TOPIC_TRACKING_STATE)
        }
        "user_status" => {
            r.is(Method::GET, &USER_STATUS)
                || r.is(Method::PUT, &USER_STATUS)
                || r.is(Method::DELETE, &USER_STATUS)
        }
        // users#bookmarks in the ics format, with the scope's allowed
        // usernames: the calendar feed is not ported.
        "bookmarks_calendar" => {
            if r.bookmarks_ics() {
                return Err(Unsupported("the bookmarks calendar feed (.ics)"));
            }
            false
        }
        _ => false,
    })
}

/// `REVOKE_MATCHER.with_allowed_param_values({"id" => [nil, id.to_s]})`:
/// POST user_api_keys#revoke with no `id`, or this key's id as a string.
fn revoke_self(key_id: i32, r: &Request) -> bool {
    if !r.is(Method::POST, &REVOKE) {
        return false;
    }
    match &r.id_param {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => *s == key_id.to_string(),
        Some(_) => false,
    }
}

/// Whether the request needs its `id` parameter read for the revoke
/// matcher.
pub fn wants_id_param(method: &Method, path: &str) -> bool {
    *method == Method::POST && REVOKE.is_match(path)
}

/// `UserApiKey#allow?`
pub fn allowed(key_id: i32, scopes: &[String], r: &Request) -> Result<bool, Unsupported> {
    for scope in scopes {
        if scope_permits(scope, r)? {
            return Ok(true);
        }
    }
    Ok(revoke_self(key_id, r))
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
}

/// The lookup, limiters, scope check, user check and `update_last_used`
/// (unless Postgres is read only), in Rails' order.
pub async fn resolve(
    state: &AppState,
    conn: &mut PgConnection,
    key: &str,
    headers: &HeaderMap,
    request: &Request<'_>,
    pg_readonly: bool,
) -> Result<Resolved, AppError> {
    let hash = hash_key(key);
    // UserApiKey.active.joins(:user).where(key_hash:).first
    let found: Option<(i32, i32, Option<i64>)> = sqlx::query_as(
        "SELECT k.id, k.user_id, k.user_api_key_client_id FROM user_api_keys k \
         JOIN users u ON u.id = k.user_id \
         WHERE k.revoked_at IS NULL AND (k.expires_at IS NULL OR k.expires_at > now()) \
           AND k.key_hash = $1 ORDER BY k.id LIMIT 1",
    )
    .bind(&hash)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((key_id, user_id, client_id)) = found else {
        return Ok(Resolved::Invalid);
    };
    let globals = &state.config.globals;
    let per_minute = globals
        .get("max_user_api_reqs_per_minute")
        .map_or(50, crate::ruby::to_i);
    let per_day = globals
        .get("max_user_api_reqs_per_day")
        .map_or(4000, crate::ruby::to_i);
    let enabled = state.config.rate_limits;
    for limiter in [
        RateLimiter::new(None, &format!("user_api_min_{hash}"), per_minute, 60)
            .error_code("user_api_key_limiter_60_secs"),
        RateLimiter::new(None, &format!("user_api_day_{hash}"), per_day, 86_400)
            .error_code("user_api_key_limiter_1_day"),
    ] {
        if let Err(refused) = limiter.performed(&state.pool, enabled).await? {
            return Ok(Resolved::Limited(refused));
        }
    }
    let scopes: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM user_api_key_scopes WHERE user_api_key_id = $1 ORDER BY id",
    )
    .bind(key_id)
    .fetch_all(&mut *conn)
    .await?;
    if !allowed(key_id, &scopes, request)? {
        return Ok(Resolved::Invalid);
    }
    let user: Option<SessionUser> = sqlx::query_as(&format!(
        "SELECT {SESSION_USER_COLUMNS} FROM users WHERE id = $1"
    ))
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(user) = user else {
        return Ok(Resolved::Invalid);
    };
    if user.suspended() || !user.active {
        return Ok(Resolved::Invalid);
    }
    if !pg_readonly {
        update_last_used(
            conn,
            key_id,
            client_id,
            header(headers, HEADER_USER_API_CLIENT_ID),
        )
        .await?;
    }
    Ok(Resolved::User(user))
}

/// `UserApiKey#update_last_used`: `update_columns` (no updated_at), and a
/// client id other than the key's client's becomes a new client with the
/// old one's application name, which the key moves to.
async fn update_last_used(
    conn: &mut PgConnection,
    key_id: i32,
    client_id: Option<i64>,
    header_client_id: Option<&str>,
) -> Result<(), AppError> {
    let current: Option<(String, String)> = match client_id {
        Some(id) => {
            sqlx::query_as(
                "SELECT client_id, application_name FROM user_api_key_clients WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?
        }
        None => None,
    };
    let new_client = match header_client_id {
        Some(wanted) => match &current {
            Some((existing, _)) if existing == wanted => None,
            Some((_, application_name)) => Some((wanted, application_name.clone())),
            // `client.client_id` on a key without a client: NoMethodError.
            None => {
                return Err(
                    Unsupported("a user API client id header for a key without a client").into(),
                );
            }
        },
        None => None,
    };
    match new_client {
        Some((wanted, application_name)) => {
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO user_api_key_clients (client_id, application_name, created_at, updated_at) \
                 VALUES ($1, $2, now(), now()) RETURNING id",
            )
            .bind(wanted)
            .bind(application_name)
            .fetch_one(&mut *conn)
            .await?;
            sqlx::query(
                "UPDATE user_api_keys SET last_used_at = now(), user_api_key_client_id = $2 WHERE id = $1",
            )
            .bind(key_id)
            .bind(id)
            .execute(&mut *conn)
            .await?;
        }
        None => {
            sqlx::query("UPDATE user_api_keys SET last_used_at = now() WHERE id = $1")
                .bind(key_id)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn permits(scope: &str, method: Method, path: &str, query: Option<&str>) -> bool {
        let r = Request {
            method: &method,
            path,
            query,
            id_param: None,
        };
        scope_permits(scope, &r).unwrap()
    }

    #[test]
    fn message_bus_scopes_reach_the_bus() {
        for scope in ["message_bus", "notifications"] {
            assert!(permits(
                scope,
                Method::GET,
                "/bus/events",
                Some("channels=/latest")
            ));
            assert!(permits(scope, Method::GET, "/bus/poll", Some("position=1")));
            assert!(permits(scope, Method::POST, "/message-bus/abc/poll", None));
            assert!(!permits(scope, Method::POST, "/bus/poll", None));
            assert!(!permits(scope, Method::GET, "/bus/other", None));
            assert!(!permits(scope, Method::GET, "/latest.json", None));
        }
        assert!(!permits("session_info", Method::GET, "/bus/events", None));
    }
}
