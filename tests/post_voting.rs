//! discourse-post-voting: voting on posts and comments, comments, the
//! topic view's vote order and keys, and the keys on lists, cards, the
//! current user and categories. Cases follow the plugin's own specs
//! (plugins/discourse-post-voting/spec).
//!
//! Topic 35 is made a post voting one: its posts are 35 (#1, user0), 36
//! (#2, user1), 37 (#3, user2), 38 (#4, admin) and 52 (#5, user0).

mod common;

use axum::body::Body;
use axum::http::{Method, Request, header};
use common::{TestDb, bus_messages, config, set_setting, state};
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

    /// The response's status and its body as JSON (null when it isn't).
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

    async fn get(&mut self, path: &str) -> (u16, Value) {
        self.send(Method::GET, path, "").await
    }

    async fn post(&mut self, path: &str, body: &str) -> (u16, Value) {
        self.send(Method::POST, path, body).await
    }
}

/// Post voting on, topic 35 a post voting topic.
async fn setup(db: &TestDb) -> AppState {
    set_setting(&db.pool, "post_voting_enabled", 5, "t").await;
    sqlx::query("UPDATE topics SET subtype = 'question_answer' WHERE id = 35")
        .execute(&db.pool)
        .await
        .unwrap();
    state(db.pool.clone(), config(RailsEnv::Test, &[])).await
}

async fn scalar_i64(db: &TestDb, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&db.pool).await.unwrap()
}

/// A comment by `user_id` on `post_id`, as Fabricate(:post_voting_comment).
async fn comment(db: &TestDb, post_id: i32, user_id: i32, raw: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO post_voting_comments (post_id, user_id, raw, cooked, cooked_version, \
           created_at, updated_at, last_editor_id) \
         VALUES ($1, $2, $3, '<p>' || $3 || '</p>', 1, clock_timestamp(), clock_timestamp(), $2) RETURNING id",
    )
    .bind(post_id)
    .bind(user_id)
    .bind(raw)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn votes_count_up_and_down_and_are_published() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;

    let from = st.bus.now().await.unwrap();
    let (status, body) = user1.post("/post_voting/vote.json", "post_id=37").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, serde_json::json!({"success": "OK"}));
    assert_eq!(
        scalar_i64(&db, "SELECT qa_vote_count::bigint FROM posts WHERE id = 37").await,
        1
    );
    let messages = bus_messages(&st, from, &["/topic/35"]).await;
    let voted = messages
        .iter()
        .find(|m| m.data["type"] == "post_voting_post_voted")
        .expect("the vote published");
    assert_eq!(voted.data["id"], 37);
    assert_eq!(voted.data["post_voting_user_voted_id"], 3);
    assert_eq!(voted.data["post_voting_vote_count"], 1);
    assert_eq!(voted.data["post_voting_user_voted_direction"], "up");
    assert_eq!(voted.data["post_voting_has_votes"], true);

    // A second upvote is refused.
    let (status, body) = user1.post("/post_voting/vote.json", "post_id=37").await;
    assert_eq!(status, 403);
    assert_eq!(body["errors"][0], "You can only vote once for each post.");

    // Changing it to a downvote moves the count by two.
    let mut user0 = Client::logged_in(st.clone(), "user0").await;
    let (status, _) = user0.post("/post_voting/vote.json", "post_id=37").await;
    assert_eq!(status, 200);
    let (status, _) = user1
        .post("/post_voting/vote.json", "post_id=37&direction=down")
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        scalar_i64(&db, "SELECT qa_vote_count::bigint FROM posts WHERE id = 37").await,
        0
    );
    assert_eq!(
        scalar_i64(
            &db,
            "SELECT count(*) FROM post_voting_votes WHERE votable_id = 37 AND user_id = 3 AND direction = 'down'"
        )
        .await,
        1
    );

    // Removing the downvote puts it back.
    let (status, _) = user1
        .send(Method::DELETE, "/post_voting/vote.json?post_id=37", "")
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        scalar_i64(&db, "SELECT qa_vote_count::bigint FROM posts WHERE id = 37").await,
        1
    );
}

#[tokio::test]
async fn votes_are_refused_where_rails_refuses_them() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let mut user2 = Client::logged_in(st.clone(), "user2").await;

    // One's own post.
    let (status, body) = user2.post("/post_voting/vote.json", "post_id=37").await;
    assert_eq!(status, 403);
    assert_eq!(
        body["errors"][0],
        "You are not allowed to vote on your own post or comment."
    );
    // No post_id at all, or one that doesn't exist.
    let (status, body) = user2.post("/post_voting/vote.json", "").await;
    assert_eq!(status, 400);
    assert_eq!(
        body["errors"][0],
        "param is missing or the value is empty or invalid: post_id"
    );
    let (status, _) = user2.post("/post_voting/vote.json", "post_id=-1").await;
    assert_eq!(status, 404);
    // Never voted in the topic.
    let (status, body) = user2
        .send(Method::DELETE, "/post_voting/vote.json?post_id=36", "")
        .await;
    assert_eq!(status, 403);
    assert_eq!(body["errors"][0], "User has not voted.");

    // Archived, then closed topics.
    sqlx::query("UPDATE topics SET archived = TRUE WHERE id = 35")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body) = user2.post("/post_voting/vote.json", "post_id=36").await;
    assert_eq!(status, 403);
    assert_eq!(body["errors"][0], "You cannot vote on archived topics.");
    sqlx::query("UPDATE topics SET archived = FALSE, closed = TRUE WHERE id = 35")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body) = user2.post("/post_voting/vote.json", "post_id=36").await;
    assert_eq!(status, 403);
    assert_eq!(body["errors"][0], "You cannot vote on closed topics.");

    // A topic the voter can't see (Staff is read restricted).
    sqlx::query("UPDATE topics SET closed = FALSE, category_id = 3 WHERE id = 35")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body) = user2.post("/post_voting/vote.json", "post_id=36").await;
    assert_eq!(status, 403);
    assert_eq!(body["error_type"], "invalid_access");

    // A topic that isn't a post voting one.
    let (status, _) = user2.post("/post_voting/vote.json", "post_id=1").await;
    assert_eq!(status, 403);

    // Anonymously, and with the plugin off.
    let mut anonymous = Client::anonymous(st.clone()).await;
    let (status, body) = anonymous.post("/post_voting/vote.json", "post_id=36").await;
    assert_eq!(status, 403);
    assert_eq!(body["error_type"], "not_logged_in");
    set_setting(&db.pool, "post_voting_enabled", 5, "f").await;
    let (status, _) = user2.post("/post_voting/vote.json", "post_id=36").await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn a_vote_can_be_undone_only_within_the_window() {
    let db = TestDb::with_clock().await;
    let st = setup(&db).await;
    set_setting(&db.pool, "post_voting_undo_vote_action_window", 3, "1").await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, _) = user1.post("/post_voting/vote.json", "post_id=37").await;
    assert_eq!(status, 200);

    db.shift_clock(chrono::Duration::minutes(2)).await;
    let (status, body) = user1
        .post("/post_voting/vote.json", "post_id=37&direction=down")
        .await;
    assert_eq!(status, 403);
    assert_eq!(
        body["errors"][0],
        "You cannot undo vote 1 minute after voting."
    );
    assert_eq!(body["error_type"], "invalid_access");
    let (status, body) = user1
        .send(Method::DELETE, "/post_voting/vote.json?post_id=37", "")
        .await;
    assert_eq!(status, 403);
    // render_json_error: no error type.
    assert_eq!(
        body,
        serde_json::json!({"errors": ["You cannot undo vote 1 minute after voting."]})
    );
}

#[tokio::test]
async fn voters_are_listed_newest_first_with_their_direction() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    for (user, direction, at) in [(3, "down", "2026-10-01"), (4, "up", "2026-10-02")] {
        sqlx::query(
            "INSERT INTO post_voting_votes (user_id, created_at, direction, votable_type, votable_id) \
             VALUES ($1, $2::timestamp, $3, 'Post', 52)",
        )
        .bind(user)
        .bind(at)
        .bind(direction)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    let mut anonymous = Client::anonymous(st.clone()).await;
    let (status, _) = anonymous.get("/post_voting/voters.json?post_id=52").await;
    assert_eq!(status, 403);

    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, _) = user1.get("/post_voting/voters.json?post_id=-1").await;
    assert_eq!(status, 404);
    let (status, body) = user1.get("/post_voting/voters.json?post_id=52").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["total_voters_count"], 2);
    let voters = body["voters"].as_array().unwrap();
    assert_eq!(voters[0]["id"], 4);
    assert_eq!(voters[0]["username"], "user2");
    assert_eq!(voters[0]["direction"], "up");
    assert_eq!(voters[1]["id"], 3);
    assert_eq!(voters[1]["direction"], "down");
    let keys: Vec<&str> = voters[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        ["id", "username", "name", "avatar_template", "direction"]
    );
}

#[tokio::test]
async fn comments_can_be_upvoted_once_and_unvoted() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let theirs = comment(&db, 37, 4, "a comment by user2 here").await;
    let mine = comment(&db, 37, 3, "a comment by user1 here").await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;

    let (status, _) = user1
        .post("/post_voting/vote/comment.json", "comment_id=-999")
        .await;
    assert_eq!(status, 404);
    let (status, _) = user1
        .post(
            "/post_voting/vote/comment.json",
            &format!("comment_id={mine}"),
        )
        .await;
    assert_eq!(status, 403);
    let (status, body) = user1
        .send(
            Method::DELETE,
            &format!("/post_voting/vote/comment.json?comment_id={theirs}"),
            "",
        )
        .await;
    assert_eq!(status, 403);
    assert_eq!(body["errors"][0], "User has not voted.");

    let (status, _) = user1
        .post(
            "/post_voting/vote/comment.json",
            &format!("comment_id={theirs}"),
        )
        .await;
    assert_eq!(status, 200);
    let count =
        format!("SELECT qa_vote_count::bigint FROM post_voting_comments WHERE id = {theirs}");
    assert_eq!(scalar_i64(&db, &count).await, 1);
    let (status, _) = user1
        .send(
            Method::DELETE,
            &format!("/post_voting/vote/comment.json?comment_id={theirs}"),
            "",
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(scalar_i64(&db, &count).await, 0);
}

#[tokio::test]
async fn a_comment_is_cooked_notified_and_published() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    sqlx::query("UPDATE users SET last_seen_at = now() WHERE id = 4")
        .execute(&db.pool)
        .await
        .unwrap();
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let from = st.bus.now().await.unwrap();
    let (status, body) = user1
        .post(
            "/post_voting/comments.json",
            "post_id=37&raw=this+is+some+content",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let id = body["id"].as_i64().unwrap();
    let keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "id",
            "user_id",
            "name",
            "username",
            "created_at",
            "raw",
            "cooked",
            "post_voting_vote_count",
            "user_voted",
            "available_flags",
            "reviewable_id"
        ]
    );
    assert_eq!(body["username"], "user1");
    assert_eq!(body["cooked"], "<p>this is some content</p>");
    assert_eq!(body["post_voting_vote_count"], 0);
    assert_eq!(body["user_voted"], false);
    assert_eq!(body["available_flags"], serde_json::json!([]));

    // The post's author is told.
    let (kind, user_id, post_number, data): (i32, i32, i32, String) = sqlx::query_as(
        "SELECT notification_type, user_id, post_number, data FROM notifications \
         WHERE topic_id = 35 ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((kind, user_id, post_number), (35, 4, 3));
    assert_eq!(
        data,
        format!(r#"{{"post_voting_comment_id":{id},"display_username":"user1"}}"#)
    );
    let messages = bus_messages(&st, from, &["/topic/35", "/notification-alert/4"]).await;
    let commented = messages
        .iter()
        .find(|m| m.data["type"] == "post_voting_post_commented")
        .expect("the comment published");
    assert_eq!(commented.data["comment"]["id"], id);
    assert_eq!(commented.data["comment"]["user_voted"], false);
    assert_eq!(commented.data["comments_count"], 1);
    let alert = messages
        .iter()
        .find(|m| m.channel == "/notification-alert/4")
        .expect("the alert");
    assert_eq!(alert.data["notification_type"], 35);
    assert_eq!(alert.data["username"], "user1");

    // Commenting on one's own post tells nobody.
    let mut user2 = Client::logged_in(st.clone(), "user2").await;
    let before = scalar_i64(&db, "SELECT count(*) FROM notifications").await;
    let (status, _) = user2
        .post(
            "/post_voting/comments.json",
            "post_id=37&raw=my+own+comment+on+my+post",
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        scalar_i64(&db, "SELECT count(*) FROM notifications").await,
        before
    );
}

#[tokio::test]
async fn comments_are_validated_as_the_model_does() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;

    // Too short for min_post_length.
    set_setting(&db.pool, "min_post_length", 3, "5").await;
    let (status, body) = user1
        .post("/post_voting/comments.json", "post_id=37&raw=1234")
        .await;
    assert_eq!(status, 403);
    assert_eq!(
        body["errors"],
        serde_json::json!(["Raw is too short (minimum is 5 characters)"])
    );
    set_setting(&db.pool, "min_post_length", 3, "20").await;
    // All caps fails TextSentinel.
    let (status, body) = user1
        .post(
            "/post_voting/comments.json",
            "post_id=37&raw=THIS+IS+ALL+CAPS+STRING+HERE",
        )
        .await;
    assert_eq!(status, 403);
    assert_eq!(
        body["errors"],
        serde_json::json!(["Raw seems unclear, is it a complete sentence?"])
    );
    // Longer than post_voting_comment_max_raw_length.
    set_setting(&db.pool, "post_voting_comment_max_raw_length", 3, "21").await;
    let (status, body) = user1
        .post(
            "/post_voting/comments.json",
            "post_id=37&raw=this+string+is+too+long",
        )
        .await;
    assert_eq!(status, 403);
    assert_eq!(
        body["errors"],
        serde_json::json!(["Raw is limited to 21 characters; you entered 23."])
    );
    set_setting(&db.pool, "post_voting_comment_max_raw_length", 3, "600").await;
    // A blocked watched word.
    sqlx::query(
        "INSERT INTO watched_words (word, action, created_at, updated_at) VALUES ('kittens', 1, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let (status, body) = user1
        .post(
            "/post_voting/comments.json",
            "post_id=37&raw=this+one+mentions+kittens",
        )
        .await;
    assert_eq!(status, 403);
    assert_eq!(
        body["errors"][0],
        "Sorry, you can't post the word 'kittens'; it's not allowed."
    );
    // The per post limit.
    set_setting(&db.pool, "post_voting_comment_limit_per_post", 3, "1").await;
    comment(&db, 37, 4, "the one comment allowed").await;
    let (status, body) = user1
        .post(
            "/post_voting/comments.json",
            "post_id=37&raw=one+comment+too+many+here",
        )
        .await;
    assert_eq!(status, 403);
    assert_eq!(
        body["errors"],
        serde_json::json!(["Limit of 1 comments per post has been reached."])
    );
    // Comments off.
    set_setting(&db.pool, "post_voting_comment_enabled", 5, "f").await;
    let (status, body) = user1
        .post(
            "/post_voting/comments.json",
            "post_id=36&raw=comments+are+turned+off",
        )
        .await;
    assert_eq!(status, 403);
    assert_eq!(
        body["errors"],
        serde_json::json!(["Commenting on a post is not permitted."])
    );
    // A post outside a post voting topic, and one that doesn't exist.
    let (status, _) = user1
        .post(
            "/post_voting/comments.json",
            "post_id=1&raw=not+post+voting+here",
        )
        .await;
    assert_eq!(status, 403);
    let (status, _) = user1
        .post(
            "/post_voting/comments.json",
            "post_id=-999999&raw=this+is+some+content",
        )
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn comments_cook_with_their_few_rules() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    set_setting(&db.pool, "min_post_length", 3, "1").await;
    sqlx::query(
        "INSERT INTO watched_words (word, action, created_at, updated_at) VALUES ('testing', 2, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    for (raw, cooked) in [
        ("**bold**", "<p><strong>bold</strong></p>"),
        ("`test`", "<p><code>test</code></p>"),
        (
            "[test link](https://www.example.com)",
            "<p><a href=\"https://www.example.com\" rel=\"noopener nofollow ugc\">test link</a></p>",
        ),
        (
            "https://www.example.com",
            "<p><a href=\"https://www.example.com\" rel=\"noopener nofollow ugc\">https://www.example.com</a></p>",
        ),
        (
            ":grin: abcde",
            "<p><img src=\"/images/emoji/twitter/grin.png?v=15\" title=\":grin:\" class=\"emoji\" alt=\":grin:\" loading=\"lazy\" width=\"20\" height=\"20\"> abcde</p>",
        ),
        ("testing", "<p>■■■■■■■</p>"),
        ("line 1\n\nline 2\n", "<p>line 1 line 2</p>"),
        // No headings, lists or raw HTML: the rules aren't enabled.
        ("# not a heading", "<p># not a heading</p>"),
        ("<b>bold?</b>", "<p>&lt;b&gt;bold?&lt;/b&gt;</p>"),
    ] {
        let body = format!(
            "post_id=37&raw={}",
            form_urlencoded::byte_serialize(raw.as_bytes()).collect::<String>()
        );
        let (status, json) = user1.post("/post_voting/comments.json", &body).await;
        assert_eq!(status, 200, "{raw}: {json}");
        assert_eq!(json["cooked"], cooked, "{raw}");
    }
    // The raw keeps its newlines run together.
    let raw: String = sqlx::query_scalar(
        "SELECT raw FROM post_voting_comments WHERE cooked = '<p>line 1 line 2</p>'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(raw, "line 1 line 2 ");
}

#[tokio::test]
async fn more_comments_load_after_the_last_one() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let first = comment(&db, 37, 4, "first comment on this").await;
    let second = comment(&db, 37, 5, "second comment on this").await;
    let third = comment(&db, 37, 1, "third comment on this").await;
    let mut anonymous = Client::anonymous(st.clone()).await;
    let (status, body) = anonymous
        .get(&format!(
            "/post_voting/comments.json?post_id=37&last_comment_id={first}"
        ))
        .await;
    assert_eq!(status, 200, "{body}");
    let ids: Vec<i64> = body["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, [second, third]);
    assert_eq!(body["comments"][0]["username"], "user3");
    assert_eq!(body["comments"][0]["name"], "User3");
    assert_eq!(body["comments"][0]["user_voted"], false);

    let (status, body) = anonymous.get("/post_voting/comments.json?post_id=37").await;
    assert_eq!(status, 400, "{body}");
    let (status, _) = anonymous
        .get(&format!(
            "/post_voting/comments.json?post_id=-999999&last_comment_id={first}"
        ))
        .await;
    assert_eq!(status, 404);

    set_setting(&db.pool, "enable_names", 5, "f").await;
    let (_, body) = anonymous
        .get(&format!(
            "/post_voting/comments.json?post_id=37&last_comment_id={first}"
        ))
        .await;
    assert!(
        body["comments"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| !c.as_object().unwrap().contains_key("name"))
    );
}

#[tokio::test]
async fn comments_are_edited_and_trashed_by_their_author_or_staff() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let id = comment(&db, 37, 3, "a comment by user1 to edit").await;
    comment(&db, 37, 4, "another comment on the post").await;
    comment(&db, 37, 5, "and a third comment as well").await;

    let mut anonymous = Client::anonymous(st.clone()).await;
    let (status, _) = anonymous
        .send(
            Method::PUT,
            "/post_voting/comments.json",
            &format!("comment_id={id}&raw=this+is+some+new+raw"),
        )
        .await;
    assert_eq!(status, 403);

    let mut user2 = Client::logged_in(st.clone(), "user2").await;
    let (status, _) = user2
        .send(
            Method::PUT,
            "/post_voting/comments.json",
            &format!("comment_id={id}&raw=this+is+some+new+raw"),
        )
        .await;
    assert_eq!(status, 403);

    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, _) = user1
        .send(
            Method::PUT,
            "/post_voting/comments.json",
            "comment_id=-999999&raw=this+is+some+new+raw",
        )
        .await;
    assert_eq!(status, 404);
    let from = st.bus.now().await.unwrap();
    let (status, body) = user1
        .send(
            Method::PUT,
            "/post_voting/comments.json",
            &format!("comment_id={id}&raw=this+is+some+new+raw"),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["raw"], "this is some new raw");
    assert_eq!(body["cooked"], "<p>this is some new raw</p>");
    let messages = bus_messages(&st, from, &["/topic/35"]).await;
    let edited = messages
        .iter()
        .find(|m| m.data["type"] == "post_voting_post_comment_edited")
        .expect("the edit published");
    assert_eq!(edited.data["id"], 37);
    assert_eq!(edited.data["comment_id"], id);
    assert_eq!(edited.data["comment_raw"], "this is some new raw");
    assert_eq!(edited.data["comment_cooked"], "<p>this is some new raw</p>");

    // An admin may edit it too.
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, body) = admin
        .send(
            Method::PUT,
            "/post_voting/comments.json",
            &format!("comment_id={id}&raw=this+is+the+admin+edit"),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(body["raw"], "this is the admin edit");

    // Trashing: others can't, its author can.
    let (status, _) = user2
        .send(
            Method::DELETE,
            &format!("/post_voting/comments.json?comment_id={id}"),
            "",
        )
        .await;
    assert_eq!(status, 403);
    let from = st.bus.now().await.unwrap();
    let (status, body) = user1
        .send(
            Method::DELETE,
            &format!("/post_voting/comments.json?comment_id={id}"),
            "",
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(body, serde_json::json!({"success": "OK"}));
    assert_eq!(
        scalar_i64(
            &db,
            &format!("SELECT count(*) FROM post_voting_comments WHERE id = {id} AND deleted_at IS NOT NULL")
        )
        .await,
        1
    );
    let messages = bus_messages(&st, from, &["/topic/35"]).await;
    let trashed = messages
        .iter()
        .find(|m| m.data["type"] == "post_voting_post_comment_trashed")
        .expect("the trash published");
    assert_eq!(trashed.data["comment_id"], id);
    assert_eq!(trashed.data["comments_count"], 2);
    // Gone now.
    let (status, _) = user1
        .send(
            Method::DELETE,
            &format!("/post_voting/comments.json?comment_id={id}"),
            "",
        )
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn the_topic_view_orders_answers_by_votes() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    // #5 up two, #3 up one, #2 down one, #4 untouched; a reply to #2 and
    // a whisper leave the stream.
    for (post, count) in [(52, 2), (37, 1), (36, -1)] {
        sqlx::query("UPDATE posts SET qa_vote_count = $2 WHERE id = $1")
            .bind(post)
            .bind(count)
            .execute(&db.pool)
            .await
            .unwrap();
    }
    sqlx::query(
        "INSERT INTO post_voting_votes (user_id, created_at, direction, votable_type, votable_id) \
         VALUES (3, now(), 'up', 'Post', 52), (3, now(), 'down', 'Post', 37)",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, body) = user1.get("/t/35.json").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["post_stream"]["stream"],
        serde_json::json!([35, 52, 37, 38, 36])
    );
    let numbers: Vec<i64> = body["post_stream"]["posts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["post_number"].as_i64().unwrap())
        .collect();
    assert_eq!(numbers, [1, 5, 3, 4, 2]);
    // The last post of the vote order is the "highest".
    assert_eq!(body["highest_post_number"], 2);
    assert_eq!(body["is_post_voting"], true);
    assert_eq!(body["answer_count"], 4);
    assert_eq!(body["last_answer_post_number"], 5);
    assert_eq!(body["last_answerer"]["username"], "user0");
    assert_eq!(body["last_commented_on"], Value::Null);
    let post = &body["post_stream"]["posts"][1];
    assert_eq!(post["post_voting_vote_count"], 2);
    assert_eq!(post["post_voting_user_voted_direction"], "up");
    assert_eq!(post["post_voting_has_votes"], true);
    assert_eq!(post["comments"], serde_json::json!([]));
    assert_eq!(post["comments_count"], 0);
    assert_eq!(
        body["post_stream"]["posts"][2]["post_voting_user_voted_direction"],
        "down"
    );
    assert_eq!(
        body["post_stream"]["posts"][3]["post_voting_user_voted_direction"],
        Value::Null
    );
    assert_eq!(
        body["post_stream"]["posts"][3]["post_voting_has_votes"],
        false
    );

    // A reply to an answer leaves the stream; the activity filter keeps
    // post order (and what isn't an answer stays out).
    sqlx::query("UPDATE posts SET reply_to_post_number = 2 WHERE id = 38")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, body) = user1.get("/t/35.json").await;
    assert_eq!(
        body["post_stream"]["stream"],
        serde_json::json!([35, 52, 37, 36])
    );
    let (_, body) = user1.get("/t/35.json?filter=activity").await;
    assert_eq!(
        body["post_stream"]["stream"],
        serde_json::json!([35, 36, 37, 52])
    );
    assert_eq!(body["highest_post_number"], 5);

    // Someone who voted on nothing gets no direction key.
    let mut user2 = Client::logged_in(st.clone(), "user2").await;
    let (_, body) = user2.get("/t/35.json").await;
    assert!(
        !body["post_stream"]["posts"][1]
            .as_object()
            .unwrap()
            .contains_key("post_voting_user_voted_direction")
    );
}

#[tokio::test]
async fn whispers_and_small_actions_stay_out_of_the_vote_order() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    set_setting(&db.pool, "whispers_allowed_groups", 20, "1").await;
    // #3 a small action, #4 a whisper.
    sqlx::query("UPDATE posts SET post_type = 3 WHERE id = 37")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE posts SET post_type = 4 WHERE id = 38")
        .execute(&db.pool)
        .await
        .unwrap();
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (_, body) = admin.get("/t/35.json").await;
    assert_eq!(
        body["post_stream"]["stream"],
        serde_json::json!([35, 36, 52])
    );
    let (_, body) = admin.get("/t/35.json?filter=activity").await;
    assert_eq!(
        body["post_stream"]["stream"],
        serde_json::json!([35, 36, 37, 38, 52])
    );
}

#[tokio::test]
async fn the_topic_view_preloads_each_posts_first_comments() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let mut ids = Vec::new();
    for n in 0..7 {
        ids.push(comment(&db, 37, 4, &format!("comment number {n} on this answer")).await);
    }
    // A trashed one counts for nothing.
    sqlx::query("UPDATE post_voting_comments SET deleted_at = now() WHERE id = $1")
        .bind(ids[0])
        .execute(&db.pool)
        .await
        .unwrap();
    let on_question = comment(&db, 35, 4, "a comment on the question").await;
    sqlx::query(
        "INSERT INTO post_voting_votes (user_id, created_at, direction, votable_type, votable_id) \
         VALUES (3, now(), 'up', 'PostVotingComment', $1)",
    )
    .bind(ids[1])
    .execute(&db.pool)
    .await
    .unwrap();
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (_, body) = user1.get("/t/35.json").await;
    let posts = body["post_stream"]["posts"].as_array().unwrap();
    let answer = posts.iter().find(|p| p["id"] == 37).unwrap();
    let shown: Vec<i64> = answer["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_i64().unwrap())
        .collect();
    assert_eq!(shown, ids[1..6]);
    assert_eq!(answer["comments_count"], 6);
    assert_eq!(answer["comments"][0]["user_voted"], true);
    // comments_user_voted[id]: nil, not false, for the others.
    assert_eq!(answer["comments"][1]["user_voted"], Value::Null);
    // user1 may flag user2's comments.
    assert!(
        !answer["comments"][1]["available_flags"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let question = posts.iter().find(|p| p["id"] == 35).unwrap();
    assert_eq!(question["comments"][0]["id"], on_question);
    assert!(body["last_commented_on"].is_string());
}

#[tokio::test]
async fn lists_cards_the_current_user_and_categories_carry_the_keys() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    sqlx::query("UPDATE posts SET qa_vote_count = 3 WHERE id = 37")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO category_custom_fields (category_id, name, value, created_at, updated_at) \
         VALUES (4, 'create_as_post_voting_default', 'true', now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let mut user1 = Client::logged_in(st.clone(), "user1").await;

    let (_, body) = user1.get("/latest.json").await;
    let topics = body["topic_list"]["topics"].as_array().unwrap();
    let voting = topics.iter().find(|t| t["id"] == 35).unwrap();
    assert_eq!(voting["is_post_voting"], true);
    assert!(
        topics
            .iter()
            .filter(|t| t["id"] != 35)
            .all(|t| !t.as_object().unwrap().contains_key("is_post_voting"))
    );

    // Read through #2 of 5: the list links to #3 sorted by activity.
    sqlx::query(
        "INSERT INTO topic_users (user_id, topic_id, last_read_post_number, notification_level) \
         VALUES (3, 35, 2, 1) ON CONFLICT (user_id, topic_id) DO UPDATE SET last_read_post_number = 2",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let html = user1.page("/latest").await;
    assert!(
        html.contains("href=\"/t/parity-fixture-replies-and-posters/35/3?filter=activity\""),
        "the last unread link"
    );

    let (_, body) = user1.get("/u/user2/card.json").await;
    assert_eq!(body["user"]["vote_count"], 3);

    let (_, body) = user1.get("/session/current.json").await;
    assert_eq!(body["current_user"]["can_flag_post_voting_comments"], true);

    let (_, body) = user1.get("/site.json").await;
    let categories = body["categories"].as_array().unwrap();
    let general = categories.iter().find(|c| c["id"] == 4).unwrap();
    assert_eq!(general["create_as_post_voting_default"], true);
    assert_eq!(general["only_post_voting_in_this_category"], Value::Null);

    // Off, none of it.
    set_setting(&db.pool, "post_voting_enabled", 5, "f").await;
    let (_, body) = user1.get("/t/35.json").await;
    assert!(!body.as_object().unwrap().contains_key("is_post_voting"));
    let (_, body) = user1.get("/u/user2/card.json").await;
    assert!(!body["user"].as_object().unwrap().contains_key("vote_count"));
}

#[tokio::test]
async fn new_topics_can_be_post_voting_and_replies_only_answer() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let mut user3 = Client::logged_in(st.clone(), "user3").await;
    let (status, body) = user3
        .post(
            "/posts.json",
            "title=How+do+I+frobnicate+the+widgets&raw=I+have+tried+everything+and+nothing+works+so+far.&category=4&create_as_post_voting=true",
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let topic_id = body["topic_id"].as_i64().unwrap();
    let subtype: Option<String> = sqlx::query_scalar("SELECT subtype FROM topics WHERE id = $1")
        .bind(topic_id as i32)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(subtype.as_deref(), Some("question_answer"));

    // A reply to an answer is refused; to the question it's fine.
    let (status, body) = user3
        .post(
            "/posts.json",
            "topic_id=35&reply_to_post_number=2&raw=This+replies+to+the+second+post+here.",
        )
        .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(
        body["errors"],
        serde_json::json!(["You are not allowed to create a post in reply to another post."])
    );
    let (status, body) = user3
        .post(
            "/posts.json",
            "topic_id=35&reply_to_post_number=1&raw=This+replies+to+the+question+itself.",
        )
        .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn deleting_a_users_content_takes_their_comments_and_votes() {
    let db = TestDb::new().await;
    let _st = setup(&db).await;
    let theirs = comment(&db, 37, 4, "a comment user1 votes on").await;
    let mine = comment(&db, 36, 3, "user1's own comment here").await;
    sqlx::query("UPDATE posts SET qa_vote_count = -2 WHERE id = 37")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE post_voting_comments SET qa_vote_count = 2 WHERE id = $1")
        .bind(theirs)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO post_voting_votes (user_id, created_at, direction, votable_type, votable_id) VALUES \
         (3, now(), 'down', 'Post', 37), (5, now(), 'down', 'Post', 37), \
         (3, now(), 'up', 'PostVotingComment', $1), (5, now(), 'up', 'PostVotingComment', $1)",
    )
    .bind(theirs)
    .execute(&db.pool)
    .await
    .unwrap();
    let mut conn = db.pool.acquire().await.unwrap();
    discourse_rs::plugins::post_voting::lifecycle::user_content_deleting(&mut conn, 3)
        .await
        .unwrap();
    assert_eq!(
        scalar_i64(&db, "SELECT qa_vote_count::bigint FROM posts WHERE id = 37").await,
        -1
    );
    assert_eq!(
        scalar_i64(
            &db,
            &format!("SELECT qa_vote_count::bigint FROM post_voting_comments WHERE id = {theirs}")
        )
        .await,
        1
    );
    assert_eq!(
        scalar_i64(
            &db,
            "SELECT count(*) FROM post_voting_votes WHERE user_id = 3"
        )
        .await,
        0
    );
    assert_eq!(
        scalar_i64(&db, "SELECT count(*) FROM post_voting_votes").await,
        2
    );
    assert_eq!(
        scalar_i64(
            &db,
            &format!("SELECT count(*) FROM post_voting_comments WHERE id = {mine}")
        )
        .await,
        0
    );
}

impl Client {
    /// The page as a browser asks for it.
    async fn page(&mut self, path: &str) -> String {
        let mut request = Request::get(path).header(header::HOST, "test.localhost");
        let cookie: Vec<String> = self
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        request = request.header(header::COOKIE, cookie.join("; "));
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

#[tokio::test]
async fn the_topic_page_draws_votes_comments_and_the_answers_header() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    sqlx::query("UPDATE posts SET qa_vote_count = 2 WHERE id = 52")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO post_voting_votes (user_id, created_at, direction, votable_type, votable_id) \
         VALUES (3, now(), 'up', 'Post', 52)",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let mine = comment(&db, 52, 3, "a comment by user1 here").await;
    let theirs = comment(&db, 52, 4, "a comment by user2 here").await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let html = user1.page("/t/parity-fixture-replies-and-posters/35").await;

    assert!(html.contains("post-voting-topic"), "the body class");
    // The header before the first answer by votes (#5), with the votes
    // sort active.
    let header = html
        .split("<nav aria-label=\"Sort by\" class=\"post-voting-answers-header\">")
        .nth(1)
        .expect("the answers header");
    assert!(header.starts_with("<h5 class=\"post-voting-answers-header__count\">4 Answers</h5>"));
    assert!(header.contains(
        "<li><a aria-current=\"true\" class=\"--votes active\" href=\"/t/parity-fixture-replies-and-posters/35\">Votes</a></li>"
    ));
    assert!(header.contains(
        "<li><a class=\"--activity\" href=\"/t/parity-fixture-replies-and-posters/35?filter=activity\">Activity</a></li>"
    ));
    let next_article = &header[header.find("<article").unwrap()..];
    assert!(
        next_article.contains("data-post-id=\"52\" data-user-id=\"2\" id=\"post_5\""),
        "the header goes before post #5's article"
    );
    // #5's votes: user1 voted up; it has votes, so the count opens them.
    let controls = html
        .split("<div class=\"post-voting-post\" data-post-id=\"52\">")
        .nth(1)
        .expect("the vote controls");
    assert!(controls.starts_with(
        "<button aria-label=\"Remove upvote\" class=\"btn no-text btn-icon btn-transparent post-voting-button --upvote --voted\""
    ));
    assert!(controls.contains(
        "class=\"btn fk-d-menu__trigger post-voting-post__toggle-voters btn-transparent\""
    ));
    // The question has no votes: a plain count.
    assert!(html.contains(
        "<div class=\"post-voting-post\" data-post-id=\"35\"><button aria-label=\"Upvote\""
    ));
    assert!(html.contains("<span class=\"post-voting-post__toggle-voters\">0</span>"));
    // Comments: user1 may edit theirs, not user2's.
    let own = html
        .split(&format!("id=\"post-voting-comment-{mine}\""))
        .nth(1)
        .unwrap();
    let own = &own[..own.find("</div></div>").unwrap()];
    assert!(own.contains("post-voting-comments__comment-actions-edit-link"));
    assert!(own.contains("<a class=\"post-voting-comments__username\" data-user-card=\"user1\""));
    let other = html
        .split(&format!("id=\"post-voting-comment-{theirs}\""))
        .nth(1)
        .unwrap();
    let other = &other[..other.find("</div></div>").unwrap()];
    assert!(!other.contains("post-voting-comments__comment-actions"));
    assert!(html.contains("post-voting-comments__actions-add"));
    // The question's Answer button in place of Reply; no likes on answers.
    assert!(html.contains("post-action-menu__post-voting-answer reply create fade-out btn-flat"));
    assert!(!html.contains("post-action-menu__reply"));
    let answer = html.split("id=\"post_5\"").nth(1).unwrap();
    let answer = &answer[..answer.find("</article>").unwrap()];
    assert!(!answer.contains("post-action-menu__like"));
    assert!(html.contains("/assets/post-voting.js"));

    // By activity: the other link is current, and the body says so.
    let html = user1
        .page("/t/parity-fixture-replies-and-posters/35?filter=activity")
        .await;
    assert!(html.contains("post-voting-topic --sort-by-activity"));
    assert!(html.contains(
        "<li><a aria-current=\"true\" class=\"--activity active\" href=\"/t/parity-fixture-replies-and-posters/35?filter=activity\">Activity</a></li>"
    ));

    // Off, none of it.
    set_setting(&db.pool, "post_voting_enabled", 5, "f").await;
    let html = user1.page("/t/parity-fixture-replies-and-posters/35").await;
    assert!(!html.contains("post-voting-post"));
    assert!(!html.contains("post-voting.js"));
}

#[tokio::test]
async fn flagging_a_comment_is_checked_before_it_is_refused() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    let id = comment(&db, 37, 4, "a comment by user2 to flag").await;
    // off_topic is flag 3.
    let mut anonymous = Client::anonymous(st.clone()).await;
    let (status, body) = anonymous
        .send(
            Method::PUT,
            "/post_voting/comments/flag.json",
            &format!("comment_id={id}&flag_type_id=3"),
        )
        .await;
    assert_eq!(status, 403);
    assert_eq!(body["error_type"], "not_logged_in");

    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, _) = user1
        .send(
            Method::PUT,
            "/post_voting/comments/flag.json",
            "comment_id=-1&flag_type_id=3",
        )
        .await;
    assert_eq!(status, 404);
    let (status, body) = user1
        .send(
            Method::PUT,
            "/post_voting/comments/flag.json",
            &format!("comment_id={id}&flag_type_id=999"),
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(body["error_type"], "invalid_parameters");
    // A comment of a post the flagger can't see.
    sqlx::query("UPDATE topics SET category_id = 3 WHERE id = 35")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body) = user1
        .send(
            Method::PUT,
            "/post_voting/comments/flag.json",
            &format!("comment_id={id}&flag_type_id=3"),
        )
        .await;
    assert_eq!(status, 403);
    assert_eq!(body["error_type"], "invalid_access");
}

#[tokio::test]
async fn the_category_editor_sets_the_post_voting_defaults() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    sqlx::query(
        "INSERT INTO category_custom_fields (category_id, name, value, created_at, updated_at) \
         VALUES (4, 'only_post_voting_in_this_category', 'true', now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let html = admin.page("/c/general/edit/settings").await;
    let section = html
        .split("post-voting-category-settings\">")
        .nth(1)
        .expect("the post voting section");
    assert!(section.contains("<h2 class=\"form-kit__section-title\">Post Voting</h2>"));
    let only = section
        .split("class=\"form-kit__control-checkbox\" name=\"custom_fields.only_post_voting_in_this_category\"")
        .nth(1)
        .unwrap();
    assert!(only.starts_with(" type=\"checkbox\" checked"));
    let default = section
        .split("class=\"form-kit__control-checkbox\" name=\"custom_fields.create_as_post_voting_default\"")
        .nth(1)
        .unwrap();
    assert!(default.starts_with(" type=\"checkbox\">"));

    // What category-edit.js sends: the custom fields whole, as edited.
    let mut request = Request::builder()
        .method(Method::PUT)
        .uri("/categories/4.json")
        .header(header::HOST, "test.localhost")
        .header("x-requested-with", "XMLHttpRequest")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            admin
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; "),
        );
    if let Some(token) = &admin.csrf {
        request = request.header("x-csrf-token", token.as_str());
    }
    let response = discourse_rs::app(st.clone())
        .oneshot(
            request
                .body(Body::from(
                    r#"{"name": "General", "custom_fields": {"create_as_post_voting_default": true, "only_post_voting_in_this_category": false}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let fields: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT name, value FROM category_custom_fields WHERE category_id = 4 \
         AND name LIKE '%post_voting%' ORDER BY name",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        fields,
        [
            (
                "create_as_post_voting_default".to_string(),
                Some("t".to_string())
            ),
            (
                "only_post_voting_in_this_category".to_string(),
                Some("f".to_string())
            ),
        ]
    );
    // The category's own JSON has the keys after custom_fields.
    let (_, body) = admin.get("/c/4/show.json").await;
    let keys: Vec<&str> = body["category"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let at = keys.iter().position(|k| *k == "custom_fields").unwrap();
    assert_eq!(
        keys[at + 1..at + 3],
        [
            "create_as_post_voting_default",
            "only_post_voting_in_this_category"
        ]
    );
    assert_eq!(body["category"]["create_as_post_voting_default"], true);
    assert_eq!(body["category"]["only_post_voting_in_this_category"], false);
}

// At the vendored ref `Post#is_post_voting_topic?` calls is_post_voting? on
// the post's topic, nil once trashed: staff's view of a trashed topic
// raises while the plugin is on, and shows with it off.
#[tokio::test]
async fn a_trashed_topics_posts_are_refused_with_post_voting_on() {
    let db = TestDb::new().await;
    let st = setup(&db).await;
    sqlx::query("UPDATE topics SET deleted_at = now(), deleted_by_id = 1 WHERE id = 35")
        .execute(&db.pool)
        .await
        .unwrap();
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, _) = admin.get("/t/35.json").await;
    assert_eq!(status, 500);
    set_setting(&db.pool, "post_voting_enabled", 5, "f").await;
    let (status, body) = admin.get("/t/35.json").await;
    assert_eq!(status, 200, "{body}");
}
