//! The job queue: what runs, what waits, what fails and why.

mod common;

use common::{TestDb, recorded_config, state};
use discourse_rs::jobs;
use serde_json::json;

#[tokio::test]
async fn due_jobs_run_and_unported_ones_fail_with_the_reason() {
    let db = TestDb::new().await;
    let app = state(db.pool.clone(), recorded_config()).await;
    let mut conn = db.pool.acquire().await.unwrap();
    jobs::enqueue(
        &mut conn,
        "post_update_topic_tracking_state",
        json!({"post_id": 1}),
    )
    .await
    .unwrap();
    jobs::enqueue(&mut conn, "no_such_job", json!({}))
        .await
        .unwrap();
    jobs::enqueue_in(
        &mut conn,
        600,
        "post_update_topic_tracking_state",
        json!({"post_id": 2}),
    )
    .await
    .unwrap();
    drop(conn);

    assert_eq!(jobs::drain(&app).await.unwrap(), 2);

    let rows: Vec<(String, Option<String>, bool)> = sqlx::query_as(
        "SELECT name, last_error, failed_at IS NOT NULL FROM discourse_rs.jobs ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2, "the completed job is deleted: {rows:?}");
    assert_eq!(rows[0].0, "no_such_job");
    assert!(rows[0].2, "an unported job fails at once");
    assert!(rows[0].1.as_deref().unwrap().contains("no_such_job"));
    assert_eq!(rows[1].0, "post_update_topic_tracking_state");
    assert!(!rows[1].2, "the delayed job waits");
}
