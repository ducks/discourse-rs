//! Light and dark on the pages (crate::color_mode): the color scheme
//! stylesheets a page links and the interface color selector. The head's
//! tags for each combination of options, cookies and theme are measured
//! against Rails in parity/writes/color_modes.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;

async fn page(pool: &PgPool, cookie: Option<&str>) -> String {
    let app = discourse_rs::app(state(pool.clone(), config(RailsEnv::Test, &[])).await);
    let mut request = Request::get("/latest").header(header::HOST, "test.localhost");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

const LIGHT_AUTO: &str = r#"<link href="/assets/color_definitions_light.css" media="(prefers-color-scheme: light)" rel="stylesheet" class="light-scheme"/>"#;
const DARK_AUTO: &str = r#"<link href="/assets/color_definitions_dark.css" media="(prefers-color-scheme: dark)" rel="stylesheet" class="dark-scheme" data-scheme-id="13"/>"#;

#[tokio::test]
async fn pages_link_both_palettes_for_the_browser_to_choose() {
    let db = TestDb::new().await;
    let html = page(&db.pool, None).await;
    assert!(html.contains(&format!("{LIGHT_AUTO}{DARK_AUTO}")), "{html}");
    assert!(html.contains(r#"<meta name="color-scheme" content="light dark">"#));
    assert!(html.contains(
        r##"<meta name="theme-color" media="(prefers-color-scheme: dark)" content="#111111">"##
    ));
    // The selector is not drawn by default (interface_color_selector
    // disabled).
    assert!(!html.contains("interface-color-selector"), "{html}");
}

#[tokio::test]
async fn the_cookie_forces_a_mode() {
    let db = TestDb::new().await;
    let html = page(&db.pool, Some("forced_color_mode=dark")).await;
    assert!(html.contains(r#"media="none" rel="stylesheet" class="light-scheme"/>"#));
    assert!(
        html.contains(r#"media="all" rel="stylesheet" class="dark-scheme" data-scheme-id="13"/>"#)
    );
    let html = page(&db.pool, Some("forced_color_mode=light")).await;
    assert!(html.contains(r#"media="all" rel="stylesheet" class="light-scheme"/>"#));
    assert!(
        html.contains(r#"media="none" rel="stylesheet" class="dark-scheme" data-scheme-id="13"/>"#)
    );
}

#[tokio::test]
async fn a_theme_without_a_dark_palette_links_the_light_one_alone() {
    let db = TestDb::new().await;
    sqlx::query("UPDATE themes SET dark_color_scheme_id = NULL WHERE id = -1")
        .execute(&db.pool)
        .await
        .unwrap();
    set_setting(&db.pool, "interface_color_selector", 7, "header").await;
    let html = page(&db.pool, Some("forced_color_mode=dark")).await;
    assert!(html.contains(
        r#"<link href="/assets/color_definitions_light.css" media="all" rel="stylesheet" class="light-scheme"/>"#
    ));
    assert!(!html.contains("dark-scheme"), "{html}");
    assert!(html.contains(r#"<meta name="color-scheme" content="light">"#));
    // Without a dark palette there is nothing to choose.
    assert!(!html.contains("interface-color-selector"), "{html}");
}

#[tokio::test]
async fn the_selector_shows_in_the_header() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "interface_color_selector", 7, "header").await;
    let html = page(&db.pool, None).await;
    // First of the header icons (ahead of search), with no mode chosen
    // (I18n.js's placeholder in its label) and the half circle.
    html.find(r#"<ul class="icons d-header-icons"><li class="header-dropdown-toggle header-color-scheme-toggle"><button aria-expanded="false" aria-label="Color mode, currently [missing %{mode} value]" class="btn no-text btn-icon fk-d-menu__trigger interface-color-selector-trigger btn-flat sidebar-footer-actions-button interface-color-selector icon" title="Color mode" data-identifier="interface-color-selector" data-trigger="" type="button"><svg class="fa d-icon d-icon-circle-half-stroke"#)
        .unwrap_or_else(|| panic!("{html}"));
    assert!(html.contains("interface-color.js"));
    // The footer has none.
    let footer = &html[html.find("sidebar-footer-actions\"").unwrap()..];
    assert!(!footer[..footer.find("</div>").unwrap()].contains("interface-color-selector"));

    let html = page(&db.pool, Some("forced_color_mode=dark")).await;
    assert!(html.contains(r#"aria-label="Color mode, currently dark""#));
    assert!(html.contains(r#"data-current-mode="dark"><svg class="fa d-icon d-icon-moon"#));
}

#[tokio::test]
async fn the_selector_shows_in_the_sidebar_footer() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "interface_color_selector", 7, "sidebar_footer").await;
    let html = page(&db.pool, Some("forced_color_mode=light")).await;
    assert!(
        html.contains(r#"<div class="sidebar-footer-actions"><button aria-expanded="false" aria-label="Color mode, currently light""#),
        "{html}"
    );
    assert!(html.contains(r#"data-current-mode="light"><svg class="fa d-icon d-icon-sun"#));
    assert!(!html.contains("header-color-scheme-toggle"));
}
