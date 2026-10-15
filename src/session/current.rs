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
}

/// `User.should_update_last_seen?`'s once-a-minute-per-day gate, which
/// Rails keeps in Redis, namespaced by site.
#[derive(Default)]
pub struct LastSeenGate(Mutex<HashMap<(i32, String), Instant>>);

impl Keys {
    pub fn new(secret_key_base: String) -> Keys {
        Keys {
            codec: Codec::new(&secret_key_base),
            secret_key_base,
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
    pub uploaded_avatar_id: Option<i32>,
    pub name: Option<String>,
    pub previous_visit_at: Option<NaiveDateTime>,
    /// Set on the puppet of a live impersonation (`is_impersonating` and
    /// `impersonation_expires_at`): the user an admin acts as.
    #[sqlx(skip)]
    pub impersonation_expires_at: Option<NaiveDateTime>,
}

pub const SESSION_USER_COLUMNS: &str = "users.id, users.username, users.trust_level, users.admin, users.moderator, users.staged, users.active, \
    users.suspended_till, users.last_seen_at, host(users.ip_address) AS ip_address, \
    users.uploaded_avatar_id, users.name, users.previous_visit_at";

impl SessionUser {
    /// The session user columns for a user id.
    pub async fn load(
        conn: &mut sqlx::PgConnection,
        id: i32,
    ) -> Result<Option<SessionUser>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {SESSION_USER_COLUMNS} FROM users WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(conn)
        .await
    }
    pub fn suspended(&self) -> bool {
        self.suspended_till
            .is_some_and(|t| t > crate::clock::now_naive())
    }

    /// `is_impersonating`
    pub fn is_impersonating(&self) -> bool {
        self.impersonation_expires_at.is_some()
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
    let oldest = crate::clock::now().timestamp() - max_age_hours * 3600;
    if issued_at < oldest {
        return None;
    }
    map.get("token")?.as_str().map(str::to_string)
}

/// `COOKIE_ATTEMPTS_PER_MIN`: lookups of tokens that find no user, per
/// `request.ip`.
const COOKIE_ATTEMPTS_PER_MIN: i64 = 10;

/// `current_user`: the token row's user unless suspended or inactive.
/// None when a token that finds no user is over its limit
/// (`Discourse::InvalidAccess` with the cookie deleted). Past the limit,
/// tokens are not looked up at all.
pub async fn resolve(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    headers: &HeaderMap,
    peer: Option<std::net::SocketAddr>,
) -> Result<Option<Incoming>, AppError> {
    let keys = &state.keys;
    let anonymous = |had_token_cookie| Incoming {
        had_token_cookie,
        session: None,
        guardian: crate::guardian::Guardian::anonymous(),
    };
    let Some(raw) = cookie(headers, TOKEN_COOKIE) else {
        return Ok(Some(anonymous(false)));
    };
    let max_age = settings.get("maximum_session_age")?.to_i();
    let Some(unhashed) = find_auth_token(keys, raw, max_age) else {
        if headers.contains_key("discourse-logged-in") {
            crate::request_tracking::mark_bad_token();
        }
        return Ok(Some(anonymous(true)));
    };
    let limiter = crate::rate_limiter::RateLimiter::new(
        None,
        &format!(
            "cookie_auth_{}",
            crate::remote_ip::request_ip(headers, peer)
        ),
        COOKIE_ATTEMPTS_PER_MIN,
        60,
    );
    let enabled = state.config.rate_limits;
    let found = if limiter.can_perform(&state.pool, enabled).await? {
        match token::lookup(conn, &unhashed, &keys.secret_key_base, max_age).await? {
            Some(token) => {
                let user: Option<SessionUser> = sqlx::query_as(&format!(
                    "SELECT {SESSION_USER_COLUMNS} FROM users WHERE id = $1"
                ))
                .bind(token.user_id)
                .fetch_optional(&mut *conn)
                .await?;
                match user {
                    Some(acting) => {
                        let user = impersonated_user(conn, state, &token, &acting)
                            .await?
                            .unwrap_or(acting);
                        Some((token, user))
                    }
                    None => None,
                }
            }
            None => None,
        }
    } else {
        None
    };
    // No user for the token: BAD_TOKEN, and the attempt counted.
    let Some((token, user)) = found else {
        crate::request_tracking::mark_bad_token();
        if limiter.performed(&state.pool, enabled).await?.is_err() {
            return Ok(None);
        }
        return Ok(Some(anonymous(true)));
    };
    let user = Some(user);
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
    Ok(Some(Incoming {
        had_token_cookie: true,
        session,
        guardian,
    }))
}

/// `UserAuthToken#impersonated_user`: the puppet while the impersonation
/// is live and the acting admin may still impersonate them (checked on
/// every request), marked as impersonated.
async fn impersonated_user(
    conn: &mut PgConnection,
    state: &AppState,
    token: &AuthToken,
    acting: &SessionUser,
) -> Result<Option<SessionUser>, AppError> {
    let (Some(puppet_id), Some(expires_at)) =
        (token.impersonated_user_id, token.impersonation_expires_at)
    else {
        return Ok(None);
    };
    if expires_at <= crate::clock::now_naive() {
        return Ok(None);
    }
    let Some(mut puppet) = SessionUser::load(&mut *conn, puppet_id).await? else {
        return Ok(None);
    };
    let development = state.config.rails_env == crate::config::RailsEnv::Development;
    if !crate::impersonation::can_impersonate(
        conn,
        &state.config.globals,
        development,
        acting.id,
        acting.admin,
        puppet.admin,
    )
    .await?
    {
        return Ok(None);
    }
    puppet.impersonation_expires_at = Some(expires_at);
    Ok(Some(puppet))
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
        Some(crate::clock::now() + chrono::Duration::hours(hours))
    } else {
        None
    };
    let map = [
        ("token", Scalar::Str(unhashed_token.to_string())),
        ("user_id", Scalar::Int(i64::from(user.id))),
        ("username", Scalar::Str(user.username.clone())),
        ("trust_level", Scalar::Int(i64::from(user.trust_level))),
        ("issued_at", Scalar::Int(crate::clock::now().timestamp())),
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
    // A key in a parameter (api_parameter_allowed?): of the routes that
    // allow one, the port serves only handle_mail (no RSS or calendar
    // feeds), and reads keys from headers only.
    if path.starts_with("/admin/email/handle_mail")
        && request.uri().query().is_some_and(|q| {
            form_urlencoded::parse(q.as_bytes()).any(|(k, _)| k == "api_key" || k == "user_api_key")
        })
    {
        return Err(crate::Unsupported("API keys in request parameters").into());
    }
    // BAD_TOKEN: a client that thinks it is signed in, without a cookie.
    if cookie(&headers, TOKEN_COOKIE).is_none() && headers.contains_key("discourse-logged-in") {
        crate::request_tracking::mark_bad_token();
    }
    let user_api_key = headers
        .get(super::user_api_key::HEADER_USER_API_KEY)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    if user_api_key.is_some() {
        // Rails looks up the cookie (and an admin key) first and lets the
        // user key overwrite them; the port takes a user key alone.
        if headers.contains_key(super::api_key::HEADER_API_KEY) {
            return Err(crate::Unsupported("a user API key with an admin API key").into());
        }
        if cookie(&headers, TOKEN_COOKIE).is_some() {
            return Err(crate::Unsupported("a user API key with an auth cookie").into());
        }
    }
    // An admin API key (lookup_api_user) or a user API key stands in for
    // the session. An admin route that does not let the request in is a
    // 404, as Rails' AdminConstraint makes it; elsewhere it is invalid
    // access.
    if user_api_key.is_some()
        || request
            .headers()
            .contains_key(super::api_key::HEADER_API_KEY)
    {
        let mut conn = state.pool.acquire().await?;
        let ip = crate::remote_ip::remote_ip(&headers, peer)?;
        // Rails renders the InvalidAccess message as plain text unless the
        // request is JSON or XHR.
        let header_has = |name: header::HeaderName, needle: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.contains(needle))
        };
        let json = path.ends_with(".json")
            || header_has(header::ACCEPT, "application/json")
            || header_has(
                header::HeaderName::from_static("x-requested-with"),
                "XMLHttpRequest",
            );
        let mode = crate::readonly::mode(&mut conn).await?;
        let refused_key = match &user_api_key {
            Some(key) => {
                // The revoke matcher reads the request's `id`.
                let id_param = if super::user_api_key::wants_id_param(&method, &path) {
                    let (params, rebuilt) = request_params(request).await;
                    request = rebuilt;
                    params.get("id").cloned()
                } else {
                    None
                };
                let matched = super::user_api_key::Request {
                    method: &method,
                    path: &path,
                    query: request.uri().query(),
                    id_param,
                };
                let resolved = super::user_api_key::resolve(
                    &state,
                    &mut conn,
                    key,
                    &headers,
                    &matched,
                    mode.postgres,
                )
                .await?;
                match resolved {
                    super::user_api_key::Resolved::User(user) => Err(user),
                    super::user_api_key::Resolved::Invalid => Ok("invalid_access"),
                    super::user_api_key::Resolved::Limited(refused) => {
                        return Ok(refused.into_response(&state.i18n));
                    }
                }
            }
            None => {
                // Granular scopes may restrict the request's parameters.
                let (params, rebuilt) = request_params(request).await;
                request = rebuilt;
                let format = request_format(&path, request.uri().query(), &headers);
                let matched = crate::api_key_scopes::Request {
                    method: &method,
                    path: &path,
                    query: request.uri().query(),
                    params: &params,
                    format: &format,
                };
                match super::api_key::resolve(&mut conn, &headers, &ip, &matched).await? {
                    super::api_key::Resolved::User(user) => Err(user),
                    super::api_key::Resolved::Invalid => Ok("invalid_api_credentials"),
                    super::api_key::Resolved::Refused => Ok("invalid_access"),
                }
            }
        };
        let guardian = match refused_key {
            Err(user) if user_api_key.is_some() => {
                crate::request_tracking::mark_api(true, user.id);
                // should_update_last_seen?: an API request only with
                // Discourse-Present: true.
                if !mode.postgres
                    && headers
                        .get("discourse-present")
                        .is_some_and(|v| v.as_bytes() == b"true")
                {
                    let settings = SiteSettings::load(
                        &mut conn,
                        &state.site_setting_defs,
                        &state.config.globals,
                    )
                    .await?;
                    update_last_seen(&mut conn, &state, &settings, &user, &ip).await?;
                }
                crate::guardian::Guardian::for_user(&mut conn, &user).await?
            }
            Err(user) => {
                // admin_api_key_limiter: one budget for every key on the
                // site (GlobalSetting max_admin_api_reqs_per_minute).
                let limit = state
                    .config
                    .globals
                    .get("max_admin_api_reqs_per_minute")
                    .map_or(60, crate::ruby::to_i);
                let limiter =
                    crate::rate_limiter::RateLimiter::new(None, "admin_api_min", limit, 60)
                        .error_code("admin_api_key_rate_limit");
                if let Err(refused) = limiter
                    .performed(&state.pool, state.config.rate_limits)
                    .await?
                {
                    return Ok(refused.into_response(&state.i18n));
                }
                // Not the per-IP limits for an authenticated admin API
                // request: the tracker's counts given back.
                let counted = request
                    .extensions()
                    .get::<crate::request_tracker::Limiters>()
                    .cloned();
                for l in counted.map(|c| c.0).unwrap_or_default() {
                    l.rollback(&state.pool, true).await?;
                }
                crate::request_tracking::mark_api(false, user.id);
                crate::guardian::Guardian::for_user(&mut conn, &user).await?
            }
            Ok(_) if path.starts_with("/admin/") => {
                return Ok(crate::routes::not_found_response(&state));
            }
            Ok(key) if json => {
                drop(conn);
                return crate::routes::invalid_access_at(&state, key, &method, &path).await;
            }
            Ok(key) => {
                let text = state
                    .i18n
                    .t(key)
                    .unwrap_or("You are not permitted to view the requested resource.");
                return Ok(axum::response::IntoResponse::into_response((
                    axum::http::StatusCode::FORBIDDEN,
                    text.to_string(),
                )));
            }
        };
        let cleared = match (guardian.user_id(), ClearNotifications::from(&headers)) {
            (Some(user_id), Some(clear)) if !mode.readonly => {
                let settings =
                    SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals)
                        .await?;
                clear.run(&state, &mut conn, &settings, user_id).await?
            }
            _ => false,
        };
        drop(conn);
        request.extensions_mut().insert(Incoming {
            had_token_cookie: false,
            session: None,
            guardian,
        });
        let mut response = next.run(request).await;
        if cleared {
            response.headers_mut().append(
                header::SET_COOKIE,
                HeaderValue::from_static(DELETE_CN_COOKIE),
            );
        }
        return Ok(response);
    }
    // A request without the cookie has no session to resolve, rotate or
    // clear, so it never touches the pool here.
    let (incoming, settings, mode) = if cookie(&headers, TOKEN_COOKIE).is_some() {
        let mut conn = state.pool.acquire().await?;
        let settings =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
        let Some(incoming) = resolve(&mut conn, &state, &settings, &headers, peer).await? else {
            // InvalidAccess with delete_cookie: the 403, the cookie gone.
            if !crate::routes::wants_json(request.uri().query(), &headers)
                && !path.ends_with(".json")
            {
                return Err(
                    crate::Unsupported("the invalid access page for a bad auth cookie").into(),
                );
            }
            let mut response = crate::routes::invalid_access(&state);
            response.headers_mut().append(
                header::SET_COOKIE,
                HeaderValue::from_str(&delete_auth_cookie(&settings)?)?,
            );
            return Ok(response);
        };
        // check_readonly_mode, for the filters around the action: no
        // notifications cleared, no session refreshed while read only, no
        // last seen in Postgres failover (can_write?).
        let mode = crate::readonly::mode(&mut conn).await?;
        (incoming, Some(settings), mode)
    } else {
        let anonymous = Incoming {
            had_token_cookie: false,
            session: None,
            guardian: crate::guardian::Guardian::anonymous(),
        };
        (anonymous, None, crate::readonly::Mode::default())
    };
    // A connection only when the request names notifications to clear.
    let cleared = match (
        &settings,
        incoming.guardian.user_id(),
        ClearNotifications::from(&headers),
    ) {
        (Some(settings), Some(user_id), Some(clear)) if !mode.readonly => {
            let mut conn = state.pool.acquire().await?;
            clear.run(&state, &mut conn, settings, user_id).await?
        }
        _ => false,
    };
    // The session user's last seen, as Rails updates it when the current
    // user is looked up: before the action, which then sees what it did
    // (chat's auto-join on a first sight).
    if let (Some(settings), Some(session)) = (&settings, &incoming.session)
        && !mode.postgres
        && !session.user.is_impersonating()
        && should_update_last_seen(&headers, &method)
    {
        let mut conn = state.pool.acquire().await?;
        update_last_seen(
            &mut conn,
            &state,
            settings,
            &session.user,
            &crate::remote_ip::remote_ip(&headers, peer)?,
        )
        .await?;
    }
    request.extensions_mut().insert(incoming.clone());
    let mut response = next.run(request).await;
    if cleared {
        response.headers_mut().append(
            header::SET_COOKIE,
            HeaderValue::from_static(DELETE_CN_COOKIE),
        );
    }

    let Some(settings) = settings else {
        return Ok(response);
    };
    // perform_refresh_session: `unless @readonly_mode`.
    if mode.readonly {
        return Ok(response);
    }
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
    match &incoming.session {
        Some(session) => {
            let mut conn = state.pool.acquire().await?;
            if token::needs_rotation(&session.token) {
                let ip = crate::remote_ip::remote_ip(&headers, peer)?;
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

/// `request.parameters` less the path's: the query string over a JSON or
/// form body (a JSON body's values keeping their type), with the request
/// rebuilt around the body it read. Other bodies (uploads) are not read.
async fn request_params(request: Request) -> (serde_json::Map<String, serde_json::Value>, Request) {
    use serde_json::Value;
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let json = content_type.starts_with("application/json");
    let form = content_type.starts_with("application/x-www-form-urlencoded");
    let query = request.uri().query().map(crate::params::parse_query);
    let (request, mut params) = if json || form {
        let (parts, body) = request.into_parts();
        let bytes = match axum::body::to_bytes(body, 16 << 20).await {
            Ok(bytes) => bytes,
            Err(_) => axum::body::Bytes::new(),
        };
        let params = if json {
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(Value::Object(map)) => map,
                _ => serde_json::Map::new(),
            }
        } else {
            crate::params::parse_query(std::str::from_utf8(&bytes).unwrap_or_default())
        };
        (Request::from_parts(parts, Body::from(bytes)), params)
    } else {
        (request, serde_json::Map::new())
    };
    for (k, v) in query.unwrap_or_default() {
        params.insert(k, v);
    }
    (params, request)
}

/// `request.formats.first.symbol`, as far as scopes ask: the path's
/// extension, else JSON by the format param or Accept header, else html.
fn request_format(path: &str, query: Option<&str>, headers: &HeaderMap) -> String {
    let last = path.rsplit('/').next().unwrap_or("");
    if let Some((_, ext)) = last.rsplit_once('.')
        && !ext.is_empty()
    {
        return ext.to_string();
    }
    if crate::routes::wants_json(query, headers) {
        "json".to_string()
    } else {
        "html".to_string()
    }
}

/// `cookies.delete("cn")`
const DELETE_CN_COOKIE: &str = "cn=; path=/; max-age=0; expires=Thu, 01 Jan 1970 00:00:00 GMT";

/// ApplicationController#clear_notifications, a before_action for every
/// signed-in request: the ids in the Discourse-Clear-Notifications header
/// and the `cn` cookie are marked read and the counts republished, unless
/// the site is read only. Most requests name none, and take no connection.
struct ClearNotifications {
    ids: Vec<i64>,
    /// From the cookie, which the response then deletes.
    from_cookie: bool,
}

impl ClearNotifications {
    fn from(headers: &HeaderMap) -> Option<ClearNotifications> {
        let header_ids = headers
            .get("discourse-clear-notifications")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let cookie_ids = cookie(headers, "cn");
        let ids = match &cookie_ids {
            Some(c) if !header_ids.trim().is_empty() => format!("{header_ids},{c}"),
            Some(c) => c.to_string(),
            None => header_ids.to_string(),
        };
        if ids.trim().is_empty() {
            return None;
        }
        Some(ClearNotifications {
            ids: ids.split(',').map(crate::ruby::to_i).collect(),
            from_cookie: cookie_ids.is_some(),
        })
    }

    /// Marks them read and publishes the user's notification state; true
    /// when the cookie is to be deleted.
    async fn run(
        self,
        state: &AppState,
        conn: &mut PgConnection,
        settings: &SiteSettings,
        user_id: i32,
    ) -> Result<bool, AppError> {
        sqlx::query(
            "UPDATE notifications SET read = TRUE WHERE id = ANY($1) AND user_id = $2 AND NOT read",
        )
        .bind(&self.ids)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
        crate::bus::publish_notifications_state(&state.bus, conn, settings, user_id).await?;
        Ok(self.from_cookie)
    }
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
    state: &AppState,
    settings: &SiteSettings,
    user: &SessionUser,
    ip: &str,
) -> Result<(), AppError> {
    let limit = settings.get("active_user_rate_limit_secs")?.to_i();
    let today = crate::clock::now().format("%Y-%m-%d").to_string();
    let allowed = {
        let mut gate = state
            .site
            .last_seen_gate
            .0
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
        let first_seen: Option<bool> = sqlx::query_scalar(
            "UPDATE users SET last_seen_at = now(), first_seen_at = COALESCE(first_seen_at, now()) WHERE id = $1 \
             RETURNING first_seen_at = last_seen_at",
        )
        .bind(user.id)
        .fetch_optional(&mut *conn)
        .await?
        .flatten();
        // DiscourseEvent :user_seen: chat auto-joins a user seen for the
        // first time.
        if first_seen == Some(true) {
            crate::plugins::chat::auto_join::user(&mut *conn, &state.bus, state, settings, user.id)
                .await?;
        }
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
