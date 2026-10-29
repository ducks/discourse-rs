//! `DiscourseNarrativeBot::Store`: each user's tutorial state, one JSON
//! hash per user in `plugin_store_rows` under the plugin's name, keyed by
//! the user id (PluginStore).

use serde_json::{Map, Value};
use sqlx::PgConnection;

use super::PLUGIN_NAME;
use crate::{AppError, Unsupported};

/// A user's state as the plugin reads it: `state`, `topic_id`, `track`,
/// `last_post_id`, `attempted`, `skip_attempted` and a hash per state
/// (`set_state_data`), in the order Ruby's hash had them.
pub type Data = Map<String, Value>;

/// `Store.get(user_id)`: the stored hash, or None when there is no row
/// (or a row holding nothing).
pub async fn get(conn: &mut PgConnection, user_id: i32) -> Result<Option<Data>, AppError> {
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT type_name, value FROM plugin_store_rows WHERE plugin_name = $1 AND key = $2",
    )
    .bind(PLUGIN_NAME)
    .bind(user_id.to_string())
    .fetch_optional(&mut *conn)
    .await?;
    let Some((type_name, value)) = row else {
        return Ok(None);
    };
    match (type_name.as_str(), value) {
        (_, None) => Ok(None),
        ("JSON", Some(value)) => match serde_json::from_str::<Value>(&value) {
            Ok(Value::Object(map)) => Ok(Some(map)),
            _ => Err(Unsupported("narrative bot data that is not a JSON hash").into()),
        },
        _ => Err(Unsupported("narrative bot data that is not a JSON hash").into()),
    }
}

/// `Store.set(user_id, hash)`: the row's type JSON and its value the
/// hash's JSON.
pub async fn set(conn: &mut PgConnection, user_id: i32, data: &Data) -> Result<(), AppError> {
    let value = serde_json::to_string(data).expect("JSON values serialize");
    let key = user_id.to_string();
    let updated = sqlx::query(
        "UPDATE plugin_store_rows SET type_name = 'JSON', value = $3 \
         WHERE plugin_name = $1 AND key = $2",
    )
    .bind(PLUGIN_NAME)
    .bind(&key)
    .bind(&value)
    .execute(&mut *conn)
    .await?;
    if updated.rows_affected() == 0 {
        sqlx::query(
            "INSERT INTO plugin_store_rows (plugin_name, key, type_name, value) \
             VALUES ($1, $2, 'JSON', $3)",
        )
        .bind(PLUGIN_NAME)
        .bind(&key)
        .bind(&value)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `Store.remove(user_id)`
pub async fn remove(conn: &mut PgConnection, user_id: i32) -> Result<(), AppError> {
    sqlx::query("DELETE FROM plugin_store_rows WHERE plugin_name = $1 AND key = $2")
        .bind(PLUGIN_NAME)
        .bind(user_id.to_string())
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `data[:key]` as a string (a symbol or string in Ruby).
pub fn str_of<'a>(data: &'a Data, key: &str) -> Option<&'a str> {
    data.get(key).and_then(Value::as_str)
}

/// `data[:key]` as an integer id.
pub fn int_of(data: &Data, key: &str) -> Option<i64> {
    data.get(key).and_then(Value::as_i64)
}

/// Ruby truthiness of `data[:key]`.
pub fn truthy(data: &Data, key: &str) -> bool {
    !matches!(
        data.get(key),
        None | Some(Value::Null) | Some(Value::Bool(false))
    )
}
