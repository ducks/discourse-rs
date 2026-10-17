//! Direct uploads end to end, where the write cases stop (the recorder
//! cannot be the browser that puts the file): on MinIO, a presigned PUT
//! sent with its signed headers, then completed as an upload; a multipart
//! upload's parts put at their presigned URLs, then completed as an
//! upload and, for an admin, as a backup. Skipped without MinIO.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, header};
use common::s3::S3Server;
use common::{TestDb, recorded_config, state};
use discourse_rs::AppState;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

/// Logged in as a seeded user, with its cookies and CSRF token.
struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn send(&mut self, method: Method, path: &str, body: Option<&Value>) -> (u16, Value) {
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
            let v = v.to_str().unwrap_or("");
            let (pair, _) = v.split_once(';').unwrap_or((v, ""));
            if let Some((name, value)) = pair.split_once('=') {
                self.cookies.retain(|(n, _)| n != name);
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn login(state: AppState, user: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client.send(Method::GET, "/session/csrf.json", None).await;
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        let (status, body) = client
            .send(
                Method::POST,
                "/session.json",
                Some(&json!({"login": user, "password": "password"})),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        let (_, csrf) = client.send(Method::GET, "/session/csrf.json", None).await;
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
    }

    async fn post(&mut self, path: &str, body: Value) -> (u16, Value) {
        self.send(Method::POST, path, Some(&body)).await
    }
}

/// A site with uploads (and backups) on the server, direct uploads on.
async fn site(server: &S3Server, db: &TestDb) -> AppState {
    let mut config = recorded_config();
    config.object_storage_endpoints = vec![server.endpoint_mapping()];
    let st = state(db.pool.clone(), config).await;
    let mut settings = S3Server::upload_settings("direct-uploads");
    settings.extend([
        ("enable_direct_s3_uploads", "t".to_string()),
        ("s3_backup_bucket", "direct-backups".to_string()),
        ("backup_location", "s3".to_string()),
    ]);
    for (name, value) in settings {
        let data_type = st.site_setting_defs.get(name).unwrap().data_type as i32;
        common::set_setting(&db.pool, name, data_type, &value).await;
    }
    st
}

/// What the browser does with a presigned URL: a PUT with the headers
/// it was signed with; the part's etag.
async fn put(url: &str, headers: &serde_json::Map<String, Value>, body: Vec<u8>) -> String {
    let mut request = reqwest::Client::new().put(url).body(body);
    for (name, value) in headers {
        request = request.header(name, value.as_str().unwrap());
    }
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), 200, "{url}");
    response.headers()["etag"]
        .to_str()
        .unwrap()
        .replace('"', "")
}

fn gif() -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/writes/files/static.gif"),
    )
    .unwrap()
}

#[tokio::test]
async fn a_presigned_put_becomes_an_upload() {
    if let Err(why) = S3Server::available() {
        eprintln!("skipped: {why}");
        return;
    }
    let server = S3Server::start().unwrap();
    server
        .bucket("direct-uploads")
        .ensure_bucket()
        .await
        .unwrap();
    let db = TestDb::new().await;
    let st = site(&server, &db).await;
    let mut client = Client::login(st, "user1").await;
    let bytes = gif();
    let sha1 = {
        use sha1::Digest;
        format!("{:x}", sha1::Sha1::digest(&bytes))
    };
    let (status, put_url) = client
        .post(
            "/uploads/generate-presigned-put.json",
            json!({"file_name": "static.gif", "file_size": bytes.len(), "type": "composer",
                   "metadata": {"sha1-checksum": sha1}}),
        )
        .await;
    assert_eq!(status, 200, "{put_url}");
    // The URL names the configured endpoint; the browser here reaches the
    // test's own port.
    let url = put_url["url"].as_str().unwrap().replace(
        "minio.localhost:9122",
        server.endpoint.trim_start_matches("http://"),
    );
    put(
        &url,
        put_url["signed_headers"].as_object().unwrap(),
        bytes.clone(),
    )
    .await;
    let (status, upload) = client
        .post(
            "/uploads/complete-external-upload.json",
            json!({"unique_identifier": put_url["unique_identifier"]}),
        )
        .await;
    assert_eq!(status, 200, "{upload}");
    assert_eq!(
        upload["url"],
        format!("//direct-uploads.minio.localhost:9122/original/1X/{sha1}.gif")
    );
    // Moved, not copied: the temporary object is gone.
    let bucket = server.bucket("direct-uploads");
    let keys: Vec<String> = bucket
        .list("")
        .await
        .unwrap()
        .into_iter()
        .map(|o| o.key)
        .collect();
    assert_eq!(keys, [format!("original/1X/{sha1}.gif")]);
    let meta = bucket
        .head(&format!("original/1X/{sha1}.gif"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta.content_type.as_deref(), Some("image/gif"));
    assert_eq!(
        meta.content_disposition.as_deref(),
        Some("inline; filename=\"static.gif\"; filename*=UTF-8''static.gif")
    );
}

#[tokio::test]
async fn multipart_uploads_become_an_upload_and_a_backup() {
    if let Err(why) = S3Server::available() {
        eprintln!("skipped: {why}");
        return;
    }
    let server = S3Server::start().unwrap();
    for bucket in ["direct-uploads", "direct-backups"] {
        server.bucket(bucket).ensure_bucket().await.unwrap();
    }
    let db = TestDb::new().await;
    let st = site(&server, &db).await;
    let mut client = Client::login(st, "admin").await;
    let host = server.endpoint.trim_start_matches("http://").to_string();

    // Parts of 5 MB and the rest, as Uppy sends them.
    let mut body = vec![b'x'; 5 * 1024 * 1024];
    body.extend_from_slice(b"the end");
    for (path, file_name, upload_type) in [
        ("/uploads", "big.txt", "composer"),
        ("/admin/backups", "uploaded-backup.tar.gz", "backup"),
    ] {
        let (status, created) = client
            .post(
                &format!("{path}/create-multipart.json"),
                json!({"file_name": file_name, "file_size": body.len(), "upload_type": upload_type}),
            )
            .await;
        assert_eq!(status, 200, "{created}");
        let identifier = created["unique_identifier"].clone();
        let (status, presigned) = client
            .post(
                &format!("{path}/batch-presign-multipart-parts.json"),
                json!({"part_numbers": [1, 2], "unique_identifier": identifier}),
            )
            .await;
        assert_eq!(status, 200, "{presigned}");
        let mut parts = Vec::new();
        for (n, chunk) in [(1, &body[..5 * 1024 * 1024]), (2, &body[5 * 1024 * 1024..])] {
            let url = presigned["presigned_urls"][n.to_string()]
                .as_str()
                .unwrap()
                .replace("minio.localhost:9122", &host);
            let etag = put(&url, &Default::default(), chunk.to_vec()).await;
            parts.push(json!({"part_number": n, "etag": etag}));
        }
        let (status, completed) = client
            .post(
                &format!("{path}/complete-multipart.json"),
                json!({"unique_identifier": identifier, "parts": parts}),
            )
            .await;
        if upload_type == "backup" {
            assert_eq!((status, &completed), (200, &json!({})));
        } else {
            // big.txt is no authorized extension: the upload's errors, the
            // stub kept.
            assert_eq!(status, 422, "{completed}");
        }
    }
    let backups: Vec<String> = server
        .bucket("direct-backups")
        .list("")
        .await
        .unwrap()
        .into_iter()
        .map(|o| o.key)
        .collect();
    assert_eq!(backups, ["default/uploaded-backup.tar.gz"]);
    let (status, files) = client.send(Method::GET, "/admin/backups.json", None).await;
    assert_eq!(status, 200);
    assert_eq!(files[0]["filename"], "uploaded-backup.tar.gz");
    assert_eq!(files[0]["size"], body.len());
}

/// UserAvatarsController#show_in_site on an external store: the
/// thumbnail made from the object and stored beside it, then served
/// (proxy_avatar) with the upload's date.
#[tokio::test]
async fn avatars_on_s3_are_made_and_proxied() {
    if let Err(why) = S3Server::available() {
        eprintln!("skipped: {why}");
        return;
    }
    let server = S3Server::start().unwrap();
    server
        .bucket("direct-uploads")
        .ensure_bucket()
        .await
        .unwrap();
    let db = TestDb::new().await;
    let st = site(&server, &db).await;
    let mut client = Client::login(st.clone(), "user1").await;
    let bytes = gif();
    let (status, put_url) = client
        .post(
            "/uploads/generate-presigned-put.json",
            json!({"file_name": "me.gif", "file_size": bytes.len(), "type": "avatar"}),
        )
        .await;
    assert_eq!(status, 200, "{put_url}");
    let url = put_url["url"].as_str().unwrap().replace(
        "minio.localhost:9122",
        server.endpoint.trim_start_matches("http://"),
    );
    put(&url, put_url["signed_headers"].as_object().unwrap(), bytes).await;
    let (status, upload) = client
        .post(
            "/uploads/complete-external-upload.json",
            json!({"unique_identifier": put_url["unique_identifier"]}),
        )
        .await;
    assert_eq!(status, 200, "{upload}");
    let id = upload["id"].as_i64().unwrap();
    sqlx::query("UPDATE users SET uploaded_avatar_id = $1 WHERE username = 'user1'")
        .bind(id as i32)
        .execute(&db.pool)
        .await
        .unwrap();

    let request = Request::builder()
        .uri(format!("/user_avatar/test.localhost/user1/48/{id}_2.png"))
        .header(header::HOST, "test.localhost")
        .body(Body::empty())
        .unwrap();
    let response = discourse_rs::app(st).oneshot(request).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "image/gif");
    assert!(response.headers().contains_key("last-modified"));
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(body.starts_with(b"GIF8"));
    let keys: Vec<String> = server
        .bucket("direct-uploads")
        .list("optimized/")
        .await
        .unwrap()
        .into_iter()
        .map(|o| o.key)
        .collect();
    assert_eq!(keys.len(), 1, "{keys:?}");
    assert!(keys[0].ends_with("_2_48x48.gif"), "{keys:?}");
}
