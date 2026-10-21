//! The theme's html fields on the pages (crate::theme_html): where each
//! is drawn, which target the user agent picks, and what is refused. The
//! fields' bytes are measured against Rails in parity/writes/theme_html.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;

const IPHONE: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";
const DESKTOP: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
const PLACEHOLDER: &str = "__CSP__NONCE__PLACEHOLDER__f72bff1b1768168a34ee092ce759f192__";

async fn page(pool: &PgPool, path: &str, ua: &str, cookie: Option<&str>) -> (StatusCode, String) {
    let app = discourse_rs::app(state(pool.clone(), config(RailsEnv::Test, &[])).await);
    let mut request = Request::get(path)
        .header(header::HOST, "test.localhost")
        .header(header::USER_AGENT, ua);
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// A field of the default theme (Foundation, -1) as Rails bakes it.
async fn field(
    pool: &PgPool,
    theme_id: i32,
    target_id: i32,
    name: &str,
    value: &str,
    baked: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO theme_fields (theme_id, target_id, name, value, value_baked, created_at, \
         updated_at, compiler_version, type_id) \
         VALUES ($1, $2, $3, $4, $5, now(), now(), \
           (SELECT compiler_version FROM theme_fields WHERE compiler_version <> '0' LIMIT 1), 0)",
    )
    .bind(theme_id)
    .bind(target_id)
    .bind(name)
    .bind(value)
    .bind(baked)
    .execute(pool)
    .await
    .unwrap();
}

async fn fielded(pool: &PgPool) {
    for (target, label) in [(0, "common"), (1, "desktop"), (2, "mobile")] {
        for name in ["head_tag", "header", "after_header", "footer", "body_tag"] {
            let html = format!("<i class=\"{name}-{label}\"></i>");
            field(pool, -1, target, name, &html, Some(&html)).await;
        }
    }
}

fn before(body: &str, a: &str, b: &str) -> bool {
    match (body.find(a), body.find(b)) {
        (Some(a), Some(b)) => a < b,
        _ => false,
    }
}

#[tokio::test]
async fn each_field_is_drawn_where_rails_and_ember_draw_it() {
    let db = TestDb::new().await;
    fielded(&db.pool).await;
    let (status, body) = page(&db.pool, "/latest", DESKTOP, None).await;
    assert_eq!(status, StatusCode::OK);
    // theme_lookup("head_tag") in the head, common then the target's.
    assert!(body.contains("<i class=\"head_tag-common\"></i>\n<i class=\"head_tag-desktop\"></i>"));
    assert!(before(&body, "head_tag-desktop", "</head>"));
    // theme_lookup("header") before Ember's root.
    assert!(body.contains(
        "<i class=\"header-common\"></i>\n<i class=\"header-desktop\"></i>\n    <section id=\"main\">"
    ));
    // DCustomHtml "top" first in the main container.
    assert!(body.contains(
        "<div class=\"container\" id=\"main-container\"><div><i class=\"after_header-common\"></i>\n<i class=\"after_header-desktop\"></i></div>"
    ));
    // DCustomHtml "footer" after the main outlet's wrapper.
    assert!(body.contains(
        "<div class=\"custom-footer-content\"><i class=\"footer-common\"></i>\n<i class=\"footer-desktop\"></i></div>"
    ));
    assert!(before(
        &body,
        "id=\"main-outlet-wrapper\"",
        "custom-footer-content"
    ));
    // theme_lookup("body_tag") last in the body.
    assert!(body.contains(
        "<i class=\"body_tag-common\"></i>\n<i class=\"body_tag-desktop\"></i>\n  </body>"
    ));
    assert!(!body.contains("-mobile\"></i>"));
}

#[tokio::test]
async fn the_user_agent_picks_the_target_whatever_the_viewport() {
    let db = TestDb::new().await;
    fielded(&db.pool).await;
    // A phone drawn in the desktop layout still gets the mobile fields.
    let (_, body) = page(&db.pool, "/latest", IPHONE, Some("mobile_view=0")).await;
    assert!(body.contains("<i class=\"header-common\"></i>\n<i class=\"header-mobile\"></i>"));
    assert!(!body.contains("-desktop\"></i>"));
    let (_, body) = page(&db.pool, "/latest", DESKTOP, Some("mobile_view=1")).await;
    assert!(body.contains("<i class=\"footer-common\"></i>\n<i class=\"footer-desktop\"></i>"));
    assert!(!body.contains("-mobile\"></i>"));
}

#[tokio::test]
async fn pages_that_hide_the_footer_hide_the_theme_s_and_the_admin_has_no_top() {
    let db = TestDb::new().await;
    fielded(&db.pool).await;
    // The no_ember layout draws the footer after the page, no top.
    let (status, body) = page(&db.pool, "/t/nope-nope", DESKTOP, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains(
        "</section>\n    <i class=\"footer-common\"></i>\n<i class=\"footer-desktop\"></i>\n    <i class=\"body_tag-common\"></i>"
    ));
    assert!(before(&body, "header-desktop", "<section id=\"main\">"));
    assert!(before(&body, "head_tag-desktop", "</head>"));
    assert!(!body.contains("after_header-"));
}

#[tokio::test]
async fn components_follow_their_parent_and_disabled_ones_are_left_out() {
    let db = TestDb::new().await;
    for (id, name, enabled) in [(50, "On", true), (51, "Off", false)] {
        sqlx::query(
            "INSERT INTO themes (id, name, user_id, created_at, updated_at, compiler_version, \
             component, enabled) VALUES ($1, $2, -1, now(), now(), 0, true, $3)",
        )
        .bind(id)
        .bind(name)
        .bind(enabled)
        .execute(&db.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO child_themes (parent_theme_id, child_theme_id, created_at, updated_at) \
             VALUES (-1, $1, now(), now())",
        )
        .bind(id)
        .execute(&db.pool)
        .await
        .unwrap();
        let html = format!("<b>{name}</b>");
        field(&db.pool, id, 0, "body_tag", &html, Some(&html)).await;
    }
    field(
        &db.pool,
        -1,
        1,
        "body_tag",
        "<b>Parent</b>",
        Some("<b>Parent</b>"),
    )
    .await;
    let (_, body) = page(&db.pool, "/latest", DESKTOP, None).await;
    assert!(body.contains("<b>Parent</b>\n<b>On</b>\n  </body>"));
    assert!(!body.contains("<b>Off</b>"));
}

#[tokio::test]
async fn an_unbaked_field_is_baked_and_the_nonce_dropped() {
    let db = TestDb::new().await;
    field(
        &db.pool,
        -1,
        0,
        "head_tag",
        "<script src=\"/x.js\"></script><p>a & b",
        None,
    )
    .await;
    let (status, body) = page(&db.pool, "/latest", DESKTOP, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("<script src=\"/x.js\"></script><p>a &amp; b</p>"),
        "{body}"
    );
    let baked: Option<String> = sqlx::query_scalar(
        "SELECT value_baked FROM theme_fields WHERE theme_id = -1 AND name = 'head_tag'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        baked.as_deref(),
        Some(
            format!("<script src=\"/x.js\" nonce=\"{PLACEHOLDER}\"></script><p>a &amp; b</p>")
                .as_str()
        )
    );
}

#[tokio::test]
async fn inline_scripts_and_scripts_ember_would_not_run_are_refused() {
    let db = TestDb::new().await;
    // As Rails bakes an inline script: a javascript cache's url.
    field(
        &db.pool,
        -1,
        0,
        "body_tag",
        "<script>console.log(1)</script>",
        Some(&format!(
            "<script defer src='/theme-javascripts/abc.js?__ws=test.localhost' data-theme-id='-1' nonce=\"{PLACEHOLDER}\"></script>"
        )),
    )
    .await;
    let (status, body) = page(&db.pool, "/latest", DESKTOP, None).await;
    // Unsupported: the 500 that names it in the log.
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");

    let db = TestDb::new().await;
    let script = format!("<script src=\"/x.js\" nonce=\"{PLACEHOLDER}\"></script>");
    field(
        &db.pool,
        -1,
        0,
        "footer",
        "<script src=\"/x.js\"></script>",
        Some(&script),
    )
    .await;
    let (status, body) = page(&db.pool, "/latest", DESKTOP, None).await;
    // Unsupported: the 500 that names it in the log.
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");

    // A script that never runs (data) is drawn.
    let db = TestDb::new().await;
    let data =
        format!("<script type=\"application/ld+json\" nonce=\"{PLACEHOLDER}\">{{}}</script>");
    field(
        &db.pool,
        -1,
        0,
        "after_header",
        "<script type=\"application/ld+json\">{}</script>",
        Some(&data),
    )
    .await;
    let (status, body) = page(&db.pool, "/latest", DESKTOP, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("<div><script type=\"application/ld+json\">{}</script></div>"),
        "{body}"
    );
}
