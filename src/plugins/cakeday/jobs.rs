//! The plugin's Jobs::Onceoff: FixInvalidDateOfBirth and
//! MigrateDateOfBirthToUsersTable, from when the date of birth was a user
//! custom field. Each runs once (an `onceoff_logs` row by its name) unless
//! forced.

use chrono::NaiveDate;
use serde_json::Value;
use sqlx::PgConnection;

use crate::Unsupported;

/// The jobs' Sidekiq names.
pub const FIX_INVALID_DATE_OF_BIRTH: &str = "fix_invalid_date_of_birth";
pub const MIGRATE_DATE_OF_BIRTH: &str = "migrate_date_of_birth_to_users_table";

/// Runs one of the jobs by its name, in a transaction.
pub async fn run(state: &crate::AppState, name: &str, args: &Value) -> Result<(), crate::AppError> {
    let mut tx = state.pool.begin().await?;
    match name {
        FIX_INVALID_DATE_OF_BIRTH => fix_invalid_date_of_birth(&mut tx, args).await?,
        MIGRATE_DATE_OF_BIRTH => migrate_date_of_birth_to_users_table(&mut tx, args).await?,
        _ => return Err(Unsupported("an unknown discourse-cakeday job").into()),
    }
    tx.commit().await?;
    Ok(())
}

/// `Date.parse(value)` for a stored date of birth: Ok(None) where Ruby
/// raises ArgumentError. A string naming only part of a date, which Ruby
/// completes from today, is refused.
fn date_parse(value: &str) -> Result<Option<NaiveDate>, Unsupported> {
    if !value.is_ascii() {
        return Err(Unsupported(
            "a date of birth custom field with non-ASCII text",
        ));
    }
    let h = crate::ruby_date::parse(value);
    match (h.year, h.mon, h.mday) {
        (Some(y), Some(m), Some(d)) => Ok(i32::try_from(y).ok().and_then(|y| {
            NaiveDate::from_ymd_opt(y, u32::try_from(m).ok()?, u32::try_from(d).ok()?)
        })),
        (None, None, None) if h.yday.is_none() && h.cwyear.is_none() && h.wday.is_none() => {
            Ok(None)
        }
        _ => Err(Unsupported(
            "a date of birth custom field naming part of a date",
        )),
    }
}

/// Jobs::Onceoff#execute: false when it already ran (and isn't forced).
async fn start(conn: &mut PgConnection, name: &str, args: &Value) -> Result<bool, sqlx::Error> {
    if args["force"].as_bool() == Some(true) {
        return Ok(true);
    }
    let ran: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM onceoff_logs WHERE job_name = $1)")
            .bind(name)
            .fetch_one(conn)
            .await?;
    Ok(!ran)
}

/// `OnceoffLog.create!(job_name:)`
async fn log(conn: &mut PgConnection, name: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO onceoff_logs (job_name, created_at, updated_at) VALUES ($1, now(), now())",
    )
    .bind(name)
    .execute(conn)
    .await?;
    Ok(())
}

/// FixInvalidDateOfBirth: a date of birth custom field Ruby can't parse
/// is blanked.
pub async fn fix_invalid_date_of_birth(
    conn: &mut PgConnection,
    args: &Value,
) -> Result<(), crate::AppError> {
    if !start(&mut *conn, "FixInvalidDateOfBirth", args).await? {
        return Ok(());
    }
    let fields: Vec<(i32, String)> = sqlx::query_as(
        "SELECT id, value FROM user_custom_fields \
         WHERE name = 'date_of_birth' AND value != '' ORDER BY id",
    )
    .fetch_all(&mut *conn)
    .await?;
    for (id, value) in fields {
        if date_parse(&value)?.is_none() {
            sqlx::query(
                "UPDATE user_custom_fields SET value = '', updated_at = now() WHERE id = $1",
            )
            .bind(id)
            .execute(&mut *conn)
            .await?;
        }
    }
    log(&mut *conn, "FixInvalidDateOfBirth").await?;
    Ok(())
}

/// MigrateDateOfBirthToUsersTable: each date of birth custom field with a
/// value sets the user's date_of_birth (none when it isn't a date), and
/// one without is deleted.
pub async fn migrate_date_of_birth_to_users_table(
    conn: &mut PgConnection,
    args: &Value,
) -> Result<(), crate::AppError> {
    if !start(&mut *conn, "MigrateDateOfBirthToUsersTable", args).await? {
        return Ok(());
    }
    let fields: Vec<(i32, i32, Option<String>)> = sqlx::query_as(
        "SELECT id, user_id, value FROM user_custom_fields WHERE name = 'date_of_birth' ORDER BY id",
    )
    .fetch_all(&mut *conn)
    .await?;
    for (id, user_id, value) in fields {
        match value.filter(|v| !crate::ruby::is_blank(v)) {
            Some(value) => {
                let date = date_parse(&value)?;
                // user.update!: saved, with updated_at, only on a change.
                sqlx::query(
                    "UPDATE users SET date_of_birth = $2, updated_at = now() \
                     WHERE id = $1 AND date_of_birth IS DISTINCT FROM $2",
                )
                .bind(user_id)
                .bind(date)
                .execute(&mut *conn)
                .await?;
            }
            None => {
                sqlx::query("DELETE FROM user_custom_fields WHERE id = $1")
                    .bind(id)
                    .execute(&mut *conn)
                    .await?;
            }
        }
    }
    log(&mut *conn, "MigrateDateOfBirthToUsersTable").await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_parse_as_ruby_does() {
        let d = |y, m, day| Some(NaiveDate::from_ymd_opt(y, m, day).unwrap());
        assert_eq!(date_parse("1904-05-02").unwrap(), d(1904, 5, 2));
        assert_eq!(date_parse("02-05-1904").unwrap(), d(1904, 5, 2));
        assert_eq!(date_parse("02/05/1904").unwrap(), d(1904, 5, 2));
        assert_eq!(date_parse("02/13/1904").unwrap(), None);
        assert_eq!(date_parse("nonsense").unwrap(), None);
    }
}
