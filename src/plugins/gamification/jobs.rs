//! The plugin's jobs: the regular ones its controllers and events enqueue
//! (positions generated, refreshed, deleted, regenerated or brought up to
//! date, a leaderboard's or every leaderboard's scores recalculated) and
//! the scheduled UpdateScoresForToday (hourly) and UpdateScoresForTenDays
//! (daily).

use chrono::Duration;
use serde_json::{Value, json};

use super::scorables::{self, Since};
use super::{Leaderboard, cached_view};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// The regular jobs' Sidekiq names (enqueued by class).
pub const GENERATE_LEADERBOARD_POSITIONS: &str = "Jobs::GenerateLeaderboardPositions";
pub const REFRESH_LEADERBOARD_POSITIONS: &str = "Jobs::RefreshLeaderboardPositions";
pub const DELETE_LEADERBOARD_POSITIONS: &str = "Jobs::DeleteLeaderboardPositions";
pub const REGENERATE_LEADERBOARD_POSITIONS: &str = "Jobs::RegenerateLeaderboardPositions";
pub const UPDATE_STALE_LEADERBOARD_POSITIONS: &str = "Jobs::UpdateStaleLeaderboardPositions";
pub const RECALCULATE_LEADERBOARD_SCORES: &str = "Jobs::RecalculateLeaderboardScores";
pub const RECALCULATE_SCORES: &str = "Jobs::RecalculateScores";

/// The scheduled jobs.
pub const UPDATE_SCORES_FOR_TODAY: &str = "update_scores_for_today";
pub const UPDATE_SCORES_FOR_TEN_DAYS: &str = "update_scores_for_ten_days";

/// Every regular job's name, for the job runner.
pub const REGULAR: [&str; 7] = [
    GENERATE_LEADERBOARD_POSITIONS,
    REFRESH_LEADERBOARD_POSITIONS,
    DELETE_LEADERBOARD_POSITIONS,
    REGENERATE_LEADERBOARD_POSITIONS,
    UPDATE_STALE_LEADERBOARD_POSITIONS,
    RECALCULATE_LEADERBOARD_SCORES,
    RECALCULATE_SCORES,
];

/// `RateLimiter.new(nil, "recalculate_scores_remaining", 5, 24.hours)`
pub fn recalculate_limiter() -> crate::rate_limiter::RateLimiter {
    crate::rate_limiter::RateLimiter::new(None, "recalculate_scores_remaining", 5, 24 * 3600)
}

/// `args[:leaderboard_id]`, raising InvalidParameters when blank.
fn leaderboard_id(args: &Value) -> Result<i64, AppError> {
    match &args["leaderboard_id"] {
        Value::Number(n) => n
            .as_i64()
            .ok_or_else(|| Unsupported("a fractional leaderboard_id").into()),
        Value::String(s) if !s.trim().is_empty() => Ok(crate::ruby::to_i(s)),
        _ => Err(Unsupported("Discourse::InvalidParameters: leaderboard_id").into()),
    }
}

/// Runs one of the regular jobs.
pub async fn run(state: &AppState, name: &str, args: &Value) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    match name {
        // DistributedMutex per leaderboard in Rails; create is idempotent
        // (each view IF NOT EXISTS) and the port runs one worker per site.
        GENERATE_LEADERBOARD_POSITIONS => {
            let id = leaderboard_id(args)?;
            if Leaderboard::find(&mut conn, id).await?.is_some() {
                cached_view::create(&mut conn, &settings, id).await?;
            }
        }
        REFRESH_LEADERBOARD_POSITIONS => {
            let id = leaderboard_id(args)?;
            if Leaderboard::find(&mut conn, id).await?.is_some() {
                cached_view::refresh(&mut conn, id).await?;
            }
        }
        DELETE_LEADERBOARD_POSITIONS => {
            cached_view::delete(&mut conn, leaderboard_id(args)?).await?;
        }
        REGENERATE_LEADERBOARD_POSITIONS => {
            cached_view::regenerate_all(&mut conn, &settings).await?;
        }
        UPDATE_STALE_LEADERBOARD_POSITIONS => {
            cached_view::update_all(&mut conn, &settings).await?;
        }
        RECALCULATE_LEADERBOARD_SCORES => {
            let id = leaderboard_id(args)?;
            let Some(lb) = Leaderboard::find(&mut conn, id).await? else {
                return Ok(());
            };
            let since = Since::Date(
                lb.from_date
                    .unwrap_or(chrono::NaiveDate::from_ymd_opt(2000, 1, 1).expect("a date")),
            );
            scorables::calculate_scores(&mut conn, &settings, &lb, &since, None).await?;
            cached_view::delete(&mut conn, id).await?;
            cached_view::create(&mut conn, &settings, id).await?;
        }
        RECALCULATE_SCORES => recalculate_scores(state, &mut conn, &settings, args).await?,
        _ => return Err(Unsupported("an unknown discourse-gamification job").into()),
    }
    Ok(())
}

/// Jobs::RecalculateScores: every leaderboard's scores since the date
/// (ten days ago without one), every view rebuilt, and the admin who
/// asked told through /recalculate_scores.
async fn recalculate_scores(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    args: &Value,
) -> Result<(), AppError> {
    let user_id = match &args["user_id"] {
        Value::Null => return Err(Unsupported("Discourse::InvalidParameters: user_id").into()),
        Value::String(s) if s.trim().is_empty() => {
            return Err(Unsupported("Discourse::InvalidParameters: user_id").into());
        }
        v => v.clone(),
    };
    let since = match &args["since"] {
        Value::String(s) => Since::Text(s.clone()),
        _ => Since::Time(crate::clock::now_naive() - Duration::days(10)),
    };
    scorables::calculate_all(conn, settings, &since).await?;
    cached_view::regenerate_all(conn, settings).await?;
    let remaining = recalculate_limiter().remaining(&state.pool).await?;
    state
        .bus
        .publish(
            conn,
            "/recalculate_scores",
            &json!({ "success": true, "remaining": remaining, "user_id": [user_id] }),
            None,
        )
        .await?;
    Ok(())
}

/// Runs one of the scheduled jobs: nothing while the plugin is off.
pub async fn run_scheduled(state: &AppState, name: &str) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !super::enabled(&settings)? {
        return Ok(());
    }
    let now = crate::clock::now_naive();
    match name {
        UPDATE_SCORES_FOR_TODAY => {
            scorables::calculate_all(&mut conn, &settings, &Since::Date(now.date())).await?;
            cached_view::purge_all_stale(&mut conn, &settings).await?;
            cached_view::refresh_all(&mut conn).await?;
            cached_view::create_all(&mut conn, &settings).await?;
        }
        UPDATE_SCORES_FOR_TEN_DAYS => {
            let since = (now - Duration::days(10))
                .date()
                .and_hms_opt(0, 0, 0)
                .expect("midnight");
            scorables::calculate_all(&mut conn, &settings, &Since::Time(since)).await?;
        }
        _ => return Err(Unsupported("an unknown discourse-gamification scheduled job").into()),
    }
    Ok(())
}
