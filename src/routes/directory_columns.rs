//! DirectoryColumnsController (`/directory-columns`, the enabled columns
//! the users directory draws) and EditDirectoryColumnsController
//! (`/edit-directory-columns`, staff: every column, with the user fields
//! shown on profiles or cards given a column first, and saving which are
//! enabled and their positions). Both answer JSON only.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::params;
use crate::session::current::AuthGuardian;
use crate::{AppError, AppState, Unsupported};

/// `DirectoryColumn.types`, by value.
const TYPES: [&str; 3] = ["automatic", "user_field", "plugin"];

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    name: Option<String>,
    automatic_position: Option<i32>,
    icon: Option<String>,
    user_field_id: Option<i32>,
    enabled: bool,
    position: i32,
    #[sqlx(rename = "type")]
    kind: i32,
    field_name: Option<String>,
}

const SELECT: &str = "SELECT dc.id, dc.name, dc.automatic_position, dc.icon, dc.user_field_id, \
    dc.enabled, dc.position, dc.type, uf.name AS field_name \
    FROM directory_columns dc LEFT JOIN user_fields uf ON uf.id = dc.user_field_id";

/// DirectoryColumnSerializer
fn column_json(r: &Row) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("id".into(), json!(r.id));
    // `object.name || object.user_field.name`
    out.insert(
        "name".into(),
        json!(r.name.clone().or_else(|| r.field_name.clone())),
    );
    out.insert("type".into(), json!(TYPES.get(r.kind as usize).copied()));
    out.insert("position".into(), json!(r.position));
    out.insert("icon".into(), json!(r.icon));
    out.insert("user_field_id".into(), json!(r.user_field_id));
    out
}

/// GET /directory-columns(.json)
pub async fn index(State(state): State<AppState>) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    // where(enabled: true).order(:position)
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} WHERE dc.enabled ORDER BY dc.position"))
        .fetch_all(&mut *conn)
        .await?;
    let columns: Vec<Value> = rows.iter().map(|r| Value::Object(column_json(r))).collect();
    Ok(Json(json!({ "directory_columns": columns })).into_response())
}

/// requires_login, then ensure_staff.
fn refuse(state: &AppState, guardian: &crate::guardian::Guardian) -> Option<Response> {
    if guardian.is_anonymous() {
        Some(super::login_required::not_logged_in(state))
    } else if !guardian.is_staff() {
        Some(super::search::invalid_access(state))
    } else {
        None
    }
}

/// `ensure_user_fields_have_columns`: a disabled column, after the last,
/// for each user field shown on profiles or cards that has none (Rails'
/// query has no order; the reference's plan reads the fields by id).
async fn ensure_user_fields_have_columns(conn: &mut PgConnection) -> Result<(), AppError> {
    let missing: Vec<i32> = sqlx::query_scalar(
        "SELECT user_fields.id FROM user_fields \
         LEFT OUTER JOIN directory_columns ON directory_columns.user_field_id = user_fields.id \
         WHERE directory_columns.user_field_id IS NULL \
           AND (show_on_profile = TRUE OR show_on_user_card = TRUE) ORDER BY user_fields.id",
    )
    .fetch_all(&mut *conn)
    .await?;
    if missing.is_empty() {
        return Ok(());
    }
    let max: Option<i32> = sqlx::query_scalar("SELECT MAX(position) FROM directory_columns")
        .fetch_one(&mut *conn)
        .await?;
    let first = max.ok_or(Unsupported(
        "user field columns without any column (nil + 1)",
    ))? + 1;
    for (next, id) in (first..).zip(missing) {
        sqlx::query(
            "INSERT INTO directory_columns (user_field_id, enabled, type, position, created_at) \
             VALUES ($1, FALSE, 1, $2, clock_timestamp())",
        )
        .bind(id)
        .bind(next)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// GET /edit-directory-columns(.json): EditDirectoryColumnSerializer for
/// every column, in the table's order.
pub async fn edit_index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    if let Some(r) = refuse(&state, &guardian) {
        return Ok(r);
    }
    let mut tx = state.pool.begin().await?;
    ensure_user_fields_have_columns(&mut tx).await?;
    // DirectoryColumn.all: no ORDER BY.
    let rows: Vec<Row> = sqlx::query_as(SELECT).fetch_all(&mut *tx).await?;
    let mut columns = Vec::new();
    for r in &rows {
        let mut c = column_json(r);
        c.insert("enabled".into(), json!(r.enabled));
        c.insert("automatic_position".into(), json!(r.automatic_position));
        let field = match r.user_field_id {
            Some(id) => match crate::user_fields::find(&mut tx, id).await? {
                Some(f) => crate::user_fields::serialize(&mut tx, &f).await?,
                None => Value::Null,
            },
            None => Value::Null,
        };
        c.insert("user_field".into(), field);
        columns.push(Value::Object(c));
    }
    tx.commit().await?;
    Ok(Json(json!({ "directory_columns": columns })).into_response())
}

fn pairs(p: &Map<String, Value>) -> Vec<(String, String)> {
    p.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// `StaffActionLogger#edit_directory_columns_details`: the column's
/// attributes as strings, keeping `name` and each attribute the params
/// change to something present; (previous, new) as JSON.
fn details(r: &Row, data: &Map<String, Value>) -> Result<(String, String), AppError> {
    let s = |v: Option<String>| v.unwrap_or_default();
    let attributes: Vec<(&str, String)> = vec![
        ("id", r.id.to_string()),
        ("name", s(r.name.clone())),
        (
            "automatic_position",
            s(r.automatic_position.map(|p| p.to_string())),
        ),
        ("icon", s(r.icon.clone())),
        ("user_field_id", s(r.user_field_id.map(|p| p.to_string()))),
        ("enabled", r.enabled.to_string()),
        ("position", r.position.to_string()),
        ("created_at", String::new()),
        (
            "type",
            TYPES
                .get(r.kind as usize)
                .copied()
                .unwrap_or("")
                .to_string(),
        ),
    ];
    let mut previous = Map::new();
    let mut new = Map::new();
    for (key, value) in attributes {
        let given = match data.get(key) {
            None | Some(Value::Null) => None,
            Some(v) => Some(params::scalar(v).ok_or(Unsupported(
                "a directory column attribute that isn't a string",
            ))?),
        };
        if key == "created_at" && given.is_some() {
            return Err(Unsupported("a directory column's created_at in the params").into());
        }
        match given {
            Some(g) if g != value && !crate::ruby::is_blank(&g) => {
                previous.insert(key.into(), json!(value));
                new.insert(key.into(), json!(g));
            }
            _ if key == "name" => {
                previous.insert(key.into(), json!(value));
                new.insert(key.into(), json!(value));
            }
            _ => {}
        }
    }
    Ok((
        Value::Object(previous).to_string(),
        Value::Object(new).to_string(),
    ))
}

/// PUT /edit-directory-columns(.json)
pub async fn edit_update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &pairs(&p), uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    if let Some(r) = refuse(&state, &guardian) {
        return Ok(r);
    }
    let given = match p.get("directory_columns") {
        None | Some(Value::Null) => return Ok(super::accounts::param_missing("directory_columns")),
        Some(Value::Object(m)) if m.is_empty() => {
            return Ok(super::accounts::param_missing("directory_columns"));
        }
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err(Unsupported("directory_columns that isn't a hash").into()),
    };
    let mut columns_data = Vec::new();
    for v in given.values() {
        let Value::Object(data) = v else {
            return Err(Unsupported("a directory column that isn't a hash").into());
        };
        columns_data.push(data.clone());
    }
    let enabled_given = |d: &Map<String, Value>| d.get("enabled").and_then(params::scalar);
    if !columns_data
        .iter()
        .any(|d| enabled_given(d).as_deref() == Some("true"))
    {
        return Ok(super::search::invalid_parameters(
            &state,
            "Must have at least one column enabled",
        ));
    }
    let actor = guardian
        .user_id()
        .ok_or(Unsupported("an anonymous staff member"))?;
    let mut tx = state.pool.begin().await?;
    let existing: Vec<Row> = sqlx::query_as(SELECT).fetch_all(&mut *tx).await?;
    let mut new_values = String::new();
    let mut previous_values = String::new();
    for data in &columns_data {
        let id = data
            .get("id")
            .and_then(params::scalar)
            .map(|s| crate::ruby::to_i(&s))
            .unwrap_or(0);
        let Some(column) = existing.iter().find(|c| c.id == id) else {
            return Err(
                Unsupported("a directory column that doesn't exist (NoMethodError)").into(),
            );
        };
        let enabled = crate::ruby::cast_boolean(enabled_given(data).as_deref());
        let position = data
            .get("position")
            .and_then(params::scalar)
            .map(|s| crate::ruby::to_i(&s) as i32)
            .unwrap_or(0);
        if Some(column.enabled) != enabled || column.position != position {
            // `new_value, previous_value = edit_directory_columns_details`:
            // the pair comes back the other way round.
            let (first, second) = details(column, data)?;
            new_values.push_str(&first);
            previous_values.push_str(&second);
            let Some(enabled) = enabled else {
                return Err(Unsupported("a directory column without enabled (NOT NULL)").into());
            };
            sqlx::query("UPDATE directory_columns SET enabled = $2, position = $3 WHERE id = $1")
                .bind(column.id)
                .bind(enabled)
                .bind(position)
                .execute(&mut *tx)
                .await?;
        }
    }
    crate::staff_action_logs::log_custom(
        &mut tx,
        actor,
        "update_directory_columns",
        &[],
        Some(&previous_values),
        Some(&new_values),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}
