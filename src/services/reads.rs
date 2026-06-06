//! Read-tracking service. Records the fact that a user entered a topic
//! and how long they spent there, in one all-or-nothing transaction.
//!
//! Why a service: the work is multi-table (topic_views + user_stats),
//! has business rules (idempotency, time cap, only-bump-counter-on-
//! first-view), and we want to call the same logic from both the route
//! and future background work. The route is a thin wrapper.

use chrono::Utc;
use diesel::prelude::*;
use diesel::result::Error as DieselError;

use crate::models::{NewTopicView, TopicView};
use crate::schema::{topic_views, topics};
use crate::services::user_stats;

/// Per-call upper bound on reported seconds. A client claiming 999 gets
/// clamped to this. Matches the spirit of Discourse's per-heartbeat cap
/// and prevents a single inflated POST from skewing trust-level math.
pub const MAX_SECONDS_PER_CALL: i32 = 60;

#[derive(Debug)]
pub enum ReadError {
    TopicNotFound,
    Db(DieselError),
}

impl From<DieselError> for ReadError {
    fn from(e: DieselError) -> Self {
        ReadError::Db(e)
    }
}

#[derive(Debug, PartialEq)]
pub enum ReadOutcome {
    /// First time the user has viewed this topic. user_stats.topics_entered
    /// was bumped.
    NewView,
    /// User has viewed this topic before. last_viewed_at was updated; the
    /// topics_entered counter was NOT bumped (idempotency).
    Revisit,
}

/// Record that `user_id` spent `seconds` viewing `topic_id`. Idempotent:
/// the second call updates `last_viewed_at` but doesn't double-count the
/// topic in `topics_entered`.
///
/// Returns `TopicNotFound` if the topic doesn't exist (or was deleted).
/// Seconds is server-side capped to [0, MAX_SECONDS_PER_CALL]; pass any
/// value, the floor and cap are applied internally.
pub fn record_topic_view(
    conn: &mut PgConnection,
    user_id: i32,
    topic_id: i32,
    seconds: i32,
) -> Result<ReadOutcome, ReadError> {
    conn.transaction::<ReadOutcome, ReadError, _>(|conn| {
        // Reject views on nonexistent topics. Soft-deleted topics aren't a
        // concept in this codebase yet (topics don't have a deleted_at),
        // so a plain existence check is enough.
        let exists: bool = diesel::select(diesel::dsl::exists(
            topics::table.filter(topics::id.eq(topic_id)),
        ))
        .get_result(conn)
        .map_err(ReadError::from)?;
        if !exists {
            return Err(ReadError::TopicNotFound);
        }

        // Upsert the view row. If a row already existed for this
        // (user, topic), we just bump last_viewed_at. `RETURNING id` lets
        // us tell which path we took: an INSERT returns the new row, the
        // ON CONFLICT path doesn't.
        let new = NewTopicView { user_id, topic_id };
        let inserted: Option<TopicView> = diesel::insert_into(topic_views::table)
            .values(&new)
            .on_conflict((topic_views::user_id, topic_views::topic_id))
            .do_nothing()
            .returning(TopicView::as_returning())
            .get_result(conn)
            .optional()
            .map_err(ReadError::from)?;

        let outcome = if inserted.is_some() {
            // First view — bump the counter the trust-level evaluator reads.
            user_stats::incr_topics_entered(conn, user_id).map_err(ReadError::from)?;
            ReadOutcome::NewView
        } else {
            // Revisit — just update last_viewed_at. (We didn't need to
            // SELECT first; a targeted UPDATE is fine.)
            diesel::update(
                topic_views::table
                    .filter(topic_views::user_id.eq(user_id))
                    .filter(topic_views::topic_id.eq(topic_id)),
            )
            .set(topic_views::last_viewed_at.eq(Utc::now().naive_utc()))
            .execute(conn)
            .map_err(ReadError::from)?;
            ReadOutcome::Revisit
        };

        // Cap seconds and add to time_read. Negative or zero values
        // become no-ops (handled inside add_time_read).
        let capped = seconds.clamp(0, MAX_SECONDS_PER_CALL);
        user_stats::add_time_read(conn, user_id, capped).map_err(ReadError::from)?;

        Ok(outcome)
    })
}
