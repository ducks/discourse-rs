//! Schema verification against the test database built from structure.sql.
//! Each test mutates schema_migrations inside a transaction that is rolled
//! back, so the shared test database is left untouched.

mod common;

use discourse_rs::schema::{self, SchemaError};
use sqlx::Connection;
use sqlx::PgConnection;

async fn connect() -> PgConnection {
    PgConnection::connect(&common::test_database_url())
        .await
        .expect("connecting to TEST_DATABASE_URL; run `make db-test` first")
}

#[tokio::test]
async fn loaded_structure_sql_verifies_cleanly() {
    let mut conn = connect().await;
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
    let mut conn = connect().await;
    let mut tx = conn.begin().await.unwrap();
    let dropped = schema::expected_versions()[0];
    sqlx::query("DELETE FROM schema_migrations WHERE version = $1")
        .bind(dropped)
        .execute(&mut *tx)
        .await
        .unwrap();

    match schema::verify(&mut tx).await {
        Err(SchemaError::Missing(v)) => assert_eq!(v, vec![dropped.to_string()]),
        other => panic!("expected Missing, got {other:?}"),
    }
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn newer_migrations_are_tolerated_and_reported() {
    let mut conn = connect().await;
    let mut tx = conn.begin().await.unwrap();
    sqlx::query("INSERT INTO schema_migrations (version) VALUES ('99990101000000')")
        .execute(&mut *tx)
        .await
        .unwrap();

    let report = schema::verify(&mut tx).await.unwrap();
    assert_eq!(report.extra, vec!["99990101000000"]);
    tx.rollback().await.unwrap();
}
