//! TagGroup's writes (app/models/tag_group.rb).

use sqlx::PgConnection;

/// `TagGroup.create!(name:)`: the row, with init_permissions' default
/// (everyone may use its tags).
pub async fn create_minimal(conn: &mut PgConnection, name: &str) -> Result<i32, sqlx::Error> {
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO tag_groups (name, one_per_topic, created_at, updated_at) \
         VALUES ($1, FALSE, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(name)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO tag_group_permissions (tag_group_id, group_id, permission_type, created_at, updated_at) \
         VALUES ($1, 0, $2, clock_timestamp(), clock_timestamp())",
    )
    .bind(id)
    .bind(crate::tagging::PERMISSION_FULL)
    .execute(&mut *conn)
    .await?;
    Ok(id)
}
