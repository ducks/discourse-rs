//! Scheduled jobs that keep penalties and per-user records in bounds:
//! timed silences ending, expiring ignores expiring, notifications kept
//! to each user's cap.

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `Jobs::UnsilenceUsers`, every 15 minutes.
pub const UNSILENCE_USERS: &str = "unsilence_users";
/// `Jobs::PurgeExpiredIgnoredUsers`, daily.
pub const PURGE_EXPIRED_IGNORED_USERS: &str = "purge_expired_ignored_users";
/// `Jobs::PurgeOldNotifications`, weekly.
pub const PURGE_OLD_NOTIFICATIONS: &str = "purge_old_notifications";

/// `Discourse.system_user`
const SYSTEM_USER_ID: i32 = -1;

/// `Jobs::UnsilenceUsers`: each user whose silence has run out
/// (find_each, by id) unsilenced by the system user.
pub async fn unsilence_users(state: &AppState) -> Result<(), AppError> {
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM users WHERE silenced_till IS NOT NULL AND silenced_till < now() ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await?;
    if ids.is_empty() {
        return Ok(());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    drop(conn);
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    for id in ids {
        crate::user_penalties::lift_silence(&state.pool, &ctx, SYSTEM_USER_ID, id).await?;
    }
    Ok(())
}

/// `Jobs::PurgeExpiredIgnoredUsers`
pub async fn purge_expired_ignored_users(state: &AppState) -> Result<(), AppError> {
    sqlx::query("DELETE FROM ignored_users WHERE expiring_at <= now()")
        .execute(&state.pool)
        .await?;
    Ok(())
}

/// `Notification.purge_old!`: each user's notifications beyond the newest
/// max_notifications_per_user go (none when the cap is 0).
pub async fn purge_old_notifications(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let cap = settings.get("max_notifications_per_user")?.to_i();
    if cap == 0 {
        return Ok(());
    }
    sqlx::query(
        "DELETE FROM notifications n1 USING ( \
           SELECT * FROM ( \
             SELECT user_id, id, rank() OVER (PARTITION BY user_id ORDER BY id DESC) FROM notifications \
           ) AS x WHERE rank = $1 \
         ) n2 WHERE n1.user_id = n2.user_id AND n1.id < n2.id",
    )
    .bind(cap)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
