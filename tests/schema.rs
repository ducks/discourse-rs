//! Schema verification against a clone of the test database.

mod common;

use common::TestDb;
use discourse_rs::schema::{self, SchemaError};

#[tokio::test]
async fn loaded_structure_sql_verifies_cleanly() {
    let db = TestDb::new().await;
    let mut conn = db.pool.acquire().await.unwrap();
    let report = schema::verify(&mut conn).await.unwrap();
    assert_eq!(report.expected, schema::expected_versions().len());
    assert!(
        report.extra.is_empty(),
        "unexpected extra versions: {:?}",
        report.extra
    );
}

#[tokio::test]
async fn missing_migration_is_rejected() {
    let db = TestDb::new().await;
    let dropped = schema::expected_versions()[0];
    sqlx::query("DELETE FROM schema_migrations WHERE version = $1")
        .bind(dropped)
        .execute(&db.pool)
        .await
        .unwrap();

    let mut conn = db.pool.acquire().await.unwrap();
    match schema::verify(&mut conn).await {
        Err(SchemaError::Missing(v)) => assert_eq!(v, vec![dropped.to_string()]),
        other => panic!("expected Missing, got {other:?}"),
    }
}

#[tokio::test]
async fn newer_migrations_are_tolerated_and_reported() {
    let db = TestDb::new().await;
    sqlx::query("INSERT INTO schema_migrations (version) VALUES ('99990101000000')")
        .execute(&db.pool)
        .await
        .unwrap();

    let mut conn = db.pool.acquire().await.unwrap();
    let report = schema::verify(&mut conn).await.unwrap();
    assert_eq!(report.extra, vec!["99990101000000"]);
}
