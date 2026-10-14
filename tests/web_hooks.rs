//! Web hooks: which hooks an event reaches (`WebHook.active_web_hooks`).

mod common;

use common::{TestDb, recorded_config, state};
use discourse_rs::{jobs, web_hooks};
use serde_json::json;
use sqlx::PgPool;

async fn hook(pool: &PgPool, id: i32, wildcard: bool, active: bool, types: &[i32]) {
    sqlx::query(
        "INSERT INTO web_hooks (id, payload_url, wildcard_web_hook, active, created_at, updated_at) \
         VALUES ($1, 'https://example.com/hook', $2, $3, now(), now())",
    )
    .bind(id)
    .bind(wildcard)
    .bind(active)
    .execute(pool)
    .await
    .unwrap();
    for t in types {
        sqlx::query(
            "INSERT INTO web_hook_event_types_hooks (web_hook_id, web_hook_event_type_id) VALUES ($1, $2)",
        )
        .bind(id)
        .bind(t)
        .execute(pool)
        .await
        .unwrap();
    }
}

/// Rails joins the event types (an INNER JOIN): a wildcard hook without
/// any never fires; with any, it fires for every event.
#[tokio::test]
async fn wildcard_hooks_need_an_event_type() {
    let db = TestDb::new().await;
    let mut conn = db.pool.acquire().await.unwrap();
    hook(&db.pool, 9001, true, true, &[]).await;
    assert!(!web_hooks::active(&mut conn, "post_created").await.unwrap());
    sqlx::query("INSERT INTO web_hook_event_types_hooks (web_hook_id, web_hook_event_type_id) VALUES (9001, 304)")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(web_hooks::active(&mut conn, "post_created").await.unwrap());
    assert_eq!(
        web_hooks::active_ids(&mut conn, "topic_closed_status_updated")
            .await
            .unwrap(),
        vec![9001]
    );
}

#[tokio::test]
async fn hooks_match_their_event_types_once_translated() {
    let db = TestDb::new().await;
    let mut conn = db.pool.acquire().await.unwrap();
    // topic_edited, post_created; an inactive one for post_created too.
    hook(&db.pool, 9001, false, true, &[103, 201]).await;
    hook(&db.pool, 9002, false, false, &[201]).await;
    hook(&db.pool, 9003, false, true, &[902]).await;
    let ids = |event: &'static str| {
        let pool = db.pool.clone();
        async move {
            let mut conn = pool.acquire().await.unwrap();
            web_hooks::active_ids(&mut conn, event).await.unwrap()
        }
    };
    assert_eq!(ids("post_created").await, vec![9001]);
    assert_eq!(
        ids("topic_pinned_globally_status_updated").await,
        vec![9001]
    );
    assert_eq!(ids("topic_created").await, Vec::<i32>::new());
    assert_eq!(ids("reviewable_transitioned_to").await, vec![9003]);
    assert_eq!(ids("reviewable_score_updated").await, vec![9003]);
    assert!(
        !web_hooks::active(&mut conn, "reviewable_created")
            .await
            .unwrap()
    );
}

/// PublishGroupMembershipUpdates hands additions to the
/// `user_added_to_group` hooks (removals have theirs from GroupManager);
/// a payload URL on an internal address is blocked, its event logged.
#[tokio::test]
async fn group_membership_additions_reach_hooks() {
    let db = TestDb::new().await;
    let app = state(db.pool.clone(), recorded_config()).await;
    hook(&db.pool, 9001, false, true, &[1401]).await;
    sqlx::query("UPDATE web_hooks SET payload_url = 'http://127.0.0.1/hook' WHERE id = 9001")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO group_users (group_id, user_id, notification_level, created_at, updated_at) \
         VALUES (10, 1, 3, now(), now()) ON CONFLICT DO NOTHING",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let mut conn = db.pool.acquire().await.unwrap();
    for kind in ["add", "remove"] {
        jobs::enqueue(
            &mut conn,
            "publish_group_membership_updates",
            json!({"user_ids": [1], "group_id": 10, "type": kind}),
        )
        .await
        .unwrap();
    }
    drop(conn);
    jobs::drain(&app).await.unwrap();
    let events: Vec<(Option<String>,)> =
        sqlx::query_as("SELECT payload FROM web_hook_events ORDER BY id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(events.len(), 1, "{events:?}");
    let payload = events[0].0.as_deref().unwrap();
    assert!(payload.contains("\"group_id\":10"), "{payload}");
    assert!(payload.contains("\"user_id\":1,"), "{payload}");
}
