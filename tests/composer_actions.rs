//! The posts#create params the composer actions menu sends: whisper,
//! no_bump and unlist_topic (create_params, PostCreator, TopicCreator),
//! after spec/requests/posts_controller_spec.rb and
//! spec/lib/post_creator_spec.rb. Topic 35 is the seed's, its last post
//! #5; admin is a whisperer once whispers_allowed_groups names staff.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, header};
use chrono::NaiveDateTime;
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

    async fn post(&mut self, body: &str) -> (u16, Value) {
        self.send(Method::POST, "/posts.json", body).await
    }
}

async fn setup(db: &TestDb) -> AppState {
    // The staff group's members whisper.
    set_setting(&db.pool, "whispers_allowed_groups", 20, "3").await;
    state(db.pool.clone(), config(RailsEnv::Test, &[])).await
}

#[derive(sqlx::FromRow, Debug, PartialEq)]
struct TopicStats {
    highest_post_number: i32,
    highest_staff_post_number: i32,
    posts_count: i32,
    last_posted_at: Option<NaiveDateTime>,
    last_post_user_id: i32,
    bumped_at: NaiveDateTime,
}

async fn topic_stats(db: &TestDb, topic_id: i64) -> TopicStats {
    sqlx::query_as(
        "SELECT highest_post_number, highest_staff_post_number, posts_count, last_posted_at, \
                last_post_user_id, bumped_at FROM topics WHERE id = $1",
    )
    .bind(topic_id as i32)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

async fn post_count(db: &TestDb, username: &str) -> i32 {
    sqlx::query_scalar(
        "SELECT s.post_count FROM user_stats s JOIN users u ON u.id = s.user_id WHERE u.username = $1",
    )
    .bind(username)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_whisper_moves_only_the_staff_count_and_leaves_the_topic_stats() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let before = topic_stats(&db, 35).await;
    let posts_before = post_count(&db, "admin").await;

    let (status, body) = admin
        .post("topic_id=35&raw=A+whisper+only+staff+can+read+here.&whisper=true")
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["post_type"], 4);
    let number = body["post_number"].as_i64().unwrap() as i32;
    assert!(number > before.highest_post_number);

    let after = topic_stats(&db, 35).await;
    assert_eq!(
        after,
        TopicStats {
            highest_staff_post_number: number,
            ..before
        },
        "a whisper leaves the topic's counts, last post and bump"
    );
    // UserStatCountUpdater counts regular posts only.
    assert_eq!(post_count(&db, "admin").await, posts_before);
}

#[tokio::test]
async fn whispering_needs_a_whisperer() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let (status, body) = user1
        .post("topic_id=35&raw=Trying+to+whisper+without+the+right.&whisper=true")
        .await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error_type"], "invalid_access");
    assert_eq!(
        body["errors"][0],
        "Either whispers are not enabled or you do not have access to create whisper posts"
    );
    // A false whisper is a regular post.
    let (status, body) = user1
        .post("topic_id=35&raw=Not+a+whisper+after+all+in+this+one.&whisper=false")
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["post_type"], 1);
}

#[tokio::test]
async fn a_reply_to_a_whisper_whispers_or_is_refused() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, body) = admin
        .post("topic_id=35&raw=The+whisper+being+replied+to+here.&whisper=true")
        .await;
    assert_eq!(status, 200, "{body}");
    let whisper = body["post_number"].as_i64().unwrap();

    // set_reply_info: a whisperer's reply is a whisper too.
    let (status, body) = admin
        .post(&format!(
            "topic_id=35&reply_to_post_number={whisper}&raw=Replying+to+the+whisper+as+staff."
        ))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["post_type"], 4);

    // Anyone else's is aborted as if the topic were not there.
    let mut user1 = Client::logged_in(st, "user1").await;
    let (status, body) = user1
        .post(&format!(
            "topic_id=35&reply_to_post_number={whisper}&raw=Replying+to+a+whisper+I+cannot+see."
        ))
        .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(
        body["errors"][0],
        "Something has gone wrong. Perhaps this topic was closed or deleted while you were looking at it?"
    );
}

#[tokio::test]
async fn no_bump_leaves_bumped_at_for_those_who_may_skip_it() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let before = topic_stats(&db, 35).await;

    // user1 is trust level 2: InvalidParameters.
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, body) = user1
        .post("topic_id=35&raw=A+reply+that+should+not+bump+it.&no_bump=true")
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error_type"], "invalid_parameters");

    // user3 is trust level 4.
    let mut user3 = Client::logged_in(st, "user3").await;
    let (status, body) = user3
        .post("topic_id=35&raw=A+quiet+reply+from+a+leader+here.&no_bump=true")
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["post_type"], 1);
    let after = topic_stats(&db, 35).await;
    assert_eq!(after.bumped_at, before.bumped_at);
    assert_eq!(after.highest_post_number, before.highest_post_number + 1);
    assert!(after.last_posted_at > before.last_posted_at);
}

#[tokio::test]
async fn staff_may_create_unlisted_topics() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, body) = admin
        .post("title=An+unlisted+topic+from+the+composer&raw=Only+those+with+the+link+find+this+one.&category=4&unlist_topic=true")
        .await;
    assert_eq!(status, 200, "{body}");
    let visible: bool = sqlx::query_scalar("SELECT visible FROM topics WHERE id = $1")
        .bind(body["topic_id"].as_i64().unwrap() as i32)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert!(!visible);

    // unlist_topic=false lists it.
    let (status, body) = admin
        .post("title=A+listed+topic+from+the+composer+too&raw=Everyone+finds+this+one+in+the+lists.&category=4&unlist_topic=false")
        .await;
    assert_eq!(status, 200, "{body}");
    let visible: bool = sqlx::query_scalar("SELECT visible FROM topics WHERE id = $1")
        .bind(body["topic_id"].as_i64().unwrap() as i32)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert!(visible);

    // A member may not: TopicCreator#valid?'s unable_to_unlist.
    let mut user1 = Client::logged_in(st, "user1").await;
    let (status, body) = user1
        .post("title=A+member+tries+an+unlisted+topic&raw=This+should+not+be+created+unlisted.&category=4&unlist_topic=true")
        .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(
        body["errors"][0],
        "Sorry, you cannot create an unlisted topic."
    );
}

#[tokio::test]
async fn the_composer_draws_the_actions_menu_and_the_poll_builder() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let admin = Client::logged_in(st.clone(), "admin").await;
    let mut request = Request::builder()
        .uri("/latest")
        .header(header::HOST, "test.localhost");
    let cookie: Vec<String> = admin
        .cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    request = request.header(header::COOKIE, cookie.join("; "));
    let response = discourse_rs::app(st)
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let html = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains("fk-d-menu__trigger composer-actions-trigger"));
    assert!(html.contains("data-composer-actions=\""));
    assert!(html.contains("&quot;whisperer&quot;:true"));
    assert!(html.contains("&quot;canSkipBump&quot;:true"));
    // Build poll is ported (discourse-poll is on by default).
    assert!(html.contains("data-poll-builder=\""));
    assert!(html.contains("&quot;name&quot;:&quot;poll&quot;"));
    let poll = html
        .split("&quot;name&quot;:&quot;poll&quot;")
        .nth(1)
        .unwrap();
    assert!(poll.contains("&quot;ported&quot;:true"));
}
