//! Tag topic lists (/tag/..., /tags/c/...) and the tags index for
//! anonymous users. The documents are covered by the parity golden files;
//! these pin routing, redirects, visibility and the HTML.

use crate::common;

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

/// TagsController#build_topic_list_options: TopicQueryParams' options
/// (every public one, present and valid) under its own raw ones.
#[tokio::test]
async fn tag_lists_take_the_list_params() {
    let db = TestDb::new().await;
    // howto: 41 (archived, in general) and 38 (in its subcategory); guide:
    // 41 and 35 (two posts); untagged: 5 and 37 (closed, in category 2).
    for (path, want) in [
        ("/tag/howto.json?status=archived", vec![41]),
        ("/tag/howto/1.json?status=open", vec![38]),
        ("/tag/howto.json?status=closed", vec![]),
        ("/tag/none.json?status=closed", vec![37]),
        ("/tags/c/general/4/howto/1.json?status=open", vec![38]),
        ("/tag/guide/2.json?min_posts=2", vec![35]),
        ("/tag/guide/2.json?max_posts=1", vec![41]),
        ("/tag/howto/1.json?topic_ids=38,5", vec![38]),
        ("/tag/howto/1.json?topic_ids[]=41", vec![41]),
        ("/tag/howto/1.json?topic_ids=", vec![]),
        ("/tag/howto/1.json?exclude_tag=guide", vec![38]),
        ("/tag/howto/1.json?before=36500", vec![]),
        ("/tag/howto/1.json?bumped_before=36500", vec![]),
        ("/tag/howto/1.json?search=zzznothing", vec![]),
        ("/tag/howto/1.json?search=subcategory", vec![38]),
        (
            "/tag/howto/1.json?order=created&ascending=true",
            vec![38, 41],
        ),
        ("/tag/howto/1.json?state=watching_first_post", vec![]),
        // The `category` param: a top-level slug or id (set_category).
        ("/tag/howto.json?category=general", vec![41, 38]),
        (
            "/tag/howto.json?category=general&no_subcategories=true",
            vec![41],
        ),
        (
            "/tag/howto.json?category=4&no_subcategories=false",
            vec![41, 38],
        ),
        ("/tag/guide/2.json?category=2", vec![]),
        // Any of the tags, where the tag routes otherwise want them all.
        ("/tag/howto/1.json?tags[]=howto&tags[]=guide", vec![41]),
        (
            "/tag/howto/1.json?tags[]=howto&tags[]=guide&match_all_tags=false",
            vec![35, 41, 38],
        ),
        (
            "/tag/howto.json?tags[]=guide&match_all_tags=false",
            vec![35, 41, 38],
        ),
        (
            "/tag/howto.json?tags[]=nope&match_all_tags=false",
            vec![41, 38],
        ),
        ("/tag/howto.json?tags[]=guide&match_all_tags=", vec![41]),
        // Blank values filter nothing.
        (
            "/tag/howto.json?status=&state=&filter=&search=&min_posts=&max_posts=&ascending=&q=",
            vec![41, 38],
        ),
    ] {
        let (status, _, body) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(ids(&body), want, "{path}");
    }

    // The next page keeps the options: the raw ones blank or not, the
    // param's category by its slug, the route's left to the path.
    for (path, want) in [
        (
            "/tag/howto.json?status=&order=&q=a%20b&ascending=&per_page=1",
            "/tag/howto?ascending=&match_all_tags=true&order=&page=1&per_page=1&q=a+b\
             &status=&tags%5B%5D=howto",
        ),
        (
            "/tag/howto/1.json?status=listed&per_page=1&match_all_tags=false&exclude_tag=x&page=",
            "/tag/howto/1?exclude_tag=x&match_all_tags=false&page=1&per_page=1&status=listed\
             &tags%5B%5D=howto",
        ),
        (
            "/tag/howto.json?category=4&no_subcategories=true&topic_ids=41,38&per_page=1",
            "/tag/howto?category=general&match_all_tags=true&no_subcategories=true&page=1\
             &per_page=1&tags%5B%5D=howto&topic_ids%5B%5D=41&topic_ids%5B%5D=38",
        ),
        (
            "/tags/c/general/4/howto/1.json?category=2&min_posts=1&per_page=1",
            "/tags/c/general/4/howto/1?match_all_tags=true&min_posts=1&page=1&per_page=1\
             &tags%5B%5D=howto",
        ),
        (
            "/tag/none.json?max_posts=5&per_page=1&username=bob",
            "/tag/none?max_posts=5&no_tags=true&page=1&per_page=1",
        ),
    ] {
        let (status, _, body) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        let json: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["topic_list"]["more_topics_url"], want, "{path}");
    }

    // A blank order is an order all the same: "default", so bumped_at
    // with no pinned topics first.
    sqlx::query("UPDATE topics SET pinned_at = now(), pinned_globally = true WHERE id = 38")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, _, body) = get(&db.pool, "/tag/howto.json").await;
    assert_eq!(ids(&body), vec![38, 41]);
    let (_, _, body) = get(&db.pool, "/tag/howto.json?order=").await;
    assert_eq!(ids(&body), vec![41, 38]);
    let (_, _, body) = get(&db.pool, "/latest.json?tags=howto&order=").await;
    assert_eq!(ids(&body), vec![38, 41]);
}

/// TopicQueryParams validates what is present, and set_category 404s on
/// a `category` param naming no top-level category the viewer can see.
#[tokio::test]
async fn tag_lists_check_the_list_params() {
    let db = TestDb::new().await;
    for (path, param) in [
        ("/tag/howto.json?status[]=closed", "status"),
        ("/tag/howto/1.json?min_posts=abc", "min_posts"),
        ("/tag/howto/1.json?before=-1", "before"),
        ("/tag/howto/1.json?match_all_tags=no", "match_all_tags"),
        (
            "/tag/howto/1.json?no_subcategories=maybe",
            "no_subcategories",
        ),
        ("/tag/howto/1.json?search[]=x", "search"),
        ("/tag/howto/1.json?order=%20&ascending=x", "ascending"),
        // Only a name route's tag_name makes `tags` an array.
        ("/tag/howto/1.json?tags[k]=v", "tags"),
    ] {
        let (status, _, body) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
        let json: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            json["errors"][0],
            format!("You supplied invalid parameters to the request: {param}"),
            "{path}"
        );
    }
    let (status, _, _) = get(&db.pool, "/tag/howto.json?tags[k]=v").await;
    assert_eq!(status, StatusCode::OK);

    for path in [
        "/tag/howto.json?category=nope",
        "/tag/howto.json?category=",
        // A subcategory, by slug or id, and one anonymous can't see.
        "/tag/howto.json?category=sub-general",
        "/tag/howto.json?category=34",
        "/tag/howto.json?category=staff",
        "/tag/howto/1?category=nope",
    ] {
        let (status, _, _) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
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
    // The `category` param's category takes the redirect under /tags/c.
    let (status, location, _) = get(&db.pool, "/tag/howto?category=general&status=open").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/tags/c/general/4/howto/1?category=general&status=open")
    );

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
    assert!(html.contains("<title>Latest howto topics - Discourse</title>"));
    // The tag's navigation (Ember draws no heading): its drop selected,
    // the pills under the tag, and the info button with its hidden panel.
    assert!(!html.contains(r#"class="tag-heading""#));
    assert!(html.contains(r#"tag-drop has-selection tag-howto"#));
    assert!(html.contains(r#"href="/tag/howto/1/l/latest""#));
    assert!(html.contains(r#"id="show-tag-info""#));
    assert!(html.contains(r#"<section class="tag-info discovery-heading " hidden>"#));
    assert!(html.contains(r#"href="/t/parity-fixture-liked-and-archived/41/1""#));
    assert!(!html.contains("welcome-to-discourse"));

    // A tag in a category: the category's navigation (below), no heading.
    let (_, _, html) = get(&db.pool, "/tags/c/general/4/howto/1").await;
    assert!(!html.contains(r#"<h1 class="category-heading">"#));
    assert!(!html.contains(r#"class="tag-heading""#));

    // The page lists what the JSON does, and stays the tag's own with a
    // `category` param.
    let (status, _, html) = get(&db.pool, "/tag/howto/1?status=open&category=general").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains(r#"href="/t/parity-fixture-tagged-in-a-subcategory/38/1""#));
    assert!(!html.contains(r#"href="/t/parity-fixture-liked-and-archived/41/1""#));
    assert!(!html.contains(r#"<h1 class="category-heading">"#));
    assert!(html.contains(r#"id="show-tag-info""#));
    let (_, _, html) = get(&db.pool, "/tag/howto/1?match_all_tags=false&tags[]=guide").await;
    assert!(html.contains(r#"href="/t/parity-fixture-liked-and-archived/41/1""#));
    assert!(!html.contains(r#"href="/t/parity-fixture-tagged-in-a-subcategory/38/1""#));

    let (status, _, html) = get(&db.pool, "/tags").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<title>Tags - Discourse</title>"));
    assert!(html.contains("<h3>Other Tags</h3>"));
    assert!(html.contains(r#"<a href="/tag/1" class="discourse-tag simple">howto</a>"#));
    assert!(html.contains(r#"<span class="tag-count">x 2</span>"#));
}

/// DNavigation on a tag's list in a category: BreadCrumbs' category drops
/// with the category selected and TagDrop with the tag, and
/// NavItem.pathFor's `/tags/c/<category>[/none]/<tag>/l/<filter>` pills
/// without `categories`. Source-derived (the chrome is Ember's).
#[tokio::test]
async fn a_tag_in_a_category_draws_the_category_navigation() {
    let db = TestDb::new().await;
    let (status, _, html) = get(&db.pool, "/tags/c/general/4/howto/1").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains(r#"<section class="navigation-container category-navigation">"#));
    assert!(html.contains("category-drop has-selection category-breadcrumb__category-selector"));
    assert!(html.contains(r#"data-name="General" data-value="4""#));
    // General has a subcategory: its drop, nothing selected.
    assert!(html.contains("category-drop category-breadcrumb__subcategory-selector"));
    assert!(html.contains(r#"tag-drop has-selection tag-howto"#));
    assert!(html.contains(r#"data-name="howto" data-value="1""#));
    assert!(html.contains(
        r#"<a aria-current="page" class="active" href="/tags/c/general/4/howto/1/l/latest">Latest</a>"#
    ));
    assert!(html.contains(r#"href="/tags/c/general/4/howto/1/l/hot""#));
    assert!(!html.contains("nav-item_categories"));
    // The tag's own controls stay on its own page.
    assert!(!html.contains(r#"id="show-tag-info""#));

    // The filter's pill is the active one.
    let (_, _, html) = get(&db.pool, "/tags/c/general/4/howto/1/l/hot").await;
    assert!(html.contains(
        r#"<a aria-current="page" class="active" href="/tags/c/general/4/howto/1/l/hot">Hot</a>"#
    ));

    // /none: "no subcategories" selected, and kept in the pills.
    let (_, _, html) = get(&db.pool, "/tags/c/general/4/none/howto/1").await;
    assert!(html.contains("category-drop has-selection category-breadcrumb__subcategory-selector"));
    assert!(html.contains(r#"href="/tags/c/general/4/none/howto/1/l/latest""#));

    // A subcategory: both levels selected, its full path in the pills.
    let (_, _, html) = get(&db.pool, "/tags/c/general/sub-general/34/howto/1").await;
    assert!(html.contains(r#"data-name="General" data-value="4""#));
    assert!(html.contains(r#"data-name="Sub General" data-value="34""#));
    assert!(html.contains(r#"href="/tags/c/general/sub-general/34/howto/1/l/latest""#));

    // The untagged list in a category: TagDrop's "no tags", and the tag's
    // slug alone in the pills.
    let (status, _, html) = get(&db.pool, "/tags/c/general/4/none").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("tag-drop tag-none"));
    assert!(html.contains(r#"data-name="no tags" data-value="""#));
    assert!(html.contains(r#"href="/tags/c/general/4/none/l/latest""#));

    // What navigation-bar.js links from the drops with both selected.
    let (status, _, _) = get(&db.pool, "/tags/c/-/4/howto/1").await;
    assert_eq!(status, StatusCode::OK);
}

/// `/tags/intersection/:tag_name/*additional_tag_names` (tags#show): the
/// topics carrying every tag, on the name route's page_params (the next
/// page is `/tag/:tag_name` with `tags[]`), repeats redirected away.
/// Source-derived: no recording covers the route.
#[tokio::test]
async fn tag_intersections_list_the_topics_with_every_tag() {
    let db = TestDb::new().await;
    // howto: 41 and 38; guide: 41 and 35.
    let (status, _, body) = get(&db.pool, "/tags/intersection/howto/guide.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&body), vec![41]);
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["topic_list"]["tags"].as_array().unwrap().len(), 2);
    let (_, _, body) = get(
        &db.pool,
        "/tags/intersection/howto/guide.json?per_page=1&category=general",
    )
    .await;
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["topic_list"]["more_topics_url"],
        "/tag/howto?category=general&match_all_tags=true&page=1&per_page=1\
         &tags%5B%5D=howto&tags%5B%5D=guide"
    );
    // A tag nothing carries empties it; an unknown first tag is a 404.
    let (status, _, body) = get(&db.pool, "/tags/intersection/howto/nope.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&body), Vec::<i64>::new());
    let (status, _, _) = get(&db.pool, "/tags/intersection/nope/howto.json").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Repeats and the first tag again: url_for(params), a 302.
    let (status, location, _) = get(
        &db.pool,
        "/tags/intersection/howto/guide/howto/guide.json?b=2&a=1",
    )
    .await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/tags/intersection/howto/guide.json?a=1&b=2")
    );
    let (status, location, _) = get(&db.pool, "/tags/intersection/howto/howto").await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(location.as_deref(), Some("http://test.localhost/tag/howto"));
    // One tag only is `/tags/*tag_name`'s redirect.
    let (status, location, _) = get(&db.pool, "/tags/intersection/howto").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/tag/intersection/howto")
    );

    // The page: no redirect to the tag's canonical URL, Navigation's
    // `tags-intersection`, the first tag in the title.
    let (status, _, html) = get(&db.pool, "/tags/intersection/howto/guide").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<title>Latest howto topics - Discourse</title>"));
    assert!(html.contains(" tag-howto tags-page tags-intersection\""));
    assert!(html.contains(r#"href="/t/parity-fixture-liked-and-archived/41/1""#));
    assert!(!html.contains(r#"href="/t/parity-fixture-tagged-in-a-subcategory/38/1""#));
    assert!(!html.contains(r#"id="navigation-bar""#));
    assert!(!html.contains(r#"id="show-tag-info""#));
    // BreadCrumbs: the category drop, then TagsIntersectionChooser with
    // the tags selected (the first its record, the others by name) in
    // place of TagDrop, and no heading of the port's own.
    assert!(!html.contains(r#"class="tag-heading""#));
    assert!(html.contains(r#"<ol class="category-breadcrumb"><li><details class="select-kit single-select combobox combo-box category-drop category-breadcrumb__category-selector">"#));
    assert!(!html.contains("tag-drop"));
    let chooser = html
        .split(r#"<li><details class="select-kit multi-select mini-tag-chooser tags-intersection-chooser has-selection" "#)
        .nth(1)
        .expect("the intersection chooser")
        .split("</details>")
        .next()
        .unwrap();
    assert!(chooser.contains(
        r#"data-tags="[{&quot;id&quot;:1,&quot;name&quot;:&quot;howto&quot;,&quot;slug&quot;:&quot;howto&quot;},{&quot;name&quot;:&quot;guide&quot;}]""#
    ));
    assert!(chooser.contains(
        r#"<summary data-name="howto,guide" data-value="1,guide" tabindex="0" class="select-kit-header multi-select-header">"#
    ));
    assert!(chooser.contains(
        r#"<span class="formatted-selection">howto, guide</span><svg class="fa d-icon d-icon-plus "#
    ));
    // MiniTagChooser's options: max_tags_per_topic, and no new tags for
    // an anonymous visitor.
    assert!(chooser.contains("&quot;maximum&quot;:5"));
    assert!(chooser.contains("&quot;allowAny&quot;:false"));
    // The script binding it comes with every page.
    let script = discourse_rs::assets::url("", "navigation-bar.js");
    assert!(html.contains(&script));
    let (_, _, js) = get(&db.pool, &script).await;
    assert!(js.contains("Discourse.miniTagChooser = miniTagChooser;"));
    assert!(js.contains("details.tags-intersection-chooser"));

    // A synonym first goes to its target's own list, a hidden tag is a 404.
    sqlx::query(
        "INSERT INTO tags (name, target_tag_id, created_at, updated_at) VALUES ('how-to', 1, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let (status, location, _) = get(&db.pool, "/tags/intersection/how-to/guide.json").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("http://test.localhost/tag/howto/1.json")
    );
    restrict_tag(&db.pool, 1, 3).await;
    let (status, _, _) = get(&db.pool, "/tags/intersection/howto/guide.json").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
