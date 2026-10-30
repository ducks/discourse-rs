//! A member's requests to the chat API, logged in through /session: the
//! status and the JSON body. Included by the chat test files with
//! `#[path = "common/chat_client.rs"] mod chat_client;`.

use axum::body::Body;
use axum::http::{Request, header};
use discourse_rs::AppState;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

pub struct Client {
    pub state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    pub async fn request(&mut self, method: &str, path: &str, body: &str) -> (u16, Value) {
        let (status, bytes) = self.raw(method, path, body).await;
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// A page as a browser asks for it: the status and the HTML.
    #[allow(dead_code)]
    pub async fn page(&mut self, path: &str) -> (u16, String) {
        let (status, bytes) = self.send(false, "GET", path, "").await;
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn raw(&mut self, method: &str, path: &str, body: &str) -> (u16, Vec<u8>) {
        self.send(true, method, path, body).await
    }

    async fn send(&mut self, xhr: bool, method: &str, path: &str, body: &str) -> (u16, Vec<u8>) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        if xhr {
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
        let status = response.status().as_u16();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, bytes.to_vec())
    }

    pub async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client.request("GET", "/session/csrf.json", "").await;
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
            .request(
                "POST",
                "/session",
                &format!("login={username}&password=password"),
            )
            .await;
        client
    }

    pub async fn get(&mut self, path: &str) -> (u16, Value) {
        self.request("GET", path, "").await
    }
}
