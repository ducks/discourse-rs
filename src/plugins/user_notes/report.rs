//! The `user_notes` report (`add_report("user_notes")`): every note of
//! the users whose first note falls in the period, with its user and
//! author, as a table.

use chrono::{NaiveDateTime, Timelike};
use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::AppError;
use crate::reports::{Cx, Report};

/// A time as ActiveRecord quotes it into a `?`: `to_fs(:db)`, with the
/// microseconds when there are any.
fn quoted_date(t: NaiveDateTime) -> String {
    let usec = t.nanosecond() / 1000;
    let mut s = t.format("%Y-%m-%d %H:%M:%S").to_string();
    if usec > 0 {
        s.push_str(&format!(".{usec:06}"));
    }
    s
}

/// `Time#as_json` of `Time.parse(note["created_at"])`: milliseconds, `Z`
/// for UTC as written, else the offset.
fn time_json(raw: &str) -> Option<String> {
    let t = super::parse_time(raw)?;
    let base = t.format("%Y-%m-%dT%H:%M:%S%.3f").to_string();
    Some(if raw.ends_with('Z') {
        format!("{base}Z")
    } else {
        format!("{base}{}", t.format("%:z"))
    })
}

#[derive(sqlx::FromRow)]
struct User {
    id: i32,
    username_lower: String,
    uploaded_avatar_id: Option<i32>,
}

async fn user(conn: &mut PgConnection, id: &Value) -> Result<Option<User>, AppError> {
    let id = match id {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => Some(crate::ruby::to_i(s)),
        _ => None,
    };
    let Some(id) = id.and_then(|i| i32::try_from(i).ok()) else {
        return Ok(None);
    };
    Ok(
        sqlx::query_as("SELECT id, username_lower, uploaded_avatar_id FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?,
    )
}

pub async fn fill(conn: &mut PgConnection, cx: &Cx<'_>, r: &mut Report) -> Result<(), AppError> {
    r.modes = vec!["table".into()];
    r.data = json!([]);
    r.labels = json!([
        {
            "type": "user",
            "properties": {"username": "username", "id": "user_id", "avatar": "user_avatar_template"},
            "title": cx.t("reports.user_notes.labels.user")?,
        },
        {
            "type": "user",
            "properties": {"username": "moderator_username", "id": "moderator_id", "avatar": "moderator_avatar_template"},
            "title": cx.t("reports.user_notes.labels.moderator")?,
        },
        {"type": "text", "property": "note", "title": cx.t("reports.user_notes.labels.note")?},
    ]);
    // The first note's created_at compared as text with the quoted dates,
    // as Rails does, and in the table's own order (no ORDER BY there).
    let values: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT value FROM plugin_store_rows WHERE plugin_name = 'user_notes' \
         AND value::json->0->>'created_at' >= $1 AND value::json->0->>'created_at' <= $2",
    )
    .bind(quoted_date(r.start_date))
    .bind(quoted_date(r.end_date))
    .fetch_all(&mut *conn)
    .await?;
    let mut data = Vec::new();
    for value in values.into_iter().flatten() {
        let Ok(Value::Array(notes)) = serde_json::from_str::<Value>(&value) else {
            // JSON.parse raises: the report errors.
            return Err(std::io::Error::other("user notes are not a JSON array").into());
        };
        for note in notes {
            let created_at = note["created_at"]
                .as_str()
                .and_then(time_json)
                .ok_or_else(|| std::io::Error::other("no time in this string"))?;
            let (Some(u), Some(m)) = (
                user(&mut *conn, &note["user_id"]).await?,
                user(&mut *conn, &note["created_by"]).await?,
            ) else {
                continue;
            };
            data.push(json!({
                "created_at": created_at,
                "user_id": u.id,
                "username": u.username_lower,
                "user_avatar_template": cx.avatar_template(u.id, &u.username_lower, u.uploaded_avatar_id)?,
                "moderator_id": m.id,
                "moderator_username": m.username_lower,
                "moderator_avatar_template": cx.avatar_template(m.id, &m.username_lower, m.uploaded_avatar_id)?,
                "note": note["raw"],
            }));
        }
    }
    r.data = Value::Array(data);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_quote_as_active_record_does() {
        let start = chrono::NaiveDate::from_ymd_opt(2026, 10, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        assert_eq!(quoted_date(start), "2026-10-01 00:00:00");
        let end =
            crate::reports::end_of_day(chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap());
        assert_eq!(quoted_date(end), "2026-10-28 23:59:59.999999");
    }

    #[test]
    fn times_keep_their_zone() {
        assert_eq!(
            time_json("2026-10-28T12:34:56.789Z").as_deref(),
            Some("2026-10-28T12:34:56.789Z")
        );
        assert_eq!(
            time_json("2026-10-28T12:34:56.789+00:00").as_deref(),
            Some("2026-10-28T12:34:56.789+00:00")
        );
    }
}
