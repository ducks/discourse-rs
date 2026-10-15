//! `RateLimiter` (lib/rate_limiter.rb): at most `max` actions in a
//! sliding window of `secs`, per key, kept in discourse_rs.rate_limits as
//! Rails keeps them in a Redis list (epoch-second stamps, newest first).
//! A refused action is `LimitExceeded`, rendered as Rails' rescue_from
//! renders it: a 429 with the wait, its words and Retry-After.
//!
//! Limiters write outside the request's transaction (Redis is not rolled
//! back with it): perform on the pool, never on a transaction. Callers
//! often hold a connection of that pool while they do, so the limiter's
//! queries run on a small pool of their own (`own_pool`): with every
//! connection of the main pool held by a request waiting on its limiter,
//! one shared pool would starve.
//!
//! The request tracker's limiters run on every request; they keep their
//! lists in the process instead (`MemoryStore`, `.in_memory`), with the
//! same rules, saved to the table on a graceful shutdown and read back at
//! startup. Their windows are seconds long; a crash loses at most that.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use axum::Json;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use crate::i18n::I18n;

/// Connections of the limiter's own pool, each held for one query.
const OWN_POOL_SIZE: u32 = 4;

/// The limiter's pool for the database `pool` connects to, made lazily
/// from its connect options and kept per database.
fn own_pool(pool: &PgPool) -> PgPool {
    static POOLS: OnceLock<Mutex<HashMap<String, PgPool>>> = OnceLock::new();
    let options = pool.connect_options();
    let key = format!(
        "{}|{:?}|{}|{}",
        options.get_host(),
        options.get_socket(),
        options.get_port(),
        options.get_database().unwrap_or_default()
    );
    let mut pools = POOLS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    pools
        .entry(key)
        .or_insert_with(|| {
            PgPoolOptions::new()
                .max_connections(OWN_POOL_SIZE)
                .acquire_timeout(std::time::Duration::from_secs(5))
                .idle_timeout(std::time::Duration::from_secs(30))
                .connect_lazy_with((*options).clone())
        })
        .clone()
}

/// Lists a tracker keeps in the process: key -> (stamps, newest first;
/// expiry in epoch seconds), as rate_limit_perform keeps them in the table.
#[derive(Clone, Default)]
pub struct MemoryStore(Arc<Mutex<Lists>>);

/// key -> (stamps, newest first; expiry in epoch seconds)
type Lists = HashMap<String, (VecDeque<i64>, i64)>;

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MemoryStore")
    }
}

/// The keys `load` takes back: the request tracker's lists of a site
/// (per user, per crawler).
const SITE_KEYS: &str =
    "key LIKE 'l-rate-limit3:::global_limit_%' OR key LIKE '%_crawler_rate_limit'";
/// The keys `load_global` takes back: the tracker's `GLOBAL::` lists (per
/// address, the health check).
const GLOBAL_KEYS: &str = "key LIKE 'GLOBAL::l-rate-limit3:::global_limit_%'";

impl MemoryStore {
    fn lists(&self) -> std::sync::MutexGuard<'_, Lists> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// rate_limit_perform: None when allowed, else the wait. Expired lists
    /// are swept as the map grows.
    fn perform(&self, key: &str, now: i64, secs: i64, max: i64, aggressive: bool) -> Option<i64> {
        let mut lists = self.lists();
        if lists.len() >= 4096 && lists.len().is_power_of_two() {
            lists.retain(|_, (_, expires)| *expires > now);
        }
        let entry = lists.entry(key.to_string()).or_insert((VecDeque::new(), 0));
        if entry.1 <= now {
            entry.0.clear();
        }
        let s = &mut entry.0;
        let allowed = (s.len() as i64) < max || s.back().is_some_and(|oldest| now - oldest >= secs);
        if allowed || aggressive {
            s.push_front(now);
            s.truncate(max.max(0) as usize);
            entry.1 = now + secs * 2;
        }
        if allowed {
            return None;
        }
        Some(secs - (now - entry.0.back().copied().unwrap_or(0)))
    }

    fn stamps(&self, key: &str, now: i64) -> Vec<i64> {
        match self.lists().get(key) {
            Some((stamps, expires)) if *expires > now => stamps.iter().copied().collect(),
            _ => Vec::new(),
        }
    }

    fn rollback(&self, key: &str) {
        if let Some((stamps, _)) = self.lists().get_mut(key) {
            stamps.pop_front();
        }
    }

    fn clear(&self, key: &str) {
        self.lists().remove(key);
    }

    /// The live lists written to discourse_rs.rate_limits (a graceful
    /// shutdown).
    pub async fn save(&self, pool: &PgPool) -> Result<(), sqlx::Error> {
        let now = crate::clock::now().timestamp();
        let live: Vec<(String, Vec<i64>, i64)> = self
            .lists()
            .iter()
            .filter(|(_, (_, expires))| *expires > now)
            .map(|(k, (s, e))| (k.clone(), s.iter().copied().collect(), *e))
            .collect();
        for (key, stamps, expires) in live {
            sqlx::query(
                "INSERT INTO discourse_rs.rate_limits (key, stamps, expires_at) VALUES ($1, $2, $3) \
                 ON CONFLICT (key) DO UPDATE SET stamps = EXCLUDED.stamps, expires_at = EXCLUDED.expires_at",
            )
            .bind(&key)
            .bind(&stamps)
            .bind(expires)
            .execute(pool)
            .await?;
        }
        Ok(())
    }

    /// A site's tracker lists read back from its table (startup).
    pub async fn load(pool: &PgPool) -> Result<MemoryStore, sqlx::Error> {
        Self::load_keys(pool, SITE_KEYS).await
    }

    /// The process's `GLOBAL::` tracker lists read back from the table
    /// they were saved to (startup).
    pub async fn load_global(pool: &PgPool) -> Result<MemoryStore, sqlx::Error> {
        Self::load_keys(pool, GLOBAL_KEYS).await
    }

    async fn load_keys(pool: &PgPool, keys: &str) -> Result<MemoryStore, sqlx::Error> {
        let rows: Vec<(String, Vec<i64>, i64)> = sqlx::query_as(&format!(
            "SELECT key, stamps, expires_at FROM discourse_rs.rate_limits WHERE expires_at > $1 AND ({keys})"
        ))
        .bind(crate::clock::now().timestamp())
        .fetch_all(pool)
        .await?;
        let store = MemoryStore::default();
        {
            let mut lists = store.lists();
            for (key, stamps, expires) in rows {
                lists.insert(key, (stamps.into(), expires));
            }
        }
        Ok(store)
    }
}

/// `RateLimiter.key_prefix`
const KEY_PREFIX: &str = "l-rate-limit3:";

/// Who a limiter counts for: its user argument (staff are not limited
/// unless the limiter says so), not necessarily the acting user.
#[derive(Clone, Copy, Debug)]
pub struct LimitUser {
    pub id: i32,
    pub staff: bool,
}

/// A refused action: how long to wait, the limiter's type and error code.
#[derive(Debug, Clone, PartialEq)]
pub struct LimitExceeded {
    pub available_in: i64,
    pub type_: String,
    pub error_code: Option<String>,
}

impl LimitExceeded {
    /// `LimitExceeded.time_left`
    pub fn time_left(&self, i18n: &I18n) -> String {
        time_left(i18n, self.available_in)
    }

    /// `description`: the type's own words (rate_limiter.by_type, its
    /// dashes as underscores), else too_many_requests.
    pub fn description(&self, i18n: &I18n) -> String {
        let time_left = self.time_left(i18n);
        if !self.type_.is_empty() {
            let key = format!("rate_limiter.by_type.{}", self.type_.replace('-', "_"));
            if let Some(message) = i18n
                .t_with(&key, &[("time_left", &time_left)])
                .filter(|m| !m.is_empty())
            {
                return message;
            }
        }
        i18n.t_with(
            "rate_limiter.too_many_requests",
            &[("time_left", &time_left)],
        )
        .unwrap_or_default()
    }

    /// ApplicationController's rescue_from: 429, the description, the
    /// wait in extras, Retry-After and the error code header when set.
    pub fn into_response(self, i18n: &I18n) -> Response {
        let body = json!({
            "errors": [self.description(i18n)],
            "error_type": "rate_limit",
            "extras": {"wait_seconds": self.available_in, "time_left": self.time_left(i18n)},
        });
        let mut response = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
        let headers = response.headers_mut();
        if let Ok(v) = HeaderValue::from_str(&self.available_in.to_string()) {
            headers.insert("Retry-After", v);
        }
        if let Some(code) = self.error_code.as_deref()
            && let Ok(v) = HeaderValue::from_str(code)
        {
            headers.insert("Discourse-Rate-Limit-Error-Code", v);
        }
        response
    }
}

impl std::fmt::Display for LimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "rate limit {} exceeded, {}s to wait",
            self.type_, self.available_in
        )
    }
}

/// Raised (as an AppError) where Rails lets it out of a nested call, and
/// rescued with `AppError::rate_limited`.
impl std::error::Error for LimitExceeded {}

/// `LimitExceeded.time_left(available_in)`: whole units, floored.
pub fn time_left(i18n: &I18n, available_in: i64) -> String {
    let count = |key: &str, n: i64| {
        i18n.t_count(
            &format!("rate_limiter.{key}"),
            n,
            &[("count", &n.to_string())],
        )
        .unwrap_or_default()
    };
    if available_in <= 3 {
        i18n.t("rate_limiter.short_time")
            .unwrap_or_default()
            .to_string()
    } else if available_in < 60 {
        count("seconds", available_in)
    } else if available_in < 3600 {
        count("minutes", available_in / 60)
    } else {
        count("hours", available_in / 3600)
    }
}

/// A limiter: `RateLimiter.new(user, type, max, secs, ...)`.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    key: String,
    type_: String,
    max: i64,
    secs: i64,
    aggressive: bool,
    error_code: Option<String>,
    /// Staff user and not apply_limit_to_staff (rate_unlimited? without
    /// the global switch).
    staff_unlimited: bool,
    staff: bool,
    /// The process's lists instead of the table.
    memory: Option<MemoryStore>,
}

impl RateLimiter {
    pub fn new(user: Option<LimitUser>, type_: &str, max: i64, secs: i64) -> RateLimiter {
        // build_key: "l-rate-limit3::<user id>:<type>", an empty id without
        // a user.
        let id = user.map(|u| u.id.to_string()).unwrap_or_default();
        RateLimiter {
            key: format!("{KEY_PREFIX}:{id}:{type_}"),
            type_: type_.to_string(),
            max,
            secs,
            aggressive: false,
            error_code: None,
            staff_unlimited: user.is_some_and(|u| u.staff),
            staff: user.is_some_and(|u| u.staff),
            memory: None,
        }
    }

    /// `global: true`: one key for every site.
    pub fn global(mut self) -> RateLimiter {
        self.key = format!("GLOBAL::{}", self.key);
        self
    }

    /// `aggressive: true`: a refused attempt counts too.
    /// Lists kept in the process (the request tracker's).
    pub fn in_memory(mut self, store: &MemoryStore) -> RateLimiter {
        self.memory = Some(store.clone());
        self
    }

    pub fn aggressive(mut self) -> RateLimiter {
        self.aggressive = true;
        self
    }

    pub fn error_code(mut self, code: &str) -> RateLimiter {
        self.error_code = Some(code.to_string());
        self
    }

    /// `apply_limit_to_staff: true`
    pub fn apply_limit_to_staff(mut self) -> RateLimiter {
        self.staff_unlimited = false;
        self
    }

    /// `staff_limit: {max:, secs:}`: staff limited by their own numbers.
    pub fn staff_limit(mut self, max: i64, secs: i64) -> RateLimiter {
        if self.staff {
            self.max = max;
            self.secs = secs;
            self.staff_unlimited = false;
        }
        self
    }

    /// A subclass's `build_key` adding to the key (EditRateLimiter's date).
    pub fn key_suffix(mut self, suffix: &str) -> RateLimiter {
        self.key.push_str(suffix);
        self
    }

    /// `max`
    pub fn max(&self) -> i64 {
        self.max
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    fn unlimited(&self, enabled: bool) -> bool {
        !enabled || self.staff_unlimited
    }

    fn exceeded(&self, available_in: i64) -> LimitExceeded {
        LimitExceeded {
            available_in,
            type_: self.type_.clone(),
            error_code: self.error_code.clone(),
        }
    }

    /// `performed!`: counted, or refused with the wait. `enabled` is
    /// `!RateLimiter.disabled?` (Config.rate_limits).
    pub async fn performed(
        &self,
        pool: &PgPool,
        enabled: bool,
    ) -> Result<Result<(), LimitExceeded>, sqlx::Error> {
        if self.unlimited(enabled) {
            return Ok(Ok(()));
        }
        let pool = &own_pool(pool);
        let now = crate::clock::now().timestamp();
        // max <= 0: refused without touching the list; the oldest stamp of
        // an empty list reads as 0.
        if self.max <= 0 {
            let oldest = self.oldest(pool, now).await?.unwrap_or(0);
            return Ok(Err(self.exceeded(self.secs - (now - oldest))));
        }
        if let Some(store) = &self.memory {
            return Ok(
                match store.perform(&self.key, now, self.secs, self.max, self.aggressive) {
                    None => Ok(()),
                    Some(wait) => Err(self.exceeded(wait)),
                },
            );
        }
        let wait: Option<i64> =
            sqlx::query_scalar("SELECT discourse_rs.rate_limit_perform($1, $2, $3, $4, $5)")
                .bind(&self.key)
                .bind(now)
                .bind(self.secs)
                .bind(self.max as i32)
                .bind(self.aggressive)
                .fetch_one(pool)
                .await?;
        Ok(match wait {
            None => Ok(()),
            Some(wait) => Err(self.exceeded(wait)),
        })
    }

    /// The stamps, an expired list read as empty.
    async fn stamps(&self, pool: &PgPool, now: i64) -> Result<Vec<i64>, sqlx::Error> {
        if let Some(store) = &self.memory {
            return Ok(store.stamps(&self.key, now));
        }
        let pool = &own_pool(pool);
        let stamps: Option<Vec<i64>> = sqlx::query_scalar(
            "SELECT stamps FROM discourse_rs.rate_limits WHERE key = $1 AND expires_at > $2",
        )
        .bind(&self.key)
        .bind(now)
        .fetch_optional(pool)
        .await?;
        Ok(stamps.unwrap_or_default())
    }

    async fn oldest(&self, pool: &PgPool, now: i64) -> Result<Option<i64>, sqlx::Error> {
        Ok(self.stamps(pool, now).await?.last().copied())
    }

    /// `can_perform?`, read only.
    pub async fn can_perform(&self, pool: &PgPool, enabled: bool) -> Result<bool, sqlx::Error> {
        if self.unlimited(enabled) {
            return Ok(true);
        }
        let now = crate::clock::now().timestamp();
        let stamps = self.stamps(pool, now).await?;
        Ok((stamps.len() as i64) < self.max
            || stamps
                .last()
                .is_some_and(|oldest| now - oldest >= self.secs))
    }

    /// `seconds_to_wait(now)`: the window less the oldest stamp's age.
    pub async fn seconds_to_wait(&self, pool: &PgPool) -> Result<i64, sqlx::Error> {
        let now = crate::clock::now().timestamp();
        let oldest = self.oldest(pool, now).await?.unwrap_or(0);
        Ok(self.secs - (now - oldest))
    }

    /// The refusal `performed!` would raise now.
    pub async fn exceeded_now(&self, pool: &PgPool) -> Result<LimitExceeded, sqlx::Error> {
        Ok(self.exceeded(self.seconds_to_wait(pool).await?))
    }

    /// `rollback!`: the newest stamp popped, whoever pushed it.
    pub async fn rollback(&self, pool: &PgPool, enabled: bool) -> Result<(), sqlx::Error> {
        if !enabled {
            return Ok(());
        }
        if let Some(store) = &self.memory {
            store.rollback(&self.key);
            return Ok(());
        }
        let pool = &own_pool(pool);
        sqlx::query("UPDATE discourse_rs.rate_limits SET stamps = stamps[2:] WHERE key = $1")
            .bind(&self.key)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// `remaining`: max for staff; else max less the stamps in the window.
    pub async fn remaining(&self, pool: &PgPool) -> Result<i64, sqlx::Error> {
        if self.staff {
            return Ok(self.max);
        }
        let now = crate::clock::now().timestamp();
        let recent = self
            .stamps(pool, now)
            .await?
            .into_iter()
            .take(self.max.max(0) as usize + 1)
            .filter(|t| now - t <= self.secs)
            .count() as i64;
        Ok(self.max - recent)
    }

    /// `clear!`
    pub async fn clear(&self, pool: &PgPool) -> Result<(), sqlx::Error> {
        if let Some(store) = &self.memory {
            store.clear(&self.key);
            return Ok(());
        }
        let pool = &own_pool(pool);
        sqlx::query("DELETE FROM discourse_rs.rate_limits WHERE key = $1")
            .bind(&self.key)
            .execute(pool)
            .await?;
        Ok(())
    }
}

/// Limiters performed in order (a controller's `performed!` lines); the
/// first refusal stops the rest, the earlier ones having counted.
pub async fn perform_each(
    pool: &PgPool,
    enabled: bool,
    limiters: impl IntoIterator<Item = RateLimiter>,
) -> Result<Result<(), LimitExceeded>, sqlx::Error> {
    for limiter in limiters {
        if let Err(refused) = limiter.performed(pool, enabled).await? {
            return Ok(Err(refused));
        }
    }
    Ok(Ok(()))
}

/// `rescue RateLimiter::LimitExceeded` ->
/// `render_json_error(I18n.t("rate_limiter.slow_down"))`: a 422.
pub fn slow_down(i18n: &I18n) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"errors": [i18n.t("rate_limiter.slow_down").unwrap_or_default()]})),
    )
        .into_response()
}

/// Expired lists removed (Redis expires them on its own); reads already
/// treat them as empty.
pub async fn cleanup(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM discourse_rs.rate_limits WHERE expires_at <= $1")
        .bind(crate::clock::now().timestamp())
        .execute(pool)
        .await?;
    Ok(())
}
