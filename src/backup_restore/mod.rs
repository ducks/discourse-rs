//! BackupRestore: the backups the admin lists, downloads, uploads and
//! deletes (the local store), and the state Rails keeps in Redis for them
//! (the running flag, the cancel signal, the emailed download tokens),
//! here in `discourse_rs.cached_values`.

pub mod jobs;
pub mod store;

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `BackupRestore::LOGS_CHANNEL`
pub const LOGS_CHANNEL: &str = "/admin/backups/logs";
/// The file list Jobs::BackupChunksMerger publishes to staff.
pub const FILES_CHANNEL: &str = "/admin/backups";

const RUNNING_KEY: &str = "backup_restore_operation_is_running";
const SHUTDOWN_KEY: &str = "backup_restore_operation_should_shutdown";
/// A key set without a TTL, as Redis's plain SET.
const FOREVER: i64 = 100 * 365 * 24 * 3600;

/// `BackupRestore.is_operation_running?`
pub async fn is_operation_running(conn: &mut PgConnection) -> Result<bool, sqlx::Error> {
    Ok(crate::owned_schema::cached_get(conn, RUNNING_KEY)
        .await?
        .is_some())
}

/// `mark_as_running!`'s SETEX 60 (and keep_it_running's EXPIRE): whoever
/// runs the operation sets it again before it lapses.
pub async fn mark_as_running(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    crate::owned_schema::cached_setex(conn, RUNNING_KEY, 60, "1").await
}

/// `mark_as_not_running!`
pub async fn mark_as_not_running(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    crate::owned_schema::cached_del(conn, RUNNING_KEY).await
}

/// `should_shutdown?`
pub async fn should_shutdown(conn: &mut PgConnection) -> Result<bool, sqlx::Error> {
    Ok(crate::owned_schema::cached_get(conn, SHUTDOWN_KEY)
        .await?
        .is_some())
}

/// `cancel!` (`set_shutdown_signal!`)
pub async fn set_shutdown_signal(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    crate::owned_schema::cached_setex(conn, SHUTDOWN_KEY, FOREVER, "1").await
}

/// `clear_shutdown_signal!`
pub async fn clear_shutdown_signal(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    crate::owned_schema::cached_del(conn, SHUTDOWN_KEY).await
}

/// `can_rollback?`: a table left in the `backup` schema by a restore.
pub async fn can_rollback(conn: &mut PgConnection) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'backup'",
    )
    .fetch_one(conn)
    .await?;
    Ok(count > 0)
}

/// `operations_status`; allow_restore is always true in development.
pub async fn operations_status(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
) -> Result<Value, AppError> {
    Ok(json!({
        "is_operation_running": is_operation_running(&mut *conn).await?,
        "can_rollback": can_rollback(&mut *conn).await?,
        "allow_restore": state.config.rails_env == crate::config::RailsEnv::Development
            || settings.get("allow_restore")?.truthy(),
    }))
}

/// EmailBackupToken: one download token per admin, a day long.
pub mod token {
    use sqlx::PgConnection;

    fn key(user_id: i32) -> String {
        format!("email-backup-token:{user_id}")
    }

    /// `EmailBackupToken.set`: a new SecureRandom.hex token.
    pub async fn set(conn: &mut PgConnection, user_id: i32) -> Result<String, sqlx::Error> {
        let token = crate::accounts::random_hex();
        crate::owned_schema::cached_setex(conn, &key(user_id), 24 * 3600, &token).await?;
        Ok(token)
    }

    /// `EmailBackupToken.compare`
    pub async fn compare(
        conn: &mut PgConnection,
        user_id: i32,
        token: &str,
    ) -> Result<bool, sqlx::Error> {
        Ok(crate::owned_schema::cached_get(conn, &key(user_id))
            .await?
            .is_some_and(|stored| stored == token))
    }

    /// `EmailBackupToken.del`
    pub async fn del(conn: &mut PgConnection, user_id: i32) -> Result<(), sqlx::Error> {
        crate::owned_schema::cached_del(conn, &key(user_id)).await
    }
}

/// The StaffActionLogger rows of the backups: backup_create,
/// backup_download and backup_destroy carry the admin's users.ip_address
/// (as text, "" without one), the latter two the filename.
pub async fn log_staff_action(
    conn: &mut PgConnection,
    action: &str,
    acting_user_id: i32,
    details: Option<&str>,
) -> Result<(), AppError> {
    let id = crate::staff_action_logs::action_id(action)
        .ok_or_else(|| std::io::Error::other(format!("no UserHistory action {action}")))?;
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, details, ip_address, admin_only, \
                                     created_at, updated_at) \
         SELECT $1, u.id, $3, COALESCE(host(u.ip_address), ''), TRUE, now(), now() \
         FROM users u WHERE u.id = $2",
    )
    .bind(id)
    .bind(acting_user_id)
    .bind(details)
    .execute(conn)
    .await?;
    Ok(())
}
