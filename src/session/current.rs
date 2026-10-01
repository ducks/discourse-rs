//! `Auth::DefaultCurrentUserProvider`: the `_t` cookie resolved to a user
//! on the way in, token rotation and cookie upkeep on the way out, and the
//! last-seen bookkeeping Rails defers past the response.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::Response;
use chrono::NaiveDateTime;
use sqlx::PgConnection;

use super::cookie::{Codec, Scalar};
use super::token::{self, AuthToken};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `TOKEN_COOKIE`
pub const TOKEN_COOKIE: &str = "_t";
/// The Rails session cookie.
pub const SESSION_COOKIE: &str = "_forum_session";

/// What `secret_key_base` gives every request.
pub struct Keys {
    pub secret_key_base: String,
    pub codec: Codec,
    /// `User.should_update_last_seen?`'s once-a-minute-per-day gate,
    /// which Rails keeps in redis.
    last_seen_gate: Mutex<HashMap<(i32, String), Instant>>,
}

impl Keys {
    pub fn new(secret_key_base: String) -> Keys {
        Keys {
            codec: Codec::new(&secret_key_base),
            secret_key_base,
            last_seen_gate: Mutex::new(HashMap::new()),
        }
    }

    /// A random secret for a process started without one: sessions work
    /// but die with the process, and Rails cookies are not readable.
    pub fn ephemeral() -> Keys {
        let mut bytes = [0u8; 64];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
        Keys::new(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }
}

/// The user a request runs as.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SessionUser {
    pub id: i32,
    pub username: String,
    pub trust_level: i32,
    pub admin: bool,
    pub moderator: bool,
    pub staged: bool,
    pub active: bool,
    pub suspended_till: Option<NaiveDateTime>,
    pub last_seen_at: Option<NaiveDateTime>,
    pub ip_address: Option<String>,
}

pub const SESSION_USER_COLUMNS: &str = "users.id, users.username, users.trust_level, users.admin, users.moderator, users.staged, users.active, \
    users.suspended_till, users.last_seen_at, host(users.ip_address) AS ip_address";

impl SessionUser {
    pub fn suspended(&self) -> bool {
        self.suspended_till
            .is_some_and(|t| t > chrono::Utc::now().naive_utc())
    }
}

/// The resolved session, stored in the request extensions. `None` in the
/// extension means an anonymous request.
#[derive(Debug, Clone)]
pub struct Session {
    pub user: SessionUser,
    pub token: AuthToken,
    pub unhashed_token: String,
}

/// What the request carried, for the response side.
#[derive(Debug, Clone)]
pub struct Incoming {
    pub had_token_cookie: bool,
    pub session: Option<Session>,
    /// The session's guardian, built once per request.
    pub guardian: crate::guardian::Guardian,
}

/// `request.cookies[name]`
pub fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get_all(header::COOKIE).iter().find_map(|h| {
        h.to_str().ok()?.split(';').find_map(|pair| {
            let (k, v) = pair.trim().split_once('=')?;
            (k == name).then_some(v)
        })
    })
}

/// `request.remote_ip`
pub fn remote_ip(headers: &HeaderMap, peer: Option<std::net::SocketAddr>) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| peer.map(|p| p.ip().to_string()))
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.chars().take(2000).collect())
}

/// `find_auth_token`: the unhashed token in the cookie, if it is one of
/// ours and not past maximum_session_age.
fn find_auth_token(keys: &Keys, raw: &str, max_age_hours: i64) -> Option<String> {
    // v0 cookies carried the unhashed token itself.
    if raw.len() == 32 && raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(raw.to_string());
    }
    let map = keys.codec.decrypt(TOKEN_COOKIE, raw).ok()?;
    let issued_at = map.get("issued_at")?.as_i64()?;
    let oldest = chrono::Utc::now().timestamp() - max_age_hours * 3600;
    if issued_at < oldest {
        return None;
    }
    map.get("token")?.as_str().map(str::to_string)
}

/// `current_user`: the token row's user unless suspended or inactive.
pub async fn resolve(
    conn: &mut PgConnection,
    keys: &Keys,
    settings: &SiteSettings,
    headers: &HeaderMap,
) -> Result<Incoming, AppError> {
    let Some(raw) = cookie(headers, TOKEN_COOKIE) else {
        return Ok(Incoming {
            had_token_cookie: false,
            session: None,
            guardian: crate::guardian::Guardian::anonymous(),
        });
    };
    let max_age = settings.get("maximum_session_age")?.to_i();
    let Some(unhashed) = find_auth_token(keys, raw, max_age) else {
        return Ok(Incoming {
            had_token_cookie: true,
            session: None,
            guardian: crate::guardian::Guardian::anonymous(),
        });
    };
    let Some(token) = token::lookup(conn, &unhashed, &keys.secret_key_base, max_age).await? else {
        return Ok(Incoming {
            had_token_cookie: true,
            session: None,
            guardian: crate::guardian::Guardian::anonymous(),
        });
    };
    let user: Option<SessionUser> = sqlx::query_as(&format!(
        "SELECT {SESSION_USER_COLUMNS} FROM users WHERE id = $1"
    ))
    .bind(token.user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let user = user.filter(|u| u.active && !u.suspended());
    let guardian = match &user {
        Some(u) => crate::guardian::Guardian::for_user(&mut *conn, u).await?,
        None => crate::guardian::Guardian::anonymous(),
    };
    let session = user.map(|user| Session {
        user,
        token,
        unhashed_token: unhashed,
    });
    Ok(Incoming {
        had_token_cookie: true,
        session,
        guardian,
    })
}

/// The `Set-Cookie` value `set_auth_cookie!` writes.
pub fn auth_cookie(
    keys: &Keys,
    settings: &SiteSettings,
    user: &SessionUser,
    unhashed_token: &str,
) -> Result<String, AppError> {
    let expires = if settings.get("persistent_sessions")?.truthy() {
        let hours = settings.get("maximum_session_age")?.to_i();
        Some(chrono::Utc::now() + chrono::Duration::hours(hours))
    } else {
        None
    };
    let map = [
        ("token", Scalar::Str(unhashed_token.to_string())),
        ("user_id", Scalar::Int(i64::from(user.id))),
        ("username", Scalar::Str(user.username.clone())),
        ("trust_level", Scalar::Int(i64::from(user.trust_level))),
        ("issued_at", Scalar::Int(chrono::Utc::now().timestamp())),
    ];
    let value = keys.codec.encrypt(TOKEN_COOKIE, &map, true, expires);
    let mut cookie = format!("{TOKEN_COOKIE}={value}; path=/");
    if let Some(t) = expires {
        cookie.push_str(&format!(
            "; expires={}",
            t.format("%a, %d %b %Y %H:%M:%S GMT")
        ));
    }
    cookie.push_str("; HttpOnly");
    if settings.get("force_https")?.truthy() {
        cookie.push_str("; Secure");
    }
    let same_site = settings.get("same_site_cookies")?.to_s();
    if same_site != "Disabled" && !same_site.is_empty() {
        cookie.push_str(&format!("; SameSite={same_site}"));
    }
    Ok(cookie)
}

/// `cookies.delete("_t")`
pub fn delete_auth_cookie(settings: &SiteSettings) -> Result<String, AppError> {
    let mut cookie =
        format!("{TOKEN_COOKIE}=; path=/; max-age=0; expires=Thu, 01 Jan 1970 00:00:00 GMT");
    let same_site = settings.get("same_site_cookies")?.to_s();
    if same_site != "Disabled" && !same_site.is_empty() {
        cookie.push_str(&format!("; SameSite={same_site}"));
    }
    Ok(cookie)
}

/// The middleware: resolve the session into the request extensions, run
/// the handler, then `refresh_session` (rotate, clear a dead cookie) and
/// the deferred last-seen update.
pub async fn layer(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let headers = request.headers().clone();
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let peer = request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0);
    let incoming = {
        let mut conn = state.pool.acquire().await?;
        let settings =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
        resolve(&mut conn, &state.keys, &settings, &headers).await?
    };
    request.extensions_mut().insert(incoming.clone());
    let mut response = next.run(request).await;

    // A handler that logged in or out owns the cookie for this response.
    if response.headers().contains_key(header::SET_COOKIE)
        && response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .any(|v| v.as_bytes().starts_with(b"_t="))
    {
        return Ok(response);
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    match &incoming.session {
        Some(session) => {
            if token::needs_rotation(&session.token) {
                let ip = remote_ip(&headers, peer);
                if let Some(unhashed) = token::rotate(
                    &mut conn,
                    &session.token,
                    &state.keys.secret_key_base,
                    user_agent(&headers).as_deref(),
                    &ip,
                    &path,
                )
                .await?
                {
                    let cookie = auth_cookie(&state.keys, &settings, &session.user, &unhashed)?;
                    response
                        .headers_mut()
                        .append(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
                }
            }
            if should_update_last_seen(&headers, &method) {
                update_last_seen(
                    &mut conn,
                    &state.keys,
                    &settings,
                    &session.user,
                    &remote_ip(&headers, peer),
                )
                .await?;
            }
        }
        None if incoming.had_token_cookie => {
            let cookie = delete_auth_cookie(&settings)?;
            response
                .headers_mut()
                .append(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
        }
        None => {}
    }
    Ok(response)
}

/// `should_update_last_seen?`: browser navigations always, XHR only with
/// `Discourse-Present: true`.
fn should_update_last_seen(headers: &HeaderMap, method: &Method) -> bool {
    let _ = method;
    let xhr = headers
        .get("x-requested-with")
        .is_some_and(|v| v.as_bytes() == b"XMLHttpRequest");
    if xhr {
        headers
            .get("discourse-present")
            .is_some_and(|v| v.as_bytes() == b"true")
    } else {
        true
    }
}

/// `User#update_last_seen!` and `User.update_ip_address!`, gated to once
/// per `active_user_rate_limit_secs` per user and day.
async fn update_last_seen(
    conn: &mut PgConnection,
    keys: &Keys,
    settings: &SiteSettings,
    user: &SessionUser,
    ip: &str,
) -> Result<(), AppError> {
    let limit = settings.get("active_user_rate_limit_secs")?.to_i();
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let allowed = {
        let mut gate = keys
            .last_seen_gate
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let key = (user.id, today);
        match gate.get(&key) {
            Some(at) if limit > 0 && at.elapsed() < Duration::from_secs(limit as u64) => false,
            _ => {
                gate.insert(key, Instant::now());
                true
            }
        }
    };
    if allowed {
        // update_visit_record!: one user_visits row per day, counted once.
        let inserted = sqlx::query(
            "INSERT INTO user_visits (user_id, visited_at, posts_read, mobile, time_read) \
             SELECT $1, now()::date, 0, false, 0 \
             WHERE NOT EXISTS (SELECT 1 FROM user_visits WHERE user_id = $1 AND visited_at = now()::date)",
        )
        .bind(user.id)
        .execute(&mut *conn)
        .await?;
        if inserted.rows_affected() == 1 {
            sqlx::query("UPDATE user_stats SET days_visited = days_visited + 1 WHERE user_id = $1")
                .bind(user.id)
                .execute(&mut *conn)
                .await?;
        }
        let timeout = settings.get("previous_visit_timeout_hours")?.to_i();
        sqlx::query(
            "UPDATE users SET previous_visit_at = last_seen_at \
             WHERE id = $1 AND last_seen_at IS NOT NULL AND last_seen_at < now() - ($2 * interval '1 hour')",
        )
        .bind(user.id)
        .bind(timeout as f64)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "UPDATE users SET last_seen_at = now(), first_seen_at = COALESCE(first_seen_at, now()) WHERE id = $1",
        )
        .bind(user.id)
        .execute(&mut *conn)
        .await?;
    }
    if user.ip_address.as_deref() != Some(ip) && !ip.is_empty() {
        sqlx::query("UPDATE users SET ip_address = $2::inet WHERE id = $1")
            .bind(user.id)
            .bind(ip)
            .execute(&mut *conn)
            .await?;
        if settings.get("keep_old_ip_address_count")?.to_i() > 0 {
            return Err(crate::Unsupported("keep_old_ip_address_count (IP history)").into());
        }
    }
    Ok(())
}

/// An empty response body helper for the routes.
pub fn empty() -> Body {
    Body::empty()
}

/// The guardian for a request: the session's user, else anonymous.
/// Built from the `Incoming` the session layer stored.
pub struct AuthGuardian(pub crate::guardian::Guardian);

impl axum::extract::FromRequestParts<AppState> for AuthGuardian {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(AuthGuardian(
            parts
                .extensions
                .get::<Incoming>()
                .map(|i| i.guardian.clone())
                .unwrap_or_else(crate::guardian::Guardian::anonymous),
        ))
    }
}
