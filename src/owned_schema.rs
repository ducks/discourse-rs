//! The tables discourse-rs owns, in their own `discourse_rs` schema so
//! Rails never sees them: the job queue (Sidekiq's role), server sessions
//! and expiring keys (what Rails keeps in Redis). Created at startup.

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
        "CREATE TABLE IF NOT EXISTS discourse_rs.expiring_keys ( \
           key text PRIMARY KEY, \
           expires_at timestamp NOT NULL)",
    ] {
        sqlx::query(statement).execute(&mut *conn).await?;
    }
    Ok(())
}

/// Redis's `SETNX key` then `EXPIRE key seconds`: true when the key was
/// not set (or had expired), and now is.
pub async fn set_once(
    conn: &mut sqlx::PgConnection,
    key: &str,
    expires_seconds: i64,
) -> Result<bool, sqlx::Error> {
    sqlx::query(
        "DELETE FROM discourse_rs.expiring_keys WHERE key = $1 AND expires_at <= clock_timestamp()",
    )
    .bind(key)
    .execute(&mut *conn)
    .await?;
    let inserted = sqlx::query(
        "INSERT INTO discourse_rs.expiring_keys (key, expires_at) \
         VALUES ($1, clock_timestamp() + make_interval(secs => $2)) ON CONFLICT (key) DO NOTHING",
    )
    .bind(key)
    .bind(expires_seconds as f64)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(inserted == 1)
}
