//! Chat's admin side, ported from chat's request specs: incoming webhooks
//! (plain and Slack compatible), the admin's webhooks (JSON and pages),
//! the messages export and the category editor's Chat tab.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn request(&mut self, method: Method, path: &str, body: Option<Value>) -> (u16, String) {
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
        let body = match body {
            Some(Value::String(form)) => {
                request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
                Body::from(form)
            }
            Some(json) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap();
            let pair = v.split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn json(&mut self, method: Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let (status, text) = self.request(method, path, body).await;
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client.json(Method::GET, "/session/csrf.json", None).await;
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
            .request(
                Method::POST,
                "/session",
                Some(Value::String(format!("login={username}&password=password"))),
            )
            .await;
        client
    }

    /// A page as a browser asks for it: the status, the Location and the
    /// body.
    async fn page(&self, path: &str) -> (u16, Option<String>, String) {
        let cookie: Vec<String> = self
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let response = discourse_rs::app(self.state.clone())
            .oneshot(
                Request::get(path)
                    .header(header::HOST, "test.localhost")
                    .header(header::COOKIE, cookie.join("; "))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|l| l.to_str().unwrap().to_string());
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            location,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }
}

async fn scalar<T>(db: &TestDb, sql: &str) -> T
where
    T: Send + Unpin + for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    sqlx::query_scalar(sql).fetch_one(&db.pool).await.unwrap()
}

async fn exec(db: &TestDb, sql: &str) {
    sqlx::query(sql).execute(&db.pool).await.unwrap();
}

async fn webhook(db: &TestDb) {
    exec(
        db,
        "INSERT INTO incoming_chat_webhooks (id, name, key, chat_channel_id, created_at, updated_at) \
         VALUES (1, 'Alerts', 'abc123', 2, now(), now())",
    )
    .await;
    exec(db, "SELECT setval('incoming_chat_webhooks_id_seq', 1)").await;
}

#[tokio::test]
async fn an_incoming_webhook_posts_as_the_system_user() {
    let db = TestDb::new().await;
    webhook(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut anon = Client {
        state: st,
        cookies: Vec::new(),
        csrf: None,
    };
    let (status, body) = anon
        .json(
            Method::POST,
            "/chat/hooks/abc123",
            Some(json!({"text": "hello from outside"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"success": "OK"}));
    let (user_id, message): (i32, String) = sqlx::query_as(
        "SELECT user_id, message FROM chat_messages WHERE chat_channel_id = 2 ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((user_id, message.as_str()), (-1, "hello from outside"));
    assert_eq!(
        scalar::<i64>(
            &db,
            "SELECT COUNT(*) FROM chat_webhook_events WHERE incoming_chat_webhook_id = 1"
        )
        .await,
        1
    );
    assert!(
        scalar::<bool>(&db, "SELECT following FROM user_chat_channel_memberships WHERE user_id = -1 AND chat_channel_id = 2").await
    );

    // No payload at all, an unknown key.
    let (status, body) = anon
        .json(Method::POST, "/chat/hooks/abc123", Some(json!({})))
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error_type"], "invalid_parameters");
    let (status, _) = anon
        .json(Method::POST, "/chat/hooks/nope", Some(json!({"text": "x"})))
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn the_slack_endpoint_translates_slack_text() {
    let db = TestDb::new().await;
    webhook(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut anon = Client {
        state: st,
        cookies: Vec::new(),
        csrf: None,
    };
    let last = async |db: &TestDb| -> String {
        scalar(
            db,
            "SELECT message FROM chat_messages ORDER BY id DESC LIMIT 1",
        )
        .await
    };
    let (status, body) = anon
        .json(
            Method::POST,
            "/chat/hooks/abc123/slack",
            Some(json!({"text": "<!here> see <https://example.com|the site>"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(last(&db).await, "@here see [the site](https://example.com)");
    // OpsGenie's payload: a JSON string with legacy attachments.
    let payload = json!({"attachments": [{"fallback": "Alert%3A%20disk%20full"}]}).to_string();
    let (status, body) = anon
        .json(
            Method::POST,
            "/chat/hooks/abc123/slack",
            Some(json!({"payload": payload})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(last(&db).await, "Alert: disk full");
    let (status, _) = anon
        .json(
            Method::POST,
            "/chat/hooks/abc123/slack",
            Some(json!({"payload": "{not json"})),
        )
        .await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn admins_manage_incoming_webhooks() {
    let db = TestDb::new().await;
    webhook(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, body) = admin
        .json(Method::GET, "/admin/plugins/chat/hooks.json", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["chat_channels"].as_array().unwrap().len(), 2);
    let hook = &body["incoming_chat_webhooks"][0];
    assert_eq!(hook["name"], "Alerts");
    assert!(
        hook["url"]
            .as_str()
            .unwrap()
            .ends_with("/chat/hooks/abc123"),
        "{hook}"
    );
    assert_eq!(hook["chat_channel"]["id"], 2);

    let (status, body) = admin
        .json(
            Method::POST,
            "/admin/plugins/chat/hooks",
            Some(json!({"name": "Deploys", "chat_channel_id": 2})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let id = body["id"].as_i64().unwrap();
    let key: String = scalar(
        &db,
        &format!("SELECT key FROM incoming_chat_webhooks WHERE id = {id}"),
    )
    .await;
    assert_eq!(key.len(), 24);
    assert!(key.bytes().all(|b| b.is_ascii_hexdigit()));
    let (status, _) = admin
        .json(
            Method::POST,
            "/admin/plugins/chat/hooks",
            Some(json!({"chat_channel_id": 2})),
        )
        .await;
    assert_eq!(status, 400);
    let (status, _) = admin
        .json(
            Method::POST,
            "/admin/plugins/chat/hooks",
            Some(json!({"name": "x", "chat_channel_id": 99})),
        )
        .await;
    assert_eq!(status, 404);
    let (status, body) = admin
        .json(
            Method::POST,
            "/admin/plugins/chat/hooks",
            Some(json!({"name": "x".repeat(101), "chat_channel_id": 2})),
        )
        .await;
    assert_eq!(status, 422);
    assert_eq!(
        body,
        json!({"errors": ["Name is too long (maximum is 100 characters)"]})
    );

    let (status, body) = admin
        .json(
            Method::PUT,
            &format!("/admin/plugins/chat/hooks/{id}"),
            Some(json!({"name": "Deploys", "chat_channel_id": 1, "username": "deploybot", "emoji": ":rocket:"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = admin
        .json(
            Method::GET,
            &format!("/admin/plugins/chat/hooks/{id}/edit.json"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["webhook"]["username"], "deploybot");
    assert_eq!(body["webhook"]["chat_channel"]["id"], 1);
    let (status, _) = admin
        .json(
            Method::DELETE,
            &format!("/admin/plugins/chat/hooks/{id}"),
            None,
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        scalar::<i64>(&db, "SELECT COUNT(*) FROM incoming_chat_webhooks").await,
        1
    );

    // Admins only.
    let mut user1 = Client::logged_in(st, "user1").await;
    let (status, _) = user1
        .json(Method::GET, "/admin/plugins/chat/hooks.json", None)
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn the_chat_admin_pages_are_drawn() {
    let db = TestDb::new().await;
    webhook(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let admin = Client::logged_in(st, "admin").await;
    let (status, _, html) = admin.page("/admin/plugins/chat/hooks").await;
    assert_eq!(status, 200, "{html}");
    assert!(
        html.contains("incoming-chat-webhooks-row\" data-webhook-id=\"1\""),
        "{html}"
    );
    assert!(html.contains("admin-chat-export"));
    assert!(html.contains("href=\"/admin/plugins/chat/hooks\">Incoming webhooks</a>"));
    assert!(html.contains("/assets/chat-admin.js"));
    let (status, _, html) = admin.page("/admin/plugins/chat/hooks/1/edit").await;
    assert_eq!(status, 200, "{html}");
    assert!(html.contains("chat-incoming-webhook-form\" data-webhook-id=\"1\""));
    assert!(
        html.contains("<code>http://test.localhost/chat/hooks/abc123</code>")
            || html.contains("/chat/hooks/abc123</code>"),
        "{html}"
    );
    let (status, _, html) = admin.page("/admin/plugins/chat/hooks/new").await;
    assert_eq!(status, 200);
    assert!(html.contains("name=\"chat_channel_id\""));
    // The settings page has the nav and the export too.
    let (status, _, html) = admin.page("/admin/plugins/chat/settings").await;
    assert_eq!(status, 200, "{html}");
    assert!(html.contains("admin-chat-export"), "{html}");
}

#[tokio::test]
async fn admins_export_chat_messages() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, _) = admin
        .request(Method::POST, "/chat/admin/export/messages", None)
        .await;
    assert_eq!(status, 204);
    let args: Value = scalar(
        &db,
        "SELECT args FROM discourse_rs.jobs WHERE name = 'export_csv_file'",
    )
    .await;
    assert_eq!(args, json!({"entity": "chat_message", "user_id": 1}));
    let subject: String = scalar(
        &db,
        "SELECT subject FROM user_histories WHERE acting_user_id = 1 ORDER BY id DESC LIMIT 1",
    )
    .await;
    assert_eq!(subject, "chat_message");
}

#[tokio::test]
async fn the_category_editor_has_a_chat_tab() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let admin = Client::logged_in(st, "admin").await;
    let (status, _, html) = admin.page("/c/general/edit/chat").await;
    assert_eq!(status, 200, "{html}");
    assert!(html.contains("<li class=\"edit-category-chat\"><a href=\"/c/general/edit/chat\" class=\"active\">Chat</a></li>"), "{html}");
    assert!(
        html.contains("<tr class=\"d-table__row\" data-channel-id=\"2\">"),
        "{html}"
    );
    assert!(html.contains("href=\"/chat/c/general/2/info/settings\""));
    let (status, _, html) = admin.page("/c/general/sub-general/edit/chat").await;
    assert_eq!(status, 200, "{html}");
    assert!(
        html.contains("There are no chat channels associated with this category"),
        "{html}"
    );
}
