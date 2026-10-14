//! Admins acting as another user: `Guardian#can_impersonate?`,
//! `is_developer?`, and the token columns `start_impersonating_user` and
//! `stop_impersonating_user` write (DefaultCurrentUserProvider).

use sqlx::PgConnection;

use crate::config::GlobalSettings;

/// GlobalSetting.allow_impersonation, true by default.
pub fn allowed(globals: &GlobalSettings) -> bool {
    globals
        .get("allow_impersonation")
        .is_none_or(|v| !matches!(v, "false" | "0" | ""))
}

/// `is_developer?` for a user: in development, a Developer row (for an
/// admin), or one of the developer emails.
pub async fn is_developer(
    conn: &mut PgConnection,
    globals: &GlobalSettings,
    development: bool,
    user_id: i32,
    admin: bool,
) -> Result<bool, sqlx::Error> {
    if development {
        return Ok(true);
    }
    if admin {
        let developer: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM developers WHERE user_id = $1)")
                .bind(user_id)
                .fetch_one(&mut *conn)
                .await?;
        if developer {
            return Ok(true);
        }
    }
    if let Some(emails) = globals.get("developer_emails").filter(|e| !e.is_empty()) {
        let email: Option<String> =
            sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"")
                .bind(user_id)
                .fetch_optional(&mut *conn)
                .await?;
        return Ok(email.is_some_and(|e| emails.split(',').any(|d| d.trim() == e)));
    }
    Ok(false)
}

/// `can_impersonate?(target)`: impersonation allowed, an admin acting,
/// and a target who is not an admin unless the actor is a developer.
pub async fn can_impersonate(
    conn: &mut PgConnection,
    globals: &GlobalSettings,
    development: bool,
    actor_id: i32,
    actor_admin: bool,
    target_admin: bool,
) -> Result<bool, sqlx::Error> {
    if !allowed(globals) || !actor_admin {
        return Ok(false);
    }
    Ok(!target_admin || is_developer(conn, globals, development, actor_id, actor_admin).await?)
}
