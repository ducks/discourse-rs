//! GET /site.json for anonymous users. Ports the anonymous-applicable
//! examples of spec/models/site_spec.rb; the full document is covered by the
//! parity golden file.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{TestDb, config, set_setting, state};
use discourse_rs::config::RailsEnv;
use discourse_rs::guardian::Guardian;
use discourse_rs::site::Site;
use discourse_rs::site_settings::SiteSettings;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const STRING: i32 = 1;
const INTEGER: i32 = 3;
const BOOL: i32 = 5;

async fn get_site(pool: &PgPool) -> Value {
    let app = discourse_rs::app(state(pool.clone(), config(RailsEnv::Test, &[])));
    let response = app
        .oneshot(Request::get("/site.json").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// `Site.json_for(Guardian.new)` itself, for states the route refuses
/// anonymously (login_required 403s before the action).
async fn json_for(pool: &PgPool) -> Value {
    let state = state(pool.clone(), config(RailsEnv::Test, &[]));
    let mut conn = pool.acquire().await.unwrap();
    let settings = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals)
        .await
        .unwrap();
    let mut site = Site {
        conn: &mut conn,
        config: &state.config,
        settings: &settings,
        defs: &state.site_setting_defs,
        i18n: &state.i18n,
        guardian: Guardian::anonymous(),
    };
    site.json_for().await.unwrap()
}

async fn create_theme(pool: &PgPool, name: &str, user_selectable: bool) -> i32 {
    sqlx::query_scalar(
        "INSERT INTO themes (name, user_id, user_selectable, created_at, updated_at) \
         VALUES ($1, -1, $2, now(), now()) RETURNING id",
    )
    .bind(name)
    .bind(user_selectable)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn theme_ids(json: &Value) -> Vec<i64> {
    json["user_themes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["theme_id"].as_i64().unwrap())
        .collect()
}

// it "includes user themes and expires them as needed"
#[tokio::test]
async fn includes_user_themes() {
    let db = TestDb::new().await;
    let default_theme = create_theme(&db.pool, "Default", false).await;
    set_setting(
        &db.pool,
        "default_theme_id",
        INTEGER,
        &default_theme.to_string(),
    )
    .await;
    let user_theme = create_theme(&db.pool, "User", true).await;
    let second_user_theme = create_theme(&db.pool, "Second", true).await;

    let json = get_site(&db.pool).await;
    let mut expected = vec![
        i64::from(default_theme),
        i64::from(second_user_theme),
        i64::from(user_theme),
    ];
    let mut ids = theme_ids(&json);
    ids.sort_unstable();
    expected.sort_unstable();
    assert_eq!(ids, expected);
    let default = json["user_themes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["default"] == true)
        .unwrap();
    assert_eq!(default["theme_id"], default_theme);
    assert_eq!(default["name"], "Default");

    // Theme.clear_default!
    set_setting(&db.pool, "default_theme_id", INTEGER, "-1").await;
    let json = get_site(&db.pool).await;
    assert!(!theme_ids(&json).contains(&i64::from(default_theme)));
    // Foundation (-1) is the default again.
    assert!(theme_ids(&json).contains(&-1));

    sqlx::query("UPDATE themes SET user_selectable = false WHERE id = $1")
        .bind(user_theme)
        .execute(&db.pool)
        .await
        .unwrap();
    let json = get_site(&db.pool).await;
    assert!(!theme_ids(&json).contains(&i64::from(user_theme)));
}

// it "omits groups user can not see"
#[tokio::test]
async fn omits_groups_anonymous_users_can_not_see() {
    let db = TestDb::new().await;
    // visibility_level: staff (3), then public (0)
    sqlx::query(
        "INSERT INTO groups (name, visibility_level, created_at, updated_at) \
         VALUES ('staff_only', 3, now(), now()), ('public_group', 0, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();

    let json = get_site(&db.pool).await;
    let names: Vec<&str> = json["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["public_group"]);
    let group = &json["groups"][0];
    assert_eq!(group["full_name"], "public_group");
    assert_eq!(group["display_name"], "public_group");
    assert_eq!(group["automatic"], false);
}

// it "includes anonymous_list_filters for anon when login_required"
#[tokio::test]
async fn includes_anonymous_list_filters_for_anon_when_login_required() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "login_required", BOOL, "t").await;

    let json = json_for(&db.pool).await;
    let filters = json["anonymous_list_filters"].as_array().unwrap();
    assert!(filters.contains(&json!("latest")));
    assert!(!filters.contains(&json!("unread")));
    // The reduced document has no groups, categories or archetypes.
    assert!(json.get("groups").is_none());
    assert!(json.get("archetypes").is_none());
    assert_eq!(json["tos_url"], Value::Null);
}

// it "includes tos_url and privacy_policy_url when login_required"
#[tokio::test]
async fn includes_tos_url_and_privacy_policy_url_when_login_required() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "login_required", BOOL, "t").await;
    set_setting(&db.pool, "tos_url", STRING, "https://discourse.org").await;
    set_setting(
        &db.pool,
        "privacy_policy_url",
        STRING,
        "https://discourse.org/privacy",
    )
    .await;

    let json = json_for(&db.pool).await;
    assert_eq!(json["tos_url"], "https://discourse.org");
    assert_eq!(json["privacy_policy_url"], "https://discourse.org/privacy");
}

#[tokio::test]
async fn legal_urls_come_from_topics_when_configured() {
    let db = TestDb::new().await;
    // Any existing topic in the snapshot serves as the ToS topic.
    let topic_id: i32 = sqlx::query_scalar("SELECT min(id) FROM topics")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    set_setting(&db.pool, "tos_topic_id", INTEGER, &topic_id.to_string()).await;
    set_setting(&db.pool, "privacy_topic_id", INTEGER, "999999").await;

    let json = get_site(&db.pool).await;
    assert_eq!(json["tos_url"], "/tos");
    assert!(json.get("privacy_policy_url").is_none(), "{json}");
}

#[tokio::test]
async fn user_tips_are_included_only_when_enabled() {
    let db = TestDb::new().await;
    let json = get_site(&db.pool).await;
    assert!(json.get("user_tips").is_none());

    set_setting(&db.pool, "enable_user_tips", BOOL, "t").await;
    let json = get_site(&db.pool).await;
    assert_eq!(json["user_tips"]["first_notification"], 1);
}

#[tokio::test]
async fn tagging_disabled_drops_tag_fields_and_permissions() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "tagging_enabled", BOOL, "f").await;

    let json = get_site(&db.pool).await;
    assert!(json.get("tags_filter_regexp").is_none());
    assert!(json.get("top_tags").is_none());
    assert_eq!(json["can_tag_topics"], false);
    assert_eq!(
        json["hashtag_configurations"]["topic-composer"],
        json!(["category"])
    );
}

#[tokio::test]
async fn flags_are_translated_with_the_serializer_quirks() {
    let db = TestDb::new().await;
    let json = get_site(&db.pool).await;
    let flags = json["post_action_types"].as_array().unwrap();
    let notify_user = flags
        .iter()
        .find(|f| f["name_key"] == "notify_user")
        .unwrap();
    // No interpolation arguments for the title, so the placeholder survives.
    assert_eq!(notify_user["name"], "Send @%{username} a message");
    assert_eq!(notify_user["is_flag"], true);
    let like = flags.iter().find(|f| f["name_key"] == "like").unwrap();
    assert_eq!(like["is_flag"], false);

    let topic_flags = json["topic_flag_types"].as_array().unwrap();
    let notify_mods = topic_flags
        .iter()
        .find(|f| f["name_key"] == "notify_moderators")
        .unwrap();
    // Its description needs %{tos_url}, which the serializer never passes.
    assert_eq!(notify_mods["description"], "");
    assert_eq!(
        notify_mods["short_description"],
        "Requires staff attention for another reason"
    );
}

async fn create_category(
    pool: &PgPool,
    name: &str,
    parent: Option<i32>,
    read_restricted: bool,
) -> i32 {
    let slug = name.to_ascii_lowercase().replace(' ', "-");
    sqlx::query_scalar(
        "INSERT INTO categories (name, name_lower, slug, color, text_color, user_id, parent_category_id, \
         read_restricted, position, created_at, updated_at) \
         VALUES ($1, lower($1), $2, '0088CC', 'FFFFFF', -1, $3, $4, \
                 (SELECT coalesce(max(position), 0) + 1 FROM categories), now(), now()) RETURNING id",
    )
    .bind(name)
    .bind(slug)
    .bind(parent)
    .bind(read_restricted)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn category_ids(json: &Value) -> Vec<i64> {
    json["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_i64().unwrap())
        .collect()
}

// it "returns correct notification level for categories"
#[tokio::test]
async fn returns_correct_notification_level_for_categories() {
    let db = TestDb::new().await;
    let category = create_category(&db.pool, "Tracked", None, false).await;

    let last = |json: &Value| {
        json["categories"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone()
    };
    let json = get_site(&db.pool).await;
    assert_eq!(last(&json)["id"], category);
    assert_eq!(last(&json)["notification_level"], 1);

    set_setting(&db.pool, "mute_all_categories_by_default", BOOL, "t").await;
    let json = get_site(&db.pool).await;
    assert_eq!(last(&json)["notification_level"], 0);

    set_setting(
        &db.pool,
        "default_categories_tracking",
        STRING,
        &category.to_string(),
    )
    .await;
    let json = get_site(&db.pool).await;
    assert_eq!(last(&json)["notification_level"], 1);
}

// describe "#categories" it "omits read restricted categories" (anonymous)
#[tokio::test]
async fn omits_read_restricted_categories() {
    let db = TestDb::new().await;
    let category = create_category(&db.pool, "Members", None, false).await;
    let json = get_site(&db.pool).await;
    assert!(category_ids(&json).contains(&i64::from(category)));
    // The snapshot's Staff category (id 3) is read restricted.
    assert!(!category_ids(&json).contains(&3));

    sqlx::query("UPDATE categories SET read_restricted = true WHERE id = $1")
        .bind(category)
        .execute(&db.pool)
        .await
        .unwrap();
    let json = get_site(&db.pool).await;
    assert!(!category_ids(&json).contains(&i64::from(category)));
}

#[tokio::test]
async fn subcategories_set_has_children_and_follow_their_parent() {
    let db = TestDb::new().await;
    let parent = create_category(&db.pool, "Parent", None, false).await;
    let child = create_category(&db.pool, "Child", Some(parent), false).await;
    let hidden_parent = create_category(&db.pool, "Hidden", None, true).await;
    let orphan = create_category(&db.pool, "Orphan", Some(hidden_parent), false).await;

    let json = get_site(&db.pool).await;
    let ids = category_ids(&json);
    assert!(ids.contains(&i64::from(parent)) && ids.contains(&i64::from(child)));
    assert!(
        !ids.contains(&i64::from(orphan)),
        "child of a hidden parent is dropped"
    );

    let by_id = |id: i32| {
        json["categories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(by_id(parent)["has_children"], true);
    assert_eq!(by_id(child)["has_children"], false);
    assert_eq!(by_id(child)["parent_category_id"], parent);
    assert!(by_id(parent).get("parent_category_id").is_none());
    assert_eq!(by_id(child)["can_edit"], false);
    assert_eq!(by_id(child)["permission"], Value::Null);
}

#[tokio::test]
async fn uncategorized_category_uses_translated_texts() {
    let db = TestDb::new().await;
    // The snapshot has uncategorized_category_id = -1; point it at category 1.
    set_setting(&db.pool, "uncategorized_category_id", INTEGER, "1").await;

    let json = get_site(&db.pool).await;
    let uncategorized = &json["categories"][0];
    assert_eq!(uncategorized["id"], 1);
    assert_eq!(uncategorized["name"], "Uncategorized");
    assert_eq!(
        uncategorized["description_text"],
        "Topics that don't need a category, or don't fit into any other existing category."
    );
    assert_eq!(
        uncategorized["description_excerpt"],
        uncategorized["description_text"]
    );
}
