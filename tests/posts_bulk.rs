//! PostsController#reply_ids, #destroy_many and #merge_posts: Post#reply_ids,
//! PostDestroyer over several posts in one transaction, and PostMerger,
//! after spec/requests/posts_controller_spec.rb, spec/models/post_spec.rb's
//! "reply_ids" and spec/lib/post_merger_spec.rb.

use crate::common;

use axum::body::Body;
use axum::http::{Method, Request, header};
use common::{TestDb, config, set_setting, state};
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
    /// A visitor holding a CSRF token.
    async fn anonymous(state: AppState) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client.send(Method::GET, "/session/csrf.json", "").await;
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client::anonymous(state).await;
        client
            .send(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
            )
            .await;
        client
    }

    /// A request with a form body, or a JSON one when it starts with `{`.
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
        if body.starts_with('{') {
            request = request.header(header::CONTENT_TYPE, "application/json");
        } else if !body.is_empty() {
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

    /// posts#create: the new post's id.
    async fn create(&mut self, body: &str) -> i32 {
        let (status, post) = self.send(Method::POST, "/posts.json", body).await;
        assert_eq!(status, 200, "{post}");
        post["id"].as_i64().unwrap() as i32
    }
}

/// A topic by user1 with replies by user2: post 2 to the first post,
/// post 3 to post 2, post 4 to post 3.
struct Topic {
    id: i32,
    posts: [i32; 4],
    state: AppState,
}

async fn new_topic(st: &AppState, title: &str) -> (i32, i32) {
    let mut author = Client::logged_in(st.clone(), "user1").await;
    let (status, created) = author
        .send(
            Method::POST,
            "/posts.json",
            &format!("title={title}&raw=The+first+post+of+{title}.&category=4"),
        )
        .await;
    assert_eq!(status, 200, "{created}");
    (
        created["topic_id"].as_i64().unwrap() as i32,
        created["id"].as_i64().unwrap() as i32,
    )
}

async fn setup(db: &TestDb) -> Topic {
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (id, op) = new_topic(&st, "A+topic+whose+replies+are+handled+in+bulk").await;
    let mut replier = Client::logged_in(st.clone(), "user2").await;
    let mut posts = [op, 0, 0, 0];
    for (i, raw) in [
        "the+first+reply+of+the+three+here",
        "the+second+reply+of+the+three+here",
        "the+third+reply+of+the+three+here",
    ]
    .iter()
    .enumerate()
    {
        posts[i + 1] = replier
            .create(&format!(
                "topic_id={id}&reply_to_post_number={}&raw={raw}",
                i + 1
            ))
            .await;
    }
    Topic {
        id,
        posts,
        state: st,
    }
}

async fn user_id(db: &TestDb, username: &str) -> i32 {
    sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
        .bind(username)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

/// Who deleted the post, when it is deleted.
async fn deleted_by(db: &TestDb, post_id: i32) -> Option<i32> {
    let (deleted, by): (bool, Option<i32>) =
        sqlx::query_as("SELECT deleted_at IS NOT NULL, deleted_by_id FROM posts WHERE id = $1")
            .bind(post_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(deleted, by.is_some());
    by
}

/// The topic's (posts_count, highest_post_number).
async fn counts(db: &TestDb, topic_id: i32) -> (i32, i32) {
    sqlx::query_as("SELECT posts_count, highest_post_number FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn post_count(db: &TestDb, username: &str) -> i32 {
    sqlx::query_scalar(
        "SELECT post_count FROM user_stats WHERE user_id = (SELECT id FROM users WHERE username = $1)",
    )
    .bind(username)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

fn ids(name: &str, ids: &[i32]) -> String {
    ids.iter()
        .map(|id| format!("{name}[]={id}"))
        .collect::<Vec<_>>()
        .join("&")
}

const PARAM_MISSING: &str = "param is missing or the value is empty or invalid: post_ids";
const INVALID_PARAMS: &str = "You supplied invalid parameters to the request: post_ids";

// reply_ids

#[tokio::test]
async fn reply_ids_lists_the_replies_below_a_post_by_id_with_their_levels() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let [op, second, third, fourth] = t.posts;
    let mut visitor = Client::anonymous(t.state.clone()).await;

    let (status, body) = visitor
        .send(Method::GET, &format!("/posts/{op}/reply-ids.json"), "")
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!([
            { "id": second, "level": 1 },
            { "id": third, "level": 2 },
            { "id": fourth, "level": 3 },
        ])
    );

    let (status, body) = visitor
        .send(Method::GET, &format!("/posts/{third}/reply-ids"), "")
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!([{ "id": fourth, "level": 1 }]));

    let (status, body) = visitor
        .send(Method::GET, &format!("/posts/{fourth}/reply-ids.json"), "")
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!([]));
}

#[tokio::test]
async fn reply_ids_skips_replies_to_several_posts_and_posts_the_viewer_cannot_see() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "whispers_allowed_groups", 20, "3").await;
    let t = setup(&db).await;
    let [op, second, third, fourth] = t.posts;
    // A reply to the first post that quotes the second replies to both.
    let mut member = Client::logged_in(t.state.clone(), "user0").await;
    let quoting = member
        .create(&format!(
            "topic_id={}&reply_to_post_number=1&raw=%5Bquote%3D%22user2%2C+post%3A2%2C+topic%3A{}%22%5D%0Athe+first+reply%0A%5B%2Fquote%5D%0A%0AAn+answer+to+the+two+of+them.",
            t.id, t.id
        ))
        .await;
    let replies: i64 =
        sqlx::query_scalar("SELECT count(*) FROM post_replies WHERE reply_post_id = $1")
            .bind(quoting)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(replies, 2);
    let mut admin = Client::logged_in(t.state.clone(), "admin").await;
    let whisper = admin
        .create(&format!(
            "topic_id={}&reply_to_post_number=1&raw=A+whisper+only+staff+can+read+here.&whisper=true",
            t.id
        ))
        .await;

    let seen = json!([
        { "id": second, "level": 1 },
        { "id": third, "level": 2 },
        { "id": fourth, "level": 3 },
    ]);
    let (status, body) = member
        .send(Method::GET, &format!("/posts/{op}/reply-ids.json"), "")
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, seen);

    let (status, body) = admin
        .send(Method::GET, &format!("/posts/{op}/reply-ids.json"), "")
        .await;
    assert_eq!(status, 200, "{body}");
    let mut with_whisper = seen.as_array().unwrap().clone();
    with_whisper.push(json!({ "id": whisper, "level": 1 }));
    assert_eq!(body, Value::Array(with_whisper));
}

#[tokio::test]
async fn reply_ids_is_not_found_for_a_post_the_viewer_cannot_see() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let mut admin = Client::logged_in(t.state.clone(), "admin").await;
    let (status, _) = admin
        .send(Method::DELETE, &format!("/posts/{}.json", t.posts[3]), "")
        .await;
    assert_eq!(status, 200);
    let mut visitor = Client::anonymous(t.state.clone()).await;

    let (status, _) = visitor
        .send(
            Method::GET,
            &format!("/posts/{}/reply-ids.json", t.posts[3]),
            "",
        )
        .await;
    assert_eq!(status, 404);
    let (status, _) = visitor
        .send(Method::GET, "/posts/999999/reply-ids.json", "")
        .await;
    assert_eq!(status, 404);
    // Those who moderate the topic find the deleted post.
    let (status, body) = admin
        .send(
            Method::GET,
            &format!("/posts/{}/reply-ids.json", t.posts[3]),
            "",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!([]));
}

// destroy_many

#[tokio::test]
async fn destroy_many_deletes_the_posts_in_one_go() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let [op, second, third, fourth] = t.posts;
    let admin_id = user_id(&db, "admin").await;
    let before = post_count(&db, "user2").await;
    assert_eq!(counts(&db, t.id).await, (4, 4));
    let mut admin = Client::logged_in(t.state.clone(), "admin").await;

    let (status, body) = admin
        .send(
            Method::DELETE,
            "/posts/destroy_many.json",
            &ids("post_ids", &[fourth, third]),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, Value::Null);

    assert_eq!(deleted_by(&db, third).await, Some(admin_id));
    assert_eq!(deleted_by(&db, fourth).await, Some(admin_id));
    assert_eq!(deleted_by(&db, second).await, None);
    assert_eq!(deleted_by(&db, op).await, None);
    assert_eq!(counts(&db, t.id).await, (2, 2));
    assert_eq!(post_count(&db, "user2").await, before - 2);
    // StaffActionLogger#log_post_deletion for each.
    let logged: Vec<i32> = sqlx::query_scalar(
        "SELECT post_id FROM user_histories \
         WHERE action = 17 AND acting_user_id = $1 AND post_id = ANY($2) ORDER BY id",
    )
    .bind(admin_id)
    .bind(vec![third, fourth])
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(logged, vec![third, fourth]);
    let reply_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM post_replies WHERE reply_post_id = ANY($1)")
            .bind(vec![third, fourth])
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(reply_rows, 0);
}

#[tokio::test]
async fn destroy_many_takes_the_replies_of_reply_post_ids_along() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let [op, second, third, fourth] = t.posts;
    let admin_id = user_id(&db, "admin").await;
    let mut admin = Client::logged_in(t.state.clone(), "admin").await;

    // PostReply rows of the second post: the third, not the fourth.
    let (status, body) = admin
        .send(
            Method::DELETE,
            "/posts/destroy_many",
            &format!(
                "{}&{}&agree_with_first_reply_flag=false",
                ids("post_ids", &[second]),
                ids("reply_post_ids", &[second])
            ),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    assert_eq!(deleted_by(&db, second).await, Some(admin_id));
    assert_eq!(deleted_by(&db, third).await, Some(admin_id));
    assert_eq!(deleted_by(&db, fourth).await, None);
    assert_eq!(deleted_by(&db, op).await, None);
    assert_eq!(counts(&db, t.id).await, (2, 4));
}

#[tokio::test]
async fn destroy_many_deletes_a_topic_through_its_first_post() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let admin_id = user_id(&db, "admin").await;
    let mut admin = Client::logged_in(t.state.clone(), "admin").await;

    let (status, body) = admin
        .send(
            Method::DELETE,
            "/posts/destroy_many.json",
            &ids("post_ids", &[t.posts[0], t.posts[1]]),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    assert_eq!(deleted_by(&db, t.posts[0]).await, Some(admin_id));
    assert_eq!(deleted_by(&db, t.posts[1]).await, Some(admin_id));
    assert_eq!(deleted_by(&db, t.posts[2]).await, None);
    let topic_deleted_by: Option<i32> = sqlx::query_scalar(
        "SELECT deleted_by_id FROM topics WHERE id = $1 AND deleted_at IS NOT NULL",
    )
    .bind(t.id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(topic_deleted_by, Some(admin_id));
}

#[tokio::test]
async fn destroy_many_deletes_a_member_s_own_posts_outright_without_a_stub_period() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "delete_removed_posts_after", 3, "0").await;
    let t = setup(&db).await;
    let author = user_id(&db, "user2").await;
    let mut member = Client::logged_in(t.state.clone(), "user2").await;

    let (status, body) = member
        .send(
            Method::DELETE,
            "/posts/destroy_many.json",
            &ids("post_ids", &[t.posts[2], t.posts[3]]),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    assert_eq!(deleted_by(&db, t.posts[2]).await, Some(author));
    assert_eq!(deleted_by(&db, t.posts[3]).await, Some(author));
    assert_eq!(counts(&db, t.id).await, (2, 2));
    // No staff log for one's own posts.
    let logged: i64 =
        sqlx::query_scalar("SELECT count(*) FROM user_histories WHERE post_id = ANY($1)")
            .bind(t.posts.to_vec())
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(logged, 0);
}

#[tokio::test]
async fn destroy_many_refuses_visitors_and_posts_the_user_may_not_delete() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "delete_removed_posts_after", 3, "0").await;
    let t = setup(&db).await;
    let mut other = Client::logged_in(t.state.clone(), "user0").await;
    let own = other
        .create(&format!(
            "topic_id={}&raw=A+reply+of+another+member+in+the+topic.",
            t.id
        ))
        .await;

    let mut visitor = Client::anonymous(t.state.clone()).await;
    let (status, body) = visitor
        .send(
            Method::DELETE,
            "/posts/destroy_many.json",
            &ids("post_ids", &[t.posts[1]]),
        )
        .await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error_type"], "not_logged_in");

    // A member on others' posts, and on their own next to another's:
    // nothing goes.
    for post_ids in [vec![t.posts[1], t.posts[2]], vec![own, t.posts[3]]] {
        let (status, body) = other
            .send(
                Method::DELETE,
                "/posts/destroy_many.json",
                &ids("post_ids", &post_ids),
            )
            .await;
        assert_eq!(status, 403, "{body}");
        assert_eq!(body["error_type"], "invalid_access");
    }
    // The topic's author on the first post of a topic with replies.
    let mut author = Client::logged_in(t.state.clone(), "user1").await;
    let (status, body) = author
        .send(
            Method::DELETE,
            "/posts/destroy_many.json",
            &ids("post_ids", &[t.posts[0]]),
        )
        .await;
    assert_eq!(status, 403, "{body}");

    for post in t.posts.into_iter().chain([own]) {
        assert_eq!(deleted_by(&db, post).await, None);
    }
    assert_eq!(counts(&db, t.id).await, (5, 5));
}

#[tokio::test]
async fn destroy_many_requires_post_ids_that_find_posts() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let mut admin = Client::logged_in(t.state.clone(), "admin").await;

    for body in ["", "post_ids=", "reply_post_ids[]=1"] {
        let (status, json) = admin
            .send(Method::DELETE, "/posts/destroy_many.json", body)
            .await;
        assert_eq!(status, 400, "{json}");
        assert_eq!(json["errors"], json!([PARAM_MISSING]));
    }
    // No such post, and a post already deleted.
    let (status, _) = admin
        .send(Method::DELETE, &format!("/posts/{}.json", t.posts[3]), "")
        .await;
    assert_eq!(status, 200);
    for post_ids in [vec![999_999], vec![t.posts[3]]] {
        let (status, json) = admin
            .send(
                Method::DELETE,
                "/posts/destroy_many.json",
                &ids("post_ids", &post_ids),
            )
            .await;
        assert_eq!(status, 400, "{json}");
        assert_eq!(json["errors"], json!([INVALID_PARAMS]));
        assert_eq!(json["error_type"], "invalid_parameters");
    }
}

// merge_posts

#[tokio::test]
async fn merge_posts_revises_the_last_post_and_destroys_the_others() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let [op, second, third, fourth] = t.posts;
    let admin_id = user_id(&db, "admin").await;
    let before = post_count(&db, "user2").await;
    let mut admin = Client::logged_in(t.state.clone(), "admin").await;

    let (status, body) = admin
        .send(
            Method::PUT,
            "/posts/merge_posts.json",
            &ids("post_ids", &[fourth, second, third]),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, Value::Null);

    // The post with the highest number survives, holding every raw in
    // post number order.
    let (raw, version, edit_reason, last_editor, author): (String, i32, Option<String>, i32, i32) =
        sqlx::query_as(
            "SELECT raw, version, edit_reason, last_editor_id, user_id FROM posts WHERE id = $1",
        )
        .bind(fourth)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        raw,
        "the first reply of the three here\n\nthe second reply of the three here\n\nthe third reply of the three here"
    );
    assert_eq!(version, 2);
    assert_eq!(
        edit_reason.as_deref(),
        Some("2 posts were merged in by admin")
    );
    assert_eq!(last_editor, admin_id);
    assert_eq!(author, user_id(&db, "user2").await);
    assert_eq!(deleted_by(&db, fourth).await, None);
    let revisions: Vec<(i32, i32, String)> = sqlx::query_as(
        "SELECT number, user_id, modifications FROM post_revisions WHERE post_id = $1 ORDER BY number",
    )
    .bind(fourth)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(revisions.len(), 1);
    assert_eq!((revisions[0].0, revisions[0].1), (2, admin_id));
    assert!(revisions[0].2.contains("raw:"), "{}", revisions[0].2);
    assert!(
        revisions[0].2.contains("2 posts were merged in by admin"),
        "{}",
        revisions[0].2
    );

    assert_eq!(deleted_by(&db, second).await, Some(admin_id));
    assert_eq!(deleted_by(&db, third).await, Some(admin_id));
    assert_eq!(deleted_by(&db, op).await, None);
    assert_eq!(counts(&db, t.id).await, (2, 4));
    assert_eq!(post_count(&db, "user2").await, before - 2);
    let logged: Vec<i32> = sqlx::query_scalar(
        "SELECT post_id FROM user_histories \
         WHERE action = 17 AND acting_user_id = $1 AND post_id = ANY($2) ORDER BY id",
    )
    .bind(admin_id)
    .bind(vec![second, third])
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(logged, vec![second, third]);
}

#[tokio::test]
async fn merge_posts_names_a_single_merged_post_in_the_edit_reason() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let mut admin = Client::logged_in(t.state.clone(), "admin").await;

    let (status, body) = admin
        .send(
            Method::PUT,
            "/posts/merge_posts",
            &ids("post_ids", &[t.posts[1], t.posts[2]]),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    let (raw, edit_reason): (String, Option<String>) =
        sqlx::query_as("SELECT raw, edit_reason FROM posts WHERE id = $1")
            .bind(t.posts[2])
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        raw,
        "the first reply of the three here\n\nthe second reply of the three here"
    );
    assert_eq!(
        edit_reason.as_deref(),
        Some("A post was merged in by admin")
    );
    assert!(deleted_by(&db, t.posts[1]).await.is_some());
    assert_eq!(deleted_by(&db, t.posts[3]).await, None);
}

#[tokio::test]
async fn merge_posts_refuses_posts_of_different_topics_or_users_or_too_long_together() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let (other_topic, _) = new_topic(&t.state, "Another+topic+with+a+reply+of+the+same+user").await;
    let mut replier = Client::logged_in(t.state.clone(), "user2").await;
    let elsewhere = replier
        .create(&format!(
            "topic_id={other_topic}&raw=a+reply+in+the+other+topic+by+the+same+user"
        ))
        .await;
    let mut member = Client::logged_in(t.state.clone(), "user0").await;
    let by_another = member
        .create(&format!(
            "topic_id={}&raw=A+reply+of+another+member+in+the+topic.",
            t.id
        ))
        .await;
    let mut admin = Client::logged_in(t.state.clone(), "admin").await;

    for (post_ids, message) in [
        (
            vec![t.posts[1], elsewhere],
            "Posts belonging to different topics cannot be merged.",
        ),
        (
            vec![t.posts[1], by_another],
            "Posts belonging to different users cannot be merged.",
        ),
    ] {
        let (status, body) = admin
            .send(
                Method::PUT,
                "/posts/merge_posts.json",
                &ids("post_ids", &post_ids),
            )
            .await;
        assert_eq!(status, 422, "{body}");
        assert_eq!(body, json!({ "errors": [message] }));
    }

    set_setting(&db.pool, "max_post_length", 3, "60").await;
    let (status, body) = admin
        .send(
            Method::PUT,
            "/posts/merge_posts.json",
            &ids("post_ids", &[t.posts[1], t.posts[2]]),
        )
        .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(
        body,
        json!({ "errors": ["Posts cannot be merged because the combined post length is more than allowed."] })
    );

    for post in t.posts.into_iter().chain([elsewhere, by_another]) {
        assert_eq!(deleted_by(&db, post).await, None);
    }
    let revised: i64 =
        sqlx::query_scalar("SELECT count(*) FROM post_revisions WHERE post_id = ANY($1)")
            .bind(t.posts.to_vec())
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(revised, 0);
}

#[tokio::test]
async fn merge_posts_refuses_those_who_do_not_moderate_the_topic_and_first_posts() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let merging = ids("post_ids", &[t.posts[1], t.posts[2]]);

    let mut visitor = Client::anonymous(t.state.clone()).await;
    let (status, body) = visitor
        .send(Method::PUT, "/posts/merge_posts.json", &merging)
        .await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error_type"], "not_logged_in");

    // The posts' own author does not moderate the topic.
    let mut author = Client::logged_in(t.state.clone(), "user2").await;
    let (status, body) = author
        .send(Method::PUT, "/posts/merge_posts.json", &merging)
        .await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error_type"], "invalid_access");

    // ensure_can_delete!: a first post cannot be deleted, so not merged.
    let mut topic_author = Client::logged_in(t.state.clone(), "user1").await;
    let own_reply = topic_author
        .create(&format!(
            "topic_id={}&raw=The+topic+author+replying+in+their+own+topic.",
            t.id
        ))
        .await;
    let mut admin = Client::logged_in(t.state.clone(), "admin").await;
    let (status, body) = admin
        .send(
            Method::PUT,
            "/posts/merge_posts.json",
            &ids("post_ids", &[t.posts[0], own_reply]),
        )
        .await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["error_type"], "invalid_access");

    for post in t.posts.into_iter().chain([own_reply]) {
        assert_eq!(deleted_by(&db, post).await, None);
    }
    let raw: String = sqlx::query_scalar("SELECT raw FROM posts WHERE id = $1")
        .bind(t.posts[2])
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(raw, "the second reply of the three here");
}

#[tokio::test]
async fn merge_posts_requires_post_ids_and_leaves_fewer_than_two_posts_alone() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let mut admin = Client::logged_in(t.state.clone(), "admin").await;

    for body in ["", "post_ids="] {
        let (status, json) = admin
            .send(Method::PUT, "/posts/merge_posts.json", body)
            .await;
        assert_eq!(status, 400, "{json}");
        assert_eq!(json["errors"], json!([PARAM_MISSING]));
    }
    // One post, or one post and an id that finds none: nothing to merge.
    for post_ids in [vec![t.posts[1]], vec![t.posts[1], 999_999], vec![999_999]] {
        let (status, json) = admin
            .send(
                Method::PUT,
                "/posts/merge_posts.json",
                &ids("post_ids", &post_ids),
            )
            .await;
        assert_eq!(status, 200, "{json}");
        assert_eq!(json, Value::Null);
    }
    // `posts.pluck(:id) == params[:post_ids]`: a JSON body's numbers, in
    // id order, are the ids found, which Rails refuses.
    let (status, json) = admin
        .send(
            Method::PUT,
            "/posts/merge_posts.json",
            &json!({ "post_ids": [t.posts[1], t.posts[2]] }).to_string(),
        )
        .await;
    assert_eq!(status, 400, "{json}");
    assert_eq!(json["errors"], json!([INVALID_PARAMS]));

    for post in t.posts {
        assert_eq!(deleted_by(&db, post).await, None);
    }
    let revised: i64 =
        sqlx::query_scalar("SELECT count(*) FROM post_revisions WHERE post_id = ANY($1)")
            .bind(t.posts.to_vec())
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(revised, 0);
}

// The other verbs on the collection routes' paths reach the member
// actions, the name their `:id`.
#[tokio::test]
async fn collection_paths_reach_the_member_actions_for_other_verbs() {
    let db = TestDb::new().await;
    let t = setup(&db).await;
    let mut admin = Client::logged_in(t.state.clone(), "admin").await;

    for (method, path) in [
        (Method::GET, "/posts/merge_posts.json"),
        (Method::GET, "/posts/destroy_many.json"),
        (Method::DELETE, "/posts/merge_posts.json"),
    ] {
        let (status, body) = admin.send(method, path, "").await;
        assert_eq!(status, 404, "{path}: {body}");
    }
    // posts#update without its `post` param.
    let (status, body) = admin
        .send(Method::PUT, "/posts/destroy_many.json", "")
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["errors"],
        json!(["param is missing or the value is empty or invalid: post"])
    );
}
