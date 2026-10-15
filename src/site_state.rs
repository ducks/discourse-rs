//! What a site keeps in the process between requests and jobs: the state
//! Rails keeps in Redis under the site's namespace (or in a cache keyed by
//! `current_db`). Each site's `AppState` holds its own, so two sites in
//! one process never see each other's.

use crate::highlight_js::VersionCache;
use crate::jobs::periodical::Periodical;
use crate::request_tracking::{Counts, PageviewQueue};
use crate::session::current::LastSeenGate;

#[derive(Default)]
pub struct SiteState {
    /// CachedCounting's counts waiting for the minute's flush.
    pub counts: Counts,
    /// BrowserPageviewEvent's queue while Postgres is read only.
    pub pageview_queue: PageviewQueue,
    /// Jobs::PeriodicalUpdates' run count and category batch start.
    pub periodical: Periodical,
    /// HighlightJs.version's last digest.
    pub highlight_js: VersionCache,
    /// `User.should_update_last_seen?`'s gate.
    pub last_seen_gate: LastSeenGate,
}
