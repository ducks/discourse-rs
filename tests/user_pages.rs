//! Public profiles for anonymous users. The documents are covered by the
//! parity golden files; these pin the HTML, view tracking, and the
//! visibility rules the fixtures only partly show.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

const BOOL: i32 = 5;

async fn get(pool: &PgPool, path: &str) -> (StatusCode, Vec<(String, String)>, String) {
    let app = discourse_rs::app(state(pool.clone(), config(RailsEnv::Test, &[])).await);
    let response = app
        .oneshot(
            Request::get(path)
                .header(header::HOST, "test.localhost")
                .header("x-forwarded-for", "10.0.0.7")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn noindex(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .any(|(k, v)| k == "x-robots-tag" && v == "noindex")
}

#[tokio::test]
async fn profile_views_are_tracked_once_per_ip_and_day() {
    let db = TestDb::new().await;
    let before: i32 = sqlx::query_scalar("SELECT views FROM user_profiles WHERE user_id = 3")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let (status, headers, body) = get(&db.pool, "/u/user1.json").await;
    assert_eq!(status, StatusCode::OK);
    assert!(noindex(&headers));
    let json: Value = serde_json::from_str(&body).unwrap();
    // Read before this request's own increment, like Rails.
    assert_eq!(json["user"]["profile_view_count"], before);
    get(&db.pool, "/u/user1.json").await;
    get(&db.pool, "/u/user1").await;
    let after: i32 = sqlx::query_scalar("SELECT views FROM user_profiles WHERE user_id = 3")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(after, before + 1);
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_profile_views WHERE user_profile_id = 3 AND host(ip_address) = '10.0.0.7'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(rows, 1);
    // skip_track_visit leaves the counter alone.
    get(&db.pool, "/u/admin.json?skip_track_visit=true").await;
    let admin: i32 = sqlx::query_scalar("SELECT views FROM user_profiles WHERE user_id = 1")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let (_, _, body) = get(&db.pool, "/u/admin.json?skip_track_visit=true").await;
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["user"]["profile_view_count"], admin);
}

#[tokio::test]
async fn hidden_profiles_follow_the_anonymous_rules() {
    let db = TestDb::new().await;
    // user0: trust level 1 with no replies (the mention fixture gave
    // them one; take it back here) -> hidden while
    // hide_new_user_profiles is on; visible once it is off.
    sqlx::query("UPDATE user_stats SET post_count = 0 WHERE user_id = 2")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, _, body) = get(&db.pool, "/u/user0.json").await;
    assert_eq!(status, StatusCode::OK);
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["user"]["profile_hidden"], true);
    assert!(json["user"].get("trust_level").is_none());
    let (status, _, html) = get(&db.pool, "/u/user0").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains(
        r#"<p class="user-profile-hidden">This user&#x27;s public profile is hidden.</p>"#
    ));
    set_setting(&db.pool, "hide_new_user_profiles", BOOL, "f").await;
    let (_, _, body) = get(&db.pool, "/u/user0.json").await;
    let json: Value = serde_json::from_str(&body).unwrap();
    assert!(json["user"].get("profile_hidden").is_none());
    assert_eq!(json["user"]["trust_level"], 1);
    let (status, _, _) = get(&db.pool, "/u/user0/summary.json").await;
    assert_eq!(status, StatusCode::OK);

    // A user who hides their profile is hidden even at trust level 2.
    sqlx::query("UPDATE user_options SET hide_profile = TRUE WHERE user_id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, _, body) = get(&db.pool, "/u/user1.json").await;
    let json: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["user"]["profile_hidden"], true);
    for path in ["/u/user1/summary.json", "/user_actions.json?username=user1"] {
        let (status, _, _) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }

    // Inactive accounts don't exist for anonymous readers.
    sqlx::query("UPDATE users SET active = FALSE WHERE id = 1")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, _, _) = get(&db.pool, "/u/admin.json").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn hiding_profiles_from_the_public_gives_403() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "hide_user_profiles_from_public", BOOL, "t").await;
    for path in [
        "/u/user1.json",
        "/u/user1",
        "/u/user1/summary.json",
        "/u/user1/messages.json",
    ] {
        let (status, headers, body) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
        assert!(noindex(&headers), "{path}");
        assert!(body.contains("invalid_access"), "{path}");
    }
    let (status, _, _) = get(&db.pool, "/user_actions.json?username=user1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn profile_page_renders_summary_and_activity() {
    let db = TestDb::new().await;
    let (status, headers, html) = get(&db.pool, "/u/user1").await;
    assert_eq!(status, StatusCode::OK);
    assert!(noindex(&headers));
    assert!(html.contains("<title>Profile - user1 - Discourse</title>"));
    // The Ember profile: the names, the summary's stats, top replies,
    // top categories and badge cards.
    assert!(html.contains(r#"<div class="username user-profile-names__primary">user1 "#));
    assert!(html.contains(
        r#"<li class="stats-topic-count linked-stat"><a href="/u/user1/activity/topics">"#
    ));
    assert!(html.contains(r#"<a href="/t/parity-fixture-replies-and-posters/35/2">"#));
    assert!(html.contains(r#"href="/c/general/sub-general/34""#));
    assert!(html.contains(r#"data-badge-slug="member""#));

    // The other tails render the same page; /users is an alias.
    for path in [
        "/u/user1/summary",
        "/u/user1/activity",
        "/u/user1/badges",
        "/users/user1",
    ] {
        let (status, _, html) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(
            html.contains(r#"<div class="username user-profile-names__primary">user1 "#),
            "{path}"
        );
    }
    let (status, _, _) = get(&db.pool, "/u/nobody").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Lists and topics link here.
    let (_, _, html) = get(&db.pool, "/latest").await;
    assert!(html.contains(r#"href="/u/user1""#));
}
