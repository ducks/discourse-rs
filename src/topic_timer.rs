//! Port of TopicTimer (app/models/base_timer.rb, topic_timer.rb) and the
//! Topic methods that keep one: `set_or_create_timer`,
//! `delete_topic_timer`, `public_topic_timer`,
//! `inherit_auto_close_from_category`, plus TopicTimerSerializer and
//! PostCreator's `update_topic_auto_close`.
//!
//! Timers live in topic_timers linked through `timerable_id` with
//! `type = 'TopicTimer'`; Rails ignores the old `topic_id` column, which
//! stays NULL. Finished and cancelled timers are trashed (`deleted_at`),
//! and every lookup here reads live ones only (Trashable's default scope).
//!
//! The `topic_timer_changed` event goes unported: no bundled plugin
//! listens for it.

use chrono::{Duration, NaiveDateTime};
use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::posting::Ctx;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `BaseTimer.types`
pub mod types {
    pub const CLOSE: i32 = 1;
    pub const OPEN: i32 = 2;
    pub const PUBLISH_TO_CATEGORY: i32 = 3;
    pub const DELETE: i32 = 4;
    pub const REMINDER: i32 = 5;
    pub const BUMP: i32 = 6;
    pub const DELETE_REPLIES: i32 = 7;
    pub const SILENT_CLOSE: i32 = 8;
    pub const CLEAR_SLOW_MODE: i32 = 9;
}

/// `TopicTimer.types[status_type]`
pub fn type_name(status_type: i32) -> Option<&'static str> {
    Some(match status_type {
        types::CLOSE => "close",
        types::OPEN => "open",
        types::PUBLISH_TO_CATEGORY => "publish_to_category",
        types::DELETE => "delete",
        types::REMINDER => "reminder",
        types::BUMP => "bump",
        types::DELETE_REPLIES => "delete_replies",
        types::SILENT_CLOSE => "silent_close",
        types::CLEAR_SLOW_MODE => "clear_slow_mode",
        _ => return None,
    })
}

/// `TopicTimer.public_types.fetch(name)`
pub fn public_type_by_name(name: &str) -> Option<i32> {
    (1..=9).find(|&t| is_public(t) && type_name(t) == Some(name))
}

/// `TopicTimer.public_types[status_type]`: all but reminder and
/// clear_slow_mode.
pub fn is_public(status_type: i32) -> bool {
    type_name(status_type).is_some()
        && !matches!(status_type, types::REMINDER | types::CLEAR_SLOW_MODE)
}

/// `TopicTimer.destructive_types`
pub fn is_destructive(status_type: i32) -> bool {
    matches!(status_type, types::DELETE | types::DELETE_REPLIES)
}

/// `Discourse.system_user`
pub const SYSTEM_USER_ID: i32 = -1;

/// A topic_timers row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Timer {
    pub id: i32,
    pub execute_at: NaiveDateTime,
    pub status_type: i32,
    pub user_id: i32,
    pub timerable_id: i32,
    pub created_at: NaiveDateTime,
    pub based_on_last_post: bool,
    pub duration_minutes: Option<i32>,
    pub category_id: Option<i32>,
}

const COLUMNS: &str = "id, execute_at, status_type, user_id, timerable_id, created_at, \
                       based_on_last_post, duration_minutes, category_id";

impl Timer {
    /// TopicTimerSerializer
    pub fn serialize(&self) -> Value {
        json!({
            "id": self.id,
            "execute_at": crate::topic_list::time_json(self.execute_at),
            "duration_minutes": self.duration_minutes,
            "based_on_last_post": self.based_on_last_post,
            "status_type": type_name(self.status_type),
            "category_id": self.category_id,
        })
    }
}

/// `topic.public_topic_timer`: `topic_timers.find_by(public_type: true)`.
pub async fn public_topic_timer(
    conn: &mut PgConnection,
    topic_id: i32,
) -> Result<Option<Timer>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM topic_timers \
         WHERE timerable_id = $1 AND type = 'TopicTimer' AND public_type AND deleted_at IS NULL \
         ORDER BY id LIMIT 1"
    ))
    .bind(topic_id)
    .fetch_optional(conn)
    .await
}

/// `TopicTimer.find_by(topic:)`: any live timer, private ones too.
pub async fn any_topic_timer(
    conn: &mut PgConnection,
    topic_id: i32,
) -> Result<Option<Timer>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM topic_timers \
         WHERE timerable_id = $1 AND type = 'TopicTimer' AND deleted_at IS NULL ORDER BY id LIMIT 1"
    ))
    .bind(topic_id)
    .fetch_optional(conn)
    .await
}

/// `TopicTimer#finish!` (and `trash!`): `update_columns(deleted_at:,
/// deleted_by_id:)` under a lock, leaving updated_at; a timer already
/// trashed stays as it is.
pub async fn finish(
    conn: &mut PgConnection,
    timer_id: i32,
    by_user: Option<i32>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE topic_timers SET deleted_at = clock_timestamp(), deleted_by_id = $2 \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(timer_id)
    .bind(by_user)
    .execute(conn)
    .await?;
    Ok(())
}

/// `Topic#delete_topic_timer(status_type, by_user:)`: the live timer of
/// that type (the user's own for a private type) finished.
pub async fn delete_topic_timer(
    conn: &mut PgConnection,
    topic_id: i32,
    status_type: i32,
    by_user: Option<i32>,
) -> Result<(), sqlx::Error> {
    let id: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM topic_timers \
         WHERE timerable_id = $1 AND type = 'TopicTimer' AND status_type = $2 AND deleted_at IS NULL \
           AND ($3 OR user_id IS NOT DISTINCT FROM $4) ORDER BY id LIMIT 1",
    )
    .bind(topic_id)
    .bind(status_type)
    .bind(is_public(status_type))
    .bind(by_user)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = id {
        finish(conn, id, by_user).await?;
    }
    Ok(())
}

/// `Guardian.new(user).can_set_topic_timer?(topic)`; None is no user.
pub async fn can_set_topic_timer(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: Option<i32>,
    topic_id: i32,
) -> Result<bool, AppError> {
    let guardian = crate::onebox::guardian_for(&mut *conn, user_id).await?;
    can_set_topic_timer_for(conn, settings, &guardian, Some(topic_id)).await
}

/// `can_set_topic_timer?(topic)` for a guardian; a missing topic (None)
/// skips the visibility check, as Rails' nil topic does.
pub async fn can_set_topic_timer_for(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    topic_id: Option<i32>,
) -> Result<bool, AppError> {
    let Some(user) = guardian.user() else {
        return Ok(false);
    };
    if guardian.is_silenced() {
        return Ok(false);
    }
    if user.id == SYSTEM_USER_ID {
        return Ok(true);
    }
    if let Some(topic_id) = topic_id
        && !crate::onebox::can_see_topic(&mut *conn, settings, guardian, topic_id).await?
    {
        return Ok(false);
    }
    if guardian.is_staff() {
        return Ok(true);
    }
    Ok(guardian.in_setting_groups(settings, "topic_timers_allowed_groups")?)
}

/// `set_or_create_timer`'s time argument.
pub enum When<'a> {
    /// nil or blank.
    Blank,
    /// A param: a number of hours, or a timestamp.
    Text(&'a str),
    /// A number of hours (a category's auto_close_hours).
    Hours(f64),
}

/// `set_or_create_timer`'s options.
#[derive(Default)]
pub struct SetOptions {
    pub by_user: Option<i32>,
    pub based_on_last_post: bool,
    /// `category_id:` as given (`Category.find_by(id:)` for publishing).
    pub category_id: Option<i64>,
    /// `duration_minutes:` as given (nil when absent).
    pub duration_minutes: Option<i64>,
}

/// What `set_or_create_timer` came to.
pub enum SetOutcome {
    /// nil: the timer was deleted, or no execute_at came of the arguments.
    Nothing,
    /// Saved; `created` when it is a new row.
    Saved { timer: Timer, created: bool },
    /// `ActiveRecord::RecordInvalid#message`.
    Invalid(String),
    /// `Discourse::InvalidParameters` (a time not in the future).
    InvalidParameters,
}

/// Ruby's `Float(time)` for a param, None when it raises. Hex and
/// underscored numbers are refused rather than read.
fn ruby_float(text: &str) -> Result<Option<f64>, Unsupported> {
    let s = text.trim();
    if s.is_empty() {
        return Ok(None);
    }
    if s.contains('_') || s.to_ascii_lowercase().contains("0x") {
        return Err(Unsupported(
            "a timer time Float() reads as hex or underscored",
        ));
    }
    if !s
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        return Ok(None);
    }
    // Float() wants digits on both sides of a point ("1." and ".5" raise).
    if s.starts_with('.') || s.ends_with('.') || s.contains(".e") || s.contains(".E") {
        return Ok(None);
    }
    Ok(s.parse::<f64>().ok())
}

/// `Time.find_zone("UTC").parse(time)` for the forms the composer sends;
/// None where Ruby finds no date at all.
fn parse_utc(text: &str) -> Result<Option<NaiveDateTime>, Unsupported> {
    let s = text.trim();
    if !s.bytes().any(|b| b.is_ascii_digit()) {
        return Ok(None);
    }
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(Some(t.naive_utc()));
    }
    // Time#to_s: "2026-10-15 22:13:43 UTC"
    let s = s.strip_suffix(" UTC").unwrap_or(s);
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(t) = NaiveDateTime::parse_from_str(s, format) {
            return Ok(Some(t));
        }
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d.and_hms_opt(0, 0, 0));
    }
    Err(Unsupported("a timer time other than ISO 8601"))
}

/// `n.hours` / `n.minutes` as a duration, to the microsecond.
fn hours(n: f64) -> Duration {
    Duration::microseconds((n * 3_600_000_000.0).round() as i64)
}

/// `BaseTimer::MAX_DURATION_MINUTES` (20.years.to_i / 60).
const MAX_DURATION_MINUTES: i64 = 631_152_000 / 60;

/// `Topic#set_or_create_timer(status_type, time, ...)`, saving the timer
/// (with TopicTimer's after_save status change) on a persisted topic.
pub async fn set_or_create_timer(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    status_type: i32,
    time: When<'_>,
    opts: &SetOptions,
) -> Result<SetOutcome, AppError> {
    let time_blank = match time {
        When::Blank => true,
        When::Text(t) => t.trim().is_empty(),
        When::Hours(_) => false,
    };
    if time_blank && opts.duration_minutes.is_none() {
        delete_topic_timer(conn, topic_id, status_type, opts.by_user).await?;
        return Ok(SetOutcome::Nothing);
    }
    let duration_minutes = opts.duration_minutes.unwrap_or(0);
    let public = is_public(status_type);
    // find_or_initialize_by(topic:, public_type:[, user:]): a public timer
    // of another type is taken over and retyped.
    let existing: Option<Timer> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM topic_timers \
         WHERE timerable_id = $1 AND type = 'TopicTimer' AND public_type = $2 AND deleted_at IS NULL \
           AND ($2 OR user_id IS NOT DISTINCT FROM $3) ORDER BY id LIMIT 1"
    ))
    .bind(topic_id)
    .bind(public)
    .bind(opts.by_user)
    .fetch_optional(&mut *conn)
    .await?;
    let now = crate::clock::now_naive();
    let mut execute_at = existing.as_ref().map(|t| t.execute_at);
    let mut created_at = existing.as_ref().map(|t| t.created_at);
    let mut duration = existing.as_ref().and_then(|t| t.duration_minutes);
    let mut category_id = existing.as_ref().and_then(|t| t.category_id);
    if status_type == types::PUBLISH_TO_CATEGORY {
        category_id = match opts.category_id {
            Some(id) => {
                sqlx::query_scalar("SELECT id FROM categories WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&mut *conn)
                    .await?
            }
            None => None,
        };
    }

    if opts.based_on_last_post {
        if duration_minutes > 0 {
            // ordered_posts.last: the highest numbered live post.
            let last: Option<NaiveDateTime> = sqlx::query_scalar(
                "SELECT created_at FROM posts WHERE topic_id = $1 AND deleted_at IS NULL \
                 ORDER BY post_number DESC LIMIT 1",
            )
            .bind(topic_id)
            .fetch_optional(&mut *conn)
            .await?;
            let from = last.unwrap_or(now);
            duration = Some(duration_minutes as i32);
            execute_at = Some(from + Duration::minutes(duration_minutes));
            created_at = Some(from);
        }
    } else if status_type == types::DELETE_REPLIES {
        if duration_minutes > 0 {
            let first_reply: Option<NaiveDateTime> = sqlx::query_scalar(
                "SELECT MIN(created_at) FROM posts WHERE topic_id = $1 AND deleted_at IS NULL AND post_number > 1",
            )
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
            let from = first_reply.unwrap_or(now);
            duration = Some(duration_minutes as i32);
            execute_at = Some(from + Duration::minutes(duration_minutes));
            created_at = Some(from);
        }
    } else {
        match time {
            When::Hours(n) => {
                if n > 0.0 {
                    execute_at = Some(now + hours(n));
                }
            }
            When::Text(text) => match ruby_float(text)? {
                Some(n) => {
                    if n > 0.0 {
                        execute_at = Some(now + hours(n));
                    }
                }
                None => match parse_utc(text)? {
                    Some(at) if at > now => execute_at = Some(at),
                    _ => return Ok(SetOutcome::InvalidParameters),
                },
            },
            // Float(nil) raises and so does Time.zone.parse(nil).
            When::Blank => {
                return Err(std::io::Error::other(
                    "TypeError: set_or_create_timer with a duration and no time",
                )
                .into());
            }
        }
    }

    let Some(execute_at) = execute_at else {
        return Ok(SetOutcome::Nothing);
    };

    // The user: by_user when they may set timers, else the timer's own,
    // else the topic's owner when they may, else system.
    let user_id = if let Some(by) = opts.by_user
        && can_set_topic_timer(&mut *conn, ctx.settings, Some(by), topic_id).await?
    {
        by
    } else if let Some(t) = &existing {
        t.user_id
    } else {
        let owner: Option<i32> = sqlx::query_scalar("SELECT user_id FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
        if owner.is_some() && can_set_topic_timer(&mut *conn, ctx.settings, owner, topic_id).await?
        {
            owner.unwrap_or(SYSTEM_USER_ID)
        } else {
            SYSTEM_USER_ID
        }
    };

    // Validations, in the order Rails runs them: BaseTimer's (category,
    // duration), then TopicTimer's (execute_at after created_at).
    let mut errors = Vec::new();
    let model = "activerecord.errors.models.topic_timer.attributes";
    if status_type == types::PUBLISH_TO_CATEGORY && category_id.is_none() {
        errors.push(format!("Category {}", ctx.t("errors.messages.blank")));
    }
    if let Some(d) = duration.map(i64::from) {
        if d <= 0 {
            let message = ctx.t(&format!("{model}.duration_minutes.cannot_be_zero"));
            errors.push(format!("Duration minutes {message}"));
        }
        if d > MAX_DURATION_MINUTES {
            let message = ctx.t(&format!("{model}.duration_minutes.exceeds_maximum"));
            errors.push(format!("Duration minutes {message}"));
        }
    }
    if let Some(created) = created_at
        && execute_at <= created
    {
        let message = ctx.t(&format!("{model}.execute_at.in_the_past"));
        errors.push(format!("Execute at {message}"));
    }
    if !errors.is_empty() {
        let errors = errors.join(", ");
        let message = ctx
            .i18n
            .t_with("errors.messages.record_invalid", &[("errors", &errors)])
            .unwrap_or_else(|| format!("Validation failed: {errors}"));
        return Ok(SetOutcome::Invalid(message));
    }

    let (timer, created, moved) = match existing {
        Some(old) => {
            let moved = old.execute_at != execute_at || old.user_id != user_id;
            let changed = moved
                || old.status_type != status_type
                || old.based_on_last_post != opts.based_on_last_post
                || old.created_at != created_at.unwrap_or(old.created_at)
                || old.duration_minutes != duration
                || old.category_id != category_id;
            let timer: Timer = if changed {
                sqlx::query_as(&format!(
                    "UPDATE topic_timers SET status_type = $2, based_on_last_post = $3, execute_at = $4, \
                            created_at = $5, duration_minutes = $6, category_id = $7, user_id = $8, \
                            public_type = $9, updated_at = clock_timestamp() \
                     WHERE id = $1 RETURNING {COLUMNS}"
                ))
                .bind(old.id)
                .bind(status_type)
                .bind(opts.based_on_last_post)
                .bind(execute_at)
                .bind(created_at.unwrap_or(old.created_at))
                .bind(duration)
                .bind(category_id)
                .bind(user_id)
                .bind(public)
                .fetch_one(&mut *conn)
                .await?
            } else {
                old
            };
            (timer, false, moved)
        }
        None => {
            let timer: Timer = sqlx::query_as(&format!(
                "INSERT INTO topic_timers (execute_at, status_type, user_id, timerable_id, type, \
                        based_on_last_post, created_at, updated_at, category_id, public_type, duration_minutes) \
                 VALUES ($1, $2, $3, $4, 'TopicTimer', $5, COALESCE($6, clock_timestamp()), clock_timestamp(), \
                         $7, $8, $9) \
                 RETURNING {COLUMNS}"
            ))
            .bind(execute_at)
            .bind(status_type)
            .bind(user_id)
            .bind(topic_id)
            .bind(opts.based_on_last_post)
            .bind(created_at)
            .bind(category_id)
            .bind(public)
            .bind(duration)
            .fetch_one(&mut *conn)
            .await?;
            (timer, true, true)
        }
    };

    // after_save: a close timer on a closed topic opens it now, an open
    // timer on an open topic closes it now (the "temporarily" timers).
    if moved {
        let closed: bool = sqlx::query_scalar("SELECT closed FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
        let flip = match status_type {
            types::CLOSE | types::SILENT_CLOSE if closed => Some(false),
            types::OPEN if !closed => Some(true),
            _ => None,
        };
        if let Some(enabled) = flip {
            Box::pin(crate::topic_status::apply(
                &mut *conn,
                ctx,
                topic_id,
                timer.user_id,
                crate::topic_status::Status::Closed,
                enabled,
                None,
                false,
            ))
            .await?;
        }
    }
    Ok(SetOutcome::Saved { timer, created })
}

/// `Topic#inherit_auto_close_from_category(timer_type:)`: an open topic
/// in a category with auto_close_hours and no public timer gets one, by
/// system, unless the time has passed already.
pub async fn inherit_auto_close_from_category(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    timer_type: i32,
) -> Result<(), AppError> {
    let row: Option<(bool, NaiveDateTime, Option<f64>, bool)> = sqlx::query_as(
        "SELECT t.closed, t.created_at, c.auto_close_hours, COALESCE(c.auto_close_based_on_last_post, FALSE) \
         FROM topics t LEFT JOIN categories c ON c.id = t.category_id WHERE t.id = $1",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((closed, created_at, Some(auto_close_hours), based_on_last_post)) = row else {
        return Ok(());
    };
    if closed || public_topic_timer(&mut *conn, topic_id).await?.is_some() {
        return Ok(());
    }
    let opts = |duration_minutes| SetOptions {
        by_user: Some(SYSTEM_USER_ID),
        based_on_last_post,
        category_id: None,
        duration_minutes,
    };
    if based_on_last_post {
        // auto_close_hours * 60, read back with to_i.
        let duration = (auto_close_hours * 60.0) as i64;
        Box::pin(set_or_create_timer(
            conn,
            ctx,
            topic_id,
            timer_type,
            When::Hours(auto_close_hours),
            &opts(Some(duration)),
        ))
        .await?;
    } else {
        let at = created_at + hours(auto_close_hours);
        if at < crate::clock::now_naive() {
            return Ok(());
        }
        // Passed as Time#to_s, so whole seconds.
        let text = at.format("%Y-%m-%d %H:%M:%S UTC").to_string();
        Box::pin(set_or_create_timer(
            conn,
            ctx,
            topic_id,
            timer_type,
            When::Text(&text),
            &opts(None),
        ))
        .await?;
    }
    Ok(())
}

/// `PostCreator#update_topic_auto_close`: a closed topic's close timer
/// finished by system; else a public timer based on the last post moved
/// to after this one (retyped close).
pub async fn update_topic_auto_close(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
) -> Result<(), AppError> {
    let closed: bool = sqlx::query_scalar("SELECT closed FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
    if closed {
        delete_topic_timer(conn, topic_id, types::CLOSE, Some(SYSTEM_USER_ID)).await?;
        return Ok(());
    }
    let Some(timer) = public_topic_timer(&mut *conn, topic_id).await? else {
        return Ok(());
    };
    let duration = timer.duration_minutes.unwrap_or(0);
    if timer.based_on_last_post && duration > 0 {
        let outcome = Box::pin(set_or_create_timer(
            conn,
            ctx,
            topic_id,
            types::CLOSE,
            When::Blank,
            &SetOptions {
                by_user: None,
                based_on_last_post: true,
                category_id: None,
                duration_minutes: Some(i64::from(duration)),
            },
        ))
        .await?;
        if let SetOutcome::Invalid(message) = outcome {
            return Err(
                std::io::Error::other(format!("ActiveRecord::RecordInvalid: {message}")).into(),
            );
        }
    }
    Ok(())
}

/// `Topic#auto_close_threshold_reached?`: enough pending flag scores, from
/// enough flaggers, on a topic not owned by staff.
pub async fn auto_close_threshold_reached(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    topic_id: i32,
) -> Result<bool, AppError> {
    let owner_staff: bool = sqlx::query_scalar(
        "SELECT COALESCE(u.admin OR u.moderator, FALSE) FROM topics t \
         LEFT JOIN users u ON u.id = t.user_id WHERE t.id = $1",
    )
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    if owner_staff {
        return Ok(false);
    }
    let min = crate::reviewables::min_score_for_priority(&mut *conn, settings).await?;
    let (flaggers, total): (i64, f64) = sqlx::query_as(
        "SELECT COUNT(DISTINCT rs.user_id), COALESCE(SUM(rs.score), 0.0) \
         FROM reviewable_scores rs JOIN reviewables r ON r.id = rs.reviewable_id \
         WHERE rs.status = $1 AND rs.score >= $2 AND r.topic_id = $3",
    )
    .bind(crate::reviewables::PENDING)
    .bind(min)
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    let to_close = crate::reviewables::sensitivity_score(
        &mut *conn,
        settings,
        "auto_close_topic_sensitivity",
        2.5,
    )
    .await?;
    Ok(flaggers >= settings.get("num_flaggers_to_close_topic")?.to_i() && total >= to_close)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_floats_as_ruby_does() {
        assert_eq!(ruby_float("24").unwrap(), Some(24.0));
        assert_eq!(ruby_float(" 1.5 ").unwrap(), Some(1.5));
        assert_eq!(ruby_float("-2").unwrap(), Some(-2.0));
        assert_eq!(ruby_float("").unwrap(), None);
        assert_eq!(ruby_float("1.").unwrap(), None);
        assert_eq!(ruby_float("2027-01-01").unwrap(), None);
        assert_eq!(ruby_float("2027-01-01 10:00").unwrap(), None);
        assert!(ruby_float("0x1A").is_err());
    }

    #[test]
    fn parses_times_in_utc() {
        let at = |s| parse_utc(s).unwrap().map(|t| t.to_string());
        assert_eq!(
            at("2027-01-01T10:00:00.000Z").as_deref(),
            Some("2027-01-01 10:00:00")
        );
        assert_eq!(
            at("2027-01-01T10:00:00+02:00").as_deref(),
            Some("2027-01-01 08:00:00")
        );
        assert_eq!(
            at("2027-01-01 10:00").as_deref(),
            Some("2027-01-01 10:00:00")
        );
        assert_eq!(
            at("2026-10-15 22:13:43 UTC").as_deref(),
            Some("2026-10-15 22:13:43")
        );
        assert_eq!(at("2027-01-01").as_deref(), Some("2027-01-01 00:00:00"));
        assert_eq!(at("soon"), None);
    }

    #[test]
    fn public_types_by_name() {
        assert_eq!(public_type_by_name("close"), Some(1));
        assert_eq!(public_type_by_name("silent_close"), Some(8));
        assert_eq!(public_type_by_name("reminder"), None);
        assert_eq!(public_type_by_name("clear_slow_mode"), None);
        assert_eq!(public_type_by_name("foo"), None);
    }
}
