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
        // Jobs.enqueue_at: due at a point in time rather than after a delay.
        "ALTER TABLE discourse_rs.jobs ADD COLUMN IF NOT EXISTS at_time boolean NOT NULL DEFAULT FALSE",
        "CREATE TABLE IF NOT EXISTS discourse_rs.server_sessions ( \
           key text PRIMARY KEY, \
           value jsonb NOT NULL, \
           expires_at timestamp NOT NULL)",
        "CREATE TABLE IF NOT EXISTS discourse_rs.expiring_keys ( \
           key text PRIMARY KEY, \
           expires_at timestamp NOT NULL)",
        // Redis values with a TTL (SETEX), such as user-last-seen.
        "CREATE TABLE IF NOT EXISTS discourse_rs.cached_values ( \
           key text PRIMARY KEY, \
           value text NOT NULL, \
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

/// Redis's `GET key` for a value set with `setex`: None once it expired.
pub async fn cached_get(
    conn: &mut sqlx::PgConnection,
    key: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT value FROM discourse_rs.cached_values WHERE key = $1 AND expires_at > clock_timestamp()",
    )
    .bind(key)
    .fetch_optional(conn)
    .await
}

/// Redis's `SETEX key seconds value`.
pub async fn cached_setex(
    conn: &mut sqlx::PgConnection,
    key: &str,
    seconds: i64,
    value: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO discourse_rs.cached_values (key, value, expires_at) \
         VALUES ($1, $2, clock_timestamp() + make_interval(secs => $3)) \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, expires_at = EXCLUDED.expires_at",
    )
    .bind(key)
    .bind(value)
    .bind(seconds as f64)
    .execute(conn)
    .await?;
    Ok(())
}
