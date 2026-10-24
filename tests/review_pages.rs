//! The review queue's page (/review): the queue drawn from the document
//! /review.json serves with the route's params, the exception page for
//! anyone who may not see it, and the action's redraw review.js fetches;
//! and the user menu's review queue with the pending reviewables the
//! recent notifications carry.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use tower::ServiceExt;

/// The about post of the Sub General category, the admin's.
const POST: &str = "34";

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn send(
        &mut self,
        method: Method,
        path: &str,
        body: &str,
        xhr: bool,
    ) -> (StatusCode, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        if xhr {
            request = request.header("x-requested-with", "XMLHttpRequest");
            if let Some(token) = &self.csrf {
                request = request.header("x-csrf-token", token.as_str());
            }
        }
        if !self.cookies.is_empty() {
            let cookies: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookies.join("; "));
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
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client
            .send(Method::GET, "/session/csrf.json", "", true)
            .await;
        let csrf: serde_json::Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        let (status, body) = client
            .send(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
                true,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        client
    }

    async fn page(&mut self, path: &str) -> (StatusCode, String) {
        self.send(Method::GET, path, "", false).await
    }
}

async fn flag(st: &AppState) {
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, body) = user1
        .send(
            Method::POST,
            "/post_actions.json",
            &format!("id={POST}&post_action_type_id=4&flag_topic=false"),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn an_empty_queue() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, html) = admin.page("/review").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("<title>Review - "), "{html}");
    assert!(html.contains("<div class=\"no-review\">There are no items to review.</div>"));
    // View All carries the sort order the route defaults to; the sidebar's
    // Review link is current with the same query.
    assert!(
        html.contains(
            "<li class=\"active\"><a class=\"active\" href=\"/review?sort_order=score\">View All</a></li>"
        ),
        "{html}"
    );
    assert!(html.contains("<a class=\"active sidebar-section-link sidebar-row\" title=\"Flagged posts and other queued items\" data-link-name=\"review\" href=\"/review?sort_order=score\">"));
    // The admin's Settings tab.
    assert!(html.contains("href=\"/review/settings\""));
    // ReviewFilters collapsed: status pending, all types.
    assert!(html.contains("<div class=\"reviewable-filters --collapsed\">"));
    assert!(html.contains("data-name=\"Pending\" data-value=\"pending\" title=\"Pending\""));
    assert!(html.contains("<span class=\"name\">(all types)</span>"));
    assert!(html.contains("<template id=\"review-secondary-filters\">"));
    assert!(html.contains("/assets/review.js?v="));
}

#[tokio::test]
async fn a_flagged_post_in_the_queue() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    flag(&st).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, html) = admin.page("/review").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(
        html.contains("<div class=\"review-item reviewable-flagged-post\" data-reviewable-id=\""),
        "{html}"
    );
    assert!(html.contains("<span class=\"review-item__flag-label\">Post flagged as</span>"));
    assert!(html.contains("<span class=\"review-item__flag-reason\">Inappropriate </span>"));
    assert!(html.contains("<div class=\"review-item__status --pending\">Pending</div>"));
    // The topic (pinned, so its toggle for a member) and the author.
    assert!(html.contains("<a class=\"topic-status --pinned pin-toggle-button\" href=\"\""));
    assert!(html.contains(">About the Sub General category</a>"));
    assert!(html.contains("<div class=\"review-item__meta-label\">Flagged user</div>"));
    // The timeline: the post's creation, then the flag.
    let created = html.find("Post created by <span>").expect("created event");
    let flagged = html
        .find("Flagged as <span><span class=\"review-item__flag-reason\">Inappropriate </span></span> by <span>")
        .expect("flag event");
    assert!(created < flagged);
    assert!(html.contains("<div class=\"reviewable-note-form\">"));
    // The context question and the Yes and No bundles.
    assert!(
        html.contains("<h3 class=\"review-item__aside-title\">Is this post inappropriate?</h3>")
    );
    assert!(html.contains(
        "<details class=\"select-kit single-select dropdown-select-box reviewable-action-dropdown btn-icon-text post-agree-and-hide\">"
    ));
    assert!(html.contains("data-name=\"No\""));
    assert!(html.contains("data-value=\"post-agree_and_keep\""));
    // What review.js performs with.
    assert!(html.contains("<script type=\"application/json\" id=\"review-page-data\">"));
    assert!(html.contains("\"server_action\":\"agree_and_keep\""));
}

#[tokio::test]
async fn a_performed_flag_redraws_in_the_topics_queue() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    flag(&st).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (_, json) = admin.send(Method::GET, "/review.json", "", true).await;
    let json: serde_json::Value = serde_json::from_str(&json).unwrap();
    let id = json["reviewables"][0]["id"].as_i64().unwrap();
    let (status, body) = admin
        .send(
            Method::PUT,
            &format!("/review/{id}/perform/agree_and_keep?version=0"),
            "",
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Gone from the pending queue; review.js redraws it from every status
    // in its topic.
    let (_, html) = admin.page("/review").await;
    assert!(!html.contains("data-reviewable-id="));
    let (status, html) = admin
        .page("/review?priority=any&status=all&topic_id=34")
        .await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains(&format!("data-reviewable-id=\"{id}\"")));
    assert!(html.contains("<div class=\"review-item__status --approved\">Flag approved</div>"));
    assert!(html.contains("Approved by <span>"));
    // Nothing left to do.
    assert!(!html.contains("review-item__moderator-actions"));
}

#[tokio::test]
async fn members_get_the_exception_page() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let (status, html) = user1.page("/review").await;
    // The Ember app loads; its model's request is refused.
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains("<div class=\"reason\">Access Denied</div>"),
        "{html}"
    );
    assert!(html.contains(
        "href=\"/review?priority&#x3D;low&amp;status&#x3D;pending&amp;sort_order&#x3D;score\">"
    ));
    assert!(!html.contains("<title>Review - "));
    assert!(!html.contains("review-page-data"));
}

#[tokio::test]
async fn an_unknown_status_is_the_models_bad_request() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, html) = admin.page("/review?status=bogus").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<div class=\"error-page\">"), "{html}");
    assert!(
        html.contains("/review?priority&#x3D;low&amp;status&#x3D;bogus&amp;sort_order&#x3D;score")
    );
}

#[tokio::test]
async fn the_user_menus_review_queue() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    flag(&st).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;

    // ReviewablesController#user_menu_list through the basic serializers.
    let (status, body) = admin
        .send(Method::GET, "/review/user-menu-list", "", true)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let item = &json["reviewables"][0];
    assert_eq!(item["flagger_username"], "user1");
    assert_eq!(item["type"], "ReviewableFlaggedPost");
    assert_eq!(item["pending"], true);
    assert_eq!(item["post_number"], 1);
    assert_eq!(item["topic_fancy_title"], "About the Sub General category");
    assert_eq!(json["reviewable_count"], 1);
    assert_eq!(json["__rest_serializer"], "1");
    let id = item["id"].as_i64().unwrap();

    // The tab: the item linking to the reviewable, View All to the queue.
    let (status, html) = admin.page("/user-menu?tab=review-queue").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains(&format!(
        "<li class=\"reviewable pending\"><a href=\"/review/{id}\" title=\"\">"
    )));
    assert!(
        html.contains("<span class=\"item-label \">user1</span>"),
        "{html}"
    );
    assert!(html.contains("post #1 - About the Sub General category</span>"));
    assert!(html.contains("class=\"btn no-text btn-default show-all\" href=\"/review\""));
    assert!(!html.contains("notifications-dismiss"));

    // The recent notifications carry them for the all notifications tab,
    // which lists them first while every notification is unread, and
    // marks them seen.
    let (_, body) = admin
        .send(
            Method::GET,
            "/notifications.json?recent=true&limit=30&bump_last_seen_reviewable=true",
            "",
            true,
        )
        .await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["pending_reviewables"][0]["id"], id, "{body}");
    let seen: Option<i32> =
        sqlx::query_scalar("SELECT last_seen_reviewable_id FROM users WHERE username = 'admin'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(seen.map(i64::from), Some(id));
    let (_, html) = admin.page("/user-menu").await;
    assert!(html.contains(&format!(
        "<li class=\"reviewable pending\"><a href=\"/review/{id}\""
    )));

    // A filtered tab lists none.
    let (_, body) = admin
        .send(
            Method::GET,
            "/notifications.json?recent=true&filter_by_types=mentioned&silent=true",
            "",
            true,
        )
        .await;
    assert!(!body.contains("pending_reviewables"), "{body}");

    // A member may not.
    let mut user1 = Client::logged_in(st, "user1").await;
    let (status, _) = user1
        .send(Method::GET, "/review/user-menu-list", "", true)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// A reviewable's notes and claim: a note added through POST
// /review/:id/notes lands in the timeline (Moderator note by, its content
// escaped, the author's delete), and with claiming on, claiming the
// topic draws who holds it and the unclaim button, logged as claimed.
#[tokio::test]
async fn notes_and_claims_draw_in_the_queue() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    flag(&st).await;
    let id: i64 = sqlx::query_scalar("SELECT id FROM reviewables ORDER BY id DESC LIMIT 1")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let topic_id: i32 = sqlx::query_scalar("SELECT topic_id FROM reviewables WHERE id = $1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, body) = admin
        .send(
            Method::POST,
            &format!("/review/{id}/notes"),
            "reviewable_note%5Bcontent%5D=Looks+%3Cfine%3E+to+me",
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let note: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(note["content"], "Looks <fine> to me");
    assert_eq!(note["user"]["username"], "admin");
    let (_, html) = admin.page("/review").await;
    assert!(html.contains("Moderator note by <span>"), "{html}");
    assert!(html.contains(
        "<div class=\"timeline-event__description\"><p>Looks &lt;fine&gt; to me</p></div></div><div class=\"timeline-event__actions\"><button class=\"btn no-text btn-icon btn-transparent --danger timeline-event__delete-note btn-transparent\" title=\"Delete this note\" type=\"button\">"
    ));
    assert!(html.contains(&format!("\"deletable_notes\":[{}]", note["id"])));
    // Claiming is off: no claim button, and a manual claim is refused.
    assert!(!html.contains("reviewable-claimed-topic"));
    let (status, _) = admin
        .send(
            Method::POST,
            "/reviewable_claimed_topics",
            &format!("reviewable_claimed_topic%5Btopic_id%5D={topic_id}"),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    common::set_setting(&db.pool, "reviewable_claiming", 7, "optional").await;
    let (_, html) = admin.page("/review").await;
    assert!(html.contains("<button class=\"btn btn-icon-text btn-default claim\""));
    let (status, body) = admin
        .send(
            Method::POST,
            "/reviewable_claimed_topics",
            &format!("reviewable_claimed_topic%5Btopic_id%5D={topic_id}"),
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, html) = admin.page("/review").await;
    assert!(html.contains("<div class=\"review-item__assigned\">"));
    assert!(
        html.contains("<button class=\"btn btn-icon-text btn-default unclaim\" type=\"button\">")
    );
    assert!(html.contains("Claimed by <span>"));
    assert!(html.contains("\"claimed_by\":{\"user_id\":1,\"automatic\":false}"));
}

/// user0, a member with one post, under approve_post_count: what they post
/// waits in the queue (NewPostManager), as Rails queues it. The
/// reviewable's id.
async fn queue(db: &TestDb, st: &AppState, params: &str) -> i64 {
    common::set_setting(&db.pool, "approve_post_count", 3, "5").await;
    let mut user0 = Client::logged_in(st.clone(), "user0").await;
    let (status, body) = user0.send(Method::POST, "/posts.json", params, true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["action"], "enqueued", "{body}");
    sqlx::query_scalar(
        "SELECT id FROM reviewables WHERE type = 'ReviewableQueuedPost' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

const QUEUED_REPLY: &str = "topic_id=35&raw=A+queued+reply+from+a+new+member+with+**bold**+text.&typing_duration_msecs=5000";

/// Approving the queued reply as `username` (staff), from the queue the
/// page draws: the card, the approval, the post it creates, the queue and
/// counts after.
async fn approve_a_queued_reply_as(username: &str) {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    if username == "user3" {
        for sql in [
            "UPDATE users SET moderator = true WHERE id = 5",
            "INSERT INTO group_users (group_id, user_id, created_at, updated_at, notification_level) VALUES (2, 5, now(), now(), 2), (3, 5, now(), now(), 2)",
        ] {
            sqlx::query(sql).execute(&db.pool).await.unwrap();
        }
    }
    let id = queue(&db, &st, QUEUED_REPLY).await;
    let mut staff = Client::logged_in(st, username).await;

    // ReviewableQueuedPostSerializer: the payload, the raw cooked, the
    // author side-loaded, the post_count reason, the approval and the
    // reject bundle.
    let (status, body) = staff.send(Method::GET, "/review.json", "", true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let r = &json["reviewables"][0];
    assert_eq!(r["id"], id, "{body}");
    assert_eq!(r["type"], "ReviewableQueuedPost");
    assert_eq!(r["topic_id"], 35);
    assert_eq!(r["created_from_flag"], false);
    assert_eq!(r["can_edit"], true);
    assert_eq!(r["fancy_title"], serde_json::Value::Null);
    assert_eq!(
        r["cooked"],
        "<p>A queued reply from a new member with <strong>bold</strong> text.</p>"
    );
    assert_eq!(
        r["payload"]["raw"],
        "A queued reply from a new member with **bold** text."
    );
    assert_eq!(r["target_created_by_id"], 2);
    assert_eq!(
        r["bundled_action_ids"],
        serde_json::json!([format!("{id}-approve_post"), format!("{id}-reject-post")])
    );
    let actions: Vec<&str> = json["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["server_action"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions,
        [
            "approve_post",
            "reject_post",
            "revise_and_reject_post",
            "reject_and_silence",
            "reject_and_suspend",
            "delete_user",
            "delete_user_block"
        ]
    );
    assert_eq!(json["reviewable_scores"][0]["reason_type"], "post_count");
    assert!(
        json["reviewable_scores"][0]["reason"]
            .as_str()
            .unwrap()
            .starts_with("<p>The first few posts from every user must be approved by staff.")
    );
    assert!(
        json["users"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["username"] == "user0")
    );
    assert_eq!(json["meta"]["reviewable_count"], 1);

    // The card as reviewable/queued-post.gjs draws it.
    let (status, html) = staff.page("/review").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(
        html.contains(&format!(
            "<div class=\"review-item reviewable-queued-post\" data-reviewable-id=\"{id}\">"
        )),
        "{html}"
    );
    assert!(html.contains("<span class=\"review-item__flag-label\">Queued post</span>"));
    assert!(html.contains("<span class=\"review-item__flag-reason\">Needs Approval </span>"));
    assert!(html.contains("<div class=\"review-item__meta-label\">Topic</div>"));
    assert!(html.contains(">Parity fixture: replies and posters</a>"));
    assert!(html.contains("<div class=\"review-item__meta-label\">User</div>"));
    assert!(html.contains(
        "<div class=\"review-item__post\"><div class=\"review-item__post-content\"><div class=\"post-body\"><p>A queued reply from a new member with <strong>bold</strong> text.</p></div><span></span></div></div>"
    ));
    assert!(html.contains("<h3 class=\"review-item__aside-title\">Approve this post?</h3>"));
    assert!(html.contains(
        "<button class=\"btn btn-text btn-default reviewable-action approve-post\" type=\"button\"><span class=\"d-button-label\">Yes</span></button>"
    ));
    assert!(html.contains("data-value=\"revise_and_reject_post\""));
    assert!(html.contains("data-value=\"delete_user\""));
    assert!(html.contains("<template id=\"review-revise-modal\">"));
    assert!(html.contains("\"server_action\":\"approve_post\""));

    // Yes: the post is created as its author, the queue empties.
    let (status, body) = staff
        .send(
            Method::PUT,
            &format!("/review/{id}/perform/approve_post?version=0"),
            "",
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let result = &json["reviewable_perform_result"];
    assert_eq!(result["created_post_topic_id"], 35, "{body}");
    assert_eq!(result["reviewable_count"], 0);
    let (author, raw): (i32, String) =
        sqlx::query_as("SELECT user_id, raw FROM posts WHERE id = $1")
            .bind(result["created_post_id"].as_i64().unwrap() as i32)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(author, 2);
    assert_eq!(raw, "A queued reply from a new member with **bold** text.");
    let (_, body) = staff.send(Method::GET, "/review.json", "", true).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["reviewables"], serde_json::json!([]), "{body}");
    assert_eq!(json["meta"]["reviewable_count"], 0);
    let (_, html) = staff.page("/review").await;
    assert!(html.contains("<div class=\"no-review\">There are no items to review.</div>"));
    // The redraw review.js asks for: approved, its post linked, nothing
    // left to do.
    let (status, html) = staff
        .page("/review?priority=any&status=all&topic_id=35")
        .await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains(&format!("data-reviewable-id=\"{id}\"")));
    assert!(html.contains("<div class=\"review-item__status --approved\">Post approved</div>"));
    assert!(!html.contains("review-item__moderator-actions"));
}

#[tokio::test]
async fn an_admin_approves_a_queued_reply() {
    approve_a_queued_reply_as("admin").await;
}

#[tokio::test]
async fn a_moderator_approves_a_queued_reply() {
    approve_a_queued_reply_as("user3").await;
}

// A queued topic: no topic yet, so the card draws the title, category and
// tags from the payload, asks whether to approve the topic, and offers
// editing its title, category, tags and raw.
#[tokio::test]
async fn a_queued_topic_draws_its_title_and_category() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let id = queue(
        &db,
        &st,
        "title=A+queued+topic+%26+its+%3Cb%3Etitle%3C%2Fb%3E&raw=The+first+post+of+a+queued+topic+from+a+new+member.&category=4&typing_duration_msecs=5000",
    )
    .await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (_, body) = admin.send(Method::GET, "/review.json", "", true).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let r = &json["reviewables"][0];
    assert_eq!(r["id"], id, "{body}");
    assert_eq!(r["topic_id"], serde_json::Value::Null);
    assert_eq!(r["removed_topic_id"], serde_json::Value::Null);
    assert_eq!(
        r["fancy_title"],
        "A queued topic &amp; its &lt;b&gt;title&lt;/b&gt;"
    );
    let fields: Vec<&str> = r["editable_fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        fields,
        [
            "payload.title",
            "category_id",
            "payload.tags",
            "payload.raw"
        ]
    );

    let (status, html) = admin.page("/review").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(
        html.contains("<span class=\"review-item__flag-label\">Queued topic</span>"),
        "{html}"
    );
    assert!(html.contains(
        "<div class=\"reviewable-topic-link\"><div class=\"title-text\">A queued topic &amp; its &lt;b&gt;title&lt;/b&gt;</div><span class=\"badge-category__wrapper\""
    ));
    assert!(html.contains("<h3 class=\"review-item__aside-title\">Approve this topic?</h3>"));
    assert!(html.contains(
        "<button class=\"btn btn-text reviewable-action btn-default edit\" type=\"button\">"
    ));

    // Approving creates the topic; the redraw finds it there.
    let (status, body) = admin
        .send(
            Method::PUT,
            &format!("/review/{id}/perform/approve_post?version=0"),
            "",
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let topic_id = json["reviewable_perform_result"]["created_post_topic_id"]
        .as_i64()
        .unwrap();
    let title: String = sqlx::query_scalar("SELECT title FROM topics WHERE id = $1")
        .bind(topic_id as i32)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(title, "A queued topic & its <b>title</b>");
    let (_, html) = admin
        .page(&format!(
            "/review?priority=any&status=all&topic_id={topic_id}"
        ))
        .await;
    assert!(
        html.contains(&format!("data-reviewable-id=\"{id}\"")),
        "{html}"
    );
}

// Request Revision: the reason and feedback go with the action, the author
// gets the system message, the post stays unposted.
#[tokio::test]
async fn a_queued_reply_is_sent_back_for_revision() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let id = queue(&db, &st, QUEUED_REPLY).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, body) = admin
        .send(
            Method::PUT,
            &format!("/review/{id}/perform/revise_and_reject_post?version=0"),
            "revise_reason=other_reason&revise_custom_reason=Please+add+a+source&revise_feedback=Thanks%21",
            true,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let status: i32 = sqlx::query_scalar("SELECT status FROM reviewables WHERE id = $1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(status, 2);
    let message: String = sqlx::query_scalar(
        "SELECT p.raw FROM posts p JOIN topics t ON t.id = p.topic_id \
         JOIN topic_allowed_users tau ON tau.topic_id = t.id AND tau.user_id = 2 \
         WHERE t.archetype = 'private_message' ORDER BY p.id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(message.contains("Please add a source"), "{message}");
    assert!(message.contains("Thanks!"), "{message}");
}
