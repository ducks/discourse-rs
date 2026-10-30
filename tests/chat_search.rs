//! GET /chat/api/search (Chat::SearchMessage): the member's channels'
//! messages matching the query, its @user and #channel filters, the
//! contract, paging and sorting.

mod common;

use axum::body::Body;
use axum::http::{Request, header};
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
    async fn request(&mut self, method: &str, path: &str, body: &str) -> (u16, Value) {
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
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client.request("GET", "/session/csrf.json", "").await;
        let csrf = csrf["csrf"].as_str().unwrap().to_string();
        client.csrf = Some(csrf.clone());
        client
            .request(
                "POST",
                "/session",
                &format!("login={username}&password=password&authenticity_token={csrf}"),
            )
            .await;
        client
    }

    async fn get(&mut self, path: &str) -> (u16, Value) {
        self.request("GET", path, "").await
    }
}

/// Messages in General (2) and staff (1), indexed as the indexer does.
async fn messages(db: &TestDb) {
    let m = |id: i32, channel: i32, user: i32, mins: i32, msg: &str| {
        [
            format!(
                "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
                 VALUES ({id}, {channel}, {user}, now() - interval '{mins} minutes', now() - interval '{mins} minutes', '{msg}', '<p>{msg}</p>', 1, {user})"
            ),
            format!(
                "INSERT INTO chat_message_search_data (chat_message_id, search_data, raw_data, locale, version) \
                 VALUES ({id}, setweight(to_tsvector('english', '{msg}'), 'A'), '{msg}', 'en', 1)"
            ),
        ]
    };
    let mut sql: Vec<String> = Vec::new();
    sql.extend(m(1, 2, 2, 50, "the quick brown fox"));
    sql.extend(m(2, 2, 3, 40, "a lazy brown dog"));
    sql.extend(m(3, 2, 2, 30, "nothing to see"));
    sql.extend(m(4, 1, 1, 20, "brown staff secret"));
    sql.extend(m(5, 2, 3, 10, "brown and gone"));
    sql.push("UPDATE chat_messages SET deleted_at = now(), deleted_by_id = 3 WHERE id = 5".into());
    sql.push("SELECT setval('chat_messages_id_seq', 5)".into());
    for s in sql {
        sqlx::query(&s).execute(&db.pool).await.unwrap();
    }
}

fn ids(body: &Value) -> Vec<i64> {
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn a_member_finds_messages_of_their_channels() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    let mut c = Client::logged_in(st, "user1").await;
    let (status, body) = c.get("/chat/api/search?query=brown&sort=latest").await;
    assert_eq!(status, 200, "{body}");
    // Not the staff channel's, nor the deleted one.
    assert_eq!(ids(&body), vec![2, 1]);
    assert_eq!(
        body["meta"],
        serde_json::json!({"has_more": false, "limit": 20, "offset": 0})
    );
    let m = &body["messages"][0];
    assert_eq!(m["message"], "a lazy brown dog");
    assert_eq!(m["user"]["username"], "user1");
    assert_eq!(m["channel"]["id"], 2);
    // The channel sits after blocks, as the serializer's attributes do.
    let keys: Vec<&String> = m.as_object().unwrap().keys().collect();
    let blocks = keys.iter().position(|k| *k == "blocks").unwrap();
    assert_eq!(keys[blocks + 1], "channel");

    // Prefixes match, as Search.ts_query's :* does.
    let (_, body) = c.get("/chat/api/search?query=qui").await;
    assert_eq!(ids(&body), vec![1]);

    // Staff in the staff channel see its message.
    let mut admin = Client::logged_in(c.state.clone(), "admin").await;
    let (_, body) = admin
        .get("/chat/api/search?query=secret&channel_id=1")
        .await;
    assert_eq!(ids(&body), vec![4]);
    // A channel the member can't see is not found.
    let (status, _) = c.get("/chat/api/search?query=secret&channel_id=1").await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn filters_narrow_to_a_user_or_a_channel() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    let mut c = Client::logged_in(st, "user1").await;
    let (_, body) = c
        .get("/chat/api/search?query=brown%20%40user0&sort=latest")
        .await;
    assert_eq!(ids(&body), vec![1]);
    let (_, body) = c.get("/chat/api/search?query=%40me&sort=latest").await;
    assert_eq!(ids(&body), vec![2]);
    let (_, body) = c.get("/chat/api/search?query=%40nobody").await;
    assert_eq!(ids(&body), Vec::<i64>::new());
    let (_, body) = c
        .get("/chat/api/search?query=%23general%20brown&sort=latest")
        .await;
    assert_eq!(ids(&body), vec![2, 1]);
    let (_, body) = c.get("/chat/api/search?query=%23staff").await;
    assert_eq!(ids(&body), Vec::<i64>::new());
}

#[tokio::test]
async fn results_page_and_the_contract_is_checked() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    messages(&db).await;
    let mut c = Client::logged_in(st.clone(), "user1").await;
    let (_, body) = c
        .get("/chat/api/search?query=brown&sort=latest&limit=1")
        .await;
    assert_eq!(ids(&body), vec![2]);
    assert_eq!(body["meta"]["has_more"], true);
    let (_, body) = c
        .get("/chat/api/search?query=brown&sort=latest&limit=1&offset=1")
        .await;
    assert_eq!(ids(&body), vec![1]);
    assert_eq!(body["meta"]["has_more"], false);

    let (status, body) = c.get("/chat/api/search?limit=41&offset=-1&sort=x").await;
    assert_eq!(status, 400);
    assert_eq!(
        body["errors"],
        serde_json::json!([
            "Query can't be blank",
            "Limit must be in 1..40",
            "Offset must be greater than or equal to 0",
            "Sort is not included in the list"
        ])
    );

    set_setting(&db.pool, "chat_search_enabled", 5, "f").await;
    let (status, _) = c.get("/chat/api/search?query=brown").await;
    assert_eq!(status, 404);
}
