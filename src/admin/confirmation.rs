//! `AdminConfirmation` (lib/admin_confirmation.rb): granting admin to a
//! user when the granting admin has no second factor goes through an
//! email to that admin. The token and its payload are Redis keys with a
//! three hour TTL in Rails; here rows of `discourse_rs.cached_values`.

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::AppError;
use crate::i18n::I18n;

/// `3.hours`
const TTL_SECONDS: i64 = 3 * 3600;

/// A pending confirmation, found by its token.
pub struct Confirmation {
    pub token: String,
    pub target_user_id: i32,
    pub performed_by: i32,
}

/// `Guardian.new(by).can_grant_admin?(target)`: an admin, on a real user
/// other than themselves who is not an admin yet.
pub async fn can_grant_admin(
    conn: &mut PgConnection,
    by: i32,
    target: i32,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COALESCE((SELECT admin FROM users WHERE id = $1), FALSE) \
            AND $2 > 0 AND $1 <> $2 \
            AND NOT COALESCE((SELECT admin FROM users WHERE id = $2), TRUE)",
    )
    .bind(by)
    .bind(target)
    .fetch_one(conn)
    .await
}

/// Redis' SETEX.
async fn setex(conn: &mut PgConnection, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO discourse_rs.cached_values (key, value, expires_at) \
         VALUES ($1, $2, clock_timestamp() + make_interval(secs => $3)) \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, expires_at = EXCLUDED.expires_at",
    )
    .bind(key)
    .bind(value)
    .bind(TTL_SECONDS as f64)
    .execute(conn)
    .await?;
    Ok(())
}

/// `AdminConfirmation#create_confirmation` once the guardian allowed it:
/// the token, then the email to the granting admin.
pub async fn create(
    conn: &mut PgConnection,
    target_user_id: i32,
    performed_by: i32,
) -> Result<(), AppError> {
    let token = crate::accounts::random_hex();
    setex(
        conn,
        &format!("admin-confirmation:{target_user_id}"),
        &token,
    )
    .await?;
    let payload = json!({ "target_user_id": target_user_id, "performed_by": performed_by });
    setex(
        conn,
        &format!("admin-confirmation-token:{token}"),
        &payload.to_string(),
    )
    .await?;
    let email = |id: i32| {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\" LIMIT 1) \
             FROM users u WHERE u.id = $1",
        )
        .bind(id)
    };
    let to_address = email(performed_by).fetch_one(&mut *conn).await?;
    let target_email = email(target_user_id).fetch_one(&mut *conn).await?;
    let target_username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(target_user_id)
        .fetch_one(&mut *conn)
        .await?;
    crate::jobs::enqueue(
        conn,
        "admin_confirmation_email",
        json!({
            "to_address": to_address,
            "target_email": target_email,
            "target_username": target_username,
            "token": token,
        }),
    )
    .await?;
    Ok(())
}

/// `AdminConfirmation.find_by_code(token)`
pub async fn find_by_code(
    conn: &mut PgConnection,
    token: &str,
) -> Result<Option<Confirmation>, AppError> {
    let payload: Option<String> = sqlx::query_scalar(
        "SELECT value FROM discourse_rs.cached_values \
         WHERE key = $1 AND expires_at > clock_timestamp()",
    )
    .bind(format!("admin-confirmation-token:{token}"))
    .fetch_optional(&mut *conn)
    .await?;
    let Some(payload) = payload else {
        return Ok(None);
    };
    let parsed: Value = serde_json::from_str(&payload)
        .map_err(|_| crate::Unsupported("an unreadable admin confirmation"))?;
    let id = |k: &str| parsed[k].as_i64().and_then(|n| i32::try_from(n).ok());
    let (Some(target_user_id), Some(performed_by)) = (id("target_user_id"), id("performed_by"))
    else {
        return Err(crate::Unsupported("an unreadable admin confirmation").into());
    };
    // User.find: a user gone since raises RecordNotFound (404).
    let both: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE id IN ($1, $2)")
        .bind(target_user_id)
        .bind(performed_by)
        .fetch_one(&mut *conn)
        .await?;
    if both < 2 && target_user_id != performed_by {
        return Ok(None);
    }
    Ok(Some(Confirmation {
        token: token.to_string(),
        target_user_id,
        performed_by,
    }))
}

/// Granting admin was refused: `ensure_can_grant_admin!` raised.
pub struct Refused;

/// `AdminConfirmation#email_confirmed!`: the grant, logged by the admin
/// who asked for it, and both keys gone.
pub async fn email_confirmed(
    conn: &mut PgConnection,
    i18n: &I18n,
    c: &Confirmation,
) -> Result<Result<(), Refused>, AppError> {
    if !can_grant_admin(&mut *conn, c.performed_by, c.target_user_id).await? {
        return Ok(Err(Refused));
    }
    grant_admin(conn, i18n, c.performed_by, c.target_user_id).await?;
    sqlx::query("DELETE FROM discourse_rs.cached_values WHERE key = $1 OR key = $2")
        .bind(format!("admin-confirmation:{}", c.target_user_id))
        .bind(format!("admin-confirmation-token:{}", c.token))
        .execute(&mut *conn)
        .await?;
    Ok(Ok(()))
}

/// `user.grant_admin!` then `StaffActionLogger#log_grant_admin`.
pub async fn grant_admin(
    conn: &mut PgConnection,
    i18n: &I18n,
    acting_user_id: i32,
    target_user_id: i32,
) -> Result<(), AppError> {
    let admin: bool = sqlx::query_scalar("SELECT admin FROM users WHERE id = $1")
        .bind(target_user_id)
        .fetch_one(&mut *conn)
        .await?;
    if !admin {
        crate::roles::grant(conn, i18n, target_user_id, crate::roles::Permission::Admin).await?;
    }
    crate::roles::log(
        conn,
        acting_user_id,
        target_user_id,
        crate::roles::GRANT_ADMIN,
    )
    .await
}
