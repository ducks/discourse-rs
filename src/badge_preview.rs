//! `BadgeGranter.preview(sql, target_posts:, explain:, trigger:)`: a badge
//! query's contract checked, its grants counted, its first ten sampled
//! (each with an existing user and a grant time, and an existing post when
//! it targets posts) and optionally explained. Every failure is the
//! response's `errors`: the contract's text, the sample's checks, the
//! named parameters ActiveRecord could not bind, and Postgres errors as
//! libpq's PQerrorMessage writes them (with the LINE excerpt and caret).

use serde_json::{Map, Value, json};
use sqlx::postgres::{PgDatabaseError, PgErrorPosition, PgRow};
use sqlx::{Column, PgConnection, Row, TypeInfo, ValueRef};

use crate::admin_badges::contract_checks;
use crate::{AppError, Unsupported};

/// What a preview failed on: the message the response's `errors` shows.
#[derive(Debug)]
struct Failure(String);

/// ActiveRecord's `replace_named_bind_variables` with the preview's
/// params (`user_ids: []`, `post_ids: []`, `backfill: true`): each
/// `:name` (not a `::` cast, and `\:` an escaped colon) replaced by its
/// quoted value, or the PreparedStatementInvalid message for an unknown
/// one.
fn bind_named(statement: &str) -> Result<String, Failure> {
    let bytes = statement.as_bytes();
    let mut out = String::with_capacity(statement.len());
    let mut i = 0;
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    while i < bytes.len() {
        // /([:\\]?):([a-zA-Z]\w*)/ at i: with the prefix first.
        let prefixed = matches!(bytes[i], b':' | b'\\')
            && bytes.get(i + 1) == Some(&b':')
            && bytes.get(i + 2).is_some_and(u8::is_ascii_alphabetic);
        let plain = bytes[i] == b':' && bytes.get(i + 1).is_some_and(u8::is_ascii_alphabetic);
        if prefixed || plain {
            let name_start = if prefixed { i + 2 } else { i + 1 };
            let mut end = name_start;
            while end < bytes.len() && word(bytes[end]) {
                end += 1;
            }
            let name = &statement[name_start..end];
            if prefixed && bytes[i] == b':' {
                out.push_str(&statement[i..end]);
            } else if prefixed {
                out.push_str(&statement[i + 1..end]);
            } else {
                match name {
                    "user_ids" | "post_ids" => out.push_str("NULL"),
                    "backfill" => out.push_str("TRUE"),
                    _ => {
                        return Err(Failure(format!("missing value for :{name} in {statement}")));
                    }
                }
            }
            i = end;
            continue;
        }
        let c = statement[i..].chars().next().unwrap_or(' ');
        out.push(c);
        i += c.len_utf8();
    }
    Ok(out)
}

/// The count query, as Rails' heredoc lays it out.
fn count_sql(sql: &str) -> String {
    format!(
        "SELECT COUNT(*) count\n           FROM (\n                  {sql}\n                ) q\n          WHERE :backfill = :backfill\n"
    )
}

/// The sample query, as Rails' heredocs lay them out.
fn grants_sql(sql: &str, target_posts: bool) -> String {
    if target_posts {
        format!(
            "   SELECT u.id, u.username, q.post_id, t.title, q.granted_at\n     FROM (\n            {sql}\n          ) q\n     JOIN users u on u.id = q.user_id\nLEFT JOIN badge_posts p on p.id = q.post_id\nLEFT JOIN topics t on t.id = p.topic_id\n    WHERE :backfill = :backfill\n    LIMIT 10\n"
        )
    } else {
        format!(
            "SELECT u.id, u.username, q.granted_at\n FROM (\n        {sql}\n      ) q\n JOIN users u on u.id = q.user_id\nWHERE :backfill = :backfill\nLIMIT 10\n"
        )
    }
}

/// `pg_encoding_dsplen` for UTF-8 up to U+02FF (the port refuses
/// positions in lines with wider scripts): control characters count as
/// one column, like libpq treats their -1.
fn display_width(c: char) -> Result<usize, AppError> {
    if (c as u32) < 0x300 {
        Ok(1)
    } else {
        Err(Unsupported("badge query error positions past Latin characters").into())
    }
}

/// libpq's `reportErrorPosition` (fe-protocol3.c): the line of `query`
/// holding the 1-based character `loc`, cut to 60 screen columns around
/// it, and a caret under it.
fn report_error_position(msg: &mut String, query: &str, loc: usize) -> Result<(), AppError> {
    const DISPLAY_SIZE: usize = 60;
    const MIN_RIGHT_CUT: usize = 10;
    let Some(loc) = loc.checked_sub(1) else {
        return Ok(());
    };
    // Tabs become spaces in the working copy.
    let chars: Vec<char> = query
        .chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .collect();
    let mut scridx: Vec<usize> = Vec::with_capacity(chars.len() + 1);
    let mut scroffset = 0usize;
    let mut loc_line = 1;
    let mut ibeg = 0usize;
    let mut iend: Option<usize> = None;
    let mut cno = 0;
    while cno < chars.len() {
        let ch = chars[cno];
        scridx.push(scroffset);
        if ch == '\r' || ch == '\n' {
            if cno < loc {
                if ch == '\r' || cno == 0 || chars[cno - 1] != '\r' {
                    loc_line += 1;
                }
                ibeg = cno + 1;
            } else {
                iend = Some(cno);
                break;
            }
        }
        scroffset += display_width(ch)?;
        cno += 1;
    }
    let mut iend = match iend {
        Some(i) => i,
        None => {
            scridx.push(scroffset);
            cno
        }
    };
    if loc > cno {
        return Ok(());
    }
    let mut ibeg_mut = ibeg;
    let mut beg_trunc = false;
    let mut end_trunc = false;
    if scridx[iend] - scridx[ibeg_mut] > DISPLAY_SIZE {
        if scridx[ibeg_mut] + DISPLAY_SIZE >= scridx[loc] + MIN_RIGHT_CUT {
            while scridx[iend] - scridx[ibeg_mut] > DISPLAY_SIZE {
                iend -= 1;
            }
            end_trunc = true;
        } else {
            while scridx[loc] + MIN_RIGHT_CUT < scridx[iend] {
                iend -= 1;
                end_trunc = true;
            }
            while scridx[iend] - scridx[ibeg_mut] > DISPLAY_SIZE {
                ibeg_mut += 1;
                beg_trunc = true;
            }
        }
    }
    let mut prefix = format!("LINE {loc_line}: ");
    if beg_trunc {
        prefix.push_str("...");
    }
    let mut caret = prefix.chars().count();
    msg.push_str(&prefix);
    msg.extend(&chars[ibeg_mut..iend]);
    if end_trunc {
        msg.push_str("...");
    }
    msg.push('\n');
    caret += scridx[loc] - scridx[ibeg_mut];
    msg.push_str(&" ".repeat(caret));
    msg.push_str("^\n");
    Ok(())
}

/// libpq's PQerrorMessage of a failed query (`pqBuildErrorMessage3`, the
/// default verbosity and context shown for errors).
fn libpq_message(e: &PgDatabaseError, query: &str) -> Result<String, AppError> {
    use sqlx::postgres::PgSeverity;
    let severity = match e.severity() {
        PgSeverity::Panic => "PANIC",
        PgSeverity::Fatal => "FATAL",
        PgSeverity::Error => "ERROR",
        PgSeverity::Warning => "WARNING",
        PgSeverity::Notice => "NOTICE",
        PgSeverity::Debug => "DEBUG",
        PgSeverity::Info => "INFO",
        PgSeverity::Log => "LOG",
    };
    let mut msg = format!("{severity}:  {}\n", e.message());
    let mut internal_query = None;
    match e.position() {
        Some(PgErrorPosition::Original(pos)) => report_error_position(&mut msg, query, pos)?,
        Some(PgErrorPosition::Internal { position, query }) => {
            report_error_position(&mut msg, query, position)?;
            internal_query = Some(query.to_string());
        }
        None => {}
    }
    if let Some(d) = e.detail() {
        msg.push_str(&format!("DETAIL:  {d}\n"));
    }
    if let Some(h) = e.hint() {
        msg.push_str(&format!("HINT:  {h}\n"));
    }
    if let Some(q) = internal_query {
        msg.push_str(&format!("QUERY:  {q}\n"));
    }
    if let Some(c) = e.r#where() {
        msg.push_str(&format!("CONTEXT:  {c}\n"));
    }
    Ok(msg)
}

/// A query run as mini_sql runs it (not prepared for reuse); a Postgres
/// error is the failure libpq would report.
async fn run(conn: &mut PgConnection, sql: &str) -> Result<Result<Vec<PgRow>, Failure>, AppError> {
    match sqlx::query(sql).persistent(false).fetch_all(conn).await {
        Ok(rows) => Ok(Ok(rows)),
        Err(sqlx::Error::Database(db)) => match db.try_downcast_ref::<PgDatabaseError>() {
            Some(pg) => Ok(Err(Failure(libpq_message(pg, sql)?))),
            None => Err(sqlx::Error::Database(db).into()),
        },
        Err(e) => Err(e.into()),
    }
}

/// A column as mini_sql decodes it and Rails writes it as JSON.
fn column_json(row: &PgRow, i: usize) -> Result<Value, AppError> {
    let raw = row.try_get_raw(i)?;
    if raw.is_null() {
        return Ok(Value::Null);
    }
    let type_name = raw.type_info().name().to_string();
    Ok(match type_name.as_str() {
        "INT2" => json!(row.try_get::<i16, _>(i)?),
        "INT4" => json!(row.try_get::<i32, _>(i)?),
        "INT8" => json!(row.try_get::<i64, _>(i)?),
        "TEXT" | "VARCHAR" | "NAME" | "BPCHAR" | "CITEXT" => json!(row.try_get::<String, _>(i)?),
        "BOOL" => json!(row.try_get::<bool, _>(i)?),
        "FLOAT4" => json!(row.try_get::<f32, _>(i)?),
        "FLOAT8" => json!(row.try_get::<f64, _>(i)?),
        // A Time in UTC: ActiveSupport's JSON with milliseconds.
        "TIMESTAMP" => json!(crate::topic_list::time_json(
            row.try_get::<chrono::NaiveDateTime, _>(i)?
        )),
        "TIMESTAMPTZ" => {
            let t = row.try_get::<chrono::DateTime<chrono::Utc>, _>(i)?;
            json!(t.format("%Y-%m-%dT%H:%M:%S%.3f+00:00").to_string())
        }
        "DATE" => json!(
            row.try_get::<chrono::NaiveDate, _>(i)?
                .format("%Y-%m-%d")
                .to_string()
        ),
        "JSON" | "JSONB" => row.try_get::<Value, _>(i)?,
        _ => return Err(Unsupported("badge preview columns of other types").into()),
    })
}

/// The preview's options from the request.
pub struct Opts {
    pub target_posts: bool,
    pub explain: bool,
    pub trigger: i64,
}

/// `BadgeGranter.preview`: the response's document.
pub async fn preview(
    conn: &mut PgConnection,
    sql: Option<&str>,
    opts: &Opts,
) -> Result<Value, AppError> {
    match preview_inner(conn, sql, opts).await? {
        Ok(v) => Ok(v),
        Err(Failure(message)) => Ok(json!({ "errors": message })),
    }
}

async fn preview_inner(
    conn: &mut PgConnection,
    sql: Option<&str>,
    opts: &Opts,
) -> Result<Result<Value, Failure>, AppError> {
    if let Some(violation) = contract_checks(sql, opts.target_posts, Some(opts.trigger)) {
        return Ok(Err(Failure(violation.to_string())));
    }
    // `#{sql}` of a missing sql is "".
    let sql = sql.unwrap_or("");
    let count = match bind_named(&count_sql(sql)) {
        Ok(s) => s,
        Err(f) => return Ok(Err(f)),
    };
    let rows = match run(conn, &count).await? {
        Ok(rows) => rows,
        Err(f) => return Ok(Err(f)),
    };
    // query_single(...).first.to_i
    let grant_count = match rows.first().map(|r| column_json(r, 0)).transpose()? {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
        _ => 0,
    };
    let mut query_plan = Value::Null;
    if opts.explain {
        let explain = match bind_named(&format!("EXPLAIN {sql} /*:backfill*/")) {
            Ok(s) => s,
            Err(f) => return Ok(Err(f)),
        };
        let rows = match run(conn, &explain).await? {
            Ok(rows) => rows,
            Err(f) => return Ok(Err(f)),
        };
        let mut plan = Vec::new();
        for row in &rows {
            plan.push(json!({ "QUERY PLAN": column_json(row, 0)? }));
        }
        query_plan = Value::Array(plan);
    }
    let grants = match bind_named(&grants_sql(sql, opts.target_posts)) {
        Ok(s) => s,
        Err(f) => return Ok(Err(f)),
    };
    let rows = match run(conn, &grants).await? {
        Ok(rows) => rows,
        Err(f) => return Ok(Err(f)),
    };
    let mut sample = Vec::new();
    for row in &rows {
        let mut item = Map::new();
        for (i, column) in row.columns().iter().enumerate() {
            item.insert(column.name().to_string(), column_json(row, i)?);
        }
        let id = item.get("id").and_then(Value::as_i64);
        let user_exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
                .bind(id)
                .fetch_one(&mut *conn)
                .await?;
        if !user_exists {
            let id = item.get("id").cloned().unwrap_or(Value::Null);
            return Ok(Err(Failure(format!(
                "Query returned a non-existent user ID:\n{}",
                ruby_to_s(&id)
            ))));
        }
        if item
            .get("granted_at")
            .is_none_or(|v| v.is_null() || *v == Value::Bool(false))
        {
            return Ok(Err(Failure(
                "Query did not return a badge grant time\n(Try using 'current_timestamp granted_at')"
                    .to_string(),
            )));
        }
        if opts.target_posts {
            let post_id = item.get("post_id").cloned().unwrap_or(Value::Null);
            if post_id.is_null() || post_id == Value::Bool(false) {
                return Ok(Err(Failure("Query did not return a post ID".to_string())));
            }
            // Post.exists?(post_id): the default scope, by primary key.
            let Some(pid) = post_id.as_i64() else {
                return Err(Unsupported("badge preview post ids that aren't integers").into());
            };
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM posts WHERE id = $1 AND deleted_at IS NULL)",
            )
            .bind(pid)
            .fetch_one(&mut *conn)
            .await?;
            if !exists {
                return Ok(Err(Failure(format!(
                    "Query returned a non-existent post ID:\n{pid}"
                ))));
            }
        }
        sample.push(Value::Object(item));
    }
    Ok(Ok(json!({
        "grant_count": grant_count,
        "sample": sample,
        "query_plan": query_plan,
    })))
}

/// `"#{value}"` of a decoded column.
fn ruby_to_s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_like_active_record() {
        assert_eq!(
            bind_named("a IN (:user_ids) OR :backfill AND x::int = 1 AND '\\:y'").unwrap(),
            "a IN (NULL) OR TRUE AND x::int = 1 AND ':y'"
        );
        let err = bind_named("x = :foo").err().unwrap();
        assert_eq!(err.0, "missing value for :foo in x = :foo");
    }

    fn position(query: &str, loc: usize) -> String {
        let mut msg = String::new();
        report_error_position(&mut msg, query, loc).unwrap();
        msg
    }

    // Recorded from Rails' preview on the reference (libpq 16).
    #[test]
    fn reports_positions_like_libpq() {
        let q = count_sql("SELECT id user_id, created_at granted_at FROM userz");
        let loc = q.find("userz").unwrap() + 1;
        assert_eq!(
            position(&q, loc),
            "LINE 3: ...         SELECT id user_id, created_at granted_at FROM userz\n                                                                  ^\n"
        );
        let sql = "SELECT id user_id, created_at granted_at, nope FROM users WHERE id > 0 AND username <> 'a really long string that pushes the context past sixty columns'";
        let q = count_sql(sql);
        let loc = q.find("nope").unwrap() + 1;
        assert_eq!(
            position(&q, loc),
            "LINE 3: ...        SELECT id user_id, created_at granted_at, nope FROM ...\n                                                             ^\n"
        );
        let q =
            count_sql("SELECT id user_id,\n created_at granted_at\n FROM users\n WHERE nope = 1");
        let loc = q.find("nope").unwrap() + 1;
        assert_eq!(
            position(&q, loc),
            "LINE 6:  WHERE nope = 1\n               ^\n"
        );
        let q = count_sql(
            "SELECT id user_id, created_at granted_at FROM users WHERE username = 'é' AND nope",
        );
        let loc = q.chars().count() - q[q.find("nope").unwrap()..].chars().count() + 1;
        assert_eq!(
            position(&q, loc),
            "LINE 3: ...eated_at granted_at FROM users WHERE username = 'é' AND nope\n                                                                   ^\n"
        );
        let q =
            count_sql("SELECT id user_id, created_at granted_at FROM users WHERE id = 1 GROUP BY");
        let loc = q.find(") q").unwrap() + 1;
        assert_eq!(
            position(&q, loc),
            "LINE 4:                 ) q\n                        ^\n"
        );
    }
}
