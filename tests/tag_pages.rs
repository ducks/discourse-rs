//! Tag topic lists (/tag/..., /tags/c/...) and the tags index for
//! anonymous users. The documents are covered by the parity golden files;
//! these pin routing, redirects, visibility and the HTML.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

async fn get(pool: &PgPool, path: &str) -> (StatusCode, Option<String>, String) {
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
    (
        status,
        location,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn ids(body: &str) -> Vec<i64> {
    let json: Value = serde_json::from_str(body).unwrap();
    json["topic_list"]["topics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect()
}

/// A tag group holding `tag_id` that only `group_id` may see (permission
/// type 1 = full).
async fn restrict_tag(pool: &PgPool, tag_id: i32, group_id: i32) {
    let group: i32 = sqlx::query_scalar(
        "INSERT INTO tag_groups (name, created_at, updated_at) VALUES ('secret', now(), now()) RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO tag_group_memberships (tag_id, tag_group_id, created_at, updated_at) VALUES ($1, $2, now(), now())")
        .bind(tag_id)
        .bind(group)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO tag_group_permissions (tag_group_id, group_id, permission_type, created_at, updated_at) VALUES ($1, $2, 1, now(), now())")
        .bind(group)
        .bind(group_id)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn tag_lists_filter_by_tag_and_intersect() {
    let db = TestDb::new().await;
    // howto: 41 and 38 (in the subcategory); guide: 41 and 35.
    let (status, _, body) = get(&db.pool, "/tag/howto.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&body), vec![41, 38]);
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["topic_list"]["tags"][0]["name"], "howto");
    assert_eq!(json["topic_list"]["tags"][0]["topic_count"], 2);

    // Query tags[] replace the path tag on canonical URLs and prepend it on
    // legacy ones (TopicQueryParams).
    let (_, _, body) = get(&db.pool, "/tag/howto/1.json?tags%5B%5D=guide&per_page=1").await;
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["topic_list"]["more_topics_url"],
        "/tag/howto/1?match_all_tags=true&page=1&per_page=1&tags%5B%5D=guide"
    );
    assert_eq!(json["topic_list"]["tags"].as_array().unwrap().len(), 1);
    let (_, _, body) = get(&db.pool, "/tag/howto.json?tags%5B%5D=guide&per_page=1").await;
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["topic_list"]["more_topics_url"],
        "/tag/howto?match_all_tags=true&page=1&per_page=1&tags%5B%5D=howto&tags%5B%5D=guide"
    );
    assert_eq!(json["topic_list"]["tags"].as_array().unwrap().len(), 2);

    // A missing tag in the intersection empties the list (result.none).
    let (status, _, body) = get(&db.pool, "/tag/howto.json?tags%5B%5D=nope").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&body), Vec::<i64>::new());

    // Filters, per_page clamp and no_subcategories in a category.
    let (_, _, body) = get(&db.pool, "/tag/howto/1/l/hot.json?per_page=50").await;
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["topic_list"]["filter"], "hot");
    assert_eq!(json["topic_list"]["per_page"], 30);
    let (_, _, body) = get(&db.pool, "/tags/c/general/4/howto/1.json").await;
    assert_eq!(ids(&body), vec![41, 38]);
    let (_, _, body) = get(&db.pool, "/tags/c/general/4/none/howto/1.json").await;
    assert_eq!(ids(&body), vec![41]);
    let (_, _, body) = get(&db.pool, "/tags/c/general/sub-general/34/howto.json").await;
    assert_eq!(ids(&body), vec![38]);
}

#[tokio::test]
async fn tag_routes_redirect_and_404() {
    let db = TestDb::new().await;
    // Legacy and wrong-slug HTML URLs go canonical, JSON ones don't.
    let (status, location, _) = get(&db.pool, "/tag/howto?page=1").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/tag/howto/1?page=1")
    );
    let (status, location, _) = get(&db.pool, "/tag/HowTo/1/l/top").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/tag/howto/1/l/top")
    );
    let (status, location, _) = get(&db.pool, "/tags/c/general/4/none/wrong/1").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/tags/c/general/4/none/howto/1")
    );
    let (status, _, _) = get(&db.pool, "/tag/wrong/1.json").await;
    assert_eq!(status, StatusCode::OK);

    for path in [
        "/tag/nope",
        "/tag/nope.json",
        "/tag/99.json",
        "/tag/nope/99.json",
        "/tags/c/nope/99/howto/1.json",
        "/tags/c/staff/3/howto/1.json",
    ] {
        let (status, _, body) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert!(!body.contains("extras"), "{path}");
    }
    // An existing tag with no topics is an empty list, not a 404.
    sqlx::query("INSERT INTO tags (name, created_at, updated_at) VALUES ('empty', now(), now())")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, _, body) = get(&db.pool, "/tag/empty.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&body), Vec::<i64>::new());

    let (status, _, _) = get(&db.pool, "/tag/howto/1/l/top.json?period=bogus").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = get(&db.pool, "/tag/howto/1.json?per_page=500").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn synonyms_redirect_to_their_target() {
    let db = TestDb::new().await;
    sqlx::query(
        "INSERT INTO tags (name, target_tag_id, created_at, updated_at) VALUES ('how-to', 1, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let (status, location, _) = get(&db.pool, "/tag/how-to.json?per_page=1").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/tag/howto/1.json?per_page=1")
    );
    // Named in tags[], a synonym resolves to its target.
    let (_, _, body) = get(&db.pool, "/tag/guide/2.json?tags%5B%5D=how-to").await;
    assert_eq!(ids(&body), vec![41, 38]);
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["topic_list"]["tags"][0]["name"], "howto");
}

#[tokio::test]
async fn hidden_tags_are_invisible_to_anonymous_users() {
    let db = TestDb::new().await;
    // howto (1) restricted to staff (group 3): its page 404s, it drops out
    // of topic tags, top_tags and the index; an everyone (0) permission
    // keeps it visible.
    restrict_tag(&db.pool, 1, 3).await;
    let (status, _, _) = get(&db.pool, "/tag/howto.json").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = get(&db.pool, "/tag/howto/1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, _, body) = get(&db.pool, "/latest.json").await;
    let json: Value = serde_json::from_str(&body).unwrap();
    let names: Vec<&str> = json["topic_list"]["top_tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["guide"]);
    let topic = json["topic_list"]["topics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == 41)
        .unwrap();
    assert_eq!(topic["tags"].as_array().unwrap().len(), 1);
    assert_eq!(topic["tags"][0]["name"], "guide");
    let (_, _, body) = get(&db.pool, "/tags.json").await;
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["tags"].as_array().unwrap().len(), 1);
    assert_eq!(json["tags"][0]["name"], "guide");
    // The intersection with a hidden tag is empty.
    let (_, _, body) = get(&db.pool, "/tag/guide/2.json?tags%5B%5D=howto").await;
    assert_eq!(ids(&body), Vec::<i64>::new());

    sqlx::query("UPDATE tag_group_permissions SET group_id = 0")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, _, body) = get(&db.pool, "/tag/howto.json").await;
    assert_eq!(status, StatusCode::OK);
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["topic_list"]["tags"][0]["staff"], false);
    sqlx::query("UPDATE tag_group_permissions SET permission_type = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, _, body) = get(&db.pool, "/tag/howto.json").await;
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["topic_list"]["tags"][0]["staff"], true);
}

#[tokio::test]
async fn category_restricted_tags_hide_outside_readable_categories() {
    let db = TestDb::new().await;
    // guide (2) allowed only in Staff (3, read_restricted).
    sqlx::query("INSERT INTO category_tags (category_id, tag_id, created_at, updated_at) VALUES (3, 2, now(), now())")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, _, _) = get(&db.pool, "/tag/guide.json").await;
    assert_eq!(status, StatusCode::OK, "hidden_tags only covers tag groups");
    let (_, _, body) = get(&db.pool, "/tags.json").await;
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["tags"].as_array().unwrap().len(), 1);
    assert_eq!(json["tags"][0]["name"], "howto");
    assert_eq!(json["extras"]["categories"], serde_json::json!([]));

    // Allowed in General instead: listed under it on the index.
    sqlx::query("UPDATE category_tags SET category_id = 4")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, _, body) = get(&db.pool, "/tags.json").await;
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["tags"].as_array().unwrap().len(), 2);
    assert_eq!(json["extras"]["categories"][0]["id"], 4);
    assert_eq!(json["extras"]["categories"][0]["tags"][0]["name"], "guide");
    assert!(json["extras"]["categories"][0]["name"].is_null());
}

#[tokio::test]
async fn tag_pages_render_html() {
    let db = TestDb::new().await;
    let (status, _, html) = get(&db.pool, "/tag/howto/1").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<title>Topics tagged howto - Discourse</title>"));
    assert!(html.contains(
        r#"<h1 class="tag-heading"><a href="/tag/howto/1" class="discourse-tag">howto</a></h1>"#
    ));
    assert!(html.contains(r#"href="/t/parity-fixture-liked-and-archived/41""#));
    assert!(!html.contains("welcome-to-discourse"));

    let (_, _, html) = get(&db.pool, "/tags/c/general/4/howto/1").await;
    assert!(html.contains(r#"<h1 class="category-heading">"#));
    assert!(html.contains(r#"class="tag-heading""#));

    let (status, _, html) = get(&db.pool, "/tags").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<title>Tags - Discourse</title>"));
    assert!(html.contains("<h3>Other Tags</h3>"));
    assert!(html.contains(r#"<a href="/tag/1" class="discourse-tag simple">howto</a>"#));
    assert!(html.contains(r#"<span class="tag-count">x 2</span>"#));
}
