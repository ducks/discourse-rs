//! GET /t/:slug/:id.json for anonymous users, on the snapshot's fixture
//! topics. The full documents are covered by the parity golden files.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

async fn get(pool: &PgPool, path: &str) -> (StatusCode, Option<String>, Value) {
    let app = discourse_rs::app(state(pool.clone(), config(RailsEnv::Test, &[])).await);
    let response = app
        .oneshot(
            Request::get(path)
                .header(header::HOST, "test.localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, location, json)
}

#[tokio::test]
async fn renders_the_post_stream_in_order() {
    let db = TestDb::new().await;
    let (status, _, json) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["id"], 35);
    assert_eq!(json["posts_count"], 5);
    assert_eq!(json["highest_post_number"], 5);
    assert_eq!(json["current_post_number"], 1);
    assert_eq!(json["chunk_size"], 20);
    let posts = json["post_stream"]["posts"].as_array().unwrap();
    let numbers: Vec<i64> = posts
        .iter()
        .map(|p| p["post_number"].as_i64().unwrap())
        .collect();
    assert_eq!(numbers, vec![1, 2, 3, 4, 5]);
    assert_eq!(json["post_stream"]["stream"].as_array().unwrap().len(), 5);
    // [post_number, days ago]: the fixture ages, so only the shape is fixed.
    let lookup = json["timeline_lookup"].as_array().unwrap();
    // Two days of posts since the mention fixture: one entry per day.
    assert_eq!(lookup.len(), 2);
    assert_eq!(lookup[0][0], 1);
    assert!(lookup[0][1].as_i64().unwrap() >= 0);
    assert_eq!(
        posts[0]["post_url"],
        "/t/parity-fixture-replies-and-posters/35/1"
    );
    assert_eq!(posts[0]["yours"], false);
    assert_eq!(posts[0]["read"], true);
    assert_eq!(posts[0]["can_edit"], false);
    assert!(posts[0]["cooked"].as_str().unwrap().starts_with("<p>"));
    // The page is complete, so suggested topics are present.
    assert!(json["suggested_topics"].is_array());
}

#[tokio::test]
async fn details_list_participants_creator_and_last_poster() {
    let db = TestDb::new().await;
    let (_, _, json) = get(&db.pool, "/t/35.json").await;
    let details = &json["details"];
    assert_eq!(details["can_edit"], false);
    assert_eq!(details["notification_level"], 1);
    let participants = details["participants"].as_array().unwrap();
    assert_eq!(participants.len(), 4);
    // user0 has the opener and the mention-fixture reply.
    assert!(
        participants
            .iter()
            .all(|p| p["post_count"] == 1 || p["id"] == 2)
    );
    assert_eq!(details["created_by"]["username"], "user0");
    assert_eq!(details["last_poster"]["username"], "user0");
    assert_eq!(json["participant_count"], 4);
    assert_eq!(json["actions_summary"].as_array().unwrap().len(), 4);
    assert_eq!(json["bookmarks"], serde_json::json!([]));
}

#[tokio::test]
async fn closed_topics_carry_the_small_action_post() {
    let db = TestDb::new().await;
    let (_, _, json) = get(&db.pool, "/t/parity-fixture-pinned-and-closed/37.json").await;
    assert_eq!(json["closed"], true);
    assert_eq!(json["pinned"], true);
    assert_eq!(json["pinned_globally"], false);
    let posts = json["post_stream"]["posts"].as_array().unwrap();
    let action = posts.iter().find(|p| p["post_type"] == 3).unwrap();
    assert_eq!(action["action_code"], "closed.enabled");
    assert!(action.get("action_code_who").is_none());
}

#[tokio::test]
async fn post_number_selects_the_current_post() {
    let db = TestDb::new().await;
    let (status, _, json) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35/3.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["current_post_number"], 3);
    // Past the end clamps to the highest post.
    let (_, _, json) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35/99.json").await;
    assert_eq!(json["current_post_number"], 5);
}

#[tokio::test]
async fn redirects_to_the_canonical_url() {
    let db = TestDb::new().await;
    let (status, location, _) = get(&db.pool, "/t/wrong-slug/35.json").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/t/parity-fixture-replies-and-posters/35.json")
    );

    let (status, location, _) = get(&db.pool, "/t/parity-fixture-replies-and-posters.json").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/t/parity-fixture-replies-and-posters/35.json")
    );

    let (status, location, _) = get(&db.pool, "/t/wrong/35/2.json").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/t/parity-fixture-replies-and-posters/35/2.json")
    );

    // A JSON request without a slug renders directly.
    let (status, _, json) = get(&db.pool, "/t/35.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["id"], 35);

    // Pages past the end go to the last page.
    let (status, location, _) = get(
        &db.pool,
        "/t/parity-fixture-replies-and-posters/35.json?page=5",
    )
    .await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/t/parity-fixture-replies-and-posters/35.json")
    );
}

#[tokio::test]
async fn hidden_topics_are_not_found() {
    let db = TestDb::new().await;
    for path in ["/t/999999.json", "/t/4.json", "/t/40.json", "/t/nope.json"] {
        let (status, _, json) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(json["error_type"], "not_found", "{path}");
    }
    // Unlisted topics are readable, just not listed.
    let (status, _, json) = get(&db.pool, "/t/39.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["visible"], false);
    // A negative page is a 404 too.
    let (status, _, _) = get(&db.pool, "/t/35.json?page=-1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// An image upload for topic 35's first post, `width`x`height`, as
/// process_post leaves a topic: image_upload_id set, plus the share-size
/// TopicThumbnail when `thumbnail` (with its optimized image when given).
async fn topic_image(
    pool: &PgPool,
    width: i32,
    height: i32,
    thumbnail: Option<Option<(i32, i32)>>,
) {
    let sha = "0000000000000000000000000000000000009741";
    sqlx::query(
        "INSERT INTO uploads (id, user_id, original_filename, filesize, width, height, url, sha1, \
         extension, created_at, updated_at) VALUES (9741, 3, 'pond.png', 120000, $1, $2, \
         '/uploads/default/original/1X/' || $3 || '.png', $3, 'png', now(), now())",
    )
    .bind(width)
    .bind(height)
    .bind(sha)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE topics SET image_upload_id = 9741 WHERE id = 35")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE posts SET image_upload_id = 9741 WHERE id = 35")
        .execute(pool)
        .await
        .unwrap();
    let Some(optimized) = thumbnail else {
        return;
    };
    let optimized_id: Option<i32> = match optimized {
        Some((w, h)) => Some(
            sqlx::query_scalar(
                "INSERT INTO optimized_images (sha1, extension, width, height, upload_id, url, \
                 filesize, version, created_at, updated_at) VALUES ($1, '.png', $2, $3, 9741, \
                 '/uploads/default/optimized/1X/' || $4 || '_2_' || $2 || 'x' || $3 || '.png', \
                 4000, 2, now(), now()) RETURNING id",
            )
            .bind("1111111111111111111111111111111111119741")
            .bind(w)
            .bind(h)
            .bind(sha)
            .fetch_one(pool)
            .await
            .unwrap(),
        ),
        None => None,
    };
    sqlx::query(
        "INSERT INTO topic_thumbnails (upload_id, optimized_image_id, max_width, max_height) \
         VALUES (9741, $1, 1024, 1024)",
    )
    .bind(optimized_id.map(i64::from))
    .execute(pool)
    .await
    .unwrap();
}

/// Jobs waiting in the queue, by name with their arguments.
async fn queued(pool: &PgPool, name: &str) -> Vec<Value> {
    sqlx::query_scalar("SELECT args FROM discourse_rs.jobs WHERE name = $1 ORDER BY id")
        .bind(name)
        .fetch_all(pool)
        .await
        .unwrap()
}

/// TopicViewSerializer#thumbnails after process_post took the first
/// post's image: the original, then each optimized size, largest first;
/// image_url the share-size thumbnail.
#[tokio::test]
async fn thumbnails_list_the_original_and_its_optimized_sizes() {
    let db = TestDb::new().await;
    topic_image(&db.pool, 2048, 1536, Some(Some((1024, 768)))).await;
    let (status, _, json) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35.json").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let original =
        "//test.localhost/uploads/default/original/1X/0000000000000000000000000000000000009741.png";
    let optimized = "//test.localhost/uploads/default/optimized/1X/0000000000000000000000000000000000009741_2_1024x768.png";
    assert_eq!(
        json["thumbnails"],
        serde_json::json!([
            {"max_width": null, "max_height": null, "width": 2048, "height": 1536, "url": original},
            {"max_width": 1024, "max_height": 1024, "width": 1024, "height": 768, "url": optimized},
        ])
    );
    assert_eq!(json["image_url"], optimized);
    assert!(
        queued(&db.pool, "generate_topic_thumbnails")
            .await
            .is_empty()
    );
    // The JSON's image_url is the topic's on any post's page (the page's
    // og:image is the desired post's, html_pages covers it).
    let (status, _, json) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35/3.json").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["image_url"], optimized);
    assert_eq!(json["thumbnails"][0]["width"], 2048);
}

/// The real path: process_post takes an image smaller than the share size
/// for the topic, whose TopicThumbnail needs no optimized image, so only
/// the original is listed.
#[tokio::test]
async fn thumbnails_after_process_post() {
    let db = TestDb::new().await;
    let app = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let src = "/uploads/default/original/1X/0000000000000000000000000000000000009742.png";
    sqlx::query(
        "INSERT INTO uploads (id, user_id, original_filename, filesize, width, height, url, sha1, \
         extension, created_at, updated_at) VALUES (9742, 3, 'dot.png', 900, 100, 80, $1, \
         '0000000000000000000000000000000000009742', 'png', now(), now())",
    )
    .bind(src)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE posts SET raw = $1, cooked = $2 WHERE id = 35")
        .bind(format!("![dot|100x80]({src})"))
        .bind(format!(
            "<p><img src=\"{src}\" alt=\"dot\" data-base62-sha1=\"x\" width=\"100\" height=\"80\"></p>"
        ))
        .execute(&db.pool)
        .await
        .unwrap();
    let mut conn = db.pool.acquire().await.unwrap();
    discourse_rs::jobs::enqueue(
        &mut conn,
        "process_post",
        serde_json::json!({"post_id": 35}),
    )
    .await
    .unwrap();
    drop(conn);
    discourse_rs::jobs::drain(&app).await.unwrap();
    let failed: Option<String> = sqlx::query_scalar(
        "SELECT last_error FROM discourse_rs.jobs WHERE last_error IS NOT NULL LIMIT 1",
    )
    .fetch_optional(&db.pool)
    .await
    .unwrap();
    assert_eq!(failed, None);
    let image: Option<i64> = sqlx::query_scalar("SELECT image_upload_id FROM topics WHERE id = 35")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(image, Some(9742));
    let (status, _, json) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35.json").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let url = format!("//test.localhost{src}");
    assert_eq!(
        json["thumbnails"],
        serde_json::json!([{"max_width": null, "max_height": null, "width": 100, "height": 80, "url": url}])
    );
    assert_eq!(json["image_url"], url);
    assert!(
        queued(&db.pool, "generate_topic_thumbnails")
            .await
            .is_empty()
    );
}

/// Without its TopicThumbnails the view enqueues generate_topic_thumbnails
/// once a minute (the Redis key), which makes them.
#[tokio::test]
async fn missing_thumbnails_are_enqueued_once_and_generated() {
    let db = TestDb::new().await;
    let app = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    topic_image(&db.pool, 800, 600, None).await;
    for _ in 0..2 {
        let (status, _, json) =
            get(&db.pool, "/t/parity-fixture-replies-and-posters/35.json").await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["thumbnails"].as_array().unwrap().len(), 1, "{json}");
        assert_eq!(json["thumbnails"][0]["max_width"], Value::Null);
    }
    assert_eq!(
        queued(&db.pool, "generate_topic_thumbnails").await,
        vec![serde_json::json!({"topic_id": 35, "extra_sizes": []})]
    );
    discourse_rs::jobs::drain(&app).await.unwrap();
    assert!(
        queued(&db.pool, "generate_topic_thumbnails")
            .await
            .is_empty()
    );
    let made: Vec<(i32, i32, Option<i64>)> = sqlx::query_as(
        "SELECT max_width, max_height, optimized_image_id FROM topic_thumbnails WHERE upload_id = 9741",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(made, vec![(1024, 1024, None)]);
}

/// Originals at or over max_image_size_kb, or without dimensions, have no
/// thumbnail info.
#[tokio::test]
async fn oversized_or_unsized_images_have_no_thumbnails() {
    let db = TestDb::new().await;
    topic_image(&db.pool, 800, 600, Some(None)).await;
    sqlx::query("UPDATE uploads SET width = NULL WHERE id = 9741")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, _, json) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35.json").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["thumbnails"], Value::Null);
    sqlx::query("UPDATE uploads SET width = 800, filesize = 10240 * 1024 WHERE id = 9741")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, _, json) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35.json").await;
    assert_eq!(json["thumbnails"], Value::Null);
}
