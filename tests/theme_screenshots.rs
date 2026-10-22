//! The core themes' screenshots: the seed's uploads 1 to 4 (what
//! SystemThemesManager.sync! stores on a fresh install) are served at
//! their urls once ensure_screenshots has written the vendored files.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::config::RailsEnv;
use discourse_rs::system_themes::{SCREENSHOTS, ensure_screenshots};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn seed_screenshot_uploads_are_served() {
    let db = TestDb::new().await;
    let public = std::env::temp_dir().join(format!("rs-theme-screenshots-{}", std::process::id()));
    let mut cfg = config(RailsEnv::Test, &[]);
    cfg.public_dir = public.clone();

    let mut conn = db.pool.acquire().await.unwrap();
    assert_eq!(ensure_screenshots(&mut conn, &cfg).await.unwrap(), 4);
    // Present files are left alone.
    assert_eq!(ensure_screenshots(&mut conn, &cfg).await.unwrap(), 0);

    let rows: Vec<(i32, String, String, i64)> = sqlx::query_as(
        "SELECT f.theme_id, f.name, u.url, u.filesize FROM theme_fields f \
         JOIN uploads u ON u.id = f.upload_id WHERE f.type_id = 7 ORDER BY f.id",
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap();
    drop(conn);
    assert_eq!(rows.len(), SCREENSHOTS.len());

    let app = discourse_rs::app(state(db.pool.clone(), cfg).await);
    for (theme_id, name, url, filesize) in rows {
        let shot = SCREENSHOTS
            .iter()
            .find(|s| s.theme_id == theme_id && s.field == name)
            .unwrap_or_else(|| panic!("no vendored screenshot for {theme_id} {name}"));
        assert!(url.starts_with("/uploads/default/original/1X/"), "{url}");
        let response = app
            .clone()
            .oneshot(
                Request::get(&url)
                    .header(header::HOST, "test.localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{url}");
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(bytes.len(), filesize as usize, "{url}");
        assert_eq!(&bytes[..], shot.bytes, "{url}");
    }
    std::fs::remove_dir_all(&public).unwrap();
}
