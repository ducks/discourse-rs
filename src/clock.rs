//! The time the port reads as now.
//!
//! Everything that compares against the current time (edit windows,
//! "recent" sitemaps, session expiry) reads it here, so a test replaying a
//! recorded Rails response can pin it to when the recording was made, or
//! shift it there and let it run. The database side is pinned or shifted
//! separately (tests/common's clock schema).

use std::sync::RwLock;

use chrono::{DateTime, Duration, NaiveDateTime, Utc};

static PINNED: RwLock<Option<DateTime<Utc>>> = RwLock::new(None);
static SHIFT: RwLock<Option<Duration>> = RwLock::new(None);

/// `Time.zone.now`: the pinned time, else the system clock, shifted.
pub fn now() -> DateTime<Utc> {
    if let Some(t) = *PINNED.read().unwrap_or_else(|e| e.into_inner()) {
        return t;
    }
    match *SHIFT.read().unwrap_or_else(|e| e.into_inner()) {
        Some(by) => Utc::now() + by,
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

/// Shifts the running clock (None releases it). For tests replaying
/// recordings that need time to pass.
#[doc(hidden)]
pub fn shift(by: Option<Duration>) {
    *SHIFT.write().unwrap_or_else(|e| e.into_inner()) = by;
}
