//! UserNotificationSchedule: a user's weekly do not disturb hours (day 0
//! Monday, minutes of the day), its validations, and
//! UserNotificationScheduleProcessor, which turns it into scheduled
//! do_not_disturb_timings for today and the next two days, in the user's
//! timezone. Jobs::ProcessUserNotificationSchedules runs it daily for
//! every enabled schedule.
//!
//! Rails' quirks are kept: a day starting at minute 0 carries a timing
//! from the day before through the whole day, a timing's minute is placed
//! with the offset of the current wall clock on that day (not the
//! offset at that minute), and a found timing carried to the last day is
//! created again rather than extended.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde_json::{Map, Value};
use sqlx::PgConnection;

use crate::{AppError, AppState, Unsupported};

/// `Jobs::ProcessUserNotificationSchedules`, daily.
pub const PROCESS_USER_NOTIFICATION_SCHEDULES: &str = "process_user_notification_schedules";

/// A schedule's columns.
#[derive(Clone, Debug, PartialEq, sqlx::FromRow)]
pub struct Schedule {
    pub id: Option<i64>,
    pub enabled: Option<bool>,
    pub day_0_start_time: Option<i32>,
    pub day_0_end_time: Option<i32>,
    pub day_1_start_time: Option<i32>,
    pub day_1_end_time: Option<i32>,
    pub day_2_start_time: Option<i32>,
    pub day_2_end_time: Option<i32>,
    pub day_3_start_time: Option<i32>,
    pub day_3_end_time: Option<i32>,
    pub day_4_start_time: Option<i32>,
    pub day_4_end_time: Option<i32>,
    pub day_5_start_time: Option<i32>,
    pub day_5_end_time: Option<i32>,
    pub day_6_start_time: Option<i32>,
    pub day_6_end_time: Option<i32>,
}

const COLUMNS: &str = "id, enabled, day_0_start_time, day_0_end_time, day_1_start_time, \
     day_1_end_time, day_2_start_time, day_2_end_time, day_3_start_time, day_3_end_time, \
     day_4_start_time, day_4_end_time, day_5_start_time, day_5_end_time, day_6_start_time, \
     day_6_end_time";

impl Schedule {
    /// `UserNotificationSchedule.new(user:)`: the column defaults, so
    /// disabled and without times (not DEFAULT, which only the serializer
    /// shows).
    pub fn new_record() -> Self {
        Self {
            id: None,
            enabled: Some(false),
            day_0_start_time: None,
            day_0_end_time: None,
            day_1_start_time: None,
            day_1_end_time: None,
            day_2_start_time: None,
            day_2_end_time: None,
            day_3_start_time: None,
            day_3_end_time: None,
            day_4_start_time: None,
            day_4_end_time: None,
            day_5_start_time: None,
            day_5_end_time: None,
            day_6_start_time: None,
            day_6_end_time: None,
        }
    }

    fn day_mut(&mut self, day: usize) -> (&mut Option<i32>, &mut Option<i32>) {
        match day {
            0 => (&mut self.day_0_start_time, &mut self.day_0_end_time),
            1 => (&mut self.day_1_start_time, &mut self.day_1_end_time),
            2 => (&mut self.day_2_start_time, &mut self.day_2_end_time),
            3 => (&mut self.day_3_start_time, &mut self.day_3_end_time),
            4 => (&mut self.day_4_start_time, &mut self.day_4_end_time),
            5 => (&mut self.day_5_start_time, &mut self.day_5_end_time),
            _ => (&mut self.day_6_start_time, &mut self.day_6_end_time),
        }
    }

    /// A day's (start, end) minutes.
    pub fn day(&self, day: usize) -> (Option<i32>, Option<i32>) {
        let mut copy = self.clone();
        let (s, e) = copy.day_mut(day);
        (*s, *e)
    }

    /// The user's schedule row.
    pub async fn find(conn: &mut PgConnection, user_id: i32) -> Result<Option<Self>, AppError> {
        Ok(sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM user_notification_schedules WHERE user_id = $1 ORDER BY id LIMIT 1"
        ))
        .bind(user_id)
        .fetch_optional(&mut *conn)
        .await?)
    }

    /// `assign_attributes` of the permitted params (enabled,
    /// day_N_start_time, day_N_end_time), cast as the columns cast them.
    pub fn assign(&mut self, params: &Map<String, Value>) -> Result<(), AppError> {
        if let Some(v) = params.get("enabled") {
            self.enabled = match v {
                Value::Bool(b) => Some(*b),
                Value::Null => None,
                other => crate::ruby::cast_boolean(crate::params::scalar(other).as_deref()),
            };
        }
        for day in 0..7 {
            for (end, key) in [
                (false, format!("day_{day}_start_time")),
                (true, format!("day_{day}_end_time")),
            ] {
                let Some(v) = params.get(&key) else {
                    continue;
                };
                let value = cast_integer(v)?;
                let (s, e) = self.day_mut(day);
                if end {
                    *e = value;
                } else {
                    *s = value;
                }
            }
        }
        Ok(())
    }

    /// `has_valid_times` and the enabled inclusion: whether it saves.
    pub fn valid(&self) -> bool {
        if self.enabled.is_none() {
            return false;
        }
        (0..7).all(|day| {
            let (start, end) = self.day(day);
            let start_ok = start.is_some_and(|s| (-1..=1410).contains(&s));
            let end_ok = end.is_some_and(|e| e <= 1440);
            let ordered = match (start, end) {
                (Some(s), Some(e)) => s <= e,
                _ => true,
            };
            start_ok && end_ok && ordered
        })
    }

    /// `save` of a valid schedule: an INSERT when new, an UPDATE when a
    /// column changed.
    pub async fn save(
        &mut self,
        conn: &mut PgConnection,
        user_id: i32,
        before: Option<&Schedule>,
    ) -> Result<(), AppError> {
        let values: Vec<Option<i32>> = (0..7)
            .flat_map(|d| {
                let (s, e) = self.day(d);
                [s, e]
            })
            .collect();
        match self.id {
            None => {
                let id: i64 = sqlx::query_scalar(
                    "INSERT INTO user_notification_schedules (user_id, enabled, day_0_start_time, \
                       day_0_end_time, day_1_start_time, day_1_end_time, day_2_start_time, \
                       day_2_end_time, day_3_start_time, day_3_end_time, day_4_start_time, \
                       day_4_end_time, day_5_start_time, day_5_end_time, day_6_start_time, \
                       day_6_end_time) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16) \
                     RETURNING id",
                )
                .bind(user_id)
                .bind(self.enabled)
                .bind(values[0])
                .bind(values[1])
                .bind(values[2])
                .bind(values[3])
                .bind(values[4])
                .bind(values[5])
                .bind(values[6])
                .bind(values[7])
                .bind(values[8])
                .bind(values[9])
                .bind(values[10])
                .bind(values[11])
                .bind(values[12])
                .bind(values[13])
                .fetch_one(&mut *conn)
                .await?;
                self.id = Some(id);
            }
            Some(id) => {
                if before == Some(self) {
                    return Ok(());
                }
                sqlx::query(
                    "UPDATE user_notification_schedules SET enabled = $2, day_0_start_time = $3, \
                       day_0_end_time = $4, day_1_start_time = $5, day_1_end_time = $6, \
                       day_2_start_time = $7, day_2_end_time = $8, day_3_start_time = $9, \
                       day_3_end_time = $10, day_4_start_time = $11, day_4_end_time = $12, \
                       day_5_start_time = $13, day_5_end_time = $14, day_6_start_time = $15, \
                       day_6_end_time = $16 WHERE id = $1",
                )
                .bind(id)
                .bind(self.enabled)
                .bind(values[0])
                .bind(values[1])
                .bind(values[2])
                .bind(values[3])
                .bind(values[4])
                .bind(values[5])
                .bind(values[6])
                .bind(values[7])
                .bind(values[8])
                .bind(values[9])
                .bind(values[10])
                .bind(values[11])
                .bind(values[12])
                .bind(values[13])
                .execute(&mut *conn)
                .await?;
            }
        }
        Ok(())
    }
}

/// ActiveModel's integer cast of a param: numbers truncated, numeric
/// strings by their leading number, blank strings nil.
fn cast_integer(v: &Value) -> Result<Option<i32>, AppError> {
    Ok(match v {
        Value::Null => None,
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f.trunc() as i64))
            .map(|i| i as i32),
        Value::Bool(b) => Some(i32::from(*b)),
        Value::String(s) if s.trim().is_empty() => None,
        Value::String(s) => {
            let t = s.trim_start();
            let numeric = t
                .trim_start_matches(['-', '+'])
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit());
            if !numeric {
                return Err(Unsupported("a non-numeric notification schedule time").into());
            }
            Some(crate::ruby::to_i(t) as i32)
        }
        _ => return Err(Unsupported("a notification schedule time that is not a scalar").into()),
    })
}

/// `UserNotificationSchedule#destroy_scheduled_timings`
pub async fn destroy_scheduled_timings(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM do_not_disturb_timings WHERE user_id = $1 AND scheduled")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Microseconds, as the column stores a Ruby time (truncated).
fn stored(t: NaiveDateTime) -> NaiveDateTime {
    use chrono::Timelike;
    t.with_nanosecond(t.nanosecond() / 1_000 * 1_000)
        .unwrap_or(t)
}

/// The zone `Time.now.in_time_zone(user_option.timezone)` runs in: the
/// server's (UTC on the reference) without one.
fn zone(timezone: Option<&str>) -> Result<Tz, AppError> {
    match timezone {
        None => Ok(chrono_tz::UTC),
        // ActiveSupport::TimeZone[] also takes Rails' own zone names
        // ("Mountain Time (US & Canada)"); those aren't mapped, and Rails
        // raises for anything else.
        Some(name) => name.parse::<Tz>().map_err(|_| {
            Unsupported("a notification schedule timezone that is not a tz name").into()
        }),
    }
}

/// A local wall-clock time in the zone, as TimeWithZone resolves it (the
/// earlier of an ambiguous pair); a time in a DST gap is refused.
fn local(tz: Tz, t: NaiveDateTime) -> Result<DateTime<Tz>, AppError> {
    tz.from_local_datetime(&t)
        .earliest()
        .ok_or_else(|| Unsupported("a notification schedule time in a DST gap").into())
}

fn end_of_day(date: NaiveDate) -> NaiveDateTime {
    date.and_time(NaiveTime::from_hms_nano_opt(23, 59, 59, 999_999_999).unwrap_or_default())
}

/// `utc_time_at_minute(base_time, total_minutes)`: that minute of the
/// base's date, at the base's offset (the minute 1440 is the next
/// midnight).
fn utc_time_at_minute(base: &DateTime<Tz>, total_minutes: i32) -> NaiveDateTime {
    use chrono::Offset;
    let offset = base.offset().fix().local_minus_utc();
    let wall = base.date_naive().and_time(NaiveTime::MIN) + Duration::minutes(total_minutes.into());
    wall - Duration::seconds(offset.into())
}

/// A timing being built: unsaved (no id) or found.
struct Timing {
    id: Option<i64>,
    starts_at: NaiveDateTime,
}

/// `user.do_not_disturb_timings.find_or_create_by(attrs)`: the scheduled
/// timing with these very times, else a new one when it validates.
async fn find_or_create(
    conn: &mut PgConnection,
    user_id: i32,
    starts_at: NaiveDateTime,
    ends_at: NaiveDateTime,
) -> Result<(), AppError> {
    let (starts_at, ends_at) = (stored(starts_at), stored(ends_at));
    let found: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM do_not_disturb_timings WHERE user_id = $1 \
         AND starts_at = $2 AND ends_at = $3 AND scheduled = TRUE)",
    )
    .bind(user_id)
    .bind(starts_at)
    .bind(ends_at)
    .fetch_one(&mut *conn)
    .await?;
    if found || starts_at > ends_at {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO do_not_disturb_timings (user_id, starts_at, ends_at, scheduled) \
         VALUES ($1, $2, $3, TRUE)",
    )
    .bind(user_id)
    .bind(starts_at)
    .bind(ends_at)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `UserNotificationSchedule#create_do_not_disturb_timings(delete_existing:)`
pub async fn create_do_not_disturb_timings(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    user_id: i32,
    schedule: &Schedule,
    delete_existing: bool,
) -> Result<(), AppError> {
    if delete_existing {
        destroy_scheduled_timings(conn, user_id).await?;
    }
    // `user.user_option.timezone`
    let option: Option<Option<String>> =
        sqlx::query_scalar("SELECT timezone FROM user_options WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(timezone) = option else {
        return Err(Unsupported("a notification schedule of a user without options").into());
    };
    let tz = zone(timezone.as_deref())?;
    let now: DateTime<Utc> = crate::clock::now();
    let today = now.with_timezone(&tz);

    let mut previous: Option<Timing> = None;
    for offset in 0..3i64 {
        // local_time + n.days: the same wall clock, n dates on.
        let wall = today.naive_local() + Duration::days(offset);
        let local_time = local(tz, wall)?;
        let weekday = local_time.weekday().num_days_from_monday() as usize;
        let (start, end) = schedule.day(weekday);
        let (Some(start), Some(end)) = (start, end) else {
            return Err(Unsupported("a notification schedule without a day's times").into());
        };
        if previous.is_none() && start != 0 {
            // find_previous_timing: one ending at last local midnight.
            let date = local_time.date_naive();
            let last_midnight = local(tz, end_of_day(date - Duration::days(1)))?.naive_utc();
            let found: Option<(i64, NaiveDateTime)> = sqlx::query_as(
                "SELECT id, starts_at FROM do_not_disturb_timings WHERE user_id = $1 \
                 AND ends_at = $2 AND scheduled = TRUE LIMIT 1",
            )
            .bind(user_id)
            .bind(stored(last_midnight))
            .fetch_optional(&mut *conn)
            .await?;
            previous = Some(match found {
                Some((id, starts_at)) => Timing {
                    id: Some(id),
                    starts_at,
                },
                None => Timing {
                    id: None,
                    starts_at: local(tz, date.and_time(NaiveTime::MIN))?.naive_utc(),
                },
            });
        }
        if start > 0 {
            if let Some(timing) = previous.as_mut() {
                let ends_at = utc_time_at_minute(&local_time, start - 1);
                match timing.id {
                    // save: an UPDATE, unless it no longer validates.
                    Some(id) => {
                        if timing.starts_at <= stored(ends_at) {
                            sqlx::query(
                                "UPDATE do_not_disturb_timings SET ends_at = $2 WHERE id = $1",
                            )
                            .bind(id)
                            .bind(stored(ends_at))
                            .execute(&mut *conn)
                            .await?;
                        }
                    }
                    None => find_or_create(conn, user_id, timing.starts_at, ends_at).await?,
                }
            }
            previous = Some(Timing {
                id: None,
                starts_at: utc_time_at_minute(&local_time, end),
            });
        }
        if offset == 2 {
            if let Some(timing) = previous.as_ref() {
                let ends_at = local(tz, end_of_day(local_time.date_naive()))?.naive_utc();
                find_or_create(conn, user_id, timing.starts_at, ends_at).await?;
            }
            let until = crate::do_not_disturb::until(conn, user_id).await?;
            crate::do_not_disturb::publish(bus, conn, user_id, until).await?;
        }
    }
    Ok(())
}

/// `Jobs::ProcessUserNotificationSchedules`: every enabled schedule's
/// timings, each on its own (Rails logs a schedule that fails and goes
/// on; a refusal here stops the job, naming what isn't ported).
pub async fn process_user_notification_schedules(state: &AppState) -> Result<(), AppError> {
    let schedules: Vec<(i32, Schedule)> = {
        let mut conn = state.pool.acquire().await?;
        let rows: Vec<(i32, i64)> = sqlx::query_as(
            "SELECT user_id, id FROM user_notification_schedules WHERE enabled ORDER BY id",
        )
        .fetch_all(&mut *conn)
        .await?;
        let mut out = Vec::new();
        for (user_id, id) in rows {
            let schedule: Schedule = sqlx::query_as(&format!(
                "SELECT {COLUMNS} FROM user_notification_schedules WHERE id = $1"
            ))
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
            out.push((user_id, schedule));
        }
        out
    };
    for (user_id, schedule) in schedules {
        let mut tx = state.pool.begin().await?;
        create_do_not_disturb_timings(&mut tx, &state.bus, user_id, &schedule, false).await?;
        tx.commit().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(tz: Tz, y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Tz> {
        let wall = NaiveDate::from_ymd_opt(y, m, d)
            .and_then(|date| date.and_hms_opt(h, min, 0))
            .expect("a valid time");
        local(tz, wall).expect("not in a gap")
    }

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .and_then(|date| date.and_hms_opt(h, min, 0))
            .expect("a valid time")
    }

    #[test]
    fn minutes_take_the_offset_of_the_wall_clock_now() {
        let denver: Tz = "America/Denver".parse().expect("a zone");
        // 1 Nov 2026 is the fall-back day: at 01:30 MDT (-6) the 22:00
        // minute is placed at -6 too, an hour off its real -7.
        let base = at(denver, 2026, 11, 1, 1, 30);
        assert_eq!(utc_time_at_minute(&base, 22 * 60), utc(2026, 11, 2, 4, 0));
        let base = at(denver, 2026, 11, 1, 12, 0);
        assert_eq!(utc_time_at_minute(&base, 22 * 60), utc(2026, 11, 2, 5, 0));
        // 1440: the next midnight.
        let base = at(chrono_tz::UTC, 2026, 10, 14, 3, 0);
        assert_eq!(utc_time_at_minute(&base, 1440), utc(2026, 10, 15, 0, 0));
    }

    #[test]
    fn validations() {
        let mut s = Schedule::new_record();
        assert!(!s.valid(), "a new record has no times");
        let mut params = Map::new();
        for day in 0..7 {
            params.insert(format!("day_{day}_start_time"), serde_json::json!(-1));
            params.insert(format!("day_{day}_end_time"), serde_json::json!("1440"));
        }
        s.assign(&params).expect("castable");
        assert!(s.valid());
        s.day_3_start_time = Some(1411);
        assert!(!s.valid());
        s.day_3_start_time = Some(1410);
        s.day_3_end_time = Some(1409);
        assert!(!s.valid(), "start after end");
        s.day_3_end_time = Some(1441);
        assert!(!s.valid());
        s.day_3_end_time = Some(1410);
        s.enabled = None;
        assert!(!s.valid());
    }
}
