//! Middleware::AnonymousCache: anonymous GETs of the actions that opt in
//! (`discourse_expires_in 1.minute`, marked here per route with
//! `expires_in_one_minute`) are kept for their duration and served without
//! running the handler. As in Rails, a response is stored only once its
//! key has been asked for `anon_cache_store_threshold` times within the
//! duration (2 by default; 1 stores at once, 0 turns the cache off), only
//! a 200, and without its cookies; responses say what happened in
//! `X-Discourse-Cached` (`skip`, `store`, `true`). Entries are never
//! purged early: they expire.
//!
//! Rails keeps the entries in Redis; discourse-rs runs as one process and
//! keeps them in memory, at most `MAX_BYTES` of bodies. A body is kept as
//! it was sent, compressed when the request accepted it, with the encoding
//! in the key as Rails keys on brotli; a hit compresses nothing. The key
//! leaves out
//! the segments for what the port doesn't vary its pages on (mobile and
//! crawler layouts, old browsers, the anonymous locale, translation);
//! the theme and color scheme cookies stay in it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::AppState;

/// The bodies kept at most, in bytes.
const MAX_BYTES: usize = 64 * 1024 * 1024;
/// A response larger than this is served but not kept.
const MAX_BODY: usize = 4 * 1024 * 1024;

/// `env["ANON_CACHE_DURATION"]`: set on a response by the actions that may
/// be cached.
#[derive(Clone, Copy)]
pub struct CacheFor(pub Duration);

/// `discourse_expires_in 1.minute`, as a route layer.
pub async fn expires_in_one_minute(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .extensions_mut()
        .insert(CacheFor(Duration::from_secs(60)));
    response
}

struct Entry {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
    expires: Instant,
}

#[derive(Default)]
struct Store {
    entries: HashMap<String, Entry>,
    /// `<key>_count`: how often the key was asked for, until when.
    counts: HashMap<String, (u32, Instant)>,
    bytes: usize,
}

impl Store {
    fn sweep(&mut self, now: Instant) {
        let mut freed = 0;
        self.entries.retain(|_, e| {
            let keep = e.expires > now;
            if !keep {
                freed += e.body.len();
            }
            keep
        });
        self.bytes -= freed;
        self.counts.retain(|_, (_, until)| *until > now);
    }
}

/// An app's cache (AppState::anonymous_cache). On in production and test,
/// as config/initializers/099-anon-cache.rb inserts the middleware, unless
/// DISCOURSE_DISABLE_ANON_CACHE is set.
pub struct Cache {
    enabled: bool,
    store: Mutex<Store>,
}

impl Cache {
    pub fn new(config: &crate::config::Config) -> Cache {
        let env_on = matches!(
            config.rails_env,
            crate::config::RailsEnv::Production | crate::config::RailsEnv::Test
        );
        let disabled = std::env::var_os("DISCOURSE_DISABLE_ANON_CACHE").is_some();
        Cache {
            enabled: env_on && !disabled,
            store: Mutex::new(Store::default()),
        }
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

/// GlobalSetting anon_cache_store_threshold.
fn threshold(state: &AppState) -> u32 {
    state
        .config
        .globals
        .get("anon_cache_store_threshold")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(2)
}

/// Helper#cacheable?: a GET without a session cookie, a bypass or an API
/// key, other than /srv/status.
fn cacheable(request: &Request) -> bool {
    let headers = request.headers();
    let query_has = |name: &str| {
        request.uri().query().is_some_and(|q| {
            q.split('&')
                .any(|pair| pair.split('=').next() == Some(name))
        })
    };
    request.method() == Method::GET
        && cookie(headers, crate::session::current::TOKEN_COOKIE).is_none()
        && cookie(headers, "_bypass_cache").is_none()
        && cookie(headers, "authentication_data").is_none()
        && request.uri().path() != "/srv/status"
        && !query_has("api_key")
        && !headers.contains_key("api-key")
        && !headers.contains_key("user-api-key")
}

/// Helper#cache_key
fn key(request: &Request) -> String {
    let headers = request.headers();
    let h = |name: header::HeaderName| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    };
    let xhr = if h(header::HeaderName::from_static("x-requested-with"))
        .eq_ignore_ascii_case("XMLHttpRequest")
    {
        "t"
    } else {
        "f"
    };
    let uri = request
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");
    let mut key = format!(
        "ANON_CACHE_{xhr}_{}_http_{}{uri}",
        h(header::ACCEPT),
        h(header::HOST)
    );
    // key_has_brotli?, and gzip, which the port negotiates too: the stored
    // body is the one sent, compressed for that encoding.
    let encodings = h(header::ACCEPT_ENCODING);
    let encoding = if encodings.contains("br") {
        "br"
    } else if encodings.contains("gzip") {
        "gzip"
    } else {
        ""
    };
    key.push_str(&format!("|b={encoding}"));
    for name in [
        "theme_ids",
        "forced_color_mode",
        "color_scheme_id",
        "dark_scheme_id",
    ] {
        key.push_str(&format!("|{name}={}", cookie(headers, name).unwrap_or("")));
    }
    key
}

fn with_cached_header(mut response: Response, value: &'static str) -> Response {
    response
        .headers_mut()
        .insert("x-discourse-cached", HeaderValue::from_static(value));
    response
}

pub async fn layer(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let cache = &state.anonymous_cache;
    let threshold = threshold(&state);
    if !cache.enabled || threshold == 0 || !cacheable(&request) {
        return next.run(request).await;
    }
    let key = key(&request);
    let now = Instant::now();
    {
        let store = cache.store();
        if let Some(entry) = store.entries.get(&key).filter(|e| e.expires > now) {
            let mut response = (entry.status, entry.body.clone()).into_response();
            *response.headers_mut() = entry.headers.clone();
            return with_cached_header(response, "true");
        }
    }

    let response = next.run(request).await;
    let Some(CacheFor(duration)) = response.extensions().get::<CacheFor>().copied() else {
        return response;
    };
    if response.status() != StatusCode::OK {
        return response;
    }
    // Helper#cache: count the ask, and keep the response from the
    // threshold on.
    {
        let mut store = cache.store();
        let count = store
            .counts
            .entry(format!("{key}_count"))
            .and_modify(|(n, until)| {
                if *until <= now {
                    *n = 0;
                }
                *n += 1;
                *until = now + duration;
            })
            .or_insert((1, now + duration))
            .0;
        if count < threshold {
            drop(store);
            return with_cached_header(response, "skip");
        }
    }
    let (parts, body) = response.into_parts();
    let body = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(body) => body,
        Err(e) => {
            tracing::error!(error = %e, "anonymous cache: reading the response");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let mut headers = parts.headers.clone();
    headers.remove(header::SET_COOKIE);
    headers.insert("x-discourse-cached", HeaderValue::from_static("true"));
    {
        let mut store = cache.store();
        if store.bytes + body.len() > MAX_BYTES {
            store.sweep(now);
        }
        if body.len() <= MAX_BODY && store.bytes + body.len() <= MAX_BYTES {
            store.bytes += body.len();
            if let Some(old) = store.entries.insert(
                key,
                Entry {
                    status: parts.status,
                    headers,
                    body: body.clone(),
                    expires: now + duration,
                },
            ) {
                store.bytes -= old.body.len();
            }
        }
    }
    let response = Response::from_parts(parts, Body::from(body));
    with_cached_header(response, "store")
}
