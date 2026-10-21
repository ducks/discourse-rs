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
