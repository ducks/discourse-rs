//! The favicon and apple touch icon: the layout links them as Rails'
//! layouts/_head does, and ensure_optimized makes the sized copies the
//! seed's optimized_images rows point at.

mod common;

use axum::body::Body;
use axum::http::{Request, header};
use common::{TestDb, config, state};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use tower::ServiceExt;

/// The sketch logo's (upload -6) optimized copies in the seed.
const FAVICON: &str =
    "uploads/default/optimized/1X/_129430568242d1b7f853bb13ebea28b3f6af4e7_2_32x32.png";
const TOUCH_ICON: &str =
    "uploads/default/optimized/1X/_129430568242d1b7f853bb13ebea28b3f6af4e7_2_180x180.png";

#[tokio::test]
async fn pages_link_the_favicon_and_touch_icon() {
    let db = TestDb::new().await;
    let app = discourse_rs::app(state(db.pool.clone(), config(RailsEnv::Test, &[])).await);
    let response = app
        .oneshot(
            Request::get("/latest")
                .header(header::HOST, "test.localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&bytes);
    assert!(html.contains(&format!(
        r#"<link rel="icon" type="image/png" href="http://test.localhost/{FAVICON}">"#
    )));
    assert!(html.contains(&format!(
        r#"<link rel="apple-touch-icon" type="image/png" href="http://test.localhost/{TOUCH_ICON}">"#
    )));
}

#[tokio::test]
async fn missing_icon_copies_are_made_from_the_original() {
    let db = TestDb::new().await;
    let public = std::env::temp_dir().join(format!("rs-site-icons-{}", std::process::id()));
    std::fs::create_dir_all(public.join("images")).unwrap();
    let logo = image::RgbaImage::from_pixel(64, 64, image::Rgba([200, 30, 30, 255]));
    logo.save(public.join("images/discourse-logo-sketch-small.png"))
        .unwrap();

    let mut cfg = config(RailsEnv::Test, &[]);
    cfg.public_dir = public.clone();
    let mut conn = db.pool.acquire().await.unwrap();
    let defs = discourse_rs::site_settings::Definitions::vendored().unwrap();
    let settings = discourse_rs::site_settings::SiteSettings::load(&mut conn, &defs, &cfg.globals)
        .await
        .unwrap();
    // favicon (32), apple_touch_icon (180), manifest_icon (512).
    let written = discourse_rs::site_icons::ensure_optimized(&mut conn, &settings, &cfg)
        .await
        .unwrap();
    assert_eq!(written, 3);
    let favicon = image::open(public.join(FAVICON)).unwrap();
    assert_eq!((favicon.width(), favicon.height()), (32, 32));
    // Present files are left alone.
    let again = discourse_rs::site_icons::ensure_optimized(&mut conn, &settings, &cfg)
        .await
        .unwrap();
    assert_eq!(again, 0);
    std::fs::remove_dir_all(&public).unwrap();
}
