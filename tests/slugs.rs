//! SlugsController#generate (POST /slugs.json), ported from
//! spec/requests/slugs_controller_spec.rb.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, header};
use common::{TestDb, config, state};
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
    fn new(state: AppState) -> Client {
        Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        }
    }

    async fn request(&mut self, method: Method, path: &str, form: Option<&str>) -> (u16, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest");
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
        let body = match form {
            Some(form) => {
                request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
                Body::from(form.to_string())
            }
            None => Body::empty(),
        };
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let pair = v.to_str().unwrap().split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }

    async fn csrf(&mut self) {
        let (_, csrf) = self.request(Method::GET, "/session/csrf.json", None).await;
        self.csrf = csrf["csrf"].as_str().map(str::to_string);
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client::new(state);
        client.csrf().await;
        client
            .request(
                Method::POST,
                "/session",
                Some(&format!("login={username}&password=password")),
            )
            .await;
        client
    }
}

async fn set_trust_level(db: &TestDb, username: &str, level: i32) {
    sqlx::query(
        "UPDATE users SET trust_level = $1, admin = false, moderator = false WHERE username = $2",
    )
    .bind(level)
    .bind(username)
    .execute(&db.pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn generate_answers_the_slug_for_trust_level_4_and_staff() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;

    // Not logged in: 403.
    let mut visitor = Client::new(st.clone());
    visitor.csrf().await;
    let (status, _) = visitor
        .request(Method::POST, "/slugs.json", Some("name=Arts+%26+Media"))
        .await;
    assert_eq!(status, 403);

    set_trust_level(&db, "user1", 4).await;
    let mut member = Client::logged_in(st.clone(), "user1").await;
    let (status, body) = member
        .request(Method::POST, "/slugs.json", Some("name=Arts+%26+Media"))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["success"], "OK");
    assert_eq!(body["slug"], "arts-media");

    // The name is required.
    let (status, body) = member.request(Method::POST, "/slugs.json", None).await;
    assert_eq!(status, 400, "{body}");

    // Below trust level 4: 403.
    set_trust_level(&db, "user1", 1).await;
    let (status, _) = member
        .request(Method::POST, "/slugs.json?name=Arts+%26+Media", None)
        .await;
    assert_eq!(status, 403);

    let mut admin = Client::logged_in(st, "admin").await;
    let (status, body) = admin
        .request(Method::POST, "/slugs.json", Some("name=Arts+%26+Media"))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["slug"], "arts-media");
}

#[tokio::test]
async fn generate_is_rate_limited() {
    let db = TestDb::new().await;
    let mut cfg = config(RailsEnv::Test, &[]);
    cfg.rate_limits = true;
    let st = state(db.pool.clone(), cfg).await;
    set_trust_level(&db, "user1", 4).await;
    let mut member = Client::logged_in(st, "user1").await;
    let mut last = 0;
    for _ in 0..21 {
        last = member
            .request(Method::POST, "/slugs.json?name=Arts", None)
            .await
            .0;
    }
    assert_eq!(last, 429);
}
