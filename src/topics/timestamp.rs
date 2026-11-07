//! TopicTimestampChanger (TopicsController#change_timestamps, PUT
//! /t/:id/change-timestamp): the first post moved to the time given, the
//! other posts by the same difference (none past now), the bump date
//! reset, the topic's dates, and the staff action log.
//!
//! Each post is saved with validations as `post.update` saves it: one the
//! validators refuse (a small action's empty raw, say) keeps its dates.

use chrono::{DateTime, NaiveDateTime};
use sqlx::PgConnection;

use crate::posting::validate::{self, PostInput};
use crate::posting::{Ctx, topic_save};
use crate::{AppError, Unsupported};

/// `UserHistory.actions[:topic_timestamps_changed]`
const TOPIC_TIMESTAMPS_CHANGED: i32 = 68;

/// How a change ends when it isn't a server error.
pub enum Outcome {
    Done,
    /// `Topic.with_deleted.find` found nothing.
    NotFound,
    /// InvalidTimestampError (a time past now): failed_json, 422.
    Invalid,
}

/// Microseconds since the epoch, as a timestamp column holds them.
fn micros(t: NaiveDateTime) -> i64 {
    t.and_utc().timestamp_micros()
}

/// `Time.at(f)` stored in a timestamp column: the float's exact binary
/// value, cut to microseconds (no rounding on the way).
fn from_float(f: f64) -> Option<NaiveDateTime> {
    if !f.is_finite() || f < 0.0 {
        return None;
    }
    let bits = f.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = i128::from(bits & ((1u64 << 52) - 1));
    let (mantissa, shift) = if exponent == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), exponent - 1075)
    };
    let scaled = mantissa * 1_000_000;
    let micros = if shift >= 0 {
        scaled.checked_shl(shift as u32)?
    } else if -shift >= 127 {
        0
    } else {
        scaled >> -shift
    };
    DateTime::from_timestamp_micros(i64::try_from(micros).ok()?).map(|t| t.naive_utc())
}

/// `Time#to_f` (and a difference of times) as time.c's
/// rb_time_unmagnify_to_float computes it from nanoseconds: whole seconds
/// exactly, else the nanoseconds made a double first, then divided.
fn nanos_to_float(nanos: i64) -> f64 {
    if nanos % 1_000_000_000 == 0 {
        (nanos / 1_000_000_000) as f64
    } else {
        nanos as f64 / 1e9
    }
}

fn to_float(t: NaiveDateTime) -> f64 {
    nanos_to_float(micros(t) * 1000)
}

/// `TopicTimestampChanger.new(topic:, timestamp:).change!` and the log,
/// for a staff member (`ensure_can_change_post_timestamps!` checked).
pub async fn change(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    acting_user_id: i32,
    topic_id: i32,
    timestamp: f64,
) -> Result<Outcome, AppError> {
    let topic: Option<(NaiveDateTime, String)> =
        sqlx::query_as("SELECT created_at, archetype FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((topic_created_at, archetype)) = topic else {
        return Ok(Outcome::NotFound);
    };
    // topic.first_post.created_at
    let previous: Option<NaiveDateTime> = sqlx::query_scalar(
        "SELECT created_at FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL \
         ORDER BY id LIMIT 1",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let previous = previous.ok_or(Unsupported(
        "changing the timestamp of a topic without its first post",
    ))?;
    let current = crate::clock::now_naive();
    let Some(new_time) = from_float(timestamp) else {
        return Ok(Outcome::Invalid);
    };
    if timestamp > to_float(current) {
        return Ok(Outcome::Invalid);
    }
    // Time - Time: a Float.
    let difference = nanos_to_float((micros(new_time) - micros(topic_created_at)) * 1000);

    // @topic.posts: the live ones, in the order the table gives them.
    let posts: Vec<(i32, i32, Option<i32>, NaiveDateTime, String, String)> = sqlx::query_as(
        "SELECT id, post_number, user_id, created_at, raw, cooked FROM posts \
         WHERE topic_id = $1 AND deleted_at IS NULL",
    )
    .bind(topic_id)
    .fetch_all(&mut *conn)
    .await?;
    let base_path = ctx.config.globals.relative_url_root().to_string();
    let chat = crate::plugins::chat::enabled(ctx.settings)?;
    let mut last_posted_at = new_time;
    for (id, post_number, user_id, created_at, raw, cooked) in posts {
        let at = if post_number == 1 {
            new_time
        } else {
            let moved = from_float(to_float(created_at) + difference)
                .ok_or(Unsupported("a post moved past the times Ruby holds"))?;
            let moved = moved.min(current);
            last_posted_at = last_posted_at.max(moved);
            moved
        };
        // post.update(created_at:, updated_at:), validated as its author.
        let author = user_id.ok_or(Unsupported(
            "changing the timestamp of a post without an author",
        ))?;
        let user = crate::session::current::SessionUser::load(&mut *conn, author)
            .await?
            .ok_or(Unsupported(
                "changing the timestamp of a post whose author is gone",
            ))?;
        let acting = crate::guardian::Guardian::for_user(&mut *conn, &user).await?;
        let analysis = validate::analyze_stripped(&cooked, &base_path, chat)?;
        let errors = validate::validate_post(
            &mut *conn,
            ctx,
            &acting,
            &PostInput {
                raw: &raw,
                topic_id: Some(topic_id),
                first_post: post_number == 1,
                private_message: archetype == "private_message",
                new_record: false,
                post_id: Some(id),
                user_id: author,
            },
            &analysis,
        )
        .await?;
        if !errors.is_empty() {
            continue;
        }
        // before_save: last_editor_id ||= user_id, a blank locale cleared.
        sqlx::query(
            "UPDATE posts SET created_at = $2, updated_at = $2, \
                              last_editor_id = COALESCE(last_editor_id, user_id), \
                              locale = NULLIF(locale, '') \
             WHERE id = $1",
        )
        .bind(id)
        .bind(at)
        .execute(&mut *conn)
        .await?;
    }
    crate::post_destroyer::reset_bumped_at(&mut *conn, topic_id).await?;
    // @topic.update(created_at:, updated_at:, last_posted_at:): validated,
    // the title reassigned.
    let slug = topic_save::reassigned_slug(&mut *conn, ctx.settings, topic_id).await?;
    sqlx::query(
        "UPDATE topics SET created_at = $2, updated_at = $2, last_posted_at = $3, slug = $4, \
                           fancy_title = NULL \
         WHERE id = $1",
    )
    .bind(topic_id)
    .bind(new_time)
    .bind(last_posted_at)
    .bind(&slug)
    .execute(&mut *conn)
    .await?;
    // StaffActionLogger#log_topic_timestamps_changed: the times as strings.
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, topic_id, new_value, previous_value, \
                                     admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, FALSE, clock_timestamp(), clock_timestamp())",
    )
    .bind(TOPIC_TIMESTAMPS_CHANGED)
    .bind(acting_user_id)
    .bind(topic_id)
    .bind(ruby_time(new_time))
    .bind(ruby_time(previous))
    .execute(&mut *conn)
    .await?;
    Ok(Outcome::Done)
}

/// `TimeWithZone#to_s` in UTC.
fn ruby_time(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_cut_to_microseconds() {
        let t = from_float(1_700_000_000.123_456_7).unwrap();
        assert_eq!(micros(t), 1_700_000_000_123_456);
        assert_eq!(to_float(t), 1_700_000_000.123_456);
        assert_eq!(
            micros(from_float(1_760_000_000.5).unwrap()),
            1_760_000_000_500_000
        );
        // As Ruby moves a post 0.214571s after its topic to a topic
        // moved to 2026-01-01: Time#to_f rounds the nanoseconds first.
        let post = DateTime::from_timestamp_micros(1_759_217_309_079_439)
            .unwrap()
            .naive_utc();
        let topic = DateTime::from_timestamp_micros(1_759_217_308_864_868)
            .unwrap()
            .naive_utc();
        let at = DateTime::from_timestamp(1_767_225_600, 0)
            .unwrap()
            .naive_utc();
        let difference = nanos_to_float((micros(at) - micros(topic)) * 1000);
        let moved = to_float(post) + difference;
        assert_eq!(moved, 1_767_225_600.214_571_2);
        assert_eq!(micros(from_float(moved).unwrap()), 1_767_225_600_214_571);
        assert_eq!(
            ruby_time(t),
            "2023-11-14 22:13:20 UTC",
            "TimeWithZone#to_s leaves the fraction out"
        );
    }
}
