//! Middleware::RequestTracker's limits: every request counted per
//! address (or per signed-in user from trust level 1) in 10 and 60 second
//! windows, asset requests in their own budget, and crawlers that ask too
//! often slowed down. A refusal is a plain text 429 before the app runs.
//!
//! Rails only loads the tracker outside development, and skips private and
//! loopback addresses, so none of this can be recorded from the dev
//! reference: tests/request_tracker.rs pins it to lib/middleware.
//! The message bus is mounted ahead of the tracker in Rails; /message-bus
//! and the port's /bus stay outside it here too.

use std::net::IpAddr;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::rate_limiter::RateLimiter;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `UserAuthToken::ROTATE_TIME_MINS`
const ROTATE_TIME_MINS: i64 = 10;

/// The tracker's 10 and 60 second limiters for this request, which an
/// admin API key's request gives back (the session layer rolls them back).
#[derive(Clone)]
pub struct Limiters(pub Vec<RateLimiter>);

/// GlobalSetting's request limits (discourse_defaults.conf).
struct Limits {
    warn: bool,
    block: bool,
    per_10_seconds: i64,
    per_minute: i64,
    assets_per_10_seconds: i64,
    on_private: bool,
    skip_trust_level: i64,
}

impl Limits {
    fn from_state(state: &AppState) -> Limits {
        let globals = &state.config.globals;
        let int = |key: &str, default: i64| globals.get(key).map_or(default, crate::ruby::to_i);
        let mode = globals.get("max_reqs_per_ip_mode").unwrap_or("block");
        Limits {
            warn: matches!(mode, "warn" | "warn+block"),
            block: matches!(mode, "block" | "warn+block"),
            per_10_seconds: int("max_reqs_per_ip_per_10_seconds", 50),
            per_minute: int("max_reqs_per_ip_per_minute", 200),
            assets_per_10_seconds: int("max_asset_reqs_per_ip_per_10_seconds", 200),
            on_private: globals
                .get("max_reqs_rate_limit_on_private")
                .is_some_and(|v| v == "true"),
            skip_trust_level: int("skip_per_ip_rate_limit_trust_level", 1),
        }
    }
}

/// `is_private_ip?`: IPAddr#private? or #loopback?; unparsable is not.
fn private_ip(ip: &str) -> bool {
    match ip.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_private() || v4.is_loopback(),
        Ok(IpAddr::V6(v6)) => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00,
        Err(_) => false,
    }
}

/// `IPAddr.new(entry).include?(ip)` for an address or a CIDR range.
fn cidr_includes(entry: &str, ip: &IpAddr) -> bool {
    let (net, bits) = match entry.split_once('/') {
        Some((net, bits)) => (net, bits.parse::<u32>().ok()),
        None => (entry, None),
    };
    match (net.parse::<IpAddr>(), ip) {
        (Ok(IpAddr::V4(net)), IpAddr::V4(ip)) => {
            let bits = bits.unwrap_or(32).min(32);
            let mask = if bits == 0 {
                0
            } else {
                u32::MAX << (32 - bits)
            };
            u32::from(net) & mask == u32::from(*ip) & mask
        }
        (Ok(IpAddr::V6(net)), IpAddr::V6(ip)) => {
            let bits = bits.unwrap_or(128).min(128);
            let mask = if bits == 0 {
                0
            } else {
                u128::MAX << (128 - bits)
            };
            u128::from(net) & mask == u128::from(*ip) & mask
        }
        _ => false,
    }
}

/// `STATIC_IP_SKIPPER`: DISCOURSE_MAX_REQS_PER_IP_EXCEPTIONS, addresses and
/// ranges split on whitespace.
fn skipped(state: &AppState, ip: &str) -> bool {
    let Some(exceptions) = state.config.globals.get("max_reqs_per_ip_exceptions") else {
        return false;
    };
    let Ok(ip) = ip.parse::<IpAddr>() else {
        return false;
    };
    exceptions.split_whitespace().any(|e| cidr_includes(e, &ip))
}

/// What the tracker reads of the auth cookie (`find_auth_cookie`): only
/// one issued within the rotation time and a minute.
struct Cookie {
    user_id: Option<i64>,
    trust_level: Option<i64>,
    username: Option<String>,
}

fn auth_cookie(state: &AppState, request: &Request) -> Option<Cookie> {
    let raw =
        crate::session::current::cookie(request.headers(), crate::session::current::TOKEN_COOKIE)?;
    let map = state
        .keys
        .codec
        .decrypt(crate::session::current::TOKEN_COOKIE, raw)
        .ok()?;
    let issued_at = map.get("issued_at")?.as_i64()?;
    if issued_at < crate::clock::now().timestamp() - (ROTATE_TIME_MINS + 1) * 60 {
        return None;
    }
    Some(Cookie {
        user_id: map.get("user_id").and_then(|v| v.as_i64()),
        trust_level: map.get("trust_level").and_then(|v| v.as_i64()),
        username: map
            .get("username")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    })
}

/// Requests DISCOURSE_IS_ASSET_PATH marks: what uploads#show, the avatar
/// and stylesheet actions, the static assets and the like serve.
fn asset_path(path: &str) -> bool {
    [
        "/assets/",
        "/fonts/",
        "/images/",
        "/uploads/",
        "/user_avatar/",
        "/letter_avatar/",
        "/letter_avatar_proxy/",
        "/stylesheets/",
        "/theme-javascripts/",
        "/extra-locales/",
        "/svg-sprite/",
        "/highlight-js/",
        "/secure-uploads/",
        "/secure-media-uploads/",
        "/favicon/proxied",
        "/service-worker",
    ]
    .iter()
    .any(|p| path.starts_with(p))
}

/// The rate limiter stack's first active one: (key, error code
/// identifier, global).
fn active_limiter(
    limits: &Limits,
    cookie: Option<&Cookie>,
    path: &str,
    ip: &str,
    hostname: &str,
) -> (String, &'static str, bool) {
    if let Some(Cookie {
        user_id: Some(id),
        trust_level: Some(tl),
        ..
    }) = cookie
        && *tl >= limits.skip_trust_level
    {
        return (format!("user/{id}"), "user", false);
    }
    if path == "/srv/status" {
        return (
            format!("health_check/{ip}/{hostname}"),
            "health_check",
            true,
        );
    }
    (format!("ip/{ip}"), "ip", true)
}

fn limiter(
    store: &crate::rate_limiter::MemoryStore,
    key: String,
    max: i64,
    secs: i64,
    code: String,
    global: bool,
) -> RateLimiter {
    let limiter = RateLimiter::new(None, &key, max, secs)
        .error_code(&code)
        .in_memory(store);
    if global { limiter.global() } else { limiter }
}

/// The plain text 429 the tracker answers before the app.
fn blocked(available_in: i64, error_code: &str, username: Option<&str>) -> Response {
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        format!(
            "Slow down, you're making too many requests.\nPlease retry again in {available_in} seconds.\nError code: {error_code}.\n"
        ),
    )
        .into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    if let Ok(v) = HeaderValue::from_str(&available_in.to_string()) {
        headers.insert("Retry-After", v);
    }
    if let Ok(v) = HeaderValue::from_str(error_code) {
        headers.insert("Discourse-Rate-Limit-Error-Code", v);
    }
    if let Some(v) = username.and_then(|u| HeaderValue::from_str(u).ok()) {
        headers.insert("X-Discourse-Username", v);
    }
    response
}

/// `check_crawler_limits` for a request without the auth cookie: a crawler
/// named on slow_down_crawler_user_agents, once per slow_down_crawler_rate
/// seconds.
async fn crawler_limit(
    state: &AppState,
    user_agent: Option<String>,
) -> Result<Option<Response>, AppError> {
    let Some(user_agent) = user_agent.filter(|u| !u.trim().is_empty()) else {
        return Ok(None);
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    drop(conn);
    let agents = settings.get("slow_down_crawler_user_agents")?.to_s();
    if agents.trim().is_empty() {
        return Ok(None);
    }
    if !crate::crawler_detection::crawler(&settings, Some(&user_agent), None)? {
        return Ok(None);
    }
    let agents = agents.to_lowercase();
    let Some(crawler) = agents.split('|').find(|c| user_agent.contains(*c)) else {
        return Ok(None);
    };
    let key = format!("{crawler}_crawler_rate_limit");
    let limiter = RateLimiter::new(
        None,
        &key,
        1,
        settings.get("slow_down_crawler_rate")?.to_i(),
    )
    .error_code(&key)
    .in_memory(&state.rate_limits);
    match limiter.performed(&state.pool, true).await? {
        Ok(()) => Ok(None),
        Err(refused) => {
            let mut response = (
                StatusCode::TOO_MANY_REQUESTS,
                format!("Too many crawling requests. Error code: {key}."),
            )
                .into_response();
            let headers = response.headers_mut();
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
            if let Ok(v) = HeaderValue::from_str(&refused.available_in.to_string()) {
                headers.insert("Retry-After", v);
            }
            if let Ok(v) = HeaderValue::from_str(&key) {
                headers.insert("Discourse-Rate-Limit-Error-Code", v);
            }
            Ok(Some(response))
        }
    }
}

/// The tracker as a layer, outside everything but tracing.
pub async fn layer(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let path = request.uri().path().to_string();
    // RateLimiter.disabled?, and the bus mounted ahead of the tracker.
    if !state.config.rate_limits || path.starts_with("/message-bus/") || path.starts_with("/bus/") {
        return Ok(next.run(request).await);
    }
    let limits = Limits::from_state(&state);
    let peer = request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0);
    let ip = crate::remote_ip::request_ip(request.headers(), peer);
    let cookie = auth_cookie(&state, &request);

    // rate_limit(request, cookie)
    let mut counted: Option<(RateLimiter, RateLimiter, RateLimiter)> = None;
    if (limits.warn || limits.block)
        && (limits.on_private || !private_ip(&ip))
        && !skipped(&state, &ip)
    {
        let hostname = std::env::var("HOSTNAME").unwrap_or_default();
        let (key, code, global) = active_limiter(&limits, cookie.as_ref(), &path, &ip, &hostname);
        let limiter10 = limiter(
            &state.rate_limits,
            format!("global_limit_10_{key}"),
            limits.per_10_seconds,
            10,
            format!("{code}_10_secs_limit"),
            global,
        )
        .aggressive();
        let limiter60 = limiter(
            &state.rate_limits,
            format!("global_limit_60_{key}"),
            limits.per_minute,
            60,
            format!("{code}_60_secs_limit"),
            global,
        )
        .aggressive();
        let assets10 = limiter(
            &state.rate_limits,
            format!("global_limit_10_assets_{key}"),
            limits.assets_per_10_seconds,
            10,
            format!("{code}_assets_10_secs_limit"),
            global,
        );
        let username = cookie.as_ref().and_then(|c| c.username.as_deref());
        if !assets10.can_perform(&state.pool, true).await? {
            if limits.warn {
                tracing::warn!("Global asset rate limit exceeded: {key}: 10 second rate limit");
            }
            if limits.block {
                let refused = assets10.exceeded_now(&state.pool).await?;
                let code = refused.error_code.unwrap_or_default();
                return Ok(blocked(refused.available_in, &code, username));
            }
        }
        for (secs, l) in [(10, &limiter10), (60, &limiter60)] {
            if let Err(refused) = l.performed(&state.pool, true).await? {
                if limits.warn {
                    tracing::warn!("Global rate limit exceeded: {key}: {secs} second rate limit");
                }
                if limits.block {
                    let code = refused.error_code.unwrap_or_default();
                    return Ok(blocked(refused.available_in, &code, username));
                }
                break;
            }
        }
        request
            .extensions_mut()
            .insert(Limiters(vec![limiter10.clone(), limiter60.clone()]));
        counted = Some((limiter10, limiter60, assets10));
    }

    if cookie.is_none() {
        let user_agent = request
            .headers()
            .get(header::USER_AGENT)
            .map(|v| String::from_utf8_lossy(v.as_bytes()).to_lowercase());
        if let Some(refused) = crawler_limit(&state, user_agent).await? {
            return Ok(refused);
        }
    }

    let response: Response<Body> = next.run(request).await;
    // ensure: an asset request counts against the asset budget only.
    if let Some((limiter10, limiter60, assets10)) = counted
        && asset_path(&path)
    {
        limiter10.rollback(&state.pool, true).await?;
        limiter60.rollback(&state.pool, true).await?;
        let _ = assets10.performed(&state.pool, true).await?;
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_addresses_and_ranges() {
        assert!(private_ip("127.0.0.1") && private_ip("10.1.2.3") && private_ip("::1"));
        assert!(private_ip("fd00::1") && !private_ip("8.8.8.8") && !private_ip("junk"));
        let ip: IpAddr = "203.0.113.9".parse().unwrap();
        assert!(cidr_includes("203.0.113.0/24", &ip));
        assert!(cidr_includes("203.0.113.9", &ip));
        assert!(!cidr_includes("203.0.114.0/24", &ip));
    }
}
