//! Topic.similar_to and SimilarTopicsController, after
//! spec/models/topic_spec.rb's ".similar_to" and
//! spec/requests/similar_topics_controller_spec.rb. Where the reference
//! stubs `Topic.count` the minimum_topics_similar setting moves instead,
//! and its moderator is the seed's admin. Not ported: the zh_CN title
//! (segmentation is refused) and the plugin candidate ids modifier (no
//! bundled plugin registers it).

use crate::common;

use axum::body::Body;
use axum::http::{Request, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use discourse_rs::guardian::Guardian;
use discourse_rs::session::current::SessionUser;
use discourse_rs::site_settings::SiteSettings;
use discourse_rs::topics::similar;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

const INTEGER: i32 = 3;
const BOOL: i32 = 5;
/// The seed's General, Staff (read restricted to staff) and user0.
const GENERAL: i32 = 4;
const STAFF: i32 = 3;
const USER0: i32 = 2;

const TITLE: &str = "has evil trout made any topics?";
const RAW: &str = "i am wondering has evil trout made any topics?";

async fn setup() -> (TestDb, AppState) {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    (db, st)
}

async fn exec(pool: &PgPool, sql: &str) {
    sqlx::query(sql).execute(pool).await.unwrap();
}

/// `create_post(title:, raw:)` in a category with the search indexer on:
/// the topic, its first post and the topic's search data.
async fn topic(pool: &PgPool, title: &str, raw: &str, category: i32) -> i32 {
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO topics (title, fancy_title, slug, user_id, last_post_user_id, category_id, archetype, \
           created_at, updated_at, bumped_at, last_posted_at, posts_count, highest_post_number, visible) \
         VALUES ($1, $1, 'a-similar-topic', 2, 2, $2, 'regular', now(), now(), now(), now(), 1, 1, TRUE) \
         RETURNING id",
    )
    .bind(title)
    .bind(category)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO posts (topic_id, user_id, post_number, raw, cooked, created_at, updated_at, \
           last_version_at, sort_order) \
         VALUES ($1, 2, 1, $2, '<p>' || $2 || '</p>', now(), now(), now(), 1)",
    )
    .bind(id)
    .bind(raw)
    .execute(pool)
    .await
    .unwrap();
    reindex(pool, id).await;
    id
}

/// SearchIndexer's topic row: the title weighted A, the first post B.
async fn reindex(pool: &PgPool, topic_id: i32) {
    sqlx::query(
        "INSERT INTO topic_search_data (topic_id, raw_data, locale, version, search_data) \
         SELECT t.id, t.title || ' ' || p.raw, 'en', 4, \
                setweight(to_tsvector('english', t.title), 'A') || \
                setweight(to_tsvector('english', p.raw), 'B') \
         FROM topics t JOIN posts p ON p.topic_id = t.id AND p.post_number = 1 WHERE t.id = $1 \
         ON CONFLICT (topic_id) DO UPDATE SET raw_data = EXCLUDED.raw_data, \
           search_data = EXCLUDED.search_data",
    )
    .bind(topic_id)
    .execute(pool)
    .await
    .unwrap();
}

/// `Topic.similar_to(title, raw, user)`'s topic ids.
async fn similar_to(st: &AppState, title: &str, raw: Option<&str>, user: Option<i32>) -> Vec<i32> {
    let mut conn = st.pool.acquire().await.unwrap();
    let settings = SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap();
    let guardian = match user {
        Some(id) => {
            let user = SessionUser::load(&mut conn, id).await.unwrap().unwrap();
            Guardian::for_user(&mut conn, &user).await.unwrap()
        }
        None => Guardian::anonymous(),
    };
    similar::similar_to(st, &mut conn, &settings, &guardian, title, raw)
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.topic.id)
        .collect()
}

/// The spec's "with a similar topic".
async fn evil_trout(db: &TestDb) -> i32 {
    topic(
        &db.pool,
        "Evil trout is the dude who posted this topic",
        "Hello there, this is the body of the similar topic",
        GENERAL,
    )
    .await
}

// Topic.similar_to

#[tokio::test]
async fn nothing_for_blank_params() {
    let (_db, st) = setup().await;
    assert!(similar_to(&st, "", None, None).await.is_empty());
    assert!(similar_to(&st, "   ", Some(""), None).await.is_empty());
}

#[tokio::test]
async fn the_category_definition_is_left_out() {
    let (_db, st) = setup().await;
    assert!(
        similar_to(&st, "About the General category", Some("no body"), None)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn no_syntax_error_when_removing_accents() {
    let (db, st) = setup().await;
    set_setting(&db.pool, "search_ignore_accents", BOOL, "t").await;
    assert!(
        similar_to(&st, "something", Some("it's"), None)
            .await
            .is_empty()
    );
    assert!(
        similar_to(&st, "'bad quotes'", Some("'bad quotes'"), None)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn no_syntax_error_when_the_raw_cooks_to_nothing() {
    let (_db, st) = setup().await;
    assert!(
        similar_to(&st, "some title", Some("#"), None)
            .await
            .is_empty()
    );
    assert!(
        similar_to(
            &st,
            "some title",
            Some("https://discourse.org/#INCORRECT#URI"),
            None
        )
        .await
        .is_empty()
    );
}

#[tokio::test]
async fn the_similar_topic_by_its_title() {
    let (db, st) = setup().await;
    let id = evil_trout(&db).await;
    assert_eq!(similar_to(&st, TITLE, Some(RAW), None).await, [id]);
    // Even if raw is blank.
    assert_eq!(similar_to(&st, TITLE, Some(""), None).await, [id]);
    assert_eq!(similar_to(&st, TITLE, None, None).await, [id]);
}

#[tokio::test]
async fn title_against_title_and_raw_against_raw() {
    let (db, st) = setup().await;
    let id = evil_trout(&db).await;
    exec(
        &db.pool,
        &format!("UPDATE topics SET title = '1 2 3 numbered titles' WHERE id = {id}"),
    )
    .await;
    exec(
        &db.pool,
        &format!("UPDATE posts SET raw = 'random toy poodle' WHERE topic_id = {id}"),
    )
    .await;
    reindex(&db.pool, id).await;
    assert!(
        similar_to(&st, "unrelated term", Some("1 2 3 poddle"), None)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn numbered_lists_do_not_match_numbers_in_a_post() {
    let (db, st) = setup().await;
    let id = evil_trout(&db).await;
    exec(
        &db.pool,
        &format!(
            "UPDATE topics SET title = 'Where are we with browser support in 2019?' WHERE id = {id}"
        ),
    )
    .await;
    exec(
        &db.pool,
        &format!(
            "UPDATE posts SET raw = 'Internet Explorer 11+ Oct 2013 Google Chrome 32+ Jan 2014 \
             Firefox 27+ Feb 2014 Safari 6.1+ Jul 2012 Safari, iOS 8+ Oct 2014' WHERE topic_id = {id}"
        ),
    )
    .await;
    reindex(&db.pool, id).await;
    assert!(
        similar_to(
            &st,
            "Videos broken in composer",
            Some("1. Do something\n2. Do something else\n3. Do more things\n"),
            None
        )
        .await
        .is_empty()
    );
}

#[tokio::test]
async fn not_from_a_category_search_ignores() {
    let (db, st) = setup().await;
    let id = evil_trout(&db).await;
    assert_eq!(similar_to(&st, TITLE, Some(""), None).await, [id]);
    exec(
        &db.pool,
        &format!("UPDATE categories SET search_priority = 1 WHERE id = {GENERAL}"),
    )
    .await;
    assert!(similar_to(&st, TITLE, Some(""), None).await.is_empty());
}

#[tokio::test]
async fn not_from_a_category_the_user_muted() {
    let (db, st) = setup().await;
    let id = evil_trout(&db).await;
    assert_eq!(similar_to(&st, TITLE, Some(""), Some(USER0)).await, [id]);
    exec(
        &db.pool,
        &format!(
            "INSERT INTO category_users (category_id, user_id, notification_level) \
             VALUES ({GENERAL}, {USER0}, 0)"
        ),
    )
    .await;
    assert!(
        similar_to(&st, TITLE, Some(""), Some(USER0))
            .await
            .is_empty()
    );
    // Anyone else still finds it.
    assert_eq!(similar_to(&st, TITLE, Some(""), None).await, [id]);
}

#[tokio::test]
async fn not_from_a_child_of_a_category_the_user_muted() {
    let (db, st) = setup().await;
    let id = evil_trout(&db).await;
    // Sub General (34) is General's child.
    exec(
        &db.pool,
        &format!("UPDATE topics SET category_id = 34 WHERE id = {id}"),
    )
    .await;
    assert_eq!(similar_to(&st, TITLE, Some(""), Some(USER0)).await, [id]);
    exec(
        &db.pool,
        &format!(
            "INSERT INTO category_users (category_id, user_id, notification_level) \
             VALUES ({GENERAL}, {USER0}, 0)"
        ),
    )
    .await;
    assert!(
        similar_to(&st, TITLE, Some(""), Some(USER0))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn secure_categories_only_for_who_sees_them() {
    let (db, st) = setup().await;
    let id = evil_trout(&db).await;
    exec(
        &db.pool,
        &format!("UPDATE topics SET category_id = {STAFF} WHERE id = {id}"),
    )
    .await;
    assert!(
        similar_to(&st, TITLE, Some(RAW), Some(USER0))
            .await
            .is_empty()
    );
    assert!(similar_to(&st, TITLE, Some(RAW), None).await.is_empty());
    // group.add(user): the staff group the category is restricted to.
    exec(
        &db.pool,
        &format!(
            "INSERT INTO group_users (group_id, user_id, created_at, updated_at) \
             SELECT group_id, {USER0}, now(), now() FROM category_groups WHERE category_id = {STAFF}"
        ),
    )
    .await;
    assert_eq!(similar_to(&st, TITLE, Some(RAW), Some(USER0)).await, [id]);
}

// SimilarTopicsController

/// The spec's title and body, in words no seed topic shares.
const SPEC_TITLE: &str = "quokka burrow survey results were published";
const SPEC_RAW: &str = "the quokka burrow survey has its results published";

async fn get(st: &AppState, path: &str, cookie: Option<&str>) -> (u16, Value) {
    let mut request = Request::builder()
        .uri(path)
        .header(header::HOST, "test.localhost")
        .header(header::ACCEPT, "application/json");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    let response = discourse_rs::app(st.clone())
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    (
        status,
        serde_json::from_str(&text).unwrap_or_else(|_| panic!("{status} {text}")),
    )
}

/// A session for the username (password "password"): its cookies.
async fn login(st: &AppState, username: &str) -> String {
    let csrf_response = discourse_rs::app(st.clone())
        .oneshot(
            Request::builder()
                .uri("/session/csrf.json")
                .header(header::HOST, "test.localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let mut cookies: Vec<String> = csrf_response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string())
        .collect();
    let bytes = csrf_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let csrf: Value = serde_json::from_slice(&bytes).unwrap();
    let response = discourse_rs::app(st.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/session")
                .header(header::HOST, "test.localhost")
                .header("x-requested-with", "XMLHttpRequest")
                .header("x-csrf-token", csrf["csrf"].as_str().unwrap())
                .header(header::COOKIE, cookies.join("; "))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("login={username}&password=password")))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    for v in response.headers().get_all(header::SET_COOKIE) {
        let pair = v.to_str().unwrap().split(';').next().unwrap().to_string();
        let name = pair.split('=').next().unwrap().to_string();
        cookies.retain(|c| c.split('=').next() != Some(name.as_str()));
        cookies.push(pair);
    }
    cookies.join("; ")
}

fn path(route: &str) -> String {
    format!(
        "{route}?title={}&raw={}",
        SPEC_TITLE.replace(' ', "+"),
        SPEC_RAW.replace(' ', "+")
    )
}

#[tokio::test]
async fn requires_a_title_param() {
    let (_db, st) = setup().await;
    let (status, _) = get(&st, "/topics/similar_to.json?raw=some+body", None).await;
    assert_eq!(status, 400);
    let (status, _) = get(&st, "/similar_topics?title=", None).await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn nothing_below_the_minimum_title_length() {
    let (db, st) = setup().await;
    set_setting(&db.pool, "minimum_topics_similar", INTEGER, "0").await;
    set_setting(&db.pool, "min_title_similar_length", INTEGER, "100").await;
    topic(&db.pool, SPEC_TITLE, SPEC_RAW, GENERAL).await;
    let (status, json) = get(&st, &path("/topics/similar_to.json"), None).await;
    assert_eq!(status, 200);
    assert_eq!(json["similar_topics"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn similar_topics_once_the_site_has_enough_topics() {
    let (db, st) = setup().await;
    let id = topic(&db.pool, SPEC_TITLE, SPEC_RAW, GENERAL).await;

    // Fewer topics than minimum_topics_similar (50): Topic.similar_to is
    // not called.
    let (status, json) = get(&st, &path("/topics/similar_to.json"), None).await;
    assert_eq!(status, 200);
    assert_eq!(json["similar_topics"].as_array().unwrap().len(), 0);

    set_setting(&db.pool, "minimum_topics_similar", INTEGER, "0").await;
    for route in ["/topics/similar_to.json", "/similar_topics"] {
        let (status, json) = get(&st, &path(route), None).await;
        assert_eq!(status, 200);
        let similar = json["similar_topics"].as_array().unwrap();
        assert_eq!(similar.len(), 1, "{json}");
        assert_eq!(similar[0]["topic_id"], id);
        assert_eq!(similar[0]["id"], id);
        assert_eq!(similar[0]["blurb"], SPEC_RAW);
        assert_eq!(
            similar[0]["url"],
            format!("http://test.localhost/t/a-similar-topic/{id}")
        );
        assert_eq!(json["topics"][0]["id"], id);
        assert_eq!(json["topics"][0]["title"], SPEC_TITLE);
        assert_eq!(json["__rest_serializer"], "1");
    }
}

#[tokio::test]
async fn passes_the_user_through_when_logged_in() {
    let (db, st) = setup().await;
    set_setting(&db.pool, "minimum_topics_similar", INTEGER, "0").await;
    let private = topic(&db.pool, &format!("{SPEC_TITLE} 02"), SPEC_RAW, STAFF).await;
    let public = topic(&db.pool, SPEC_TITLE, SPEC_RAW, GENERAL).await;

    let ids = |json: &Value| -> Vec<i64> {
        let mut ids: Vec<i64> = json["similar_topics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["topic_id"].as_i64().unwrap())
            .collect();
        ids.sort_unstable();
        ids
    };
    let (_, json) = get(&st, &path("/topics/similar_to.json"), None).await;
    assert_eq!(ids(&json), [i64::from(public)]);

    let cookie = login(&st, "admin").await;
    let (status, json) = get(&st, &path("/topics/similar_to.json"), Some(&cookie)).await;
    assert_eq!(status, 200);
    let mut both = [i64::from(private), i64::from(public)];
    both.sort_unstable();
    assert_eq!(ids(&json), both);
}

#[tokio::test]
async fn the_popup_rows_for_a_member() {
    let (db, st) = setup().await;
    set_setting(&db.pool, "minimum_topics_similar", INTEGER, "0").await;
    let id = topic(&db.pool, SPEC_TITLE, SPEC_RAW, GENERAL).await;

    let (status, _) = get(&st, &path("/composer/similar-topics"), None).await;
    assert_eq!(status, 403);

    let cookie = login(&st, "user0").await;
    let (status, json) = get(&st, &path("/composer/similar-topics"), Some(&cookie)).await;
    assert_eq!(status, 200);
    assert_eq!(json["count"], 1);
    let html = json["html"].as_str().unwrap();
    assert!(html.starts_with("<div class=\"similar-topic\">"), "{html}");
    assert!(
        html.contains(&format!(
            "<span class=\"topic-title\" data-topic-id=\"{id}\">\n<a href=\"http://test.localhost/t/a-similar-topic/{id}\">"
        )),
        "{html}"
    );
    assert!(html.contains(SPEC_TITLE), "{html}");
    assert!(html.contains("badge-category"), "{html}");
    assert!(html.contains(SPEC_RAW), "{html}");

    let (_, json) = get(
        &st,
        "/composer/similar-topics?title=nothing+like+any+other+thing+zzzz",
        Some(&cookie),
    )
    .await;
    assert_eq!(json["count"], 0);
    assert_eq!(json["html"], "");
}

/// A page as a browser asks for it: its status and markup.
async fn page(st: &AppState, path: &str, cookie: &str) -> (u16, String) {
    let response = discourse_rs::app(st.clone())
        .oneshot(
            Request::builder()
                .uri(path)
                .header(header::HOST, "test.localhost")
                .header(header::ACCEPT, "text/html")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// `_findSimilar` skips a category with form templates: the composer's
/// category rows carry their ids.
#[tokio::test]
async fn the_chooser_rows_carry_their_form_template_ids() {
    let (db, st) = setup().await;
    let cookie = login(&st, "user0").await;
    let (status, html) = page(&st, "/latest", &cookie).await;
    assert_eq!(status, 200);
    assert!(html.contains("category-row"), "the composer's chooser");
    assert!(!html.contains("data-form-template-ids"));

    let template: i64 = sqlx::query_scalar(
        "INSERT INTO form_templates (name, template, created_at, updated_at) \
         VALUES ('A form', E'- type: input\\n  id: name\\n  attributes:\\n    label: Name', now(), now()) RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    exec(
        &db.pool,
        &format!(
            "INSERT INTO category_form_templates (category_id, form_template_id, created_at, updated_at) \
             VALUES ({GENERAL}, {template}, now(), now())"
        ),
    )
    .await;
    let (status, html) = page(&st, "/latest", &cookie).await;
    assert_eq!(status, 200, "{html}");
    let at = html.find("data-value=\"4\"").unwrap_or(0);
    assert_eq!(
        html.matches(&format!(" data-form-template-ids=\"{template}\""))
            .count(),
        1,
        "General's row alone: {}",
        &html[at.saturating_sub(200)..(at + 300).min(html.len())]
    );
}
