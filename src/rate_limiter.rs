//! `RateLimiter` (lib/rate_limiter.rb): at most `max` actions in a
//! sliding window of `secs`, per key, kept in discourse_rs.rate_limits as
//! Rails keeps them in a Redis list (epoch-second stamps, newest first).
//! A refused action is `LimitExceeded`, rendered as Rails' rescue_from
//! renders it: a 429 with the wait, its words and Retry-After.
//!
//! Limiters write outside the request's transaction (Redis is not rolled
//! back with it): perform on the pool, never on a transaction.

use axum::Json;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sqlx::PgPool;

use crate::i18n::I18n;

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
        }
    }

    /// `global: true`: one key for every site.
    pub fn global(mut self) -> RateLimiter {
        self.key = format!("GLOBAL::{}", self.key);
        self
    }

    /// `aggressive: true`: a refused attempt counts too.
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
        let now = crate::clock::now().timestamp();
        // max <= 0: refused without touching the list; the oldest stamp of
        // an empty list reads as 0.
        if self.max <= 0 {
            let oldest = self.oldest(pool, now).await?.unwrap_or(0);
            return Ok(Err(self.exceeded(self.secs - (now - oldest))));
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

    /// `rollback!`: the newest stamp popped, whoever pushed it.
    pub async fn rollback(&self, pool: &PgPool, enabled: bool) -> Result<(), sqlx::Error> {
        if !enabled {
            return Ok(());
        }
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
        sqlx::query("DELETE FROM discourse_rs.rate_limits WHERE key = $1")
            .bind(&self.key)
            .execute(pool)
            .await?;
        Ok(())
    }
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
