//! `/letter_avatar_proxy`, against a local server standing in for the
//! avatar CDN.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::routing::get;
use common::{TestDb, config, set_setting, state};
use discourse_rs::config::{Config, RailsEnv};
use http_body_util::BodyExt;
use tower::ServiceExt;

const BOOL: i32 = 5;
const STRING: i32 = 1;
const LETTER: &[u8] = b"\x89PNG letter u";
const BLANK: &[u8] = b"\x89PNG blank";

/// Serves the one letter avatar it has, a body over the 1MB limit, and
/// 404 for anything else, counting requests.
async fn cdn() -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = hits.clone();
    let app = Router::new()
        .route(
            "/v4/letter/u/94ad74/24.png",
            get(|| async { LETTER.to_vec() }),
        )
        .route(
            "/v4/letter/b/94ad74/24.png",
            get(|| async { vec![0u8; 1024 * 1024 + 1] }),
        )
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                counted.fetch_add(1, Ordering::SeqCst);
                next.run(request)
            },
        ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), hits)
}

/// A scratch directory holding public/images/avatar.png and tmp/.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("user-avatars-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("public/images")).unwrap();
    std::fs::write(dir.join("public/images/avatar.png"), BLANK).unwrap();
    dir
}

fn proxy_config(cdn: &str, dir: &std::path::Path) -> Config {
    let mut config = config(RailsEnv::Test, &[]);
    config.letter_avatar_cdn = cdn.to_string();
    config.public_dir = dir.join("public");
    config.tmp_dir = dir.join("tmp");
    config
}

async fn get_avatar(
    db: &TestDb,
    config: &Config,
    path: &str,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let app = discourse_rs::app(state(db.pool.clone(), config.clone()).await);
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
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, body.to_vec())
}

#[tokio::test]
async fn letter_avatars_are_fetched_once_and_cached() {
    let db = TestDb::new().await;
    let (cdn, hits) = cdn().await;
    let dir = scratch("cached");
    let config = proxy_config(&cdn, &dir);
    set_setting(&db.pool, "login_required", BOOL, "t").await;

    for _ in 0..2 {
        let (status, headers, body) = get_avatar(
            &db,
            &config,
            "/letter_avatar_proxy/v4/letter/u/94ad74/24.png",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, LETTER);
        assert_eq!(headers[header::CONTENT_TYPE], "image/png");
        assert_eq!(
            headers[header::CACHE_CONTROL],
            "max-age=31556952, public, immutable"
        );
        assert_eq!(
            headers[header::LAST_MODIFIED],
            "Mon, 01 Jan 1990 00:00:00 GMT"
        );
        assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    }
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "served from the cache after"
    );
    assert_eq!(
        std::fs::read_dir(dir.join("tmp/avatar_proxy"))
            .unwrap()
            .count(),
        1
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn failed_fetches_render_the_blank_avatar() {
    let db = TestDb::new().await;
    let (cdn, _) = cdn().await;
    let dir = scratch("blank");
    let config = proxy_config(&cdn, &dir);

    for path in [
        // The CDN's 404.
        "/letter_avatar_proxy/v4/letter/x/94ad74/24.png",
        // Over max_file_size.
        "/letter_avatar_proxy/v4/letter/b/94ad74/24.png",
    ] {
        let (status, headers, body) = get_avatar(&db, &config, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(body, BLANK, "{path}");
        assert_eq!(headers[header::CACHE_CONTROL], "max-age=600, public");
        assert_eq!(
            headers[header::LAST_MODIFIED],
            "Mon, 01 Jan 1990 00:00:00 GMT"
        );
    }
    assert!(!dir.join("tmp/avatar_proxy").exists(), "nothing cached");

    // Unreachable.
    let config = proxy_config("http://127.0.0.1:1", &dir);
    let (status, _, body) = get_avatar(
        &db,
        &config,
        "/letter_avatar_proxy/v4/letter/u/94ad74/24.png",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, BLANK);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn the_proxy_is_off_unless_avatars_point_at_it() {
    let db = TestDb::new().await;
    let (cdn, hits) = cdn().await;
    let dir = scratch("off");
    let config = proxy_config(&cdn, &dir);
    let path = "/letter_avatar_proxy/v4/letter/u/94ad74/24.png";

    // `:size.png`: a size with a dot is no match.
    let (status, _, _) = get_avatar(
        &db,
        &config,
        "/letter_avatar_proxy/v4/letter/u/94ad74/24.5.png",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    set_setting(
        &db.pool,
        "external_system_avatars_url",
        STRING,
        "https://avatars.example.com/{first_letter}/{color}/{size}.png",
    )
    .await;
    let (status, _, _) = get_avatar(&db, &config, path).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

fn dimensions(png: &[u8]) -> (u32, u32) {
    let image = image::load_from_memory(png).unwrap();
    (image.width(), image.height())
}

/// `/letter_avatar`, drawn here: each size from the full-size drawing,
/// cached under tmp/letter_avatars/<version>, as the reference answers
/// (sizes past 360 are 360, 0 is 1x1, another version is the blank
/// avatar, a letter outside ImageMagick's safe paths fails below 360).
#[tokio::test]
async fn local_letter_avatars_are_drawn_and_cached() {
    let db = TestDb::new().await;
    let dir = scratch("local");
    let config = proxy_config("http://127.0.0.1:1", &dir);
    let version = discourse_rs::letter_avatar::version(&config.letter_avatar_font);
    assert!(version.starts_with("5_"), "{version}");

    let path = format!("/letter_avatar/alice/48/{version}.png");
    let (status, headers, body) = get_avatar(&db, &config, &path).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dimensions(&body), (48, 48));
    assert_eq!(headers[header::CONTENT_TYPE], "image/png");
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "max-age=31556952, public, immutable"
    );
    assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert!(
        headers[header::LAST_MODIFIED]
            .to_str()
            .unwrap()
            .ends_with(" GMT")
    );
    let [r, g, b] = discourse_rs::letter_avatar::Identity::from_username("alice").color;
    let cached = dir.join(format!("tmp/letter_avatars/{version}/A/{r}_{g}_{b}"));
    assert!(cached.join("48.png").exists(), "{}", cached.display());
    // The full size, drawn on the way, is served as drawn.
    assert!(cached.join("360.png").exists());
    let (_, _, again) = get_avatar(&db, &config, &path).await;
    assert_eq!(again, body);

    for (size, want) in [
        ("1000", 360),
        ("0", 1),
        ("7", 7),
        ("99999999999999999999", 360),
    ] {
        let (status, _, body) = get_avatar(
            &db,
            &config,
            &format!("/letter_avatar/bob/{size}/{version}.png"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{size}");
        assert_eq!(dimensions(&body), (want, want), "{size}");
    }

    // Another version: the blank avatar.
    let (status, headers, body) = get_avatar(&db, &config, "/letter_avatar/alice/48/5_x.png").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, BLANK);
    assert_eq!(headers[header::CACHE_CONTROL], "max-age=600, public");

    // The route's constraints.
    for path in [
        format!("/letter_avatar/alice/48x/{version}.png"),
        format!("/letter_avatar/alice/-5/{version}.png"),
        format!("/letter_avatar/al+ice/48/{version}.png"),
        format!("/letter_avatar/alice/48/{version}.jpg"),
    ] {
        let (status, _, _) = get_avatar(&db, &config, &path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }

    // É: drawn at full size, not resized (OptimizedImage.safe_path?).
    let (status, _, _) = get_avatar(
        &db,
        &config,
        &format!("/letter_avatar/%C3%A9/48/{version}.png"),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let (status, _, body) = get_avatar(
        &db,
        &config,
        &format!("/letter_avatar/%C3%A9/360/{version}.png"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dimensions(&body), (360, 360));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Without the font nothing is drawn: the request fails rather than
/// serving something else.
#[tokio::test]
async fn local_letter_avatars_need_the_font() {
    let db = TestDb::new().await;
    let dir = scratch("nofont");
    let mut config = proxy_config("http://127.0.0.1:1", &dir);
    config.letter_avatar_font = dir.join("missing.otf");
    let version = discourse_rs::letter_avatar::version(&config.letter_avatar_font);
    let (status, _, _) = get_avatar(
        &db,
        &config,
        &format!("/letter_avatar/alice/48/{version}.png"),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    std::fs::remove_dir_all(&dir).unwrap();
}
