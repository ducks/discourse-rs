//! The tables discourse-rs owns, in their own `discourse_rs` schema so
//! Rails never sees them: the job queue (Sidekiq's role) and server
//! sessions (what Rails keeps in Redis). Created at startup.

use sqlx::PgPool;

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
        "CREATE TABLE IF NOT EXISTS discourse_rs.server_sessions ( \
           key text PRIMARY KEY, \
           value jsonb NOT NULL, \
           expires_at timestamp NOT NULL)",
    ] {
        sqlx::query(statement).execute(&mut *conn).await?;
    }
    Ok(())
}
