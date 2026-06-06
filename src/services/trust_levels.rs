//! Trust-level evaluator.
//!
//! `evaluate(conn, user_id)` is the entry point. It reads the user's current
//! trust level and their stats, determines the highest level they qualify
//! for under the v1 criteria, and updates the user row if the target level
//! is higher than their current level.
//!
//! v1 scope (subject to expansion as later PRs fill in the user_stats
//! columns):
//! - TL0 -> TL1: post_count >= TL1_MIN_POSTS (default 3)
//! - TL1 -> TL2: not implemented yet (waiting on time_read / days_visited
//!   instrumentation in PR3+).
//!
//! v1 does NOT demote. Once promoted, the user stays promoted. Manual
//! demotion via the existing admin update_user route still works.

use diesel::prelude::*;
use diesel::result::Error as DieselError;

use crate::guardian::TRUST_LEVEL_BASIC;
use crate::schema::users;
use crate::services::user_stats;

/// Posts required to promote from TL0 to TL1. Discourse default is 3.
pub const TL1_MIN_POSTS: i32 = 3;

#[derive(Debug, PartialEq)]
pub struct PromotionOutcome {
    pub previous: i32,
    pub current: i32,
}

impl PromotionOutcome {
    pub fn changed(&self) -> bool {
        self.previous != self.current
    }
}

/// Evaluate `user_id` against the trust-level criteria and apply any
/// promotion. Idempotent — safe to call repeatedly (no-ops if already at
/// target level).
pub fn evaluate(conn: &mut PgConnection, user_id: i32) -> Result<PromotionOutcome, DieselError> {
    let current_tl: i32 = users::table
        .find(user_id)
        .select(users::trust_level)
        .first(conn)?;

    let stats = user_stats::get(conn, user_id)?;
    let target = target_trust_level(current_tl, &stats);

    if target == current_tl {
        return Ok(PromotionOutcome {
            previous: current_tl,
            current: current_tl,
        });
    }

    diesel::update(users::table.find(user_id))
        .set(users::trust_level.eq(target))
        .execute(conn)?;

    Ok(PromotionOutcome {
        previous: current_tl,
        current: target,
    })
}

/// Pure decision function: given a user's current TL and stats, return
/// the level they should be at. Never returns a level *below* the
/// current one (no demotion in v1).
fn target_trust_level(current: i32, stats: &crate::models::UserStat) -> i32 {
    let mut target = current;
    // TL0 -> TL1: enough posts written.
    if target < TRUST_LEVEL_BASIC && stats.post_count >= TL1_MIN_POSTS {
        target = TRUST_LEVEL_BASIC;
    }
    // TL1 -> TL2 and beyond: not implemented yet (needs read tracking).
    // When added, follow the same shape: gate on `target < NEXT_LEVEL`
    // and check the criteria.
    target.max(current).min(crate::guardian::TRUST_LEVEL_LEADER)
}

// The .min(TRUST_LEVEL_LEADER) cap above belt-and-suspenders against an
// auto-promotion ever pushing someone past TL4 (which is manual-only in
// Discourse). The cap kicks in only if a future revision of this code
// accidentally targets a higher level.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::guardian::TRUST_LEVEL_NEW_USER;
    use crate::models::UserStat;
    use chrono::Utc;

    fn stats_with(post_count: i32) -> UserStat {
        UserStat {
            user_id: 1,
            post_count,
            topic_count: 0,
            time_read: 0,
            posts_read_count: 0,
            topics_entered: 0,
            days_visited: 0,
            created_at: Utc::now().naive_utc(),
            updated_at: Utc::now().naive_utc(),
        }
    }

    #[test]
    fn tl0_with_few_posts_stays_tl0() {
        assert_eq!(target_trust_level(TRUST_LEVEL_NEW_USER, &stats_with(0)), 0);
        assert_eq!(target_trust_level(TRUST_LEVEL_NEW_USER, &stats_with(2)), 0);
    }

    #[test]
    fn tl0_at_threshold_promotes() {
        assert_eq!(
            target_trust_level(TRUST_LEVEL_NEW_USER, &stats_with(TL1_MIN_POSTS)),
            TRUST_LEVEL_BASIC
        );
    }

    #[test]
    fn already_at_target_no_change() {
        assert_eq!(
            target_trust_level(TRUST_LEVEL_BASIC, &stats_with(TL1_MIN_POSTS)),
            TRUST_LEVEL_BASIC
        );
    }

    #[test]
    fn never_demotes() {
        // User is already TL2 but only has 2 posts (somehow); we don't drop
        // them back to TL0.
        assert_eq!(target_trust_level(2, &stats_with(0)), 2);
    }
}
