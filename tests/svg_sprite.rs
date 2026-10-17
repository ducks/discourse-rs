//! The site's icon sprite (SvgSprite): what it holds follows the badges,
//! groups, settings and theme sprites, its version with it, and the icon
//! picker pages through the icons.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn get(st: &AppState, path: &str, cookie: Option<&str>) -> (StatusCode, String) {
    let app = discourse_rs::app(st.clone());
    let mut request = Request::get(path).header(header::HOST, "test.localhost");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        location.unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned()),
    )
}

/// A member's session cookies, logged in as the seed's `username`.
async fn login_cookie(st: &AppState, username: &str) -> String {
    let mut cookies: Vec<(String, String)> = Vec::new();
    let mut send = async |request: axum::http::request::Builder, body: String| {
        let jar: Vec<String> = cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let request = request
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest")
            .header(header::COOKIE, jar.join("; "));
        let response = discourse_rs::app(st.clone())
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let pair = v.to_str().unwrap().split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let csrf = send(Request::get("/session/csrf.json"), String::new()).await;
    let csrf: serde_json::Value = serde_json::from_str(&csrf).unwrap();
    let token = csrf["csrf"].as_str().unwrap().to_string();
    send(
        Request::post("/session")
            .header("x-csrf-token", token)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded"),
        format!("login={username}&password=password"),
    )
    .await;
    drop(send);
    cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The sprite script the latest page loads, fetched.
async fn sprite(st: &AppState) -> (String, String) {
    // A fresh page each time: the anonymous cache keeps /latest a minute.
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let (_, page) = get(st, &format!("/latest?page={n}"), None).await;
    let start = page
        .find("/svg-sprite/test.localhost/svg-")
        .unwrap_or_else(|| panic!("sprite script: {}", &page[..page.len().min(600)]));
    let path = &page[start..start + page[start..].find('"').unwrap()];
    let (status, body) = get(st, path, None).await;
    assert_eq!(status, StatusCode::OK, "{path} {body}");
    (path.to_string(), body)
}

#[tokio::test]
async fn the_sprite_follows_what_names_icons() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (path, body) = sprite(&st).await;
    assert!(body.starts_with("window.__svg_sprite = \"<!--\\nDiscourse SVG subset"));
    assert!(body.contains("<symbol id=\\\"bars\\\""));
    // A plugin's own symbol, and no icon nothing names.
    assert!(body.contains("<symbol id=\\\"discobot\\\""));
    assert!(!body.contains("<symbol id=\\\"otter\\\""));
    // A badge with the icon: a new version holding it.
    sqlx::query("UPDATE badges SET icon = 'otter', updated_at = now() WHERE id = 1")
        .execute(&db.pool)
        .await
        .unwrap();
    let (new_path, body) = sprite(&st).await;
    assert_ne!(path, new_path);
    assert!(body.contains("<symbol id=\\\"otter\\\""));
    // The old version redirects to the new.
    let (status, location) = get(&st, &path, None).await;
    assert_eq!(status, StatusCode::FOUND);
    assert!(location.ends_with(&new_path), "{location}");
    // A group's flair and an icon setting.
    sqlx::query("UPDATE groups SET flair_icon = 'hippo', updated_at = now() WHERE id = 10")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, body) = sprite(&st).await;
    assert!(body.contains("<symbol id=\\\"hippo\\\""));
    // A theme's own sprite.
    let theme: i32 =
        sqlx::query_scalar("SELECT value::int FROM site_settings WHERE name = 'default_theme_id'")
            .fetch_optional(&db.pool)
            .await
            .unwrap()
            .unwrap_or(-1);
    {
        sqlx::query(
            "INSERT INTO theme_svg_sprites (theme_id, upload_id, sprite, created_at, updated_at) \
             VALUES ($1, 1, convert_to('<svg xmlns=\"http://www.w3.org/2000/svg\"><symbol id=\"my-mark\" viewBox=\"0 0 1 1\"><path d=\"M0 0\"/></symbol></svg>', 'UTF8'), now(), now())",
        )
        .bind(theme)
        .execute(&db.pool)
        .await
        .unwrap();
        let (_, body) = sprite(&st).await;
        assert!(body.contains("<symbol id=\\\"my-mark\\\" viewBox=\\\"0 0 1 1\\\">\\n  <path d=\\\"M0 0\\\"/>\\n</symbol>"));
    }
    // A malformed version has no route.
    let (status, _) = get(&st, "/svg-sprite/test.localhost/svg-1-0.js", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_icon_picker_search_and_single_icons() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    // Members only.
    let (status, _) = get(&st, "/svg-sprite/picker-search", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let cookie = login_cookie(&st, "user1").await;
    let (status, body) = get(
        &st,
        "/svg-sprite/picker-search?only_available=false",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["icons"].as_array().unwrap().len(), 100);
    assert_eq!(json["icons"][0], serde_json::json!({"id": "a"}));
    assert_eq!(json["has_more"], true);
    // Beyond the sprite: the symbol, escaped as Rails' JSON is.
    let (_, body) = get(
        &st,
        "/svg-sprite/picker-search?only_available=false&filter=otter",
        Some(&cookie),
    )
    .await;
    assert!(
        body.starts_with(
            "{\"icons\":[{\"id\":\"otter\",\"symbol\":\"\\u003csymbol id=\\\"otter\\\""
        ),
        "{body}"
    );
    let (_, body) = get(&st, "/svg-sprite/picker-search?filter=otter", Some(&cookie)).await;
    assert_eq!(body, "{\"icons\":[],\"has_more\":false}");
    let (status, _) = get(&st, "/svg-sprite/picker-search?page=x", Some(&cookie)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // One symbol, inspected; one icon as a document.
    let (status, body) = get(&st, "/svg-sprite/search/house", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("\"<symbol id=\\\"house\\\""), "{body}");
    let (status, body) = get(&st, "/svg-sprite/test.localhost/icon/f0a/house.svg", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("<?xml version=\"1.0\"?>\n<svg id=\"house\" viewBox=\"0 0 576 512\" xmlns=\"http://www.w3.org/2000/svg\" fill=\"#ff00aa\">"), "{body}");
    let (status, _) = get(&st, "/svg-sprite/test.localhost/icon/no-such.svg", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
