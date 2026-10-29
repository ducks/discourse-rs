//! discourse-templates, ported from the plugin's specs:
//! TemplatesController (templates_controller_spec), TemplatesSerializer
//! (templates_serializer_spec), the guardian and user extensions
//! (guardian_extension_spec, user_extension_spec), Topic#template? and
//! the usage count (topic_extension_spec), the private templates
//! (topic_query_extension_spec), the current user's can_use_templates
//! (current_user_serializer_spec) and the topic view's is_template, and
//! the UI drawn on the server: the filterable list with its filters and
//! variables (tag_filtering_spec, chat_composer_spec), the composer's
//! options menu item and the topic's template controls.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const BOOL: i32 = 5;
const INTEGER: i32 = 3;
const CATEGORY_LIST: i32 = 11;
const GROUP_LIST: i32 = 20;
const TAG_LIST: i32 = 21;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn send(&mut self, method: Method, path: &str, body: &str) -> (StatusCode, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest")
            .header(header::ACCEPT, "application/json");
        if !self.cookies.is_empty() {
            let cookie: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookie.join("; "));
        }
        if let Some(token) = &self.csrf {
            request = request.header("x-csrf-token", token.as_str());
        }
        if !body.is_empty() {
            request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap();
            let pair = v.split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn json(&mut self, method: Method, path: &str) -> (StatusCode, Value) {
        let (status, text) = self.send(method, path, "").await;
        (
            status,
            serde_json::from_str(&text).unwrap_or_else(|_| json!({ "text": text })),
        )
    }

    async fn anonymous(state: AppState) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client.send(Method::GET, "/session/csrf.json", "").await;
        let csrf: Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client::anonymous(state).await;
        client
            .send(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
            )
            .await;
        let (_, csrf) = client.send(Method::GET, "/session/csrf.json", "").await;
        let csrf: Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
    }

    /// The page as a browser asks for it (no XHR, no JSON).
    async fn page(&mut self, path: &str) -> (StatusCode, String) {
        let cookie: Vec<String> = self
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let mut request = Request::get(path).header(header::HOST, "test.localhost");
        if !cookie.is_empty() {
            request = request.header(header::COOKIE, cookie.join("; "));
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }
}

async fn exec(pool: &PgPool, sql: &str) {
    sqlx::query(sql).execute(pool).await.unwrap();
}

async fn group(pool: &PgPool, name: &str, users: &[i32]) -> i32 {
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO groups (name, created_at, updated_at) VALUES ($1, now(), now()) RETURNING id",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap();
    for user in users {
        sqlx::query(
            "INSERT INTO group_users (group_id, user_id, created_at, updated_at) VALUES ($1, $2, now(), now())",
        )
        .bind(id)
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
    }
    id
}

/// `Fabricate(:category_with_definition)`, private to `group` when given
/// (`:private_category_with_definition`).
async fn category(pool: &PgPool, name: &str, parent: Option<i32>, group: Option<i32>) -> i32 {
    let slug = name.to_lowercase().replace(' ', "-");
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO categories (name, name_lower, slug, color, text_color, user_id, read_restricted, parent_category_id, created_at, updated_at) \
         VALUES ($1, lower($1), $2, '0088CC', 'FFFFFF', 1, $3, $4, now(), now()) RETURNING id",
    )
    .bind(name)
    .bind(&slug)
    .bind(group.is_some())
    .bind(parent)
    .fetch_one(pool)
    .await
    .unwrap();
    if let Some(group) = group {
        sqlx::query(
            "INSERT INTO category_groups (category_id, group_id, permission_type, created_at, updated_at) \
             VALUES ($1, $2, 1, now(), now())",
        )
        .bind(id)
        .bind(group)
        .execute(pool)
        .await
        .unwrap();
    }
    let definition = topic(
        pool,
        &format!("About the {name} category"),
        Some(id),
        1,
        "regular",
        "The definition.",
    )
    .await;
    sqlx::query("UPDATE categories SET topic_id = $1 WHERE id = $2")
        .bind(definition)
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    id
}

/// A topic and its first post (`Fabricate(:template_item)`).
async fn topic(
    pool: &PgPool,
    title: &str,
    category: Option<i32>,
    user: i32,
    archetype: &str,
    raw: &str,
) -> i32 {
    let slug = title.to_lowercase().replace(' ', "-");
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO topics (title, fancy_title, slug, user_id, last_post_user_id, category_id, archetype, subtype, \
           created_at, updated_at, bumped_at, posts_count, highest_post_number, visible) \
         VALUES ($1, $1, $2, $3, $3, $4, $5, CASE WHEN $5 = 'private_message' THEN 'user_to_user' END, \
           now(), now(), now(), 1, 1, TRUE) RETURNING id",
    )
    .bind(title)
    .bind(&slug)
    .bind(user)
    .bind(category)
    .bind(archetype)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO posts (topic_id, user_id, post_number, raw, cooked, created_at, updated_at, last_version_at, sort_order) \
         VALUES ($1, $2, 1, $3, '<p>' || $3 || '</p>', now(), now(), now(), 1)",
    )
    .bind(id)
    .bind(user)
    .bind(raw)
    .execute(pool)
    .await
    .unwrap();
    if archetype == "private_message" {
        sqlx::query(
            "INSERT INTO topic_allowed_users (topic_id, user_id, created_at, updated_at) VALUES ($1, $2, now(), now())",
        )
        .bind(id)
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
    }
    id
}

async fn tag(pool: &PgPool, name: &str, topics: &[i32]) -> i32 {
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO tags (name, created_at, updated_at) VALUES ($1, now(), now()) RETURNING id",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap();
    for topic in topics {
        sqlx::query(
            "INSERT INTO topic_tags (topic_id, tag_id, created_at, updated_at) VALUES ($1, $2, now(), now())",
        )
        .bind(topic)
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    }
    id
}

/// The controller spec's fixture: users are admin (1), user0 (2), user1
/// (3, in group1), user2 (4, in group2) and user3 (5, a moderator).
struct Fixture {
    db: TestDb,
    st: AppState,
    parent: i32,
    other_parent: i32,
    sub_moderators: i32,
    sub_group1: i32,
    sub_group2: i32,
    sub_everyone: i32,
    /// template_item0 to 7.
    items: Vec<i32>,
    from_other_parent: i32,
    other_topic: i32,
}

async fn fixture() -> Fixture {
    let db = TestDb::new().await;
    let pool = &db.pool;
    set_setting(pool, "tagging_enabled", BOOL, "t").await;
    exec(pool, "UPDATE users SET moderator = TRUE WHERE id = 5").await;
    exec(
        pool,
        "INSERT INTO group_users (group_id, user_id, created_at, updated_at) VALUES (2, 5, now(), now()), (3, 5, now(), now())",
    )
    .await;
    let group1 = group(pool, "group1", &[3]).await;
    let group2 = group(pool, "group2", &[4]).await;
    let parent = category(pool, "Templates Parent", None, None).await;
    let other_parent = category(pool, "Templates Other", None, None).await;
    let sub_moderators = category(pool, "Sub Moderators", Some(parent), Some(2)).await;
    let sub_group1 = category(pool, "Sub Group One", Some(parent), Some(group1)).await;
    let sub_group2 = category(pool, "Sub Group Two", Some(parent), Some(group2)).await;
    let sub_everyone = category(pool, "Sub Everyone", Some(parent), None).await;
    let in_category = [
        parent,
        parent,
        parent,
        sub_moderators,
        sub_group1,
        sub_group2,
        sub_everyone,
        sub_everyone,
    ];
    let mut items = Vec::new();
    for (i, c) in in_category.iter().enumerate() {
        items.push(
            topic(
                pool,
                &format!("This is a test template {i:06}"),
                Some(*c),
                1,
                "regular",
                &format!("This is the content of an awesome template {i:06}"),
            )
            .await,
        );
    }
    let from_other_parent = topic(
        pool,
        "This is a test template 000008",
        Some(other_parent),
        1,
        "regular",
        "This is the content of an awesome template 000008",
    )
    .await;
    let other_topic = topic(
        pool,
        "This is a test template 000009",
        Some(1),
        1,
        "regular",
        "Uncategorized content",
    )
    .await;
    let category_tag = tag(pool, "category-tag", &[items[4]]).await;
    sqlx::query(
        "INSERT INTO category_tags (category_id, tag_id, created_at, updated_at) VALUES ($1, $2, now(), now())",
    )
    .bind(sub_moderators)
    .bind(category_tag)
    .execute(pool)
    .await
    .unwrap();
    tag(
        pool,
        "use-anywhere",
        &[items[0], items[1], items[2], items[6], items[7]],
    )
    .await;
    tag(pool, "use-group", &[items[4], items[5]]).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    Fixture {
        db,
        st,
        parent,
        other_parent,
        sub_moderators,
        sub_group1,
        sub_group2,
        sub_everyone,
        items,
        from_other_parent,
        other_topic,
    }
}

impl Fixture {
    async fn categories(&self, ids: &[i32]) {
        let value: Vec<String> = ids.iter().map(i32::to_string).collect();
        set_setting(
            &self.db.pool,
            "discourse_templates_categories",
            CATEGORY_LIST,
            &value.join("|"),
        )
        .await;
    }

    /// `serialize_topics(topics, user)`: what the list should hold for
    /// these templates, in title order, with the tags the viewer sees.
    async fn expected(&self, ids: &[i32], hidden_tags: &[&str]) -> Value {
        let mut out = Vec::new();
        for id in ids {
            let (title, slug, raw): (String, String, String) = sqlx::query_as(
                "SELECT t.title, t.slug, p.raw FROM topics t JOIN posts p ON p.topic_id = t.id AND p.post_number = 1 WHERE t.id = $1",
            )
            .bind(id)
            .fetch_one(&self.db.pool)
            .await
            .unwrap();
            let mut tags: Vec<String> = sqlx::query_scalar(
                "SELECT tags.name FROM topic_tags JOIN tags ON tags.id = topic_tags.tag_id WHERE topic_id = $1",
            )
            .bind(id)
            .fetch_all(&self.db.pool)
            .await
            .unwrap();
            tags.retain(|t| !hidden_tags.contains(&t.as_str()));
            tags.sort();
            let usages: Option<i32> = sqlx::query_scalar(
                "SELECT usage_count FROM discourse_templates_usage_count WHERE topic_id = $1",
            )
            .bind(id)
            .fetch_optional(&self.db.pool)
            .await
            .unwrap();
            out.push(json!({
                "id": id, "title": title, "slug": slug, "content": raw,
                "tags": tags, "usages": usages.unwrap_or(0),
            }));
        }
        out.sort_by(|a, b| a["title"].as_str().cmp(&b["title"].as_str()));
        Value::Array(out)
    }

    async fn list(&self, username: &str) -> (StatusCode, Value) {
        let mut client = Client::logged_in(self.st.clone(), username).await;
        client.json(Method::GET, "/discourse_templates").await
    }
}

#[tokio::test]
async fn lists_topics_in_the_templates_category() {
    let f = fixture().await;
    f.categories(&[f.sub_everyone]).await;
    let (status, body) = f.list("user0").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["templates"],
        f.expected(&[f.items[6], f.items[7]], &[]).await
    );
}

#[tokio::test]
async fn omits_tags_hidden_by_tag_group_permissions() {
    let f = fixture().await;
    f.categories(&[f.sub_everyone]).await;
    let hidden = tag(&f.db.pool, "staff-only", &[f.items[6]]).await;
    let tag_group: i32 = sqlx::query_scalar(
        "INSERT INTO tag_groups (name, created_at, updated_at) VALUES ('staff tags', now(), now()) RETURNING id",
    )
    .fetch_one(&f.db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO tag_group_memberships (tag_id, tag_group_id, created_at, updated_at) VALUES ($1, $2, now(), now())",
    )
    .bind(hidden)
    .bind(tag_group)
    .execute(&f.db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO tag_group_permissions (tag_group_id, group_id, permission_type, created_at, updated_at) VALUES ($1, 3, 1, now(), now())",
    )
    .bind(tag_group)
    .execute(&f.db.pool)
    .await
    .unwrap();
    let (status, body) = f.list("user0").await;
    assert_eq!(status, StatusCode::OK);
    let item = body["templates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == f.items[6])
        .unwrap();
    assert_eq!(item["tags"], json!(["use-anywhere"]));
}

#[tokio::test]
async fn lists_topics_from_multiple_parent_categories() {
    let f = fixture().await;
    f.categories(&[f.sub_everyone, f.other_parent]).await;
    let (_, body) = f.list("user0").await;
    assert_eq!(
        body["templates"],
        f.expected(&[f.items[6], f.items[7], f.from_other_parent], &[])
            .await
    );
}

#[tokio::test]
async fn lists_visible_topics_in_the_parent_category_and_subcategories() {
    let f = fixture().await;
    f.categories(&[f.parent]).await;
    let (_, body) = f.list("user0").await;
    let i = &f.items;
    assert_eq!(
        body["templates"],
        f.expected(&[i[0], i[1], i[2], i[6], i[7]], &[]).await
    );
}

#[tokio::test]
async fn rejects_templates_when_the_user_cannot_see_the_category() {
    let f = fixture().await;
    for category in [f.sub_moderators, f.sub_group1, f.sub_group2] {
        f.categories(&[category]).await;
        let (status, body) = f.list("user0").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["error_type"], "invalid_access");
    }
}

#[tokio::test]
async fn a_moderator_sees_the_moderators_subcategory() {
    let f = fixture().await;
    f.categories(&[f.parent, f.other_parent]).await;
    let (_, body) = f.list("user3").await;
    let i = &f.items;
    assert_eq!(
        body["templates"],
        f.expected(
            &[i[0], i[1], i[2], i[3], i[6], i[7], f.from_other_parent],
            &[]
        )
        .await
    );
}

#[tokio::test]
async fn group_members_see_their_groups_subcategories() {
    let f = fixture().await;
    f.categories(&[f.parent, f.other_parent]).await;
    let i = &f.items;
    // template_item4 carries category-tag, which only shows in the
    // moderators' subcategory.
    let (_, body) = f.list("user1").await;
    assert_eq!(
        body["templates"],
        f.expected(
            &[i[0], i[1], i[2], i[4], i[6], i[7], f.from_other_parent],
            &["category-tag"]
        )
        .await
    );
    let (_, body) = f.list("user2").await;
    assert_eq!(
        body["templates"],
        f.expected(
            &[i[0], i[1], i[2], i[5], i[6], i[7], f.from_other_parent],
            &[]
        )
        .await
    );
}

#[tokio::test]
async fn an_admin_sees_everything_but_deleted_archived_and_unlisted_topics() {
    let f = fixture().await;
    f.categories(&[f.parent, f.other_parent]).await;
    let i = &f.items;
    let (_, body) = f.list("admin").await;
    assert_eq!(
        body["templates"],
        f.expected(
            &[
                i[0],
                i[1],
                i[2],
                i[3],
                i[4],
                i[5],
                i[6],
                i[7],
                f.from_other_parent
            ],
            &[]
        )
        .await
    );
    for (sql, id) in [
        (
            "UPDATE topics SET deleted_at = now(), deleted_by_id = 1 WHERE id = $1",
            i[0],
        ),
        ("UPDATE topics SET archived = TRUE WHERE id = $1", i[1]),
        ("UPDATE topics SET visible = FALSE WHERE id = $1", i[2]),
    ] {
        sqlx::query(sql).bind(id).execute(&f.db.pool).await.unwrap();
    }
    let (_, body) = f.list("admin").await;
    assert_eq!(
        body["templates"],
        f.expected(&[i[3], i[4], i[5], i[6], i[7], f.from_other_parent], &[])
            .await
    );
}

#[tokio::test]
async fn excludes_the_category_definition_and_puts_pinned_templates_first() {
    let f = fixture().await;
    f.categories(&[f.sub_everyone]).await;
    // A globally pinned template leads; the definition topic never shows.
    sqlx::query("UPDATE topics SET pinned_globally = TRUE, pinned_at = now() WHERE id = $1")
        .bind(f.items[7])
        .execute(&f.db.pool)
        .await
        .unwrap();
    let (_, body) = f.list("user0").await;
    let ids: Vec<i64> = body["templates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, vec![i64::from(f.items[7]), i64::from(f.items[6])]);
}

#[tokio::test]
async fn honours_the_max_replies_fetched() {
    let f = fixture().await;
    f.categories(&[f.parent]).await;
    set_setting(
        &f.db.pool,
        "discourse_templates_max_replies_fetched",
        INTEGER,
        "2",
    )
    .await;
    let (_, body) = f.list("user0").await;
    assert_eq!(body["templates"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn tags_are_left_out_without_tagging() {
    // templates_serializer_spec
    let f = fixture().await;
    f.categories(&[f.sub_everyone]).await;
    set_setting(&f.db.pool, "tagging_enabled", BOOL, "f").await;
    let (_, body) = f.list("user0").await;
    let first = &body["templates"][0];
    assert!(first.get("tags").is_none(), "{first}");
    assert_eq!(
        first.as_object().unwrap().keys().collect::<Vec<_>>(),
        vec!["id", "title", "slug", "content", "usages"]
    );
}

#[tokio::test]
async fn an_anonymous_visitor_gets_not_found() {
    let f = fixture().await;
    f.categories(&[f.sub_everyone]).await;
    let mut anon = Client::anonymous(f.st.clone()).await;
    let (status, _) = anon.page("/discourse_templates").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = anon.json(Method::GET, "/discourse_templates.json").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error_type"], "not_logged_in");
}

#[tokio::test]
async fn the_endpoints_are_not_found_while_the_plugin_is_off() {
    let f = fixture().await;
    f.categories(&[f.sub_everyone]).await;
    set_setting(&f.db.pool, "discourse_templates_enabled", BOOL, "f").await;
    let mut client = Client::logged_in(f.st.clone(), "admin").await;
    let (status, _) = client.json(Method::GET, "/discourse_templates").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let path = format!("/discourse_templates/{}/use", f.items[6]);
    let (status, _) = client.json(Method::POST, &path).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = client.json(Method::GET, "/live/templates").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn use_checks_the_id_belongs_to_a_template() {
    let f = fixture().await;
    f.categories(&[f.parent]).await;
    let mut admin = Client::logged_in(f.st.clone(), "admin").await;
    let gone = topic(
        &f.db.pool,
        "Gone template",
        Some(f.parent),
        1,
        "regular",
        "x",
    )
    .await;
    exec(
        &f.db.pool,
        &format!("DELETE FROM posts WHERE topic_id = {gone}"),
    )
    .await;
    exec(&f.db.pool, &format!("DELETE FROM topics WHERE id = {gone}")).await;
    let (status, body) = admin
        .json(Method::POST, &format!("/discourse_templates/{gone}/use"))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body, json!({ "errors": ["Invalid template id"] }));
    let (status, body) = admin
        .json(
            Method::POST,
            &format!("/discourse_templates/{}/use", f.other_topic),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        json!({ "errors": ["Id does not belong to a template"] })
    );
    for id in [f.items[0], f.items[3], f.items[4], f.items[5], f.items[6]] {
        let (status, text) = admin
            .send(Method::POST, &format!("/discourse_templates/{id}/use"), "")
            .await;
        assert_eq!(status, StatusCode::OK, "{id}");
        // `render json: record`: the rows DB.exec wrote.
        assert_eq!(text, "1");
    }
}

#[tokio::test]
async fn use_increments_the_usage_count() {
    let f = fixture().await;
    f.categories(&[f.sub_moderators]).await;
    let mut moderator = Client::logged_in(f.st.clone(), "user3").await;
    let (_, body) = moderator.json(Method::GET, "/discourse_templates").await;
    assert_eq!(body["templates"], f.expected(&[f.items[3]], &[]).await);
    assert_eq!(body["templates"][0]["usages"], 0);
    let path = format!("/discourse_templates/{}/use", f.items[3]);
    for expected in 1..=3 {
        let (status, _) = moderator.json(Method::POST, &path).await;
        assert_eq!(status, StatusCode::OK);
        let (_, body) = moderator.json(Method::GET, "/discourse_templates").await;
        assert_eq!(body["templates"][0]["usages"], expected);
    }
}

#[tokio::test]
async fn use_refuses_a_template_the_user_cannot_see() {
    let f = fixture().await;
    f.categories(&[f.parent]).await;
    let mut user = Client::logged_in(f.st.clone(), "user0").await;
    let (status, _) = user
        .json(
            Method::POST,
            &format!("/discourse_templates/{}/use", f.items[4]),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = user
        .json(
            Method::POST,
            &format!("/discourse_templates/{}/use", f.items[0]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn use_needs_the_csrf_token() {
    let f = fixture().await;
    f.categories(&[f.parent]).await;
    let mut user = Client::logged_in(f.st.clone(), "user0").await;
    user.csrf = None;
    let (status, _) = user
        .json(
            Method::POST,
            &format!("/discourse_templates/{}/use", f.items[0]),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// The current user's can_use_templates.
async fn can_use(st: &AppState, username: &str) -> Value {
    let mut client = Client::logged_in(st.clone(), username).await;
    let (_, body) = client.json(Method::GET, "/session/current.json").await;
    body["current_user"]["can_use_templates"].clone()
}

#[tokio::test]
async fn can_use_category_templates_follows_category_access() {
    // guardian_extension_spec: can_use_category_templates?
    let f = fixture().await;
    assert_eq!(can_use(&f.st, "user0").await, false);
    f.categories(&[f.parent]).await;
    assert_eq!(can_use(&f.st, "user0").await, true);
    f.categories(&[f.sub_moderators]).await;
    assert_eq!(can_use(&f.st, "user3").await, true);
    assert_eq!(can_use(&f.st, "user0").await, false);
    // Any of several, in order: one the member sees allows.
    f.categories(&[f.sub_moderators, f.sub_group1]).await;
    assert_eq!(can_use(&f.st, "user3").await, true);
    assert_eq!(can_use(&f.st, "user1").await, true);
    assert_eq!(can_use(&f.st, "user0").await, false);
    // A missing category first refuses, whatever follows.
    f.categories(&[99999, f.parent]).await;
    assert_eq!(can_use(&f.st, "user0").await, false);
}

#[tokio::test]
async fn can_use_private_templates_for_staff_and_allowed_groups() {
    // guardian_extension_spec: can_use_private_templates?
    let f = fixture().await;
    let pool = &f.db.pool;
    set_setting(
        pool,
        "discourse_templates_enable_private_templates",
        BOOL,
        "t",
    )
    .await;
    set_setting(
        pool,
        "discourse_templates_private_templates_tags",
        TAG_LIST,
        "private-templates|templates",
    )
    .await;
    assert_eq!(can_use(&f.st, "user3").await, true);
    assert_eq!(can_use(&f.st, "user1").await, false);
    let group1: i32 = sqlx::query_scalar("SELECT id FROM groups WHERE name = 'group1'")
        .fetch_one(pool)
        .await
        .unwrap();
    let group2: i32 = sqlx::query_scalar("SELECT id FROM groups WHERE name = 'group2'")
        .fetch_one(pool)
        .await
        .unwrap();
    let allowed = "discourse_templates_groups_allowed_private_templates";
    set_setting(pool, allowed, GROUP_LIST, &group1.to_string()).await;
    assert_eq!(can_use(&f.st, "user1").await, true);
    assert_eq!(can_use(&f.st, "user2").await, false);
    set_setting(pool, allowed, GROUP_LIST, &format!("{group1}|{group2}")).await;
    assert_eq!(can_use(&f.st, "user2").await, true);
    // everyone allows anyone.
    set_setting(pool, allowed, GROUP_LIST, "0").await;
    assert_eq!(can_use(&f.st, "user0").await, true);
    // Without tagging, nobody.
    set_setting(pool, "tagging_enabled", BOOL, "f").await;
    assert_eq!(can_use(&f.st, "user3").await, false);
}

/// The private templates' settings on, staff only, with `tag-a`.
async fn private_templates(f: &Fixture) -> (i32, i32, i32) {
    let pool = &f.db.pool;
    set_setting(
        pool,
        "discourse_templates_enable_private_templates",
        BOOL,
        "t",
    )
    .await;
    set_setting(
        pool,
        "discourse_templates_private_templates_tags",
        TAG_LIST,
        "tag-a|tag-b",
    )
    .await;
    set_setting(
        pool,
        "discourse_templates_groups_allowed_private_templates",
        GROUP_LIST,
        "0",
    )
    .await;
    let mine = topic(
        pool,
        "A private test template 000001",
        None,
        3,
        "private_message",
        "Private content",
    )
    .await;
    let untagged = topic(
        pool,
        "A private test template 000002",
        None,
        3,
        "private_message",
        "Untagged",
    )
    .await;
    let theirs = topic(
        pool,
        "A private test template 000003",
        None,
        4,
        "private_message",
        "Theirs",
    )
    .await;
    sqlx::query(
        "INSERT INTO topic_allowed_users (topic_id, user_id, created_at, updated_at) VALUES ($1, 3, now(), now())",
    )
    .bind(theirs)
    .execute(pool)
    .await
    .unwrap();
    tag(pool, "tag-a", &[mine, theirs]).await;
    tag(pool, "tag-c", &[untagged]).await;
    (mine, untagged, theirs)
}

#[tokio::test]
async fn private_templates_are_the_members_own_tagged_messages() {
    let f = fixture().await;
    let (mine, _, theirs) = private_templates(&f).await;
    let (status, body) = f.list("user1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["templates"], f.expected(&[mine], &[]).await);
    // Archived by the member: gone.
    sqlx::query(
        "INSERT INTO user_archived_messages (user_id, topic_id, created_at, updated_at) VALUES (3, $1, now(), now())",
    )
    .bind(mine)
    .execute(&f.db.pool)
    .await
    .unwrap();
    let (_, body) = f.list("user1").await;
    assert_eq!(body["templates"], json!([]));
    // The author's, tagged: theirs.
    let (_, body) = f.list("user2").await;
    assert_eq!(body["templates"], f.expected(&[theirs], &[]).await);
}

#[tokio::test]
async fn category_templates_come_before_private_ones() {
    let f = fixture().await;
    f.categories(&[f.sub_everyone]).await;
    let (mine, _, _) = private_templates(&f).await;
    let (_, body) = f.list("user1").await;
    let ids: Vec<i64> = body["templates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![
            i64::from(f.items[6]),
            i64::from(f.items[7]),
            i64::from(mine)
        ]
    );
}

#[tokio::test]
async fn a_private_template_can_be_used_by_its_author_only() {
    // topic_extension_spec: template? on private messages.
    let f = fixture().await;
    let (mine, untagged, theirs) = private_templates(&f).await;
    let mut user1 = Client::logged_in(f.st.clone(), "user1").await;
    let path = |id: i32| format!("/discourse_templates/{id}/use");
    let (status, _) = user1.json(Method::POST, &path(mine)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = user1.json(Method::POST, &path(untagged)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    // A message user1 is in, but not theirs.
    let (status, _) = user1.json(Method::POST, &path(theirs)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    // Only tag-b left: tag-a's no longer count.
    set_setting(
        &f.db.pool,
        "discourse_templates_private_templates_tags",
        TAG_LIST,
        "tag-b",
    )
    .await;
    set_setting(
        &f.db.pool,
        "discourse_templates_categories",
        CATEGORY_LIST,
        &f.parent.to_string(),
    )
    .await;
    let (status, _) = user1.json(Method::POST, &path(mine)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn the_topic_view_marks_templates() {
    let f = fixture().await;
    f.categories(&[f.parent]).await;
    let mut user = Client::logged_in(f.st.clone(), "user0").await;
    let (_, view) = user
        .json(Method::GET, &format!("/t/{}.json", f.items[6]))
        .await;
    assert_eq!(view["is_template"], true);
    let (_, view) = user
        .json(Method::GET, &format!("/t/{}.json", f.other_topic))
        .await;
    assert!(view.get("is_template").is_none());
}

#[tokio::test]
async fn the_topic_page_shows_the_template_controls() {
    let f = fixture().await;
    f.categories(&[f.parent]).await;
    exec(
        &f.db.pool,
        &format!(
            "UPDATE posts SET raw = E'<!-- discourse-template\\ncategory: sub-everyone\\ntags: one, two\\n-->\\n\\nThe body' \
             WHERE topic_id = {} AND post_number = 1",
            f.items[6]
        ),
    )
    .await;
    let mut user = Client::logged_in(f.st.clone(), "user1").await;
    let (status, html) = user
        .page(&format!("/t/this-is-a-test-template-000006/{}", f.items[6]))
        .await;
    assert_eq!(status, StatusCode::OK, "{html}");
    // sub-everyone is a top-level slug only under its parent: no category.
    assert!(
        html.contains(&format!(
            "<div class=\"template-topic-controls\" data-topic-id=\"{}\" data-body=\"The body\" data-category-id=\"\" data-tags=\"one,two\">",
            f.items[6]
        )),
        "{html}"
    );
    assert!(html.contains("template-copy"));
    assert!(html.contains("New topic from template"));
    exec(
        &f.db.pool,
        &format!(
            "UPDATE posts SET raw = E'<!-- discourse-template\\ncategory: templates-parent/sub-everyone\\n-->\\nBody' \
             WHERE topic_id = {} AND post_number = 1",
            f.items[6]
        ),
    )
    .await;
    let (_, html) = user
        .page(&format!("/t/this-is-a-test-template-000006/{}", f.items[6]))
        .await;
    assert!(
        html.contains(&format!("data-category-id=\"{}\"", f.sub_everyone)),
        "{html}"
    );
    let (_, html) = user
        .page(&format!(
            "/t/this-is-a-test-template-000009/{}",
            f.other_topic
        ))
        .await;
    assert!(!html.contains("template-topic-controls"));
}

#[tokio::test]
async fn the_composer_offers_insert_template_to_members_who_can_use_them() {
    let f = fixture().await;
    let mut user = Client::logged_in(f.st.clone(), "user0").await;
    let (_, html) = user.page("/latest").await;
    assert!(!html.contains("discourse-templates"));
    assert!(!html.contains("templates.js"));
    f.categories(&[f.parent]).await;
    let (_, html) = user.page("/latest").await;
    assert!(
        html.contains("&quot;name&quot;:&quot;discourse-templates&quot;"),
        "options"
    );
    assert!(html.contains("&quot;label&quot;:&quot;Insert template&quot;"));
    // No longer refused as unported.
    assert!(html.contains("&quot;unported&quot;:[]"), "unported");
    assert!(html.contains("/assets/templates.js"));
    assert!(html.contains("data-label-insert=\"Insert template\""));
}

/// GET /live/templates with a query.
async fn live(client: &mut Client, query: &str) -> String {
    let (status, html) = client
        .send(Method::GET, &format!("/live/templates?{query}"), "")
        .await;
    assert_eq!(status, StatusCode::OK, "{html}");
    html
}

fn shown(html: &str) -> Vec<i32> {
    html.match_indices("id=\"template-item-")
        .map(|(at, m)| {
            let rest = &html[at + m.len()..];
            rest[..rest.find('"').unwrap()].parse().unwrap()
        })
        .collect()
}

#[tokio::test]
async fn the_list_filters_by_keyword_and_tag() {
    // tag_filtering_spec, with the keyword filter beside it.
    let f = fixture().await;
    let pool = &f.db.pool;
    let templates = category(pool, "Templates", None, None).await;
    f.categories(&[templates]).await;
    let mut ids = Vec::new();
    for (i, name) in ["with tag 1", "with tag 2", "both tags", "no tags"]
        .iter()
        .enumerate()
    {
        ids.push(
            topic(
                pool,
                &format!("Template {i} {name}"),
                Some(templates),
                1,
                "regular",
                &format!("Content of {name}"),
            )
            .await,
        );
    }
    tag(pool, "tag1", &[ids[0], ids[2]]).await;
    tag(pool, "tag2", &[ids[1], ids[2]]).await;
    let mut user = Client::logged_in(f.st.clone(), "user0").await;
    let html = live(&mut user, "").await;
    assert_eq!(shown(&html), ids);
    assert!(html.contains("class=\"select-kit single-select combobox combo-box tag-drop\""));
    assert!(html.contains("placeholder=\"Filter by keyword...\""));
    assert!(html.contains("data-selected-tag=\"\""));
    let html = live(&mut user, "tag=tag1").await;
    assert_eq!(shown(&html), vec![ids[0], ids[2]]);
    assert!(html.contains("data-selected-tag=\"tag1\""));
    let html = live(&mut user, "tag=tag2").await;
    assert_eq!(shown(&html), vec![ids[1], ids[2]]);
    let html = live(&mut user, "tag=no-tags").await;
    assert_eq!(shown(&html), vec![ids[3]]);
    // A stored tag no template has any more: dropped.
    let html = live(&mut user, "tag=gone").await;
    assert_eq!(shown(&html).len(), 4);
    assert!(html.contains("data-selected-tag=\"\""));
    // Title matches before content matches.
    let html = live(&mut user, "filter=BOTH").await;
    assert_eq!(shown(&html), vec![ids[2]]);
    let html = live(&mut user, "filter=content%20of%20no").await;
    assert_eq!(shown(&html), vec![ids[3]]);
    let html = live(&mut user, "filter=tag&tag=tag2").await;
    assert_eq!(shown(&html), vec![ids[1], ids[2]]);
}

#[tokio::test]
async fn the_list_replaces_the_composer_variables() {
    let f = fixture().await;
    let pool = &f.db.pool;
    let templates = category(pool, "Templates", None, None).await;
    f.categories(&[templates]).await;
    let content = discourse_rs::plugins::templates::variables::ALLOWED_VARIABLES
        .iter()
        .map(|v| format!("{v} = %{{{v}}}"))
        .collect::<Vec<_>>()
        .join("\n");
    let template = topic(
        pool,
        "Variables %{topic_title,fallback:none}",
        Some(templates),
        1,
        "regular",
        &content,
    )
    .await;
    // A topic by user2, with a reply by user1 last.
    let target = topic(pool, "Villains", Some(4), 4, "regular", "First").await;
    sqlx::query(
        "INSERT INTO posts (topic_id, user_id, post_number, raw, cooked, created_at, updated_at, last_version_at, sort_order) \
         VALUES ($1, 3, 2, 'Reply', '<p>Reply</p>', now(), now(), now(), 2)",
    )
    .bind(target)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE topics SET last_post_user_id = 3, posts_count = 2, highest_post_number = 2 WHERE id = $1")
        .bind(target)
        .execute(pool)
        .await
        .unwrap();
    exec(pool, "UPDATE users SET name = 'Walter White' WHERE id = 2").await;
    let mut user = Client::logged_in(f.st.clone(), "user0").await;
    let html = live(
        &mut user,
        &format!("context=composer&topic_id={target}&post_number=1"),
    )
    .await;
    let url = format!("/t/villains/{target}");
    let expected: Vec<String> = [
        ("my_username", "user0"),
        ("my_name", "Walter White"),
        ("chat_channel_name", ""),
        ("chat_channel_url", ""),
        ("chat_thread_name", ""),
        ("chat_thread_url", ""),
        ("context_title", "Villains"),
        ("context_url", url.as_str()),
        ("topic_title", "Villains"),
        ("topic_url", url.as_str()),
        ("original_poster_username", "user2"),
        ("original_poster_name", "User2"),
        ("reply_to_username", "user2"),
        ("reply_to_name", "User2"),
        ("last_poster_username", "user1"),
        ("reply_to_or_last_poster_username", "user2"),
    ]
    .iter()
    .map(|(k, v)| format!("{k} = {v}"))
    .collect();
    let escaped = expected.join("\n").replace('=', "&#x3D;");
    assert!(
        html.contains(&format!(
            "data-template-id=\"{template}\" data-title=\"Variables Villains\" data-content=\"{escaped}\""
        )),
        "{html}"
    );
    // Without a post replied to: the last poster stands in.
    let html = live(&mut user, &format!("context=composer&topic_id={target}")).await;
    assert!(html.contains("reply_to_or_last_poster_username &#x3D; user1"));
    assert!(html.contains("reply_to_username &#x3D; \n"));
    // Outside any composer: the member's own only, the fallback kept.
    let html = live(&mut user, "").await;
    assert!(html.contains("data-title=\"Variables none\""));
    assert!(html.contains(
        "my_username &#x3D; user0\nmy_name &#x3D; Walter White\nchat_channel_name &#x3D; \n"
    ));
    // The preview is the raw cooked, placeholders and all, with its source.
    assert!(html.contains("<p>my_username = %{my_username}"));
    assert!(html.contains(&format!("href=\"/t/variables-%{{topic_title,fallback:none}}/{template}\" class=\"template-item-source-link\"")));
}

#[tokio::test]
async fn the_list_does_not_leak_a_topic_the_member_cannot_see() {
    let f = fixture().await;
    f.categories(&[f.parent]).await;
    let mut user = Client::logged_in(f.st.clone(), "user0").await;
    let html = live(
        &mut user,
        &format!("context=composer&topic_id={}", f.items[3]),
    )
    .await;
    assert!(!html.contains("This is a test template 000003"));
}

#[tokio::test]
async fn the_list_replaces_the_chat_channel_variables() {
    // chat_composer_spec: the channel's name and url, and the message
    // replied to.
    let f = fixture().await;
    let pool = &f.db.pool;
    f.categories(&[f.parent]).await;
    set_setting(pool, "chat_enabled", BOOL, "t").await;
    let channel: i64 = sqlx::query_scalar(
        "INSERT INTO chat_channels (chatable_id, chatable_type, name, slug, created_at, updated_at) \
         VALUES (4, 'Category', 'Villains', 'villains', now(), now()) RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let message: i64 = sqlx::query_scalar(
        "INSERT INTO chat_messages (chat_channel_id, user_id, message, cooked, last_editor_id, created_at, updated_at) \
         VALUES ($1, 3, 'hi', '<p>hi</p>', 3, now(), now()) RETURNING id",
    )
    .bind(channel)
    .fetch_one(pool)
    .await
    .unwrap();
    exec(
        pool,
        &format!(
            "UPDATE posts SET raw = 'name=%{{chat_channel_name}} url=%{{chat_channel_url}} ctx=%{{context_url}} reply=%{{reply_to_username}}' \
             WHERE topic_id = {} AND post_number = 1",
            f.items[0]
        ),
    )
    .await;
    let mut user = Client::logged_in(f.st.clone(), "user0").await;
    let html = live(
        &mut user,
        &format!("context=chat&chat_channel_id={channel}&chat_reply_to_id={message}"),
    )
    .await;
    let url = format!("/chat/c/villains/{channel}");
    assert!(
        html.contains(&format!(
            "data-content=\"name&#x3D;Villains url&#x3D;{url} ctx&#x3D;{url} reply&#x3D;user1\""
        )),
        "{html}"
    );
}
