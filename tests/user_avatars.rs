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

#[test]
fn gravatar_hashes_are_of_the_trimmed_lowercased_email() {
    assert_eq!(
        discourse_rs::user_avatar::email_hash(" User1@Example.com "),
        format!("{:x}", md5::compute("user1@example.com"))
    );
}

/// UserAvatar#update_gravatar! past the download (which needs gravatar's
/// https): the avatar upload made of it, cropped, with the url as its
/// origin; the user's avatar follows when it was the old gravatar.
#[tokio::test]
async fn a_downloaded_gravatar_becomes_the_avatar_it_replaces() {
    let db = TestDb::new().await;
    let dir = scratch("gravatar");
    let config = proxy_config("http://127.0.0.1:1", &dir);
    let app = state(db.pool.clone(), config.clone()).await;
    // user3 (id 5) shows its gravatar, upload 34.
    sqlx::query("UPDATE users SET uploaded_avatar_id = 34 WHERE id = 5")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE user_avatars SET gravatar_upload_id = 34 WHERE user_id = 5")
        .execute(&db.pool)
        .await
        .unwrap();
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/parity/writes/files/small.png"
    ))
    .unwrap();
    let url = "https://www.gravatar.com/avatar/abc.png?s=288&d=404&reset_cache=x";
    let mut conn = db.pool.acquire().await.unwrap();
    let settings = discourse_rs::site_settings::SiteSettings::load(
        &mut conn,
        &app.site_setting_defs,
        &app.config.globals,
    )
    .await
    .unwrap();
    let user = discourse_rs::user_avatar::load_target(&mut conn, 5)
        .await
        .unwrap()
        .unwrap();
    discourse_rs::user_avatar::save_gravatar(
        &app,
        &mut conn,
        &settings,
        &user,
        Some(34),
        url,
        &bytes,
        ".png",
    )
    .await
    .unwrap();
    let (id, origin, width, name): (i32, Option<String>, i32, String) = sqlx::query_as(
        "SELECT id, origin, width, original_filename FROM uploads ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(origin.as_deref(), Some(url));
    assert_eq!(width, 288);
    assert_eq!(name, "gravatar.png");
    let (avatar, gravatar): (Option<i32>, Option<i32>) = sqlx::query_as(
        "SELECT u.uploaded_avatar_id, a.gravatar_upload_id FROM users u \
         JOIN user_avatars a ON a.user_id = u.id WHERE u.id = 5",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((avatar, gravatar), (Some(id), Some(id)));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `/user_avatar`: an avatar upload's square thumbnail made on demand,
/// another upload of the user's redirected to the current one, and the
/// blank avatar for what is not one, as the reference answers.
#[tokio::test]
async fn uploaded_avatars_are_served_at_each_size() {
    let db = TestDb::new().await;
    let dir = scratch("uploaded");
    let config = proxy_config("http://127.0.0.1:1", &dir);
    // discobot's avatar, upload 35 (its gravatar) in the seed.
    let original =
        dir.join("public/uploads/default/original/1X/e57ad2811b8c56c0101bc97841c44e29be36fe60.png");
    std::fs::create_dir_all(original.parent().unwrap()).unwrap();
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/parity/writes/files/small.png"),
        &original,
    )
    .unwrap();

    // 48 is in the seed, its file only on the reference: another size.
    let path = "/user_avatar/test.localhost/discobot/50/35_2.png";
    let (status, headers, body) = get_avatar(&db, &config, path).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dimensions(&body), (50, 50));
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "max-age=31556952, public, immutable"
    );
    let made: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM optimized_images WHERE upload_id = 35 AND width = 50 AND height = 50",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(made, 1);
    // The version defaults to OptimizedImage::VERSION; a size is any from
    // 8 to 1000.
    let (status, _, body) = get_avatar(
        &db,
        &config,
        "/user_avatar/test.localhost/discobot/120/35.png",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dimensions(&body), (120, 120));

    // Another upload id: the current avatar.
    let response = discourse_rs::app(state(db.pool.clone(), config.clone()).await)
        .oneshot(
            Request::get("/user_avatar/test.localhost/discobot/48/34_2.png")
                .header(header::HOST, "test.localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        response.headers()[header::LOCATION],
        "http://test.localhost/user_avatar/test.localhost/discobot/48/35_2.png"
    );

    for blank in [
        "/user_avatar/test.localhost/discobot/48/35_3.png",
        "/user_avatar/test.localhost/discobot/7/35_2.png",
        "/user_avatar/test.localhost/discobot/1001/35_2.png",
        "/user_avatar/test.localhost/nobody/48/35_2.png",
        "/user_avatar/test.localhost/discobot/48/0_2.png",
        // The system user's gravatar has no file here.
        "/user_avatar/test.localhost/system/48/34_2.png",
    ] {
        let (status, headers, body) = get_avatar(&db, &config, blank).await;
        assert_eq!(status, StatusCode::OK, "{blank}");
        assert_eq!(body, BLANK, "{blank}");
        assert_eq!(headers[header::CACHE_CONTROL], "max-age=600, public");
    }
    let (status, _, _) = get_avatar(
        &db,
        &config,
        "/user_avatar/test.localhost/discobot/4x8/35_2.png",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    std::fs::remove_dir_all(&dir).unwrap();
}
