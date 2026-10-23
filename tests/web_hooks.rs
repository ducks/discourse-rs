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

/// WebHookTopicViewSerializer keeps thumbnails: a topic whose first post's
/// image process_post took lists the original and its optimized size. Its
/// serializer has no request, so no theme's extra sizes.
#[tokio::test]
async fn topic_payloads_carry_thumbnails() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), recorded_config()).await;
    let sha = "0000000000000000000000000000000000009741";
    for sql in [
        format!(
            "INSERT INTO uploads (id, user_id, original_filename, filesize, width, height, url, sha1, \
             extension, created_at, updated_at) VALUES (9741, 3, 'pond.png', 120000, 2048, 1536, \
             '/uploads/default/original/1X/{sha}.png', '{sha}', 'png', now(), now())"
        ),
        format!(
            "INSERT INTO optimized_images (id, sha1, extension, width, height, upload_id, url, \
             filesize, version, created_at, updated_at) VALUES (9741, '{sha}', '.png', 1024, 768, \
             9741, '/uploads/default/optimized/1X/{sha}_2_1024x768.png', 4000, 2, now(), now())"
        ),
        "INSERT INTO topic_thumbnails (upload_id, optimized_image_id, max_width, max_height) \
         VALUES (9741, 9741, 1024, 1024)"
            .to_string(),
        "UPDATE topics SET image_upload_id = 9741 WHERE id = 35".to_string(),
    ] {
        sqlx::query(&sql).execute(&db.pool).await.unwrap();
    }
    let mut conn = db.pool.acquire().await.unwrap();
    let settings = discourse_rs::site_settings::SiteSettings::load(
        &mut conn,
        &st.site_setting_defs,
        &st.config.globals,
    )
    .await
    .unwrap();
    let payloads = web_hooks::Payloads {
        settings: &settings,
        i18n: &st.i18n,
        config: &st.config,
    };
    let topic = payloads.topic(&mut conn, 35, None).await.unwrap();
    let thumbnails = &topic["thumbnails"];
    assert_eq!(thumbnails.as_array().unwrap().len(), 2, "{thumbnails}");
    assert_eq!(thumbnails[0]["width"], 2048);
    assert_eq!(thumbnails[1]["max_width"], 1024);
    assert!(
        thumbnails[1]["url"].as_str().unwrap().ends_with(&format!(
            "/uploads/default/optimized/1X/{sha}_2_1024x768.png"
        )),
        "{thumbnails}"
    );
    assert!(topic.get("image_url").is_none(), "excluded from web hooks");
}
