//! Chat message uploads: Chat::CreateMessage and Chat::UpdateMessage with
//! upload_ids (only the sender's own, while chat_allow_uploads), the
//! uploads in Chat::MessageSerializer and the excerpts of upload-only
//! messages, and the channel page drawing them (ChatMessageCollapser,
//! ChatUpload) and a draft's (ChatComposerUploads).

mod common;

use axum::body::Body;
use axum::http::{Method, Request, header};
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
    async fn send(&mut self, method: Method, path: &str, body: &str) -> (u16, String) {
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

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client.send(Method::GET, "/session/csrf.json", "").await;
        let csrf: Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
            .send(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
            )
            .await;
        client
    }

    async fn page(&mut self, path: &str) -> String {
        let cookie: Vec<String> = self
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let request = Request::get(path)
            .header(header::HOST, "test.localhost")
            .header(header::COOKIE, cookie.join("; "));
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// An upload of `user_id`'s (user_uploads), named `name`. Returns its id.
async fn upload(db: &TestDb, user_id: i32, name: &str, width: i32, height: i32) -> i64 {
    let n: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(id), 0)::int8 + 1 FROM uploads")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let sha1 = format!("{:040x}", 0xc4a7_0000 + n);
    let ext = name.rsplit('.').next().unwrap();
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO uploads (user_id, original_filename, filesize, width, height, thumbnail_width, \
           thumbnail_height, url, sha1, extension, created_at, updated_at) \
         VALUES ($1, $2, 2048, $3, $4, $3, $4, $5, $6, $7, now(), now()) RETURNING id",
    )
    .bind(user_id)
    .bind(name)
    .bind(width)
    .bind(height)
    .bind(format!("/uploads/default/original/1X/{sha1}.{ext}"))
    .bind(&sha1)
    .bind(ext)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO user_uploads (user_id, upload_id, created_at) VALUES ($1, $2, now())")
        .bind(user_id)
        .bind(id)
        .execute(&db.pool)
        .await
        .unwrap();
    i64::from(id)
}

async fn references(db: &TestDb, message_id: i64) -> Vec<i64> {
    sqlx::query_scalar(
        "SELECT upload_id FROM upload_references WHERE target_type = 'ChatMessage' AND target_id = $1 ORDER BY id",
    )
    .bind(message_id)
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

async fn messages(client: &mut Client) -> Vec<Value> {
    let (status, body) = client
        .send(Method::GET, "/chat/api/channels/2/messages.json", "")
        .await;
    assert_eq!(status, 200, "{body}");
    let json: Value = serde_json::from_str(&body).unwrap();
    json["messages"].as_array().unwrap().clone()
}

#[tokio::test]
async fn an_upload_only_message_is_sent_with_its_uploads() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let cat = upload(&db, 3, "cat.png", 1380, 500).await;
    let notes = upload(&db, 3, "notes.pdf", 0, 0).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let (status, body) = user1
        .send(
            Method::POST,
            "/chat/2",
            &format!("message=&upload_ids%5B%5D={cat}&upload_ids%5B%5D={notes}&staged_id=x"),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let id = serde_json::from_str::<Value>(&body).unwrap()["message_id"]
        .as_i64()
        .unwrap();
    assert_eq!(references(&db, id).await, vec![cat, notes]);
    let excerpt: Option<String> =
        sqlx::query_scalar("SELECT excerpt FROM chat_messages WHERE id = $1")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(excerpt.as_deref(), Some("cat.png"));

    let list = messages(&mut user1).await;
    let m = list.iter().find(|m| m["id"] == id).unwrap();
    assert_eq!(m["message"], "");
    assert_eq!(m["excerpt"], "cat.png");
    let uploads = m["uploads"].as_array().unwrap();
    assert_eq!(uploads.len(), 2);
    assert_eq!(uploads[0]["id"], cat);
    assert_eq!(uploads[0]["original_filename"], "cat.png");
    assert_eq!(uploads[0]["width"], 1380);
    assert_eq!(uploads[0]["human_filesize"], "2 KB");
    assert!(
        uploads[0]["short_url"]
            .as_str()
            .unwrap()
            .starts_with("upload://")
    );
    assert_eq!(uploads[1]["id"], notes);

    // The channel page draws them behind the collapser.
    let page = user1.page("/chat/c/general/2").await;
    assert!(
        page.contains("<div class=\"chat-message-collapser\"><div class=\"chat-cooked\"></div><div class=\"chat-message-collapser-header\"><span class=\"chat-message-collapser-link-small\">2 files</span>"),
        "{page}"
    );
    assert!(page.contains(
        "<img class=\"chat-img-upload lightbox\" data-download-href=\"/uploads/short-url/"
    ));
    assert!(page.contains("height=\"250\" loading=\"lazy\""));
    assert!(page.contains("<a class=\"chat-other-upload\" data-orig-href=\"upload://"));
    assert!(page.contains(">notes.pdf</a>"));
}

#[tokio::test]
async fn only_the_senders_own_uploads_attach() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let theirs = upload(&db, 2, "theirs.png", 10, 10).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    // Someone else's upload is no upload, and an empty message too short.
    let (status, body) = user1
        .send(
            Method::POST,
            "/chat/2",
            &format!("message=&upload_ids%5B%5D={theirs}"),
        )
        .await;
    assert_eq!(status, 422, "{body}");
    assert!(body.contains("too short"), "{body}");
    // With text the message goes, without it.
    let (status, body) = user1
        .send(
            Method::POST,
            "/chat/2",
            &format!("message=hello+there&upload_ids%5B%5D={theirs}"),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let id = serde_json::from_str::<Value>(&body).unwrap()["message_id"]
        .as_i64()
        .unwrap();
    assert!(references(&db, id).await.is_empty());

    // chat_allow_uploads off: none attach.
    set_setting(&db.pool, "chat_allow_uploads", 5, "f").await;
    let mine = upload(&db, 3, "mine.png", 10, 10).await;
    let (status, body) = user1
        .send(
            Method::POST,
            "/chat/2",
            &format!("message=&upload_ids%5B%5D={mine}"),
        )
        .await;
    assert_eq!(status, 422, "{body}");
}

#[tokio::test]
async fn an_edit_replaces_the_uploads_when_all_are_the_editors() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let a = upload(&db, 3, "a.png", 10, 10).await;
    let b = upload(&db, 3, "b.png", 10, 10).await;
    let theirs = upload(&db, 2, "theirs.png", 10, 10).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let (_, body) = user1
        .send(
            Method::POST,
            "/chat/2",
            &format!("message=with+files&upload_ids%5B%5D={a}"),
        )
        .await;
    let id = serde_json::from_str::<Value>(&body).unwrap()["message_id"]
        .as_i64()
        .unwrap();
    let a_ref: i64 = sqlx::query_scalar(
        "SELECT id FROM upload_references WHERE target_type = 'ChatMessage' AND target_id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap();

    // One id not theirs: the uploads stay as they were.
    let (status, body) = user1
        .send(
            Method::PUT,
            &format!("/chat/api/channels/2/messages/{id}"),
            &format!("message=with+files&upload_ids%5B%5D={b}&upload_ids%5B%5D={theirs}"),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(references(&db, id).await, vec![a]);

    // Both theirs: b added, a's reference kept.
    let (status, body) = user1
        .send(
            Method::PUT,
            &format!("/chat/api/channels/2/messages/{id}"),
            &format!("message=with+files&upload_ids%5B%5D={a}&upload_ids%5B%5D={b}"),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(references(&db, id).await, vec![a, b]);
    let kept: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM upload_references WHERE id = $1)")
            .bind(a_ref)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(kept);

    // The text emptied, b alone: an upload-only message, named for it.
    let (status, body) = user1
        .send(
            Method::PUT,
            &format!("/chat/api/channels/2/messages/{id}"),
            &format!("message=&upload_ids%5B%5D={b}"),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(references(&db, id).await, vec![b]);
    let list = messages(&mut user1).await;
    let m = list.iter().find(|m| m["id"] == id).unwrap();
    assert_eq!(m["excerpt"], "b.png");

    // No upload_ids: every upload is taken off.
    let (status, _) = user1
        .send(
            Method::PUT,
            &format!("/chat/api/channels/2/messages/{id}"),
            "message=back+to+text",
        )
        .await;
    assert_eq!(status, 200);
    assert!(references(&db, id).await.is_empty());
}

#[tokio::test]
async fn a_draft_with_uploads_is_restored_in_the_composer() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let draft = serde_json::json!({
        "message": "see attached",
        "uploads": [
            {"id": 41, "original_filename": "cat.png", "extension": "png", "url": "/uploads/default/original/1X/cat.png"},
            {"id": 42, "original_filename": "report.pdf", "extension": "pdf", "url": "/uploads/default/original/1X/report.pdf"}
        ]
    });
    sqlx::query(
        "INSERT INTO chat_drafts (user_id, chat_channel_id, data, created_at, updated_at) VALUES (3, 2, $1, now(), now())",
    )
    .bind(draft.to_string())
    .execute(&db.pool)
    .await
    .unwrap();
    let mut user1 = Client::logged_in(st, "user1").await;
    let page = user1.page("/chat/c/general/2").await;
    assert!(page.contains(">see attached</textarea>"), "{page}");
    assert!(page.contains("<div class=\"chat-composer-uploads-container\"><div class=\"chat-composer-upload chat-composer-upload--image\" data-id=\"41\"><div class=\"preview\"><img class=\"preview-img\" src=\"/uploads/default/original/1X/cat.png\"></div><span class=\"data\"><div class=\"bottom-data\"></div></span><button class=\"btn no-text btn-icon btn-flat chat-composer-upload__remove-btn\" title=\"Remove file\" type=\"button\">"));
    assert!(page.contains("<div class=\"top-data\"><span class=\"file-name\">report.pdf</span></div><div class=\"bottom-data\"><span class=\"extension-pill\">pdf</span></div>"));
}

#[tokio::test(flavor = "multi_thread")]
async fn processing_a_message_with_small_uploads_makes_no_thumbnails() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let small = upload(&db, 3, "small.png", 100, 100).await;
    sqlx::query(
        "INSERT INTO chat_messages (id, chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         VALUES (900, 2, 3, now(), now(), '', '', 1, 3)",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO upload_references (upload_id, target_type, target_id, created_at, updated_at) VALUES ($1, 'ChatMessage', 900, now(), now())",
    )
    .bind(small)
    .execute(&db.pool)
    .await
    .unwrap();
    let mut conn = st.pool.acquire().await.unwrap();
    let settings = discourse_rs::site_settings::SiteSettings::load(
        &mut conn,
        &st.site_setting_defs,
        &st.config.globals,
    )
    .await
    .unwrap();
    discourse_rs::plugins::chat::create::process_message(
        &st,
        &mut conn,
        &settings,
        &serde_json::json!({ "chat_message_id": 900 }),
    )
    .await
    .unwrap();
    let thumbnails: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM optimized_images WHERE upload_id = $1")
            .bind(small)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(thumbnails, 0);
}
