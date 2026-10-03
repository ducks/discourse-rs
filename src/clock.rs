//! The time the port reads as now.
//!
//! Everything that compares against the current time (edit windows,
//! "recent" sitemaps, session expiry) reads it here, so a test replaying a
//! recorded Rails response can pin it to when the recording was made. The
//! database side is pinned separately (tests/common's clock schema).

use std::sync::RwLock;

use chrono::{DateTime, NaiveDateTime, Utc};

static PINNED: RwLock<Option<DateTime<Utc>>> = RwLock::new(None);

/// `Time.zone.now`: the pinned time, else the system clock.
pub fn now() -> DateTime<Utc> {
    match *PINNED.read().unwrap_or_else(|e| e.into_inner()) {
        Some(t) => t,
        None => Utc::now(),
    }
}

/// `now` as the naive UTC timestamps the database columns hold.
pub fn now_naive() -> NaiveDateTime {
    now().naive_utc()
}

/// Pins the clock (None releases it). For tests replaying recordings.
#[doc(hidden)]
pub fn pin(at: Option<DateTime<Utc>>) {
    *PINNED.write().unwrap_or_else(|e| e.into_inner()) = at;
}
