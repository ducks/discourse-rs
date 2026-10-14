//! UserField: the admin-defined profile fields, stored per user as
//! `user_custom_fields` rows named `user_field_<id>`.

use serde_json::{Value, json};
use sqlx::PgConnection;

/// `HasCustomFields::Helpers::CUSTOM_FIELD_TRUE`
pub const CUSTOM_FIELD_TRUE: &[&str] = &["1", "t", "true", "T", "True", "TRUE"];

/// `user.custom_fields["user_field_<id>"]`: no row is nil, one row its
/// value, several (a multiselect) an array in id order, the way
/// `FieldDescriptor#append_field` folds untyped fields.
pub async fn custom_field_value(
    conn: &mut PgConnection,
    user_id: i32,
    field_id: i32,
) -> Result<Value, sqlx::Error> {
    let values: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT value FROM user_custom_fields WHERE user_id = $1 AND name = $2 ORDER BY id",
    )
    .bind(user_id)
    .bind(format!("user_field_{field_id}"))
    .fetch_all(conn)
    .await?;
    Ok(match values.len() {
        0 => Value::Null,
        1 => json!(values[0]),
        _ => json!(values),
    })
}
