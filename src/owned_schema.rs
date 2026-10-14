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
        // BadgeGranter's `badge_queue` Redis list: `{type, post_ids |
        // user_ids}` payloads, popped newest first.
        "CREATE TABLE IF NOT EXISTS discourse_rs.badge_queue ( \
           id bigserial PRIMARY KEY, \
           payload jsonb NOT NULL)",
        // PresenceChannel's clients (its zlist and user hash in Redis): a
        // browser tab present in a channel until expires_at.
        "CREATE TABLE IF NOT EXISTS discourse_rs.presence_clients ( \
           channel text NOT NULL, \
           user_id integer NOT NULL, \
           client_id text NOT NULL, \
           entered_at timestamptz NOT NULL DEFAULT now(), \
           expires_at timestamptz NOT NULL, \
           PRIMARY KEY (channel, user_id, client_id))",
        "CREATE INDEX IF NOT EXISTS presence_clients_expiring ON discourse_rs.presence_clients (expires_at)",
        // RateLimiter's Redis lists: epoch-second stamps, newest first, at
        // most max; gone (as an EXPIRE) at the last push plus twice the
        // window. Unlogged, as Redis without persistence.
        "CREATE UNLOGGED TABLE IF NOT EXISTS discourse_rs.rate_limits ( \
           key text PRIMARY KEY, \
           stamps bigint[] NOT NULL, \
           expires_at bigint NOT NULL)",
        "CREATE INDEX IF NOT EXISTS rate_limits_expiring ON discourse_rs.rate_limits (expires_at)",
        // PERFORM_LUA (and _AGGRESSIVE): allowed when under max or the
        // oldest stamp is a window old; pushed when allowed (always when
        // aggressive) and trimmed to max. NULL when allowed, else the
        // seconds to wait, read after the push as Ruby reads them.
        "CREATE OR REPLACE FUNCTION discourse_rs.rate_limit_perform( \
           p_key text, p_now bigint, p_secs bigint, p_max int, p_aggressive bool) \
         RETURNS bigint LANGUAGE plpgsql AS $$ \
         DECLARE s bigint[]; n bigint[]; allowed bool; \
         BEGIN \
           INSERT INTO discourse_rs.rate_limits (key, stamps, expires_at) VALUES (p_key, '{}', 0) \
             ON CONFLICT (key) DO NOTHING; \
           SELECT CASE WHEN expires_at <= p_now THEN '{}' ELSE stamps END INTO s \
             FROM discourse_rs.rate_limits WHERE key = p_key FOR UPDATE; \
           allowed := cardinality(s) < p_max OR p_now - s[cardinality(s)] >= p_secs; \
           n := s; \
           IF allowed OR p_aggressive THEN \
             n := (ARRAY[p_now] || s)[1:p_max]; \
             UPDATE discourse_rs.rate_limits SET stamps = n, expires_at = p_now + p_secs * 2 \
               WHERE key = p_key; \
           END IF; \
           IF allowed THEN RETURN NULL; END IF; \
           RETURN p_secs - (p_now - COALESCE(n[cardinality(n)], 0)); \
         END $$",
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
