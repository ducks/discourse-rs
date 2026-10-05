//! What crawlers get: robots.txt variants, sitemaps, and the canonical,
//! description and OpenGraph tags on the HTML pages. The default outputs
//! are covered by the parity golden files.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;

const STRING: i32 = 1;
const BOOL: i32 = 5;

async fn get(pool: &PgPool, path: &str) -> (StatusCode, Vec<(String, String)>, String) {
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
async fn robots_txt_follows_the_crawler_settings() {
    let db = TestDb::new().await;
    let (status, _, body) = get(&db.pool, "/robots.txt").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("User-agent: *\nDisallow: /admin/\n"));
    assert!(body.contains("Sitemap: http://test.localhost/sitemap.xml\n"));

    // login_required drops the sitemap line but robots.txt stays open.
    set_setting(&db.pool, "login_required", BOOL, "t").await;
    let (status, _, body) = get(&db.pool, "/robots.txt").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("Sitemap:"));
    set_setting(&db.pool, "login_required", BOOL, "f").await;

    // Allowed agents: each gets the disallow list, everyone else nothing.
    set_setting(
        &db.pool,
        "allowed_crawler_user_agents",
        STRING,
        "Googlebot|Bingbot",
    )
    .await;
    let (_, _, body) = get(&db.pool, "/robots.txt").await;
    assert!(body.contains("User-agent: Googlebot\nDisallow: /admin/\n"));
    assert!(body.contains("User-agent: Bingbot\nDisallow: /admin/\n"));
    assert!(body.contains("User-agent: Bingbot\nDisallow: /admin/\nDisallow: /auth/\nDisallow: /assets/js/browser-update*.js\nDisallow: /email/\nDisallow: /session\nDisallow: /user-api-key\nDisallow: /*?api_key*\nDisallow: /*?*api_key*\nDisallow: /badges\n"));
    assert!(!body.contains("User-agent: Googlebot\nDisallow: /admin/\nDisallow: /auth/\nDisallow: /assets/js/browser-update*.js\nDisallow: /email/\nDisallow: /session\nDisallow: /user-api-key\nDisallow: /*?api_key*\nDisallow: /*?*api_key*\nDisallow: /badges"));
    assert!(body.ends_with(
        "User-agent: *\nDisallow: /\n\n\n\nSitemap: http://test.localhost/sitemap.xml\n\n\n"
    ));
    let (_, _, json) = get(&db.pool, "/robots-builder.json").await;
    assert!(json.contains(r#"{"name":"*","disallow":["/"]}"#));
    set_setting(&db.pool, "allowed_crawler_user_agents", STRING, "").await;

    set_setting(&db.pool, "allow_index_in_robots_txt", BOOL, "f").await;
    let (_, _, body) = get(&db.pool, "/robots.txt").await;
    assert_eq!(
        body,
        "User-agent: googlebot\nAllow: /\nDisallow: /uploads/*\n\nUser-agent: *\nDisallow: /\n"
    );

    set_setting(
        &db.pool,
        "overridden_robots_txt",
        STRING,
        "User-agent: *\nDisallow: /secret",
    )
    .await;
    let (_, _, body) = get(&db.pool, "/robots.txt").await;
    assert_eq!(body, "User-agent: *\nDisallow: /secret");
    let (_, _, json) = get(&db.pool, "/robots-builder.json").await;
    assert!(json.contains(r#""overridden":"User-agent: *\nDisallow: /secret""#));
}

#[tokio::test]
async fn sitemaps_regenerate_their_rows_and_honor_the_setting() {
    let db = TestDb::new().await;
    sqlx::query("DELETE FROM sitemaps")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, _, _) = get(&db.pool, "/sitemap_1.xml").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "no rows until regenerated");
    let (status, headers, body) = get(&db.pool, "/sitemap.xml").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers
            .iter()
            .any(|(k, v)| k == "content-type" && v.starts_with("application/xml"))
    );
    assert!(body.contains("<loc>http://test.localhost/sitemap_recent.xml</loc>"));
    assert!(body.contains("<loc>http://test.localhost/sitemap_1.xml</loc>"));
    assert!(!body.contains("sitemap_news"));
    let names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sitemaps WHERE enabled ORDER BY name")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(names, vec!["1", "news", "recent"]);

    let (status, _, body) = get(&db.pool, "/sitemap_1.xml").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<loc>http://test.localhost/t/welcome-to-discourse/5</loc>"));
    assert!(!body.contains("deleted-topic"), "trashed topics stay out");
    assert!(!body.contains("unlisted"), "invisible topics stay out");

    // A topic with more posts than a page links to its last page. Bumped
    // now: the recent sitemap lists only topics bumped within 3 days, and
    // the seed's dates age.
    sqlx::query("UPDATE topics SET posts_count = 45, bumped_at = now() WHERE id = 35")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, _, body) = get(&db.pool, "/sitemap_recent.xml").await;
    assert!(body.contains(
        "<loc>http://test.localhost/t/parity-fixture-replies-and-posters/35?page=3</loc>"
    ));
    let (_, _, body) = get(&db.pool, "/sitemap_1.xml").await;
    assert!(
        body.contains("<loc>http://test.localhost/t/parity-fixture-replies-and-posters/35</loc>")
    );

    for path in [
        "/sitemap_2.xml",
        "/sitemap_0.xml",
        "/sitemap_01.xml",
        "/sitemap_x.xml",
    ] {
        let (status, _, _) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }

    set_setting(&db.pool, "enable_sitemap", BOOL, "f").await;
    for path in [
        "/sitemap.xml",
        "/sitemap_1.xml",
        "/sitemap_recent.xml",
        "/news.xml",
    ] {
        let (status, _, _) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn pages_carry_canonical_and_opengraph_tags() {
    let db = TestDb::new().await;
    // allow_indexing_non_canonical_urls defaults to true (hidden); off, the
    // non-canonical URLs are marked noindex.
    set_setting(&db.pool, "allow_indexing_non_canonical_urls", BOOL, "f").await;
    let (_, headers, html) = get(&db.pool, "/latest").await;
    assert!(!noindex(&headers));
    assert!(html.contains(r#"<link rel="canonical" href="http://test.localhost/latest">"#));
    assert!(html.contains(r#"<meta property="og:site_name" content="Discourse">"#));
    assert!(html.contains(r#"<meta property="og:title" content="Discourse">"#));
    assert!(html.contains(r#"<meta property="og:url" content="http://test.localhost/latest">"#));
    assert!(html.contains(r#"<meta name="twitter:card" content="summary">"#));
    assert!(html.contains(r#"<meta property="og:image" content="http://test.localhost/images/discourse-logo-sketch-small.png">"#), "{html}");

    // Sorted or paged lists keep the page param only, and non-canonical
    // URLs are marked noindex.
    let (_, headers, html) = get(&db.pool, "/latest?page=1").await;
    assert!(html.contains(r#"<link rel="canonical" href="http://test.localhost/latest?page=1">"#));
    assert!(!noindex(&headers));
    let (_, headers, html) = get(&db.pool, "/latest?order=views&page=1").await;
    assert!(html.contains(r#"<link rel="canonical" href="http://test.localhost/latest?page=1">"#));
    assert!(noindex(&headers));

    // Category lists point at the category, whatever the filter.
    let (_, headers, html) = get(&db.pool, "/c/general/4/l/top").await;
    assert!(html.contains(r#"<link rel="canonical" href="http://test.localhost/c/general/4">"#));
    assert!(html.contains(r#"<meta property="og:title" content="General">"#));
    assert!(noindex(&headers));
    let (_, headers, _) = get(&db.pool, "/c/general/4").await;
    assert!(!noindex(&headers));

    // Topics: the paged topic URL; the description from the first post.
    let (_, headers, html) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35/2").await;
    assert!(html.contains(r#"<link rel="canonical" href="http://test.localhost/t/parity-fixture-replies-and-posters/35">"#));
    assert!(
        html.contains(
            r#"<meta property="og:title" content="Parity fixture: replies and posters">"#
        )
    );
    assert!(
        html.contains(
            r#"<meta name="description" content="First post of a topic with several repliers"#
        ),
        "{html}"
    );
    assert!(html.contains(
        r#"<meta property="og:description" content="First post of a topic with several repliers"#
    ));
    assert!(noindex(&headers));

    // Tags, users, search.
    let (_, _, html) = get(&db.pool, "/tag/howto/1/l/top").await;
    assert!(
        html.contains(r#"<link rel="canonical" href="http://test.localhost/tag/howto/1/l/top">"#)
    );
    assert!(html.contains(r#"<meta property="og:title" content="Topics tagged howto">"#));
    let (_, _, html) = get(&db.pool, "/u/user1").await;
    assert!(html.contains(r#"<link rel="canonical" href="http://test.localhost/u/user1">"#));
    assert!(html.contains(r#"<meta property="og:title" content="user1">"#));
    assert!(html.contains(r#"<meta property="og:image" content="http://test.localhost/letter_avatar_proxy/v4/letter/u/5daacb/45.png">"#));
    let (_, _, html) = get(&db.pool, "/search?q=fixture&page=2").await;
    assert!(html.contains(r#"<link rel="canonical" href="http://test.localhost/search?page=2">"#));
    assert!(!html.contains("og:title"));

    set_setting(&db.pool, "allow_indexing_non_canonical_urls", BOOL, "t").await;
    let (_, headers, _) = get(&db.pool, "/c/general/4/l/top").await;
    assert!(!noindex(&headers));
}
