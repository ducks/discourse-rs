//! discourse-graphviz on the server-rendered pages: the client's script
//! with viz.js's path, viz.js where Rails serves it, and the composer's
//! Insert Graphviz graph item (api-initializers/discourse-graphviz.js).
//! The cooking is in tests/pretty_text.rs.

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

/// The script tag of graphviz.js on a page, if any.
fn graphviz_script(page: &str) -> Option<String> {
    let at = page.find("/assets/graphviz.js")?;
    let start = page[..at].rfind("<script")?;
    let end = at + page[at..].find("</script>")?;
    Some(page[start..end].to_string())
}

#[tokio::test]
async fn the_client_loads_while_the_plugin_is_on() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (status, page) = Client::anonymous(st)
        .request(Method::GET, "/latest", "")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(graphviz_script(&page).is_none(), "off by default");

    set_setting(&db.pool, "discourse_graphviz_enabled", 5, "t").await;
    // A new process: the anonymous page cache holds the last page.
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (_, page) = Client::anonymous(st.clone())
        .request(Method::GET, "/latest", "")
        .await;
    let script = graphviz_script(&page).expect("the script while enabled");
    for attr in [
        "data-viz=\"/plugins/discourse-graphviz/javascripts/viz-3.0.1.js\"",
        "data-sample=\"digraph G {\n  a -&gt; b;\n  b -&gt; c;\n  a -&gt; c;\n}\"",
        "data-fullscreen=\"Expand diagram\"",
    ] {
        assert!(script.contains(attr), "{attr} in {script}");
    }

    // viz.js at Rails' path, the client, and the stylesheet.
    let (status, viz) = Client::anonymous(st.clone())
        .request(
            Method::GET,
            "/plugins/discourse-graphviz/javascripts/viz-3.0.1.js",
            "",
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(viz.contains("var vizRenderStringSync;"));
    let (status, js) = Client::anonymous(st.clone())
        .request(Method::GET, "/assets/graphviz.js", "")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(js.contains("Discourse.renderGraphviz"));
    let (_, css) = Client::anonymous(st)
        .request(Method::GET, "/assets/discourse.css", "")
        .await;
    assert!(css.contains("div.graphviz.is-loading"));
}

/// The composer's options menu: Insert Graphviz graph while the plugin is
/// on, and the menu no longer refuses to open for it.
#[tokio::test]
async fn the_composer_offers_a_graph() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "discourse_graphviz_enabled", 5, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let (status, page) = user1.request(Method::GET, "/latest", "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Insert Graphviz graph"), "the menu item");
    assert!(page.contains("diagram-project"), "its icon");
    assert!(page.contains("&quot;unported&quot;:[]"), "not refused");
}
