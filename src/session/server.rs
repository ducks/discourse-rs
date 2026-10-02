//! `ServerSession` (lib/server_session.rb): short-lived per-browser values
//! (honeypots, password reset codes) under the `_forum_session` cookie's
//! server_session_id. Rails keeps them in Redis; here they are rows of
//! `discourse_rs.server_sessions` that read as absent once expired.

use serde_json::Value;
use sqlx::PgConnection;

/// `ServerSession.expiry`
pub const EXPIRY_SECONDS: i64 = 3600;

pub async fn get(
    conn: &mut PgConnection,
    session_id: &str,
    key: &str,
) -> Result<Option<Value>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT value FROM discourse_rs.server_sessions WHERE key = $1 AND expires_at > clock_timestamp()",
    )
    .bind(format!("{session_id}{key}"))
    .fetch_optional(conn)
    .await
}

pub async fn set(
    conn: &mut PgConnection,
    session_id: &str,
    key: &str,
    value: &Value,
    expires_seconds: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO discourse_rs.server_sessions (key, value, expires_at) \
         VALUES ($1, $2, clock_timestamp() + make_interval(secs => $3)) \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, expires_at = EXCLUDED.expires_at",
    )
    .bind(format!("{session_id}{key}"))
    .bind(value)
    .bind(expires_seconds as f64)
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn delete(
    conn: &mut PgConnection,
    session_id: &str,
    key: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM discourse_rs.server_sessions WHERE key = $1")
        .bind(format!("{session_id}{key}"))
        .execute(conn)
        .await?;
    Ok(())
}
