//! The demo script, as an end-to-end smoke test: the core forum, chat and
//! the admin, each step driven through the app as a browser and its
//! scripts drive it. Pages are loaded as a person loads them (HTML, no
//! XHR header, redirects followed); actions go to the JSON and htmx
//! endpoints the page's scripts call. Every step asserts a 2xx (or the
//! right redirect), that nothing answered "not ported yet", and for pages
//! that the element Ember draws for it is there. Steps that write let the
//! job worker run (`settle`) and fail on a job it refused, as a running
//! server would leave it failed.
//!
//! A step the port still refuses is ignored with the ROADMAP item that
//! fixes it; that item's branch removes the ignore. ROADMAP's "Before the
//! demo" section lists them. The seeded users' password is "password":
//! user0 is trust level 1, user1 and user2 trust level 2, admin an admin.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

/// A topic in the seed with replies by several members.
const TOPIC: i32 = 35;
const TOPIC_PATH: &str = "/t/parity-fixture-replies-and-posters/35";
/// The seed's General category.
const CATEGORY: i32 = 4;
/// The seed's General chat channel, open to everyone.
const CHANNEL: i32 = 2;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

struct Reply {
    status: StatusCode,
    location: Option<String>,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

/// How a request is sent: as a page load, or as the page's scripts send
/// it (XHR, with the CSRF token).
#[derive(Clone, Copy, PartialEq)]
enum As {
    Page,
    Script,
}

impl Client {
    fn new(state: AppState) -> Client {
        Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        }
    }

    async fn send(
        &mut self,
        method: Method,
        path: &str,
        how: As,
        content_type: Option<&str>,
        body: Vec<u8>,
    ) -> Reply {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        request = match how {
            As::Page => request.header(
                header::ACCEPT,
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            ),
            As::Script => request.header("x-requested-with", "XMLHttpRequest").header(
                header::ACCEPT,
                "application/json, text/javascript, */*; q=0.01",
            ),
        };
        if !self.cookies.is_empty() {
            let cookie: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookie.join("; "));
        }
        if let Some(token) = self.csrf.as_ref().filter(|_| how == As::Script) {
            request = request.header("x-csrf-token", token.as_str());
        }
        if let Some(content_type) = content_type {
            request = request.header(header::CONTENT_TYPE, content_type);
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap_or("");
            let (pair, attrs) = v.split_once(';').unwrap_or((v, ""));
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            self.cookies.retain(|(n, _)| n != name);
            if !attrs.to_ascii_lowercase().contains("max-age=0") && !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        // The work Rails defers past the response (TopicUser.track_visit!
        // among it) is done before the browser's next request.
        discourse_rs::request_tracking::drain().await;
        Reply {
            status,
            location,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    /// A page load, following redirects as a browser does.
    async fn page(&mut self, path: &str) -> Reply {
        let mut path = path.to_string();
        for _ in 0..5 {
            let reply = self
                .send(Method::GET, &path, As::Page, None, Vec::new())
                .await;
            match &reply.location {
                Some(to) if reply.status.is_redirection() => {
                    path = to
                        .strip_prefix("http://test.localhost")
                        .unwrap_or(to)
                        .to_string();
                }
                _ => return reply,
            }
        }
        panic!("{path}: too many redirects");
    }

    /// A page load that stops at the first answer.
    async fn page_once(&mut self, path: &str) -> Reply {
        self.send(Method::GET, path, As::Page, None, Vec::new())
            .await
    }

    async fn get(&mut self, path: &str) -> Reply {
        self.send(Method::GET, path, As::Script, None, Vec::new())
            .await
    }

    async fn form(&mut self, method: Method, path: &str, form: &[(&str, &str)]) -> Reply {
        let body = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        let content_type = (!body.is_empty()).then_some("application/x-www-form-urlencoded");
        self.send(method, path, As::Script, content_type, body.into_bytes())
            .await
    }

    async fn post(&mut self, path: &str, form: &[(&str, &str)]) -> Reply {
        self.form(Method::POST, path, form).await
    }

    async fn put(&mut self, path: &str, form: &[(&str, &str)]) -> Reply {
        self.form(Method::PUT, path, form).await
    }

    async fn delete(&mut self, path: &str) -> Reply {
        self.form(Method::DELETE, path, &[]).await
    }

    /// A multipart form with one file from parity/writes/files, as the
    /// composer's uploader sends it.
    async fn upload(&mut self, path: &str, fields: &[(&str, &str)], file: &str) -> Reply {
        let boundary = "----demo-smoke-boundary";
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("parity/writes/files")
                .join(file),
        )
        .unwrap();
        let mut body = Vec::new();
        for (name, value) in fields {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
                )
                .as_bytes(),
            );
        }
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file}\"\r\n\
                 Content-Type: image/png\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend(bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let content_type = format!("multipart/form-data; boundary={boundary}");
        self.send(Method::POST, path, As::Script, Some(&content_type), body)
            .await
    }

    async fn csrf(&mut self) {
        let reply = self.get("/session/csrf.json").await;
        self.csrf = reply.json()["csrf"].as_str().map(str::to_string);
    }

    async fn login(&mut self, login: &str, password: &str) {
        self.csrf().await;
        let reply = self
            .post("/session", &[("login", login), ("password", password)])
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert!(reply.json()["error"].is_null(), "{}", reply.body);
    }
}

async fn app() -> (TestDb, AppState) {
    // A 500's cause is only logged: show it with the failing step.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::ERROR)
        .with_test_writer()
        .try_init();
    let db = TestDb::new().await;
    let state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    (db, state)
}

async fn member(state: &AppState, username: &str) -> Client {
    let mut client = Client::new(state.clone());
    client.login(username, "password").await;
    client
}

async fn anonymous(state: &AppState) -> Client {
    let mut client = Client::new(state.clone());
    client.csrf().await;
    client
}

fn excerpt(body: &str) -> &str {
    let end = body.char_indices().nth(1500).map_or(body.len(), |(i, _)| i);
    &body[..end]
}

/// Answered, and not with a "not ported yet" refusal.
#[track_caller]
fn ok(what: &str, reply: &Reply) {
    assert!(
        reply.status.is_success(),
        "{what}: {} {}",
        reply.status,
        excerpt(&reply.body)
    );
    assert!(
        !reply.body.to_ascii_lowercase().contains("not ported yet"),
        "{what}: refused as not ported in place: {}",
        excerpt(&reply.body)
    );
}

/// A page answered with the element Ember draws for it.
#[track_caller]
fn drawn(what: &str, reply: &Reply, element: &str) {
    ok(what, reply);
    assert!(
        reply.body.contains("<!DOCTYPE html>"),
        "{what}: not a page: {}",
        excerpt(&reply.body)
    );
    assert!(
        reply.body.contains(element),
        "{what}: no {element} in {}",
        excerpt(&reply.body)
    );
}

/// A redirect to `to`.
#[track_caller]
fn redirects(what: &str, reply: &Reply, to: &str) {
    assert!(
        reply.status.is_redirection(),
        "{what}: {} {}",
        reply.status,
        excerpt(&reply.body)
    );
    let location = reply.location.as_deref().unwrap_or("");
    assert!(
        location == to || location.ends_with(to),
        "{what}: to {location}, not {to}"
    );
}

/// The id of the post a POST /posts.json made.
#[track_caller]
fn created_post(reply: &Reply) -> i64 {
    ok("POST /posts.json", reply);
    let json = reply.json();
    json["post"]["id"]
        .as_i64()
        .or_else(|| json["id"].as_i64())
        .unwrap_or_else(|| panic!("no post id in {}", reply.body))
}

async fn reply_to(client: &mut Client, topic: i64, raw: &str) -> Value {
    let topic = topic.to_string();
    let reply = client
        .post("/posts.json", &[("topic_id", &topic), ("raw", raw)])
        .await;
    created_post(&reply);
    reply.json()
}

async fn user_id(db: &TestDb, username: &str) -> i32 {
    sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
        .bind(username)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

/// The job worker's turn: runs what the requests enqueued (post alerts,
/// notifications, chat processing, hotlinked images, link titles), and
/// fails the step on a job that was refused or failed.
async fn settle(db: &TestDb, state: &AppState) {
    discourse_rs::jobs::drain(state).await.unwrap();
    let failed: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT name, last_error, args::text FROM discourse_rs.jobs WHERE last_error IS NOT NULL",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert!(failed.is_empty(), "jobs failed: {failed:?}");
}

/// Another site for a post to link, kept off the internet: `html` at
/// every path of http://fixtures.localhost:<port> (loopback, both
/// families), which FinalDestination reaches as an allowed internal host
/// on the S3 endpoint's port. Its base url.
async fn linked_site(state: &AppState, html: &'static str) -> String {
    let app = axum::Router::new().fallback(move || async move {
        ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html)
    });
    let (v4, v6) = loop {
        let v4 = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = v4.local_addr().unwrap().port();
        if let Ok(v6) = tokio::net::TcpListener::bind(("::1", port)).await {
            break (v4, v6);
        }
    };
    let base = format!(
        "http://fixtures.localhost:{}",
        v4.local_addr().unwrap().port()
    );
    for listener in [v4, v6] {
        let app = app.clone();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    }
    for (name, value) in [
        ("allowed_internal_hosts", "fixtures.localhost"),
        ("s3_endpoint", base.as_str()),
    ] {
        let data_type = state.site_setting_defs.get(name).unwrap().data_type as i32;
        common::set_setting(&state.pool, name, data_type, value).await;
    }
    base
}

/// A post in the fixture topic by someone other than `not_by`, which
/// `not_by` hasn't bookmarked.
async fn post_by_another(db: &TestDb, not_by: &str) -> i32 {
    sqlx::query_scalar(
        "SELECT p.id FROM posts p JOIN users u ON u.id = p.user_id \
         WHERE p.topic_id = $1 AND p.deleted_at IS NULL AND u.username <> $2 AND u.id > 0 \
           AND NOT EXISTS (SELECT 1 FROM bookmarks b JOIN users v ON v.id = b.user_id \
                           WHERE b.bookmarkable_type = 'Post' AND b.bookmarkable_id = p.id \
                             AND v.username = $2) \
         ORDER BY p.post_number DESC LIMIT 1",
    )
    .bind(TOPIC)
    .bind(not_by)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

// Core forum: the lists.

#[tokio::test]
async fn home_page_lists_latest() {
    let (_db, state) = app().await;
    let mut anon = anonymous(&state).await;
    for path in ["/", "/latest"] {
        let page = anon.page(path).await;
        drawn(path, &page, r#"<table class="topic-list"#);
    }
    let mut user1 = member(&state, "user1").await;
    let page = user1.page("/latest").await;
    drawn("/latest as user1", &page, r#"<table class="topic-list"#);
    let json = user1.get("/latest.json").await;
    ok("/latest.json", &json);
    assert!(json.json()["topic_list"]["topics"].is_array());
}

#[tokio::test]
async fn categories_page() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let page = user1.page("/categories").await;
    drawn("/categories", &page, r#"<table class="category-list"#);
    assert!(page.body.contains("General"));
}

#[tokio::test]
async fn category_page() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let page = user1.page(&format!("/c/general/{CATEGORY}")).await;
    drawn("/c/general/4", &page, r#"<table class="topic-list"#);
    assert!(page.body.contains("Parity fixture: replies and posters"));
}

#[tokio::test]
async fn top_page() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let page = user1.page("/top").await;
    // top_menu's default has no Top pill; the period chooser heads the list.
    drawn("/top", &page, r#"<div class="top-lists">"#);
    assert!(!page.body.contains("nav-item_top"));
}

#[tokio::test]
async fn new_and_unread_pages() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    for path in ["/new", "/unread"] {
        let page = user1.page(path).await;
        drawn(path, &page, "topic-list");
        let json = user1.get(&format!("{path}.json")).await;
        ok(&format!("{path}.json"), &json);
    }
}

// Core forum: a topic.

#[tokio::test]
async fn topic_page_anonymous() {
    let (_db, state) = app().await;
    let mut anon = anonymous(&state).await;
    let page = anon.page(TOPIC_PATH).await;
    drawn(TOPIC_PATH, &page, r#"id="post_1""#);
    // The bare id redirects to the slugged URL.
    let bare = anon.page_once(&format!("/t/{TOPIC}")).await;
    redirects("/t/35", &bare, TOPIC_PATH);
}

#[tokio::test]
async fn topic_page_member() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let page = user1.page(TOPIC_PATH).await;
    drawn(TOPIC_PATH, &page, r#"id="post_1""#);
    // The reply button the composer opens from.
    assert!(
        page.body.contains("post-action-menu__reply reply create"),
        "no reply button"
    );
    let json = user1.get(&format!("/t/{TOPIC}.json")).await;
    ok("/t/35.json", &json);
    assert!(json.json()["post_stream"]["posts"].is_array());
}

/// TopicsController#track_visit_to_topic: a member's page load (or the
/// JSON with track_visit) records their visit, TopicUser.track_visit!.
#[tokio::test]
async fn opening_a_topic_records_the_visit() {
    let (db, state) = app().await;
    let mut user2 = member(&state, "user2").await;
    let visited = |topic: i32| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM topic_users tu JOIN users u ON u.id = tu.user_id \
                 WHERE u.username = 'user2' AND tu.topic_id = $1 AND tu.last_visited_at IS NOT NULL)",
            )
            .bind(topic)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let topic = 37;
    assert!(!visited(topic).await, "user2 had visited topic {topic}");
    let page = user2.page("/t/parity-fixture-pinned-and-closed/37").await;
    drawn("/t/37", &page, r#"id="post_1""#);
    assert!(visited(topic).await, "the page load recorded no visit");
}

#[tokio::test]
async fn topic_page_staff() {
    let (_db, state) = app().await;
    let mut admin = member(&state, "admin").await;
    let page = admin.page(TOPIC_PATH).await;
    drawn(TOPIC_PATH, &page, r#"id="post_1""#);
    let json = admin.get(&format!("/t/{TOPIC}.json")).await;
    ok("/t/35.json as admin", &json);
}

/// post/small-action's buttons on the archive of topic 41: a member gets
/// none; staff give it custom text through the composer's save, then
/// delete it, see it deleted with recover, and recover it. Deleting and
/// recovering queue index_category_for_search (144) and
/// notify_mailing_list_subscribers (146) as Rails does, jobs not ported
/// yet, so those steps don't settle.
#[tokio::test]
async fn staff_manage_a_small_action() {
    let (db, state) = app().await;
    let path = "/t/parity-fixture-liked-and-archived/41";
    let edit = r#"<button class="btn no-text btn-icon btn-flat btn-small small-action-edit" title="edit this post" type="button">"#;
    let delete = r#"<button class="btn no-text btn-icon btn-flat btn-small btn-danger small-action-delete" title="delete this post" type="button">"#;
    let recover = r#"<button class="btn no-text btn-icon btn-flat btn-small small-action-recover" title="undelete this post" type="button">"#;

    let mut user1 = member(&state, "user1").await;
    let page = user1.page(path).await;
    drawn(path, &page, r#"<div class="small-action-buttons"></div>"#);

    let mut admin = member(&state, "admin").await;
    let page = admin.page(path).await;
    drawn(path, &page, edit);
    assert!(page.body.contains(delete), "no delete");

    let saved = admin
        .put(
            "/posts/47",
            &[
                ("post[raw]", "Archived until the next season."),
                ("post[topic_id]", "41"),
                ("post[original_text]", ""),
            ],
        )
        .await;
    ok("PUT /posts/47", &saved);
    settle(&db, &state).await;
    let page = admin.page(path).await;
    drawn(
        path,
        &page,
        "<div class=\"small-action-custom-message\"><div class=\"cooked\"><p>Archived until the next season.</p>",
    );

    let deleted = admin
        .form(Method::DELETE, "/posts/47", &[("context", path)])
        .await;
    ok("DELETE /posts/47", &deleted);
    let html = admin.get("/live/post/47").await;
    ok("/live/post/47 deleted", &html);
    assert!(
        html.body
            .contains(r#"class="small-action onscreen-post deleted""#)
    );
    assert!(html.body.contains(recover), "no recover: {}", html.body);
    assert!(!html.body.contains("small-action-edit"));
    assert!(!html.body.contains("small-action-delete"));

    ok(
        "PUT /posts/47/recover",
        &admin.put("/posts/47/recover", &[]).await,
    );
    let html = admin.get("/live/post/47").await;
    assert!(
        html.body.contains(edit) && html.body.contains(delete),
        "{}",
        html.body
    );
}

// Core forum: search and tags.

#[tokio::test]
async fn search_page() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let page = user1.page("/search?q=fixture").await;
    drawn("/search?q=fixture", &page, "search-results");
    assert!(page.body.contains("Parity fixture"));
}

#[tokio::test]
async fn search_menu() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let json = user1.get("/search/query.json?term=fixture").await;
    ok("/search/query.json", &json);
    assert!(
        json.json()["topics"]
            .as_array()
            .is_some_and(|t| !t.is_empty()),
        "{}",
        json.body
    );
    // The menu's results, drawn for search-menu.js.
    let menu = user1.get("/search-menu?term=fixture").await;
    ok("/search-menu", &menu);
    assert!(menu.json().is_object(), "{}", menu.body);
}

#[tokio::test]
async fn tags_page() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let page = user1.page("/tags").await;
    drawn(
        "/tags",
        &page,
        r#"<a href="/tag/1" class="discourse-tag simple">howto</a>"#,
    );
}

#[tokio::test]
async fn tag_page() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let page = user1.page("/tag/howto").await;
    drawn("/tag/howto", &page, "topic-list");
}

// Core forum: profiles.

#[tokio::test]
async fn user_profile_summary_and_activity() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let profile = user1.page("/u/user0").await;
    drawn("/u/user0", &profile, r#"<section class="user-main">"#);
    let summary = user1.page("/u/user0/summary").await;
    drawn("/u/user0/summary", &summary, "top-section stats-section");
    let activity = user1.page("/u/user0/activity").await;
    drawn(
        "/u/user0/activity",
        &activity,
        "post-list-item user-stream-item",
    );
    let card = user1.get("/u/user0/card.json").await;
    ok("/u/user0/card.json", &card);
}

#[tokio::test]
async fn edits_own_bio() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let page = user1.page("/u/user1/preferences/profile").await;
    drawn(
        "/u/user1/preferences/profile",
        &page,
        "control-group pref-bio",
    );
    let saved = user1
        .put(
            "/u/user1.json",
            &[("bio_raw", "Here for the demo, and **bold** about it.")],
        )
        .await;
    ok("PUT /u/user1.json bio_raw", &saved);
    let me = user1.get("/u/user1.json").await;
    ok("/u/user1.json", &me);
    assert!(
        me.json()["user"]["bio_cooked"]
            .as_str()
            .is_some_and(|b| b.contains("<strong>bold</strong>")),
        "{}",
        me.body
    );
}

#[tokio::test]
async fn chooses_an_avatar() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let page = user1.page("/u/user1/preferences/account").await;
    drawn(
        "/u/user1/preferences/account",
        &page,
        "control-group pref-avatar",
    );
    // An uploaded picture, then back to the letter avatar.
    let upload = user1
        .upload(
            "/uploads.json",
            &[("upload_type", "avatar"), ("synchronous", "true")],
            "small.png",
        )
        .await;
    ok("POST /uploads.json avatar", &upload);
    let id = upload.json()["id"].as_i64().unwrap().to_string();
    let custom = user1
        .put(
            "/u/user1/preferences/avatar/pick.json",
            &[("upload_id", &id), ("type", "uploaded")],
        )
        .await;
    ok("pick the uploaded avatar", &custom);
    let letter = user1
        .put(
            "/u/user1/preferences/avatar/pick.json",
            &[("type", "system")],
        )
        .await;
    ok("pick the letter avatar", &letter);
}

#[tokio::test]
async fn saves_preferences() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    for tab in ["emails", "notifications", "tracking", "interface"] {
        let path = format!("/u/user1/preferences/{tab}");
        let page = user1.page(&path).await;
        drawn(
            &path,
            &page,
            r#"<section class="user-content user-preferences""#,
        );
    }
    let saved = user1
        .put(
            "/u/user1.json",
            &[
                ("external_links_in_new_tab", "true"),
                ("email_level", "0"),
                ("new_topic_duration_minutes", "1440"),
            ],
        )
        .await;
    ok("PUT /u/user1.json preferences", &saved);
    let options = &saved.json()["user"]["user_option"];
    assert_eq!(options["external_links_in_new_tab"], true, "{}", saved.body);
}

// Core forum: posting with media and talking to people.

#[tokio::test]
async fn uploads_an_image_and_posts_it() {
    let (db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let upload = user1
        .upload("/uploads.json", &[("upload_type", "composer")], "small.png")
        .await;
    ok("POST /uploads.json", &upload);
    let json = upload.json();
    let short_url = json["short_url"].as_str().expect("an upload short url");
    let (width, height) = (
        json["width"].as_i64().unwrap(),
        json["height"].as_i64().unwrap(),
    );
    let raw = format!("Here is a picture from the demo.\n\n![small|{width}x{height}]({short_url})");
    let post = reply_to(&mut user1, TOPIC as i64, &raw).await;
    settle(&db, &state).await;
    assert!(
        post["cooked"].as_str().is_some_and(|c| c.contains("<img")),
        "{post}"
    );
    let page = user1.page(TOPIC_PATH).await;
    drawn(
        TOPIC_PATH,
        &page,
        &format!("data-post-id=\"{}\"", post["id"]),
    );
}

#[tokio::test]
async fn a_mention_notifies() {
    let (db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    reply_to(
        &mut user1,
        TOPIC as i64,
        "@user2 would you take a look at this one for the demo?",
    )
    .await;
    settle(&db, &state).await;
    let mut user2 = member(&state, "user2").await;
    let recent = user2.get("/notifications.json?recent=true&limit=30").await;
    ok("/notifications.json", &recent);
    assert!(
        recent.json()["notifications"]
            .as_array()
            .is_some_and(|n| n.iter().any(|n| n["notification_type"] == 1)),
        "no mention: {}",
        recent.body
    );
    // The user menu drawn for the header's avatar.
    let menu = user2.get("/user-menu").await;
    ok("/user-menu", &menu);
    assert!(
        menu.body.contains("notification unread mentioned"),
        "{}",
        excerpt(&menu.body)
    );
}

/// Every job a reply with a link and an image enqueues runs, none
/// refused, and the linked page's title is crawled.
#[tokio::test]
async fn posting_runs_its_background_jobs() {
    let (db, state) = app().await;
    let site = linked_site(
        &state,
        "<html><head><title>The slides</title></head></html>",
    )
    .await;
    let mut user1 = member(&state, "user1").await;
    let upload = user1
        .upload("/uploads.json", &[("upload_type", "composer")], "small.png")
        .await;
    ok("POST /uploads.json", &upload);
    let short_url = upload.json()["short_url"].as_str().unwrap().to_string();
    let raw = format!("The slides are at {site}/slides and\n\n![small]({short_url})");
    reply_to(&mut user1, TOPIC as i64, &raw).await;
    settle(&db, &state).await;
    let title: Option<String> = sqlx::query_scalar(
        "SELECT title FROM topic_links WHERE url = $1 AND crawled_at IS NOT NULL",
    )
    .bind(format!("{site}/slides"))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(title.as_deref(), Some("The slides"));
}

/// A reply that links another topic still alerts the people it mentions
/// (and the linked topic's author).
#[tokio::test]
async fn a_reply_linking_a_topic_notifies() {
    let (db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    reply_to(
        &mut user1,
        TOPIC as i64,
        "@user2 the agenda is in http://test.localhost/t/welcome-to-discourse/5 for the demo.",
    )
    .await;
    settle(&db, &state).await;
    let mut user2 = member(&state, "user2").await;
    let recent = user2.get("/notifications.json?recent=true&limit=30").await;
    ok("/notifications.json", &recent);
    assert!(
        recent.json()["notifications"]
            .as_array()
            .is_some_and(|n| n.iter().any(|n| n["notification_type"] == 1)),
        "no mention: {}",
        excerpt(&recent.body)
    );
}

#[tokio::test]
async fn a_quote_notifies() {
    let (db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let post = reply_to(
        &mut user1,
        TOPIC as i64,
        "Something worth quoting later on.",
    )
    .await;
    let number = post["post_number"].as_i64().unwrap();
    let mut user2 = member(&state, "user2").await;
    let raw = format!(
        "[quote=\"user1, post:{number}, topic:{TOPIC}\"]\nSomething worth quoting\n[/quote]\n\nAgreed."
    );
    reply_to(&mut user2, TOPIC as i64, &raw).await;
    settle(&db, &state).await;
    let quoted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id = $1 AND notification_type = 3",
    )
    .bind(user_id(&db, "user1").await)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(quoted, 1);
    let recent = user1.get("/notifications.json?recent=true&limit=30").await;
    ok("/notifications.json", &recent);
}

#[tokio::test]
async fn bookmarks_a_post() {
    let (db, state) = app().await;
    let post = post_by_another(&db, "user1").await.to_string();
    let mut user1 = member(&state, "user1").await;
    let created = user1
        .post(
            "/bookmarks.json",
            &[("bookmarkable_id", &post), ("bookmarkable_type", "Post")],
        )
        .await;
    ok("POST /bookmarks.json", &created);
    let id = created.json()["id"].as_i64().expect("a bookmark id");
    let list = user1.get("/u/user1/bookmarks.json").await;
    ok("/u/user1/bookmarks.json", &list);
    assert!(list.body.contains(&format!("\"id\":{id}")), "{}", list.body);
    let page = user1.page("/u/user1/activity/bookmarks").await;
    drawn(
        "/u/user1/activity/bookmarks",
        &page,
        "topic-list bookmark-list",
    );
    let menu = user1.get("/u/user1/user-menu-bookmarks.json").await;
    ok("/u/user1/user-menu-bookmarks.json", &menu);
    let deleted = user1.delete(&format!("/bookmarks/{id}.json")).await;
    ok("DELETE /bookmarks/:id.json", &deleted);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM bookmarks WHERE id = $1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn private_message_round_trip() {
    let (db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let sent = user1
        .post(
            "/posts.json",
            &[
                ("title", "Planning the demo together, privately"),
                ("raw", "Can we go over the demo script before the day?"),
                ("archetype", "private_message"),
                ("target_recipients", "user2"),
            ],
        )
        .await;
    created_post(&sent);
    let topic = sent.json()["topic_id"].as_i64().expect("a topic id");
    settle(&db, &state).await;

    let mut user2 = member(&state, "user2").await;
    // The new message's notification.
    let recent = user2.get("/notifications.json?recent=true&limit=30").await;
    ok("/notifications.json", &recent);
    assert!(
        recent.body.contains(&format!("\"topic_id\":{topic}")),
        "{}",
        excerpt(&recent.body)
    );
    let inbox = user2.page("/u/user2/messages").await;
    drawn("/u/user2/messages", &inbox, "Planning the demo together");
    let list = user2.get("/topics/private-messages/user2.json").await;
    ok("/topics/private-messages/user2.json", &list);
    assert!(list.body.contains(&format!("\"id\":{topic}")));
    let opened = user2.page(&format!("/t/{topic}")).await;
    drawn("the message", &opened, r#"id="post_1""#);
    reply_to(&mut user2, topic, "Yes, let us run through it on Thursday.").await;
    settle(&db, &state).await;
}

#[tokio::test]
async fn reading_clears_unread() {
    let (db, state) = app().await;
    // user2 starts a topic; user1 reads it long enough to track it.
    let mut user2 = member(&state, "user2").await;
    let category = CATEGORY.to_string();
    let started = user2
        .post(
            "/posts.json",
            &[
                ("title", "What everyone thought of the demo day"),
                ("raw", "Tell us how the demo went for you, all of it."),
                ("category", &category),
            ],
        )
        .await;
    created_post(&started);
    let topic = started.json()["topic_id"].as_i64().unwrap();
    let topic_s = topic.to_string();
    let mut user1 = member(&state, "user1").await;
    user1.page(&format!("/t/{topic}")).await;
    // The screen track's flushes as the reader stays, a minute each (the
    // most a batch counts) until they pass auto_track_topics_after_msecs
    // (5 minutes) and the topic is tracked. Seven: a first read that
    // makes the topic user counts no time.
    for msecs in ["60000"; 7] {
        let read = user1
            .post(
                "/topics/timings",
                &[
                    ("topic_id", &topic_s),
                    ("topic_time", msecs),
                    ("timings[1]", msecs),
                ],
            )
            .await;
        ok("POST /topics/timings", &read);
    }

    // A reply makes it unread for user1.
    reply_to(
        &mut user2,
        topic,
        "Adding a reply so there is more to read.",
    )
    .await;
    settle(&db, &state).await;
    let unread = user1.get("/unread.json").await;
    ok("/unread.json", &unread);
    let listed = |r: &Reply| {
        r.json()["topic_list"]["topics"]
            .as_array()
            .is_some_and(|t| t.iter().any(|t| t["id"] == topic))
    };
    let tracked: Option<(i32, Option<i32>, i32)> = sqlx::query_as(
        "SELECT notification_level, last_read_post_number, total_msecs_viewed FROM topic_users \
         WHERE topic_id = $1 AND user_id = $2",
    )
    .bind(topic as i32)
    .bind(user_id(&db, "user1").await)
    .fetch_optional(&db.pool)
    .await
    .unwrap();
    assert!(
        listed(&unread),
        "not unread (topic_users: {tracked:?}): {}",
        excerpt(&unread.body)
    );

    let read = user1
        .post(
            "/topics/timings",
            &[
                ("topic_id", &topic_s),
                ("topic_time", "5000"),
                ("timings[2]", "5000"),
            ],
        )
        .await;
    ok("POST /topics/timings", &read);
    let unread = user1.get("/unread.json").await;
    ok("/unread.json", &unread);
    assert!(!listed(&unread), "still unread: {}", unread.body);
}

#[tokio::test]
async fn flags_a_post() {
    let (db, state) = app().await;
    let post = post_by_another(&db, "user1").await.to_string();
    let mut user1 = member(&state, "user1").await;
    let flagged = user1
        .post(
            "/post_actions.json",
            &[
                ("id", &post),
                ("post_action_type_id", "4"),
                ("flag_topic", "false"),
            ],
        )
        .await;
    ok("POST /post_actions.json inappropriate", &flagged);
    settle(&db, &state).await;
    let reviewables: i64 =
        sqlx::query_scalar("SELECT count(*) FROM reviewables WHERE target_id = $1::int")
            .bind(&post)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(reviewables, 1);
}

#[tokio::test]
async fn offers_light_and_dark_palettes() {
    let (db, state) = app().await;
    // A light and a dark palette members may pick, as an admin sets up
    // for a demo of the switch.
    sqlx::query("UPDATE color_schemes SET user_selectable = TRUE WHERE id IN (1, 2)")
        .execute(&db.pool)
        .await
        .unwrap();
    let mut user1 = member(&state, "user1").await;
    let page = user1.page("/u/user1/preferences/interface").await;
    drawn(
        "/u/user1/preferences/interface",
        &page,
        "control-group color-scheme",
    );
}

#[tokio::test]
async fn switches_to_dark_mode() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    // Auto, light or dark: interface_color_mode 3 (UserOption::DARK_MODE)
    // forces dark.
    let saved = user1
        .put("/u/user1.json", &[("interface_color_mode", "3")])
        .await;
    ok("PUT /u/user1.json interface_color_mode", &saved);
    // The default theme (Foundation) has a dark palette (Dark, 13): the
    // page now draws with its definitions whatever the browser prefers.
    let home = user1.page("/latest").await;
    drawn(
        "/latest in dark mode",
        &home,
        r#"/assets/color_definitions_dark.css" media="all" rel="stylesheet" class="dark-scheme" data-scheme-id="13"/>"#,
    );
    drawn(
        "/latest in dark mode",
        &home,
        r#"/assets/color_definitions_light.css" media="none" rel="stylesheet" class="light-scheme"/>"#,
    );
}

#[tokio::test]
async fn logs_out() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let out = user1.delete("/session/user1.json").await;
    ok("DELETE /session/user1.json", &out);
    let me = user1.get("/session/current.json").await;
    assert_eq!(me.status, StatusCode::NOT_FOUND, "{}", me.body);
    let home = user1.page("/latest").await;
    drawn("/latest after logging out", &home, "sign-up-button");
}

// Chat.

#[tokio::test]
async fn chat_page() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let page = user1.page("/chat").await;
    drawn("/chat", &page, r#"<div class="chat-browse-view">"#);
}

#[tokio::test]
async fn chat_channel_page() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let page = user1.page(&format!("/chat/c/general/{CHANNEL}")).await;
    drawn("/chat/c/general/2", &page, "chat-composer");
}

async fn send_chat(client: &mut Client, channel: i64, message: &str) -> i64 {
    let path = format!("/chat/{channel}.json");
    let sent = client
        .post(&path, &[("message", message), ("staged_id", "demo-staged")])
        .await;
    ok(&path, &sent);
    sent.json()["message_id"]
        .as_i64()
        .unwrap_or_else(|| panic!("no message id in {}", sent.body))
}

#[tokio::test]
async fn sends_a_chat_message() {
    let (_db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    user1.page(&format!("/chat/c/general/{CHANNEL}")).await;
    send_chat(&mut user1, CHANNEL as i64, "Hello from the demo :wave:").await;
}

#[tokio::test]
async fn another_member_receives_the_chat_message() {
    let (db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let id = send_chat(
        &mut user1,
        CHANNEL as i64,
        "Is everyone ready for the demo?",
    )
    .await;
    settle(&db, &state).await;
    let mut user2 = member(&state, "user2").await;
    let messages = user2
        .get(&format!("/chat/api/channels/{CHANNEL}/messages.json"))
        .await;
    ok("the channel's messages", &messages);
    assert!(
        messages.body.contains("Is everyone ready for the demo?"),
        "{}",
        excerpt(&messages.body)
    );
    // What the channel page fetches when the bus says a message arrived.
    let live = user2.get(&format!("/live/chat/{CHANNEL}/{id}")).await;
    ok("/live/chat", &live);
    assert!(
        live.body.contains(&format!("data-id=\"{id}\"")),
        "{}",
        live.body
    );
}

#[tokio::test]
async fn starts_a_direct_message_and_reacts() {
    let (db, state) = app().await;
    let mut user1 = member(&state, "user1").await;
    let created = user1
        .post(
            "/chat/api/direct-message-channels.json",
            &[("target_usernames[]", "user2")],
        )
        .await;
    ok("POST /chat/api/direct-message-channels.json", &created);
    let channel = created.json()["channel"]["id"]
        .as_i64()
        .unwrap_or_else(|| panic!("no channel in {}", created.body));
    let id = send_chat(&mut user1, channel, "A direct hello before the demo").await;

    let mut user2 = member(&state, "user2").await;
    let page = user2.page(&format!("/chat/c/-/{channel}")).await;
    drawn("the DM channel", &page, "A direct hello before the demo");
    let reacted = user2
        .put(
            &format!("/chat/{channel}/react/{id}.json"),
            &[("emoji", "heart"), ("react_action", "add")],
        )
        .await;
    ok("PUT /chat/:id/react/:message_id.json", &reacted);
    settle(&db, &state).await;
}

// Admin.

#[tokio::test]
async fn admin_dashboard() {
    let (_db, state) = app().await;
    let mut admin = member(&state, "admin").await;
    let page = admin.page("/admin").await;
    drawn("/admin", &page, "admin-contents dashboard");
}

#[tokio::test]
async fn admin_changes_a_site_setting() {
    let (_db, state) = app().await;
    let mut admin = member(&state, "admin").await;
    let page = admin.page("/admin/site_settings").await;
    drawn(
        "/admin/site_settings",
        &page,
        "admin-contents admin-site-settings",
    );
    let changed = admin
        .put(
            "/admin/site_settings/title.json",
            &[("title", "Demo Night")],
        )
        .await;
    ok("PUT /admin/site_settings/title.json", &changed);
    let home = anonymous(&state).await.page("/latest").await;
    drawn("/latest", &home, "<title>Demo Night</title>");
}

#[tokio::test]
async fn admin_users_list_and_user_page() {
    let (_db, state) = app().await;
    let mut admin = member(&state, "admin").await;
    let list = admin.page("/admin/users").await;
    drawn(
        "/admin/users",
        &list,
        "admin-contents admin-users admin-users-list",
    );
    let user = admin.page("/admin/users/3/user1").await;
    drawn(
        "/admin/users/3/user1",
        &user,
        "admin-contents admin-users admin-user",
    );
}

#[tokio::test]
async fn admin_suspends_and_unsuspends() {
    let (db, state) = app().await;
    let id = user_id(&db, "user1").await;
    let mut admin = member(&state, "admin").await;
    let suspended = admin
        .put(
            &format!("/admin/users/{id}/suspend.json"),
            &[
                ("reason", "Taking a break for the demo"),
                ("suspend_until", "2030-01-01T00:00:00Z"),
            ],
        )
        .await;
    ok("PUT suspend", &suspended);
    let page = admin.page(&format!("/admin/users/{id}/user1")).await;
    drawn(
        "the suspended user's page",
        &page,
        "user-suspended display-row highlight-danger",
    );
    let back = admin
        .put(&format!("/admin/users/{id}/unsuspend.json"), &[])
        .await;
    ok("PUT unsuspend", &back);
    settle(&db, &state).await;
}

#[tokio::test]
async fn admin_silences() {
    let (db, state) = app().await;
    let id = user_id(&db, "user1").await;
    let mut admin = member(&state, "admin").await;
    let silenced = admin
        .put(
            &format!("/admin/users/{id}/silence.json"),
            &[
                ("reason", "Quiet during the demo"),
                ("silenced_till", "2030-01-01T00:00:00Z"),
            ],
        )
        .await;
    ok("PUT silence", &silenced);
    let page = admin.page(&format!("/admin/users/{id}/user1")).await;
    drawn(
        "the silenced user's page",
        &page,
        r#"<div class="display-row highlight-danger"><div class="field">Silenced?</div>"#,
    );
    let back = admin
        .put(&format!("/admin/users/{id}/unsilence.json"), &[])
        .await;
    ok("PUT unsilence", &back);
}

/// Silencing sends the member a system message, which notifies them, and
/// the account_silenced email.
#[tokio::test]
async fn a_silenced_member_is_told() {
    let (db, state) = app().await;
    let id = user_id(&db, "user1").await;
    let mut admin = member(&state, "admin").await;
    let silenced = admin
        .put(
            &format!("/admin/users/{id}/silence.json"),
            &[
                ("reason", "Quiet during the demo"),
                ("silenced_till", "2030-01-01T00:00:00Z"),
            ],
        )
        .await;
    ok("PUT silence", &silenced);
    settle(&db, &state).await;
    let mut user1 = member(&state, "user1").await;
    let recent = user1.get("/notifications.json?recent=true&limit=30").await;
    ok("/notifications.json", &recent);
    // The system message's private_message notification.
    assert!(
        recent.json()["notifications"].as_array().is_some_and(|n| n
            .iter()
            .any(|n| n["notification_type"] == 6 && n["data"]["original_username"] == "system")),
        "{}",
        excerpt(&recent.body)
    );
    let emailed: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM email_logs WHERE email_type = 'account_silenced' AND user_id = $1)",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(emailed, "no account_silenced email was sent");
}

#[tokio::test]
async fn admin_staff_action_logs() {
    let (db, state) = app().await;
    let id = user_id(&db, "user1").await;
    let mut admin = member(&state, "admin").await;
    let silenced = admin
        .put(
            &format!("/admin/users/{id}/silence.json"),
            &[
                ("reason", "For the log"),
                ("silenced_till", "2030-01-01T00:00:00Z"),
            ],
        )
        .await;
    ok("PUT silence", &silenced);
    let page = admin.page("/admin/logs/staff_action_logs").await;
    drawn(
        "/admin/logs/staff_action_logs",
        &page,
        "admin-contents admin-logs staff-action-logs",
    );
    let json = admin.get("/admin/logs/staff_action_logs.json").await;
    ok("/admin/logs/staff_action_logs.json", &json);
    assert!(
        json.body.contains("silence_user"),
        "{}",
        excerpt(&json.body)
    );
}

#[tokio::test]
async fn admin_review_queue_page() {
    let (db, state) = app().await;
    let post = post_by_another(&db, "user1").await.to_string();
    let mut user1 = member(&state, "user1").await;
    let flagged = user1
        .post(
            "/post_actions.json",
            &[
                ("id", &post),
                ("post_action_type_id", "4"),
                ("flag_topic", "false"),
            ],
        )
        .await;
    ok("POST /post_actions.json", &flagged);
    settle(&db, &state).await;
    let mut admin = member(&state, "admin").await;
    let page = admin.page("/review").await;
    drawn("/review", &page, "review-item reviewable-flagged-post");
}

#[tokio::test]
async fn admin_reviews_a_flag() {
    let (db, state) = app().await;
    let post = post_by_another(&db, "user1").await.to_string();
    let mut user1 = member(&state, "user1").await;
    let flagged = user1
        .post(
            "/post_actions.json",
            &[
                ("id", &post),
                ("post_action_type_id", "4"),
                ("flag_topic", "false"),
            ],
        )
        .await;
    ok("POST /post_actions.json", &flagged);
    settle(&db, &state).await;

    let mut admin = member(&state, "admin").await;
    let list = admin.get("/review.json").await;
    ok("/review.json", &list);
    let json = list.json();
    let reviewable = json["reviewables"]
        .as_array()
        .and_then(|r| r.first())
        .unwrap_or_else(|| panic!("nothing to review: {}", list.body));
    let (id, version) = (&reviewable["id"], &reviewable["version"]);
    let version = version.to_string();
    let performed = admin
        .put(
            &format!("/review/{id}/perform/agree_and_keep"),
            &[("version", &version)],
        )
        .await;
    ok("PUT /review/:id/perform/agree_and_keep", &performed);
    settle(&db, &state).await;
}

#[tokio::test]
async fn admin_badges_page() {
    let (_db, state) = app().await;
    let mut admin = member(&state, "admin").await;
    let page = admin.page("/admin/badges").await;
    drawn("/admin/badges", &page, "admin-contents admin-badges");
}

#[tokio::test]
async fn admin_customize_themes_palettes_and_texts() {
    let (_db, state) = app().await;
    let mut admin = member(&state, "admin").await;
    let themes = admin.page("/admin/customize/themes").await;
    drawn(
        "/admin/customize/themes",
        &themes,
        "admin-contents admin-config customize themes",
    );
    let colors = admin.page("/admin/config/colors").await;
    drawn(
        "/admin/config/colors",
        &colors,
        "admin-contents admin-config color-palettes",
    );
    let texts = admin
        .page("/admin/customize/site_texts?q=create%20topic")
        .await;
    drawn(
        "/admin/customize/site_texts",
        &texts,
        "admin-contents admin-customize admin-site-text",
    );
}

#[tokio::test]
async fn admin_email_settings_page() {
    let (_db, state) = app().await;
    let mut admin = member(&state, "admin").await;
    let page = admin.page("/admin/email").await;
    drawn("/admin/email", &page, "admin-contents admin-email settings");
}

#[tokio::test]
async fn admin_backups_page() {
    let (_db, state) = app().await;
    let mut admin = member(&state, "admin").await;
    let page = admin.page("/admin/backups").await;
    drawn("/admin/backups", &page, "admin-contents backups");
}

#[tokio::test]
async fn admin_reports() {
    let (_db, state) = app().await;
    let mut admin = member(&state, "admin").await;
    let index = admin.page("/admin/reports").await;
    drawn("/admin/reports", &index, "/admin/reports/signups");
    let report = admin.page("/admin/reports/signups").await;
    drawn(
        "/admin/reports/signups",
        &report,
        "admin-report is-enabled is-visible signups",
    );
}

#[tokio::test]
async fn admin_whats_new() {
    let (_db, state) = app().await;
    let mut admin = member(&state, "admin").await;
    let page = admin.page("/admin/whats-new").await;
    drawn("/admin/whats-new", &page, "admin-contents whats-new");
}

#[tokio::test]
async fn admin_search() {
    let (_db, state) = app().await;
    let mut admin = member(&state, "admin").await;
    let page = admin.page("/admin/search?filter=logo").await;
    drawn("/admin/search", &page, "admin-contents admin-search");
    let all = admin.get("/admin/search/all.json").await;
    ok("/admin/search/all.json", &all);
}
