//! Background jobs: Sidekiq's role, on Postgres.
//!
//! Jobs live in `discourse_rs.jobs`, the one table the port owns. It is
//! kept in its own schema so Rails never sees it, and created at startup
//! (`migrate`). Enqueueing is an insert, so a job enqueued inside a
//! transaction exists exactly when the rows it is about do. Workers claim
//! due jobs with `FOR UPDATE SKIP LOCKED`, run them, and delete them;
//! failures are retried with Sidekiq's backoff, and a job whose kind is not
//! ported fails at once with the reason kept on the row.

pub mod accounts;
pub mod bookmark_reminders;
pub mod cleanup;
pub mod consistency;
pub mod group_messages;
mod handlers;
pub mod heat;
pub mod housekeeping;
pub mod list_stats;
pub mod moderation;
pub mod periodical;
pub mod scores;
pub mod search_jobs;
pub mod upload_cleanup;
pub mod upload_jobs;
pub(crate) use handlers::choose_featured_users;
pub(crate) mod post_alert;
mod user_email;

use std::time::Duration;

use serde_json::Value;
use sqlx::{PgConnection, PgPool};

use crate::{AppError, AppState};

/// Sidekiq's default retry count.
const MAX_ATTEMPTS: i32 = 25;
/// How long a claimed job stays locked before another worker may take it.
const LOCK_SECONDS: i64 = 300;

/// `Jobs.enqueue(name, args)`
pub async fn enqueue(conn: &mut PgConnection, name: &str, args: Value) -> Result<(), sqlx::Error> {
    insert(conn, 0, false, name, args).await
}

/// `Jobs.enqueue_in(seconds, name, args)`, due after the delay (marked
/// delayed even for none, as Rails' scheduled set has it).
pub async fn enqueue_in(
    conn: &mut PgConnection,
    seconds: i64,
    name: &str,
    args: Value,
) -> Result<(), sqlx::Error> {
    insert(conn, seconds, true, name, args).await
}

async fn insert(
    conn: &mut PgConnection,
    seconds: i64,
    delayed: bool,
    name: &str,
    args: Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO discourse_rs.jobs (name, args, run_at, created_at, delayed) \
         SELECT $1, $2, now + make_interval(secs => $3), now, $4 FROM clock_timestamp() AS now",
    )
    .bind(name)
    .bind(args)
    .bind(seconds as f64)
    .bind(delayed)
    .execute(conn)
    .await?;
    Ok(())
}

/// `Jobs.enqueue_at(time, name, args)`: due at `at` (now, if it has passed).
pub async fn enqueue_at(
    conn: &mut PgConnection,
    at: chrono::NaiveDateTime,
    name: &str,
    args: Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO discourse_rs.jobs (name, args, run_at, created_at, at_time) \
         SELECT $1, $2, GREATEST($3, now), now, TRUE FROM clock_timestamp() AS now",
    )
    .bind(name)
    .bind(args)
    .bind(at)
    .execute(conn)
    .await?;
    Ok(())
}

/// A claimed job.
#[derive(Debug, sqlx::FromRow)]
pub struct Job {
    pub id: i64,
    pub name: String,
    pub args: Value,
    pub attempts: i32,
}

/// Why a job did not complete.
#[derive(Debug)]
pub enum JobError {
    /// The job (or a case it hit) is not ported: failed without retries.
    Unported(String),
    /// Anything else: retried.
    Failed(String),
}

impl From<AppError> for JobError {
    fn from(e: AppError) -> Self {
        if e.is_unsupported() {
            JobError::Unported(e.to_string())
        } else {
            JobError::Failed(e.to_string())
        }
    }
}

/// Claims the next due job, if any.
pub async fn claim(pool: &PgPool) -> Result<Option<Job>, sqlx::Error> {
    sqlx::query_as(
        "UPDATE discourse_rs.jobs SET locked_until = clock_timestamp() + make_interval(secs => $1), \
                attempts = attempts + 1 \
         WHERE id = (SELECT id FROM discourse_rs.jobs \
                     WHERE failed_at IS NULL AND run_at <= clock_timestamp() \
                       AND (locked_until IS NULL OR locked_until < clock_timestamp()) \
                     ORDER BY run_at, id FOR UPDATE SKIP LOCKED LIMIT 1) \
         RETURNING id, name, args, attempts",
    )
    .bind(LOCK_SECONDS as f64)
    .fetch_optional(pool)
    .await
}

/// Runs a claimed job and records how it ended.
pub async fn perform(state: &AppState, job: Job) -> Result<(), sqlx::Error> {
    let result = handlers::run(state, &job).await;
    let pool = &state.pool;
    match result {
        Ok(()) => {
            sqlx::query("DELETE FROM discourse_rs.jobs WHERE id = $1")
                .bind(job.id)
                .execute(pool)
                .await?;
        }
        Err(JobError::Unported(reason)) => {
            tracing::warn!(job = %job.name, id = job.id, %reason, "job not ported");
            fail(pool, job.id, &reason).await?;
        }
        Err(JobError::Failed(reason)) if job.attempts >= MAX_ATTEMPTS => {
            tracing::error!(job = %job.name, id = job.id, %reason, "job failed for good");
            fail(pool, job.id, &reason).await?;
        }
        Err(JobError::Failed(reason)) => {
            // Sidekiq's backoff: attempts^4 + 15 seconds (its random jitter
            // left out).
            let delay = i64::from(job.attempts).pow(4) + 15;
            tracing::warn!(job = %job.name, id = job.id, %reason, delay, "job failed, retrying");
            sqlx::query(
                "UPDATE discourse_rs.jobs SET locked_until = NULL, last_error = $2, \
                        run_at = clock_timestamp() + make_interval(secs => $3) WHERE id = $1",
            )
            .bind(job.id)
            .bind(&reason)
            .bind(delay as f64)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

async fn fail(pool: &PgPool, id: i64, reason: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE discourse_rs.jobs SET locked_until = NULL, last_error = $2, failed_at = clock_timestamp() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}

/// Runs every due job until none is left; the number run.
pub async fn drain(state: &AppState) -> Result<usize, sqlx::Error> {
    let mut count = 0;
    while let Some(job) = claim(&state.pool).await? {
        perform(state, job).await?;
        count += 1;
    }
    Ok(count)
}

/// Jobs::Scheduled the port runs: the name and how often. They run when
/// the worker starts and then on their interval; with one worker per site
/// there is no lock between workers as MiniScheduler keeps.
const SCHEDULED: &[(&str, Duration)] = &[
    ("regenerate_sitemaps", Duration::from_secs(3600)),
    (
        crate::plugins::chat::summary_email::EMAIL_NOTIFICATIONS_JOB,
        Duration::from_secs(300),
    ),
    // Jobs::PresenceChannelAutoLeave, every DEFAULT_TIMEOUT.
    ("presence_channel_auto_leave", Duration::from_secs(60)),
    (bookmark_reminders::JOB, Duration::from_secs(300)),
    (
        crate::do_not_disturb::PROCESS_SHELVED_NOTIFICATIONS,
        Duration::from_secs(300),
    ),
    (
        crate::notification_schedule::PROCESS_USER_NOTIFICATION_SCHEDULES,
        Duration::from_secs(24 * 3600),
    ),
    (housekeeping::UNSILENCE_USERS, Duration::from_secs(15 * 60)),
    (
        housekeeping::PURGE_EXPIRED_IGNORED_USERS,
        Duration::from_secs(24 * 3600),
    ),
    (
        housekeeping::PURGE_OLD_NOTIFICATIONS,
        Duration::from_secs(7 * 24 * 3600),
    ),
    (list_stats::TOP_REFRESH_TODAY, Duration::from_secs(3600)),
    (
        list_stats::TOP_REFRESH_OLDER,
        Duration::from_secs(24 * 3600),
    ),
    (
        list_stats::UPDATE_TOPIC_HOT_SCORES,
        Duration::from_secs(10 * 60),
    ),
    (list_stats::CATEGORY_STATS, Duration::from_secs(24 * 3600)),
    (
        crate::plugins::chat::scheduled::PERIODICAL_UPDATES,
        Duration::from_secs(15 * 60),
    ),
    (
        crate::plugins::chat::scheduled::AUTO_JOIN_USERS,
        Duration::from_secs(3600),
    ),
    // Rails runs it daily at midnight; here, daily from the worker's start.
    (
        crate::plugins::chat::scheduled::DELETE_OLD_MESSAGES,
        Duration::from_secs(24 * 3600),
    ),
    (
        crate::plugins::reactions::like_sync::JOB,
        Duration::from_secs(3600),
    ),
    (moderation::TL3_PROMOTIONS, Duration::from_secs(24 * 3600)),
    (
        group_messages::IGNORED_USERS_SUMMARY,
        Duration::from_secs(24 * 3600),
    ),
    (
        group_messages::PENDING_REVIEWABLES_REMINDER,
        Duration::from_secs(15 * 60),
    ),
    (
        moderation::REVIEWABLE_PRIORITIES,
        Duration::from_secs(24 * 3600),
    ),
    (
        moderation::AUTO_QUEUE_HANDLER,
        Duration::from_secs(24 * 3600),
    ),
    (
        moderation::DESTROY_OLD_DELETION_STUBS,
        Duration::from_secs(30 * 60),
    ),
    (
        moderation::DESTROY_OLD_HIDDEN_POSTS,
        Duration::from_secs(24 * 3600),
    ),
    (
        accounts::ACTIVATION_REMINDER_EMAILS,
        Duration::from_secs(2 * 3600),
    ),
    (
        accounts::INVALIDATE_INACTIVE_ADMINS,
        Duration::from_secs(24 * 3600),
    ),
    (
        accounts::CLEAN_UP_API_KEYS_MAX_LIFE,
        Duration::from_secs(24 * 3600),
    ),
    (
        accounts::CLEAN_UP_UNUSED_API_KEYS,
        Duration::from_secs(24 * 3600),
    ),
    (
        accounts::CLEAN_UP_EMAIL_TOKENS,
        Duration::from_secs(24 * 3600),
    ),
    (
        accounts::CLEAN_UP_EMAIL_LOGIN_CODES,
        Duration::from_secs(24 * 3600),
    ),
    (
        accounts::CLEAN_UP_EMAIL_CHANGE_REQUESTS,
        Duration::from_secs(24 * 3600),
    ),
    (
        accounts::CLEAN_UP_UNSUBSCRIBE_KEYS,
        Duration::from_secs(24 * 3600),
    ),
    (
        accounts::CLEAN_UP_POST_REPLY_KEYS,
        Duration::from_secs(24 * 3600),
    ),
    (
        accounts::CLEAN_UP_EMAIL_LOGS,
        Duration::from_secs(24 * 3600),
    ),
    (
        accounts::DELETE_REJECTED_EMAILS,
        Duration::from_secs(7 * 24 * 3600),
    ),
    (
        cleanup::CLEAN_DISMISSED_TOPIC_USERS,
        Duration::from_secs(24 * 3600),
    ),
    (cleanup::CLEAN_UP_DRAFTS, Duration::from_secs(7 * 24 * 3600)),
    (
        cleanup::CLEAN_UP_BOOKMARKS,
        Duration::from_secs(7 * 24 * 3600),
    ),
    (
        cleanup::CLEAN_UP_USER_AUTH_TOKENS,
        Duration::from_secs(7 * 24 * 3600),
    ),
    (
        cleanup::CLEAN_UP_SEARCH_LOGS,
        Duration::from_secs(7 * 24 * 3600),
    ),
    (cleanup::CLEAN_UP_TAGS, Duration::from_secs(24 * 3600)),
    (
        cleanup::CLEAN_UP_UNMATCHED_EMAILS,
        Duration::from_secs(24 * 3600),
    ),
    (
        cleanup::CLEAN_UP_UNMATCHED_IPS,
        Duration::from_secs(24 * 3600),
    ),
    (
        heat::UPDATE_HEAT_SETTINGS,
        Duration::from_secs(30 * 24 * 3600),
    ),
    (search_jobs::REINDEX_SEARCH, Duration::from_secs(2 * 3600)),
    (
        search_jobs::CREATE_RECENT_POST_SEARCH_INDEXES,
        Duration::from_secs(24 * 3600),
    ),
    (scores::CALCULATE_SCORES, Duration::from_secs(7 * 24 * 3600)),
    (periodical::PERIODICAL_UPDATES, Duration::from_secs(15 * 60)),
    (
        consistency::ENSURE_DB_CONSISTENCY,
        Duration::from_secs(12 * 3600),
    ),
    (
        upload_jobs::UPDATE_ANIMATED_UPLOADS,
        Duration::from_secs(3600),
    ),
    (
        upload_jobs::BACKFILL_DOMINANT_COLORS,
        Duration::from_secs(15 * 60),
    ),
    (upload_cleanup::CLEAN_UP_UPLOADS, Duration::from_secs(3600)),
    (
        upload_cleanup::PURGE_DELETED_UPLOADS,
        Duration::from_secs(24 * 3600),
    ),
    // The rate limiters' expired lists (Redis expires them itself).
    ("rate_limits_cleanup", Duration::from_secs(15 * 60)),
];

/// A SCHEDULED job's name: it runs from the queue too (a case enqueues
/// it, as Jobs.enqueue would).
pub fn is_scheduled(name: &str) -> bool {
    SCHEDULED.iter().any(|(n, _)| *n == name)
}

/// Runs one of the SCHEDULED jobs now.
pub async fn run_scheduled(state: &AppState, name: &str) -> Result<(), AppError> {
    match name {
        "regenerate_sitemaps" => crate::routes::regenerate_sitemaps(state).await,
        "presence_channel_auto_leave" => crate::presence::auto_leave_all(state).await,
        bookmark_reminders::JOB => bookmark_reminders::run(state).await,
        crate::do_not_disturb::PROCESS_SHELVED_NOTIFICATIONS => {
            crate::do_not_disturb::process_shelved_notifications(state).await
        }
        crate::notification_schedule::PROCESS_USER_NOTIFICATION_SCHEDULES => {
            crate::notification_schedule::process_user_notification_schedules(state).await
        }
        housekeeping::UNSILENCE_USERS => housekeeping::unsilence_users(state).await,
        housekeeping::PURGE_EXPIRED_IGNORED_USERS => {
            housekeeping::purge_expired_ignored_users(state).await
        }
        housekeeping::PURGE_OLD_NOTIFICATIONS => housekeeping::purge_old_notifications(state).await,
        list_stats::TOP_REFRESH_TODAY => list_stats::top_refresh_today(state).await,
        list_stats::TOP_REFRESH_OLDER => list_stats::top_refresh_older(state).await,
        list_stats::UPDATE_TOPIC_HOT_SCORES => list_stats::update_topic_hot_scores(state).await,
        list_stats::CATEGORY_STATS => list_stats::category_stats(state).await,
        crate::plugins::chat::scheduled::PERIODICAL_UPDATES => {
            crate::plugins::chat::scheduled::periodical_updates(state).await
        }
        crate::plugins::chat::scheduled::AUTO_JOIN_USERS => {
            crate::plugins::chat::scheduled::auto_join_users(state, &serde_json::Value::Null).await
        }
        crate::plugins::chat::scheduled::DELETE_OLD_MESSAGES => {
            crate::plugins::chat::scheduled::delete_old_messages(state).await
        }
        crate::plugins::reactions::like_sync::JOB => {
            crate::plugins::reactions::like_sync::run(state).await
        }
        crate::plugins::chat::summary_email::EMAIL_NOTIFICATIONS_JOB => {
            crate::plugins::chat::summary_email::send_unread_mentions_summary(state).await
        }
        moderation::TL3_PROMOTIONS => moderation::tl3_promotions(state).await,
        group_messages::IGNORED_USERS_SUMMARY => group_messages::ignored_users_summary(state).await,
        group_messages::PENDING_REVIEWABLES_REMINDER => {
            group_messages::pending_reviewables_reminder(state).await
        }
        moderation::REVIEWABLE_PRIORITIES => moderation::reviewable_priorities(state).await,
        moderation::AUTO_QUEUE_HANDLER => moderation::auto_queue_handler(state).await,
        moderation::DESTROY_OLD_DELETION_STUBS => {
            moderation::destroy_old_deletion_stubs(state).await
        }
        moderation::DESTROY_OLD_HIDDEN_POSTS => moderation::destroy_old_hidden_posts(state).await,
        accounts::ACTIVATION_REMINDER_EMAILS => accounts::activation_reminder_emails(state).await,
        accounts::INVALIDATE_INACTIVE_ADMINS => accounts::invalidate_inactive_admins(state).await,
        accounts::CLEAN_UP_API_KEYS_MAX_LIFE => accounts::clean_up_api_keys_max_life(state).await,
        accounts::CLEAN_UP_UNUSED_API_KEYS => accounts::clean_up_unused_api_keys(state).await,
        accounts::CLEAN_UP_EMAIL_TOKENS => accounts::clean_up_email_tokens(state).await,
        accounts::CLEAN_UP_EMAIL_LOGIN_CODES => accounts::clean_up_email_login_codes(state).await,
        accounts::CLEAN_UP_EMAIL_CHANGE_REQUESTS => {
            accounts::clean_up_email_change_requests(state).await
        }
        accounts::CLEAN_UP_UNSUBSCRIBE_KEYS => accounts::clean_up_unsubscribe_keys(state).await,
        accounts::CLEAN_UP_POST_REPLY_KEYS => accounts::clean_up_post_reply_keys(state).await,
        accounts::CLEAN_UP_EMAIL_LOGS => accounts::clean_up_email_logs(state).await,
        accounts::DELETE_REJECTED_EMAILS => accounts::delete_rejected_emails(state).await,
        cleanup::CLEAN_DISMISSED_TOPIC_USERS => cleanup::clean_dismissed_topic_users(state).await,
        cleanup::CLEAN_UP_DRAFTS => cleanup::clean_up_drafts(state).await,
        cleanup::CLEAN_UP_BOOKMARKS => cleanup::clean_up_bookmarks(state).await,
        cleanup::CLEAN_UP_USER_AUTH_TOKENS => cleanup::clean_up_user_auth_tokens(state).await,
        cleanup::CLEAN_UP_SEARCH_LOGS => cleanup::clean_up_search_logs(state).await,
        cleanup::CLEAN_UP_TAGS => cleanup::clean_up_tags(state).await,
        cleanup::CLEAN_UP_UNMATCHED_EMAILS => cleanup::clean_up_unmatched_emails(state).await,
        cleanup::CLEAN_UP_UNMATCHED_IPS => cleanup::clean_up_unmatched_ips(state).await,
        heat::UPDATE_HEAT_SETTINGS => heat::update_heat_settings(state).await,
        search_jobs::REINDEX_SEARCH => search_jobs::reindex_search(state).await,
        search_jobs::CREATE_RECENT_POST_SEARCH_INDEXES => {
            search_jobs::create_recent_post_search_indexes(state).await
        }
        scores::CALCULATE_SCORES => scores::calculate_scores(state).await,
        periodical::PERIODICAL_UPDATES => periodical::periodical_updates(state).await,
        consistency::ENSURE_DB_CONSISTENCY => consistency::ensure_db_consistency(state).await,
        upload_jobs::UPDATE_ANIMATED_UPLOADS => upload_jobs::update_animated_uploads(state).await,
        upload_jobs::BACKFILL_DOMINANT_COLORS => upload_jobs::backfill_dominant_colors(state).await,
        upload_cleanup::CLEAN_UP_UPLOADS => upload_cleanup::clean_up_uploads(state).await,
        upload_cleanup::PURGE_DELETED_UPLOADS => upload_cleanup::purge_deleted_uploads(state).await,
        "rate_limits_cleanup" => Ok(crate::rate_limiter::cleanup(&state.pool).await?),
        _ => unreachable!("every SCHEDULED job has a handler"),
    }
}

/// A worker: drains the queue, then polls once a second, until `stop`
/// resolves. Scheduled jobs run between polls when due.
pub async fn work(state: AppState, stop: impl std::future::Future<Output = ()>) {
    tokio::pin!(stop);
    let mut last_run: Vec<Option<std::time::Instant>> = vec![None; SCHEDULED.len()];
    loop {
        if let Err(e) = drain(&state).await {
            tracing::error!("job queue: {e}");
        }
        for (i, (name, every)) in SCHEDULED.iter().enumerate() {
            if last_run[i].is_some_and(|at| at.elapsed() < *every) {
                continue;
            }
            last_run[i] = Some(std::time::Instant::now());
            if let Err(e) = run_scheduled(&state, name).await {
                tracing::error!("scheduled job {name}: {e}");
            }
        }
        tokio::select! {
            _ = &mut stop => return,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
    }
}

/// Runs one job now, whether due or not (a recording runs every job its
/// requests enqueued, delayed ones too). Its row ends as `perform` leaves it.
pub async fn perform_now(state: &AppState, id: i64) -> Result<(), sqlx::Error> {
    let job: Option<Job> = sqlx::query_as(
        "UPDATE discourse_rs.jobs SET locked_until = clock_timestamp() + make_interval(secs => $2), \
                attempts = attempts + 1 \
         WHERE id = $1 AND failed_at IS NULL RETURNING id, name, args, attempts",
    )
    .bind(id)
    .bind(LOCK_SECONDS as f64)
    .fetch_optional(&state.pool)
    .await?;
    if let Some(job) = job {
        perform(state, job).await?;
    }
    Ok(())
}
