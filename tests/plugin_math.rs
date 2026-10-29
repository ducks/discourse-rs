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
        "data-mathjax-url=\"https://cdn.jsdelivr.net/npm/mathjax@4.1.0\"",
        "data-katex-url=\"https://cdn.jsdelivr.net/npm/katex@0.16.22/dist\"",
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
