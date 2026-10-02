//! Background jobs: Sidekiq's role, on Postgres.
//!
//! Jobs live in `discourse_rs.jobs`, the one table the port owns. It is
//! kept in its own schema so Rails never sees it, and created at startup
//! (`migrate`). Enqueueing is an insert, so a job enqueued inside a
//! transaction exists exactly when the rows it is about do. Workers claim
//! due jobs with `FOR UPDATE SKIP LOCKED`, run them, and delete them;
//! failures are retried with Sidekiq's backoff, and a job whose kind is not
//! ported fails at once with the reason kept on the row.

mod handlers;
mod post_alert;

use std::time::Duration;

use serde_json::Value;
use sqlx::{PgConnection, PgPool};

use crate::{AppError, AppState};

/// Sidekiq's default retry count.
const MAX_ATTEMPTS: i32 = 25;
/// How long a claimed job stays locked before another worker may take it.
const LOCK_SECONDS: i64 = 300;

/// Creates the queue's schema and table when missing.
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut conn = pool.acquire().await?;
    for statement in [
        "CREATE SCHEMA IF NOT EXISTS discourse_rs",
        "CREATE TABLE IF NOT EXISTS discourse_rs.jobs ( \
           id bigserial PRIMARY KEY, \
           name text NOT NULL, \
           args jsonb NOT NULL, \
           run_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'utc'), \
           attempts integer NOT NULL DEFAULT 0, \
           locked_until timestamp, \
           last_error text, \
           failed_at timestamp, \
           created_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'utc'))",
        "CREATE INDEX IF NOT EXISTS jobs_due ON discourse_rs.jobs (run_at, id) WHERE failed_at IS NULL",
    ] {
        sqlx::query(statement).execute(&mut *conn).await?;
    }
    Ok(())
}

/// `Jobs.enqueue(name, args)`
pub async fn enqueue(conn: &mut PgConnection, name: &str, args: Value) -> Result<(), sqlx::Error> {
    enqueue_in(conn, 0, name, args).await
}

/// `Jobs.enqueue_in(seconds, name, args)`
pub async fn enqueue_in(
    conn: &mut PgConnection,
    seconds: i64,
    name: &str,
    args: Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO discourse_rs.jobs (name, args, run_at, created_at) \
         VALUES ($1, $2, clock_timestamp() + make_interval(secs => $3), clock_timestamp())",
    )
    .bind(name)
    .bind(args)
    .bind(seconds as f64)
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

/// A worker: drains the queue, then polls once a second, until `stop`
/// resolves.
pub async fn work(state: AppState, stop: impl std::future::Future<Output = ()>) {
    tokio::pin!(stop);
    loop {
        if let Err(e) = drain(&state).await {
            tracing::error!("job queue: {e}");
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
