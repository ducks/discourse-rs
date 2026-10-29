//! discourse-math on the server-rendered pages: the renderer's script
//! with the settings buildDiscourseMathOptions reads, and the composer's
//! Insert Math item (initializers/discourse-math.js). The cooking is in
//! tests/pretty_text.rs.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    fn anonymous(state: AppState) -> Client {
        Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        }
    }

    async fn request(&mut self, method: Method, path: &str, body: &str) -> (StatusCode, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        if path.ends_with(".json") || !body.is_empty() {
            request = request.header("x-requested-with", "XMLHttpRequest");
        }
        if !self.cookies.is_empty() {
            let cookie: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookie.join("; "));
        }
        if let Some(token) = &self.csrf {
            request = request.header("x-csrf-token", token.as_str());
        }
        if !body.is_empty() {
            request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let pair = v.to_str().unwrap().split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client::anonymous(state);
        let (_, csrf) = client.request(Method::GET, "/session/csrf.json", "").await;
        let csrf: Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
            .request(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
            )
            .await;
        client
    }
}

/// The script tag of discourse-math.js on a page, if any.
fn math_script(page: &str) -> Option<String> {
    let at = page.find("/assets/discourse-math.js")?;
    let start = page[..at].rfind("<script")?;
    let end = at + page[at..].find("</script>")?;
    Some(page[start..end].to_string())
}

#[tokio::test]
async fn the_renderer_loads_while_the_plugin_is_on() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (status, page) = Client::anonymous(st.clone())
        .request(Method::GET, "/latest", "")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(math_script(&page).is_none(), "off by default");

    set_setting(&db.pool, "discourse_math_enabled", 5, "t").await;
    let (_, page) = Client::anonymous(st.clone())
        .request(Method::GET, "/latest", "")
        .await;
    let script = math_script(&page).expect("the script while enabled");
    for attr in [
        "data-provider=\"mathjax\"",
        "data-mathjax-output=\"html\"",
        "data-enable-menu=\"true\"",
        "data-enable-asciimath=\"false\"",
        "data-enable-accessibility=\"false\"",
        "data-zoom-on-click=\"false\"",
        "data-bundle-url=\"/plugins/discourse-math/1.0.1\"",
        "Insert Block Math",
    ] {
        assert!(script.contains(attr), "{attr} in {script}");
    }

    set_setting(&db.pool, "discourse_math_provider", 7, "katex").await;
    set_setting(&db.pool, "discourse_math_zoom_on_click", 5, "t").await;
    // A new process: the anonymous page cache holds the last page.
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (_, page) = Client::anonymous(st.clone())
        .request(Method::GET, "/latest", "")
        .await;
    let script = math_script(&page).unwrap();
    assert!(script.contains("data-provider=\"katex\""), "{script}");
    assert!(script.contains("data-zoom-on-click=\"true\""), "{script}");

    // The script itself, and the plugin's stylesheet in discourse.css.
    let (status, js) = Client::anonymous(st.clone())
        .request(Method::GET, "/assets/discourse-math.js", "")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(js.contains("Discourse.renderMath"));
    let (_, css) = Client::anonymous(st)
        .request(Method::GET, "/assets/discourse.css", "")
        .await;
    assert!(css.contains("[hidden].math-hidden"));
}

/// The composer's options menu: Insert Math (Shift+M) while the plugin is
/// on, and the menu no longer refuses to open for it.
#[tokio::test]
async fn the_composer_offers_insert_math() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "discourse_math_enabled", 5, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let (status, page) = user1.request(Method::GET, "/latest", "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("insert-math"), "the menu item");
    assert!(page.contains("square-root-variable"), "its icon");
    assert!(page.contains("&quot;unported&quot;:[]"), "not refused");
}

/// GET `path` with no session: status, headers and the body's bytes.
async fn fetch(state: AppState, path: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let request = Request::builder()
        .uri(path)
        .header(header::HOST, "test.localhost")
        .body(Body::empty())
        .unwrap();
    let response = discourse_rs::app(state).oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, bytes.to_vec())
}

/// The gem's files at Rails' public paths (BundlePaths.public_url_base),
/// byte for byte the vendored copies, with nginx.sample.conf's
/// `^/plugins/` caching; linked at boot, so served with the plugin off.
#[tokio::test]
async fn the_bundle_is_served_at_the_rails_paths() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let vendored = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/static/vendor/discourse-math-bundle"
    );
    for (file, content_type) in [
        ("mathjax/tex-mml-chtml.js", "text/javascript; charset=utf-8"),
        ("mathjax/woff-v2/mjx-ncm-br.woff2", "font/woff2"),
        ("katex/fonts/KaTeX_AMS-Regular.woff", "font/woff"),
        ("katex/katex.min.css", "text/css; charset=utf-8"),
        (
            "katex/contrib/mhchem.min.js",
            "text/javascript; charset=utf-8",
        ),
        ("mathjax/sre/mathmaps/en.json", "application/json"),
    ] {
        let (status, headers, body) =
            fetch(st.clone(), &format!("/plugins/discourse-math/1.0.1/{file}")).await;
        assert_eq!(status, StatusCode::OK, "{file}");
        assert_eq!(headers[header::CONTENT_TYPE], content_type, "{file}");
        assert_eq!(
            headers[header::CACHE_CONTROL],
            "max-age=31556952, public, immutable",
            "{file}"
        );
        assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*", "{file}");
        let expected = std::fs::read(format!("{vendored}/{file}")).unwrap();
        assert!(body == expected, "{file}: the vendored bytes");
    }

    for missing in [
        "/plugins/discourse-math/1.0.1/mathjax/nope.js",
        "/plugins/discourse-math/1.0.1/../Cargo.toml",
        "/plugins/discourse-math/1.0.0/katex/katex.min.css",
    ] {
        assert_eq!(
            fetch(st.clone(), missing).await.0,
            StatusCode::NOT_FOUND,
            "{missing}"
        );
    }
}

/// add_to_serializer(:site, :discourse_math_bundle_url), only while the
/// plugin is on (the serializer respects enabled_site_setting), after
/// core's keys.
#[tokio::test]
async fn the_site_names_the_bundle_url_while_enabled() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (status, site) = Client::anonymous(st)
        .request(Method::GET, "/site.json", "")
        .await;
    assert_eq!(status, StatusCode::OK);
    let site: Value = serde_json::from_str(&site).unwrap();
    assert!(site.get("discourse_math_bundle_url").is_none(), "off");

    set_setting(&db.pool, "discourse_math_enabled", 5, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (_, site) = Client::anonymous(st)
        .request(Method::GET, "/site.json", "")
        .await;
    let site: Value = serde_json::from_str(&site).unwrap();
    assert_eq!(
        site["discourse_math_bundle_url"],
        "/plugins/discourse-math/1.0.1"
    );
    let keys: Vec<&String> = site.as_object().unwrap().keys().collect();
    assert_eq!(
        keys.last().map(|k| k.as_str()),
        Some("discourse_math_bundle_url")
    );
}
