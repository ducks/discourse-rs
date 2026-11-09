//! Changing the post an edit replies to (ROADMAP 196r6): posts#update's
//! `reply_to_post_number`, PostRevisor's resolve_reply_to_change and
//! cleanup_previous_reply_to_relationship, and PostRevisionSerializer's
//! reply_to_post_number_changes, after spec/lib/post_revisor_spec.rb's
//! "revising reply_to_post_number", spec/requests/posts_controller_spec.rb
//! and spec/serializers/post_revision_serializer_spec.rb.

use crate::common;

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
    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client.send(Method::GET, "/session/csrf.json", "").await;
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

    async fn send(&mut self, method: Method, path: &str, body: &str) -> (u16, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest")
            .header(header::ACCEPT, "application/json");
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
            let v = v.to_str().unwrap();
            let pair = v.split(';').next().unwrap();
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

    /// posts#create: the new post's (id, post_number).
    async fn create(&mut self, body: &str) -> (i32, i32) {
        let (status, post) = self.send(Method::POST, "/posts.json", body).await;
        assert_eq!(status, 200, "{post}");
        (
            post["id"].as_i64().unwrap() as i32,
            post["post_number"].as_i64().unwrap() as i32,
        )
    }

    /// posts#update with the post's raw and a reply target: blank for none.
    async fn revise(&mut self, db: &TestDb, post_id: i32, reply_to: &str) -> (u16, Value) {
        let raw: String = sqlx::query_scalar("SELECT raw FROM posts WHERE id = $1")
            .bind(post_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        self.send(
            Method::PUT,
            &format!("/posts/{post_id}.json"),
            &format!(
                "post[raw]={}&post[reply_to_post_number]={reply_to}",
                urlencoding(&raw)
            ),
        )
        .await
    }
}

fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The spec's topic: an OP by user1, a first and a second reply to it by
/// user2, and user0's own reply to it, which the tests edit.
struct Topic {
    id: i32,
    op: (i32, i32),
    first_reply: (i32, i32),
    post_to_edit: (i32, i32),
    state: AppState,
}

async fn setup(db: &TestDb) -> Topic {
    // A new version for every edit, as force_new_version gives the spec.
    set_setting(&db.pool, "editing_grace_period", 3, "0").await;
    set_setting(&db.pool, "whispers_allowed_groups", 20, "3").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut op_author = Client::logged_in(st.clone(), "user1").await;
    let (status, created) = op_author
        .send(
            Method::POST,
            "/posts.json",
            "title=A+topic+whose+replies+change+target&raw=The+first+post+of+the+topic+replied+to.&category=4",
        )
        .await;
    assert_eq!(status, 200, "{created}");
    let id = created["topic_id"].as_i64().unwrap() as i32;
    let op = (created["id"].as_i64().unwrap() as i32, 1);
    let mut replier = Client::logged_in(st.clone(), "user2").await;
    let first_reply = replier
        .create(&format!(
            "topic_id={id}&reply_to_post_number=1&raw=first+reply+body,+long+enough+to+be+valid"
        ))
        .await;
    replier
        .create(&format!(
            "topic_id={id}&reply_to_post_number=1&raw=second+reply+body,+long+enough+to+be+valid"
        ))
        .await;
    let mut editor = Client::logged_in(st.clone(), "user0").await;
    let post_to_edit = editor
        .create(&format!(
            "topic_id={id}&reply_to_post_number=1&raw=my+reply+body,+long+enough+to+be+valid"
        ))
        .await;
    Topic {
        id,
        op,
        first_reply,
        post_to_edit,
        state: st,
    }
}

async fn reply_target(db: &TestDb, post_id: i32) -> (Option<i32>, Option<i32>) {
    sqlx::query_as("SELECT reply_to_post_number, reply_to_user_id FROM posts WHERE id = $1")
        .bind(post_id)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn reply_count(db: &TestDb, post_id: i32) -> i32 {
    sqlx::query_scalar("SELECT reply_count FROM posts WHERE id = $1")
        .bind(post_id)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn post_reply(db: &TestDb, parent: i32, reply: i32) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM post_replies WHERE post_id = $1 AND reply_post_id = $2)",
    )
    .bind(parent)
    .bind(reply)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

async fn user_id(db: &TestDb, username: &str) -> i32 {
    sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
        .bind(username)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn revisions(db: &TestDb, post_id: i32) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM post_revisions WHERE post_id = $1")
        .bind(post_id)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

const INVALID: &str = "Reply to post number The selected post isn't a valid reply target. It must be an earlier post in the same topic.";

#[tokio::test]
async fn reparents_to_another_earlier_post_and_syncs_reply_to_user_id() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let original_parent_reply_count = reply_count(&db, t.op.0).await;
    let mut editor = Client::logged_in(t.state.clone(), "user0").await;

    let (status, body) = editor
        .revise(&db, t.post_to_edit.0, &t.first_reply.1.to_string())
        .await;
    assert_eq!(status, 200, "{body}");

    assert_eq!(
        reply_target(&db, t.post_to_edit.0).await,
        (Some(t.first_reply.1), Some(user_id(&db, "user2").await))
    );
    assert_eq!(reply_count(&db, t.first_reply.0).await, 1);
    assert_eq!(
        reply_count(&db, t.op.0).await,
        original_parent_reply_count - 1
    );
    assert_eq!(body["post"]["reply_to_post_number"], t.first_reply.1);
}

#[tokio::test]
async fn removes_the_reply_relationship_when_set_to_nil() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let mut editor = Client::logged_in(t.state.clone(), "user0").await;

    let (status, body) = editor.revise(&db, t.post_to_edit.0, "").await;
    assert_eq!(status, 200, "{body}");

    assert_eq!(reply_target(&db, t.post_to_edit.0).await, (None, None));
    assert!(!post_reply(&db, t.op.0, t.post_to_edit.0).await);
}

#[tokio::test]
async fn tracks_the_change_in_a_post_revision() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let mut editor = Client::logged_in(t.state.clone(), "user0").await;
    let (status, body) = editor
        .revise(&db, t.post_to_edit.0, &t.first_reply.1.to_string())
        .await;
    assert_eq!(status, 200, "{body}");

    let modifications: String = sqlx::query_scalar(
        "SELECT modifications FROM post_revisions WHERE post_id = $1 ORDER BY number DESC LIMIT 1",
    )
    .bind(t.post_to_edit.0)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        modifications,
        format!(
            "!ruby/hash:ActiveSupport::HashWithIndifferentAccess\nreply_to_post_number:\n- 1\n- {}\n",
            t.first_reply.1
        )
    );
}

#[tokio::test]
async fn does_not_create_a_revision_when_the_value_is_unchanged() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let mut editor = Client::logged_in(t.state.clone(), "user0").await;
    let before = revisions(&db, t.post_to_edit.0).await;

    let (status, body) = editor.revise(&db, t.post_to_edit.0, "1").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(revisions(&db, t.post_to_edit.0).await, before);
}

#[tokio::test]
async fn rejects_invalid_targets() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let mut editor = Client::logged_in(t.state.clone(), "user0").await;

    // a self-reference
    let (status, body) = editor
        .revise(&db, t.post_to_edit.0, &t.post_to_edit.1.to_string())
        .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["errors"][0], INVALID);
    assert_eq!(reply_target(&db, t.post_to_edit.0).await.0, Some(1));

    // a later post in the topic
    let mut other = Client::logged_in(t.state.clone(), "user2").await;
    let later = other
        .create(&format!(
            "topic_id={}&raw=a+later+post,+long+enough+to+be+valid",
            t.id
        ))
        .await;
    let (status, body) = editor
        .revise(&db, t.post_to_edit.0, &later.1.to_string())
        .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["errors"][0], INVALID);

    // a post that does not exist in the topic
    let (status, body) = editor.revise(&db, t.post_to_edit.0, "999").await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["errors"][0], INVALID);

    // a deleted post
    sqlx::query("UPDATE posts SET deleted_at = now() WHERE id = $1")
        .bind(t.first_reply.0)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body) = editor
        .revise(&db, t.post_to_edit.0, &t.first_reply.1.to_string())
        .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(reply_target(&db, t.post_to_edit.0).await.0, Some(1));
}

#[tokio::test]
async fn rejects_a_target_the_editor_cannot_see() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let mut admin = Client::logged_in(t.state.clone(), "admin").await;
    let (status, whisper) = admin
        .send(
            Method::POST,
            "/posts.json",
            &format!(
                "topic_id={}&raw=a+whisper+the+editor+cannot+see&whisper=true",
                t.id
            ),
        )
        .await;
    assert_eq!(status, 200, "{whisper}");
    let mut editor = Client::logged_in(t.state.clone(), "user0").await;

    let (status, body) = editor
        .revise(
            &db,
            t.post_to_edit.0,
            &whisper["post_number"].as_i64().unwrap().to_string(),
        )
        .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["errors"][0], INVALID);
}

#[tokio::test]
async fn keeps_the_post_reply_row_for_the_old_parent_if_the_post_still_quotes_it() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let mut editor = Client::logged_in(t.state.clone(), "user0").await;
    let (status, body) = editor
        .send(
            Method::PUT,
            &format!("/posts/{}.json", t.post_to_edit.0),
            &format!(
                "post[raw]={}&post[reply_to_post_number]={}",
                urlencoding(&format!(
                    "quoting\n[quote=\"user1, post:1, topic:{}\"]hi[/quote]",
                    t.id
                )),
                t.first_reply.1
            ),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(post_reply(&db, t.op.0, t.post_to_edit.0).await);
    assert!(post_reply(&db, t.first_reply.0, t.post_to_edit.0).await);
}

#[tokio::test]
async fn does_not_create_new_post_reply_rows_when_the_post_save_fails() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    set_setting(&db.pool, "min_post_length", 3, "500").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let new_parent_reply_count = reply_count(&db, t.first_reply.0).await;
    let mut editor = Client::logged_in(st, "user0").await;

    let (status, body) = editor
        .send(
            Method::PUT,
            &format!("/posts/{}.json", t.post_to_edit.0),
            &format!(
                "post[raw]=too+short&post[reply_to_post_number]={}",
                t.first_reply.1
            ),
        )
        .await;
    assert_eq!(status, 422, "{body}");
    assert!(!post_reply(&db, t.first_reply.0, t.post_to_edit.0).await);
    assert_eq!(
        reply_count(&db, t.first_reply.0).await,
        new_parent_reply_count
    );
}

#[tokio::test]
async fn cleans_up_the_post_reply_row_when_the_previous_parent_is_already_trashed() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    sqlx::query("UPDATE posts SET deleted_at = now() WHERE id = $1")
        .bind(t.op.0)
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(post_reply(&db, t.op.0, t.post_to_edit.0).await);
    let mut editor = Client::logged_in(t.state.clone(), "user0").await;

    let (status, body) = editor
        .revise(&db, t.post_to_edit.0, &t.first_reply.1.to_string())
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(!post_reply(&db, t.op.0, t.post_to_edit.0).await);
}

// post_revision_serializer_spec.rb's "reply_to_post_number edits" and
// posts_controller_spec.rb's "omits unseen reply target post numbers".
#[tokio::test]
async fn revisions_show_the_reply_target_change() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let mut editor = Client::logged_in(t.state.clone(), "user0").await;
    let (status, body) = editor
        .revise(&db, t.post_to_edit.0, &t.first_reply.1.to_string())
        .await;
    assert_eq!(status, 200, "{body}");
    let number: i32 = sqlx::query_scalar("SELECT version FROM posts WHERE id = $1")
        .bind(t.post_to_edit.0)
        .fetch_one(&db.pool)
        .await
        .unwrap();

    let mut viewer = Client::logged_in(t.state.clone(), "user1").await;
    let (status, json) = viewer
        .send(
            Method::GET,
            &format!("/posts/{}/revisions/{number}.json", t.post_to_edit.0),
            "",
        )
        .await;
    assert_eq!(status, 200, "{json}");
    let changes = &json["reply_to_post_number_changes"];
    assert_eq!(changes["previous"]["post_number"], 1);
    assert_eq!(changes["previous"]["username"], "user1");
    assert_eq!(changes["current"]["post_number"], t.first_reply.1);
    assert_eq!(changes["current"]["username"], "user2");
    assert_eq!(changes["current"]["display_username"], "user2");
    assert!(changes["current"]["avatar_template"].is_string());

    // the side that has no reply target is nil
    let (status, body) = editor.revise(&db, t.post_to_edit.0, "").await;
    assert_eq!(status, 200, "{body}");
    let (status, json) = viewer
        .send(
            Method::GET,
            &format!("/posts/{}/revisions/{}.json", t.post_to_edit.0, number + 1),
            "",
        )
        .await;
    assert_eq!(status, 200, "{json}");
    assert_eq!(
        json["reply_to_post_number_changes"]["previous"]["post_number"],
        t.first_reply.1
    );
    assert!(json["reply_to_post_number_changes"]["current"].is_null());

    // omitted when reply_to_post_number didn't change
    let (status, body) = editor
        .send(
            Method::PUT,
            &format!("/posts/{}.json", t.post_to_edit.0),
            "post[raw]=my+reply+body,+long+enough+to+be+valid+and+changed",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, json) = viewer
        .send(
            Method::GET,
            &format!("/posts/{}/revisions/{}.json", t.post_to_edit.0, number + 2),
            "",
        )
        .await;
    assert_eq!(status, 200, "{json}");
    assert!(json.get("reply_to_post_number_changes").is_none(), "{json}");
}

#[tokio::test]
async fn revisions_omit_a_reply_target_the_viewer_cannot_see() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let mut admin = Client::logged_in(t.state.clone(), "admin").await;
    let (status, whisper) = admin
        .send(
            Method::POST,
            "/posts.json",
            &format!(
                "topic_id={}&raw=a+whisper+only+staff+can+see&whisper=true",
                t.id
            ),
        )
        .await;
    assert_eq!(status, 200, "{whisper}");
    // a reply after the whisper, which staff reparent to it
    let mut editor = Client::logged_in(t.state.clone(), "user0").await;
    let revised = editor
        .create(&format!(
            "topic_id={}&reply_to_post_number=1&raw=a+reply+after+the+whisper,+long+enough",
            t.id
        ))
        .await;
    let (status, body) = admin
        .revise(
            &db,
            revised.0,
            &whisper["post_number"].as_i64().unwrap().to_string(),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let number: i32 = sqlx::query_scalar("SELECT version FROM posts WHERE id = $1")
        .bind(revised.0)
        .fetch_one(&db.pool)
        .await
        .unwrap();

    let mut viewer = Client::logged_in(t.state.clone(), "user1").await;
    let (status, json) = viewer
        .send(
            Method::GET,
            &format!("/posts/{}/revisions/{number}.json", revised.0),
            "",
        )
        .await;
    assert_eq!(status, 200, "{json}");
    assert_eq!(
        json["reply_to_post_number_changes"]["previous"]["post_number"],
        1
    );
    assert!(json["reply_to_post_number_changes"]["current"].is_null());
}
