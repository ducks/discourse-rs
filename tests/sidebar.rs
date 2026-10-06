//! The anonymous sidebar on the server-rendered pages.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;

const BOOL: i32 = 5;
const ENUM: i32 = 7;
const CATEGORY_LIST: i32 = 11;

async fn get(pool: &PgPool, path: &str) -> (StatusCode, String) {
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
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The link (the `<a ...>` tag) inside the li that has `li_attr`.
fn link_tag<'a>(html: &'a str, li_attr: &str) -> &'a str {
    let li = html
        .find(li_attr)
        .unwrap_or_else(|| panic!("no {li_attr} in {html}"));
    let a = li + html[li..].find("<a ").unwrap();
    &html[a..a + html[a..].find('>').unwrap()]
}

#[tokio::test]
async fn anonymous_pages_have_the_sidebar() {
    let db = TestDb::new().await;
    let (status, html) = get(&db.pool, "/latest").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains(r#"<body class="uc-"#) && html.contains(" has-sidebar-page\""));
    assert!(html.contains(r#"<span class="header-sidebar-toggle">"#));
    assert!(
        html.contains(r#"<nav aria-label="Sidebar" class="sidebar-container" id="d-sidebar">"#)
    );
    assert!(html.contains(r#"<script src="/assets/sidebar.js"></script>"#));

    // The community section: Topics, active on the latest list, and the
    // secondary links behind More. A member's links are left out.
    assert_eq!(
        link_tag(&html, r#"data-list-item-name="everything""#),
        r#"<a class="active sidebar-section-link sidebar-row" title="All topics" data-link-name="everything" href="/latest""#
    );
    assert!(html.contains(r#"<span class="sidebar-section-link-content-text">Topics</span>"#));
    // The fixture renames FAQ to Guidelines, as the reference does.
    for name in ["users", "about", "guidelines", "groups", "badges", "filter"] {
        assert!(
            html.contains(&format!(
                r#"<li class="sidebar-section-link-wrapper dropdown-menu__item" data-list-item-name="{name}">"#
            )),
            "{name}"
        );
    }
    for name in ["my-posts", "my-messages", "review", "admin", "invite"] {
        assert!(
            !html.contains(&format!(r#"data-list-item-name="{name}""#)),
            "{name}"
        );
    }

    // Categories: the top level ones by topic count, with their style.
    assert!(html.contains(r#"<li class="sidebar-section-link-wrapper" data-category-id="4"><a class="sidebar-section-link sidebar-row" href="/c/general/4">"#));
    assert!(
        !html.contains(r#"<li class="sidebar-section-link-wrapper" data-category-id="34""#),
        "a subcategory"
    );
    assert!(html.contains(r#"data-list-item-name="all-categories""#));

    // Tags: the site's top tags.
    assert!(html.contains(r#"<li class="sidebar-section-link-wrapper" data-tag-name="howto"><a class="sidebar-section-link sidebar-row" href="/tag/howto/1">"#));
    assert!(html.contains(r#"data-list-item-name="all-tags""#));

    let (status, js) = get(&db.pool, "/assets/sidebar.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(js.contains("discourse_"));
}

#[tokio::test]
async fn the_current_page_marks_its_link() {
    let db = TestDb::new().await;
    let (_, html) = get(&db.pool, "/c/general/4").await;
    assert!(
        link_tag(
            &html,
            r#"<li class="sidebar-section-link-wrapper" data-category-id="4""#
        )
        .starts_with(r#"<a class="active "#)
    );
    assert!(!link_tag(&html, r#"data-list-item-name="everything""#).contains("active"));

    let (_, html) = get(&db.pool, "/tag/howto/1").await;
    assert!(link_tag(&html, r#"data-tag-name="howto""#).starts_with(r#"<a class="active "#));
    assert!(!link_tag(&html, r#"data-tag-name="guide""#).contains("active"));

    let (_, html) = get(&db.pool, "/categories").await;
    assert!(
        link_tag(&html, r#"data-list-item-name="all-categories""#)
            .starts_with(r#"<a class="active "#)
    );

    let (_, html) = get(&db.pool, "/tags").await;
    assert!(
        link_tag(&html, r#"data-list-item-name="all-tags""#).starts_with(r#"<a class="active "#)
    );

    let (_, html) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35").await;
    assert!(html.contains(r#"id="d-sidebar""#));
    assert!(!html.contains(r#"class="active sidebar-section-link"#));
}

#[tokio::test]
async fn settings_shape_the_sidebar() {
    let db = TestDb::new().await;

    // default_navigation_menu_categories picks the categories, sorted by
    // name with each child under its parent: Sub General follows General
    // (not shown), so it comes before Site Feedback.
    set_setting(
        &db.pool,
        "default_navigation_menu_categories",
        CATEGORY_LIST,
        "34|2",
    )
    .await;
    let (_, html) = get(&db.pool, "/latest").await;
    let site_feedback = html
        .find(r#"<li class="sidebar-section-link-wrapper" data-category-id="2""#)
        .unwrap();
    let sub_general = html
        .find(r#"<li class="sidebar-section-link-wrapper" data-category-id="34""#)
        .unwrap();
    assert!(sub_general < site_feedback);
    assert!(!html.contains(r#"<li class="sidebar-section-link-wrapper" data-category-id="4""#));

    // Directory links follow their settings; FAQ stays FAQ unless renamed.
    set_setting(&db.pool, "enable_user_directory", BOOL, "f").await;
    set_setting(&db.pool, "enable_badges", BOOL, "f").await;
    set_setting(&db.pool, "rename_faq_to_guidelines", BOOL, "f").await;
    let (_, html) = get(&db.pool, "/latest").await;
    assert!(!html.contains(r#"data-list-item-name="users""#));
    assert!(!html.contains(r#"data-list-item-name="badges""#));
    assert!(html.contains(r#"data-link-name="faq" href="/faq""#));

    // No tags section without tagging.
    set_setting(&db.pool, "tagging_enabled", BOOL, "f").await;
    let (_, html) = get(&db.pool, "/latest").await;
    assert!(!html.contains(r#"data-section-name="tags""#));

    // The header dropdown menu instead of a sidebar.
    set_setting(&db.pool, "navigation_menu", ENUM, "header dropdown").await;
    let (_, html) = get(&db.pool, "/latest").await;
    assert!(!html.contains(r#"id="d-sidebar""#));
    assert!(!html.contains("header-sidebar-toggle"));
    assert!(!html.contains("has-sidebar-page"));
}
