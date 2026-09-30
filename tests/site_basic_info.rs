//! GET /site/basic-info.json. The first two tests port
//! spec/requests/site_controller_spec.rb `describe "#basic_info"`; the rest
//! pin the fallback chains in site_icon_manager.rb and color_scheme.rb.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{TestDb, config, fabricate_upload, set_setting, state};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

// SiteSettings::TypeSupervisor.types
const STRING: i32 = 1;
const INTEGER: i32 = 3;
const BOOL: i32 = 5;
const UPLOAD: i32 = 18;

async fn get_basic_info(pool: &PgPool, env: RailsEnv, globals: &[(&str, &str)]) -> Value {
    let app = discourse_rs::app(state(pool.clone(), config(env, globals)));
    let response = app
        .oneshot(
            Request::get("/site/basic-info.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

// it "is visible always even for sites requiring login"
#[tokio::test]
async fn is_visible_always_even_for_sites_requiring_login() {
    let db = TestDb::new().await;
    let (upload_id, upload_url) = fabricate_upload(&db.pool).await;
    let id = upload_id.to_string();

    set_setting(&db.pool, "login_required", BOOL, "t").await;
    set_setting(&db.pool, "title", STRING, "Hammer Time").await;
    set_setting(&db.pool, "site_description", STRING, "A time for Hammer").await;
    set_setting(&db.pool, "logo", UPLOAD, &id).await;
    set_setting(&db.pool, "logo_small", UPLOAD, &id).await;
    set_setting(&db.pool, "apple_touch_icon", UPLOAD, &id).await;
    set_setting(&db.pool, "mobile_logo", UPLOAD, &id).await;
    set_setting(&db.pool, "include_in_discourse_discover", BOOL, "t").await;
    // Theme.clear_default!
    set_setting(&db.pool, "default_theme_id", INTEGER, "-1").await;

    let json = get_basic_info(&db.pool, RailsEnv::Test, &[]).await;

    // UrlHelper.absolute(upload.url) in the test env.
    let expected_url = format!("http://test.localhost{upload_url}");
    assert_eq!(json["title"], "Hammer Time");
    assert_eq!(json["description"], "A time for Hammer");
    assert_eq!(json["logo_url"], expected_url);
    assert_eq!(json["apple_touch_icon_url"], expected_url);
    assert_eq!(json["logo_small_url"], expected_url);
    assert_eq!(json["mobile_logo_url"], expected_url);
    assert_eq!(json["header_primary_color"], "333");
    assert_eq!(json["header_background_color"], "fff");
    assert_eq!(json["login_required"], true);
    assert_eq!(json["locale"], "en");
    assert_eq!(json["include_in_discourse_discover"], true);
}

// it "includes false values for include_in_discourse_discover and login_required"
#[tokio::test]
async fn includes_false_values_for_include_in_discourse_discover_and_login_required() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "include_in_discourse_discover", BOOL, "f").await;
    set_setting(&db.pool, "login_required", BOOL, "f").await;

    let json = get_basic_info(&db.pool, RailsEnv::Test, &[]).await;

    assert_eq!(json["include_in_discourse_discover"], false);
    assert_eq!(json["login_required"], false);
}

#[tokio::test]
async fn fresh_install_uses_seeded_sketch_logos() {
    let db = TestDb::new().await;
    let json = get_basic_info(&db.pool, RailsEnv::Production, &[]).await;

    let logo = "http://www.example.com/images/discourse-logo-sketch.png";
    let small = "http://www.example.com/images/discourse-logo-sketch-small.png";
    assert_eq!(
        json,
        json!({
            "logo_url": logo,
            "logo_small_url": small,
            "apple_touch_icon_url": small,
            "favicon_url": small,
            "title": "Discourse",
            "description": "",
            "header_primary_color": "333",
            "header_background_color": "fff",
            "login_required": false,
            "locale": "en",
            "include_in_discourse_discover": false,
            "mobile_logo_url": logo,
        })
    );
}

#[tokio::test]
async fn icons_fall_back_through_large_icon_and_logo_small() {
    let db = TestDb::new().await;
    let (small_id, small_url) = fabricate_upload(&db.pool).await;
    let (large_id, large_url) = fabricate_upload(&db.pool).await;

    set_setting(&db.pool, "logo_small", UPLOAD, &small_id.to_string()).await;
    let json = get_basic_info(&db.pool, RailsEnv::Test, &[]).await;
    assert_eq!(
        json["favicon_url"],
        format!("http://test.localhost{small_url}")
    );

    set_setting(&db.pool, "large_icon", UPLOAD, &large_id.to_string()).await;
    let json = get_basic_info(&db.pool, RailsEnv::Test, &[]).await;
    assert_eq!(
        json["favicon_url"],
        format!("http://test.localhost{large_url}")
    );
    assert_eq!(
        json["apple_touch_icon_url"],
        format!("http://test.localhost{large_url}")
    );
    // logo_small itself has no fallback chain.
    assert_eq!(
        json["logo_small_url"],
        format!("http://test.localhost{small_url}")
    );
}

#[tokio::test]
async fn favicon_prefers_an_optimized_image_at_its_size() {
    let db = TestDb::new().await;
    sqlx::query(
        "INSERT INTO optimized_images (sha1, extension, width, height, upload_id, url, \
         created_at, updated_at) \
         VALUES ('x', 'png', 32, 32, -6, '/uploads/default/optimized/1X/fav_32x32.png', now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();

    let json = get_basic_info(&db.pool, RailsEnv::Test, &[]).await;
    assert_eq!(
        json["favicon_url"],
        "http://test.localhost/uploads/default/optimized/1X/fav_32x32.png"
    );
    // apple_touch_icon wants 180x180, so it keeps the original.
    assert_eq!(
        json["apple_touch_icon_url"],
        "http://test.localhost/images/discourse-logo-sketch-small.png"
    );
}

#[tokio::test]
async fn unset_logos_give_empty_urls_and_no_mobile_logo() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "logo", UPLOAD, "").await;
    set_setting(&db.pool, "logo_small", UPLOAD, "").await;

    let json = get_basic_info(&db.pool, RailsEnv::Test, &[]).await;
    assert_eq!(json["logo_url"], "");
    assert_eq!(json["logo_small_url"], "");
    // favicon still falls back to the sketch logo.
    assert_eq!(
        json["favicon_url"],
        "http://test.localhost/images/discourse-logo-sketch-small.png"
    );
    assert!(json.get("mobile_logo_url").is_none(), "{json}");
}

#[tokio::test]
async fn header_colors_come_from_the_default_themes_scheme() {
    let db = TestDb::new().await;
    let scheme_id: i32 = sqlx::query_scalar(
        "INSERT INTO color_schemes (name, version, created_at, updated_at) \
         VALUES ('Custom', 1, now(), now()) RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO color_scheme_colors (name, hex, color_scheme_id, created_at, updated_at) \
         VALUES ('header_primary', 'ABCDEF', $1, now(), now())",
    )
    .bind(scheme_id)
    .execute(&db.pool)
    .await
    .unwrap();
    let theme_id: i32 = sqlx::query_scalar(
        "INSERT INTO themes (name, user_id, color_scheme_id, created_at, updated_at) \
         VALUES ('Custom', -1, $1, now(), now()) RETURNING id",
    )
    .bind(scheme_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    set_setting(&db.pool, "default_theme_id", INTEGER, &theme_id.to_string()).await;

    let json = get_basic_info(&db.pool, RailsEnv::Test, &[]).await;
    assert_eq!(json["header_primary_color"], "ABCDEF");
    // The scheme lacks header_background, so the controller's fallback wins
    // rather than the base color.
    assert_eq!(json["header_background_color"], "ffffff");
}

#[tokio::test]
async fn globals_shadow_settings_and_move_the_base_url() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "title", STRING, "From DB").await;

    let json = get_basic_info(
        &db.pool,
        RailsEnv::Production,
        &[
            ("title", "From Env"),
            ("hostname", "forum.example.org"),
            ("force_https", "true"),
        ],
    )
    .await;
    assert_eq!(json["title"], "From Env");
    assert_eq!(
        json["logo_url"],
        "https://forum.example.org/images/discourse-logo-sketch.png"
    );
}

#[tokio::test]
async fn development_urls_carry_the_unicorn_port() {
    let db = TestDb::new().await;
    let json = get_basic_info(&db.pool, RailsEnv::Development, &[]).await;
    assert_eq!(
        json["logo_url"],
        "http://localhost:3000/images/discourse-logo-sketch.png"
    );
}
