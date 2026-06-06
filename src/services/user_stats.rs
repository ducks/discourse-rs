//! User stats service. Pure functions over `&mut PgConnection`.
//!
//! Each user has exactly one row in `user_stats` (created on signup, or
//! lazily by `ensure_for`). Mutating functions update the row in place
//! with a touched `updated_at`.
//!
//! The trust-level evaluator reads these counters in PR2; nothing else
//! should read them in hot paths — production code should read from the
//! authoritative tables (posts, post_likes, ...) instead. The stats
//! table is a denormalization for promotion decisions, not a source of
//! truth.

use chrono::Utc;
use diesel::prelude::*;
use diesel::result::Error as DieselError;

use crate::models::{NewUserStat, UserStat};
use crate::schema::user_stats;

/// Insert the user_stats row for a freshly-created user. Idempotent: if a
/// row already exists it returns the existing one (no error).
pub fn ensure_for(conn: &mut PgConnection, user_id: i32) -> Result<UserStat, DieselError> {
    let new = NewUserStat { user_id };
    let inserted: Option<UserStat> = diesel::insert_into(user_stats::table)
        .values(&new)
        .on_conflict(user_stats::user_id)
        .do_nothing()
        .returning(UserStat::as_returning())
        .get_result(conn)
        .optional()?;

    if let Some(row) = inserted {
        return Ok(row);
    }

    user_stats::table
        .find(user_id)
        .select(UserStat::as_select())
        .first(conn)
}

/// Fetch a user's stats. Returns NotFound if the user has no stats row
/// (shouldn't happen for any user created after this migration ran).
pub fn get(conn: &mut PgConnection, user_id: i32) -> Result<UserStat, DieselError> {
    user_stats::table
        .find(user_id)
        .select(UserStat::as_select())
        .first(conn)
}

/// Increment post_count by one. Used when a user creates a post.
pub fn incr_post_count(conn: &mut PgConnection, user_id: i32) -> Result<(), DieselError> {
    diesel::update(user_stats::table.find(user_id))
        .set((
            user_stats::post_count.eq(user_stats::post_count + 1),
            user_stats::updated_at.eq(Utc::now().naive_utc()),
        ))
        .execute(conn)?;
    Ok(())
}

/// Increment topic_count by one. Used when a user creates a topic.
pub fn incr_topic_count(conn: &mut PgConnection, user_id: i32) -> Result<(), DieselError> {
    diesel::update(user_stats::table.find(user_id))
        .set((
            user_stats::topic_count.eq(user_stats::topic_count + 1),
            user_stats::updated_at.eq(Utc::now().naive_utc()),
        ))
        .execute(conn)?;
    Ok(())
}

/// Decrement post_count by one, floored at zero. Used when a post is
/// hard-deleted (soft-deletes don't touch the counter — the trust-level
/// criteria intentionally count soft-deleted posts the user wrote).
pub fn decr_post_count(conn: &mut PgConnection, user_id: i32) -> Result<(), DieselError> {
    diesel::sql_query(
        "UPDATE user_stats
         SET post_count = GREATEST(post_count - 1, 0),
             updated_at = NOW()
         WHERE user_id = $1",
    )
    .bind::<diesel::sql_types::Integer, _>(user_id)
    .execute(conn)?;
    Ok(())
}
