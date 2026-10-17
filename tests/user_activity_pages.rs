//! The activity tabs the stream doesn't draw (`/u/:username/activity/
//! {topics,read,drafts,pending,bookmarks}`): what each lists, the empty
//! states, where Ember's routes redirect, and the strings user-activity.js
//! reads.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use tower::ServiceExt;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
}

struct Page {
    status: StatusCode,
    location: Option<String>,
    html: String,
}

impl Client {
    fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    async fn request(&mut self, request: axum::http::request::Builder, body: String) -> Page {
        let mut request = request.header(header::HOST, "test.localhost");
        if !self.cookies.is_empty() {
            request = request.header(header::COOKIE, self.cookie_header());
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body)).unwrap())
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
        let status = response.status();
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|l| l.to_str().unwrap().to_string());
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Page {
            status,
            location,
            html: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
        };
        let csrf = client
            .request(
                Request::get("/session/csrf.json").header("x-requested-with", "XMLHttpRequest"),
                String::new(),
            )
            .await;
        let csrf: serde_json::Value = serde_json::from_str(&csrf.html).unwrap();
        client
            .request(
                Request::builder()
                    .method(Method::POST)
                    .uri("/session")
                    .header("x-requested-with", "XMLHttpRequest")
                    .header("x-csrf-token", csrf["csrf"].as_str().unwrap())
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded"),
                format!("login={username}&password=password"),
            )
            .await;
        client
    }

    /// The page as a browser asks for it.
    async fn page(&mut self, path: &str) -> Page {
        self.request(Request::get(path), String::new()).await
    }

    /// The JSON an XHR gets.
    async fn json(&mut self, path: &str) -> (StatusCode, serde_json::Value) {
        let page = self
            .request(
                Request::get(path).header("x-requested-with", "XMLHttpRequest"),
                String::new(),
            )
            .await;
        (
            page.status,
            serde_json::from_str(&page.html).unwrap_or(serde_json::Value::Null),
        )
    }
}

async fn client(db: &TestDb, username: &str) -> Client {
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    Client::logged_in(st, username).await
}

async fn anonymous(db: &TestDb) -> Client {
    Client {
        state: state(db.pool.clone(), config(RailsEnv::Test, &[])).await,
        cookies: Vec::new(),
    }
}

async fn queue_post(db: &TestDb) {
    sqlx::query(
        "INSERT INTO reviewables (id, type, type_source, status, created_by_id, reviewable_by_moderator, category_id, topic_id, score, potential_spam, target_created_by_id, payload, version, latest_score, force_review, potentially_illegal, created_at, updated_at) \
         VALUES (9321, 'ReviewableQueuedPost', 'core', 0, 3, TRUE, 4, 35, 6, FALSE, 3, '{\"raw\": \"A reply <b>waiting</b> for approval.\"}', 0, '2026-10-02 10:00:00', TRUE, FALSE, '2026-10-02 10:00:00', '2026-10-02 10:00:00')",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE user_stats SET pending_posts_count = 1 WHERE user_id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn the_topics_tab_lists_the_users_topics() {
    let db = TestDb::new().await;
    let mut user1 = client(&db, "user1").await;
    let page = user1.page("/u/user0/activity/topics").await;
    assert_eq!(page.status, StatusCode::OK);
    let html = page.html;
    assert!(
        html.contains("<title>Topics - Activity - user0 - "),
        "{html}"
    );
    assert!(html.contains("user-activity-page"));
    assert!(html.contains(
        "<li aria-current=\"location\" class=\"user-nav__activity-topics\"><a class=\"active\" href=\"/u/user0/activity/topics\">"
    ));
    assert!(html.contains("class=\"paginated-topics-list\""));
    assert!(html.contains("data-topic-id=\"35\""));
    // showPosters={{false}}
    assert!(!html.contains("topic-list-data posters"));

    // Someone without topics: the others' empty state.
    let empty = user1.page("/u/user3/activity/topics").await.html;
    assert!(empty.contains("empty-state__title"), "{empty}");
    assert!(
        empty.contains("user3 has not started any topics yet"),
        "{empty}"
    );
}

#[tokio::test]
async fn the_topic_lists_sort_and_load_the_next_page() {
    let db = TestDb::new().await;
    let mut user1 = client(&db, "user1").await;
    // order and ascending are the routes' query params; the sorted column
    // says so (ascending as Ember reads the param: any value).
    let sorted = user1
        .page("/u/user0/activity/topics?ascending=false&order=views&per_page=1")
        .await
        .html;
    assert!(sorted.contains(
        "<th aria-sort=\"ascending\" class=\"topic-list-data views sortable sorting num\" data-sort-order=\"views\" scope=\"col\"><button aria-pressed=\"\">Views "
    ));
    // per_page is no query param of the route.
    assert!(!sorted.contains("data-next-page"));
    // DLoadMore's request: an XHR for the next page.
    let first = user1
        .request(
            Request::get("/u/user0/activity/topics?per_page=1")
                .header("x-requested-with", "XMLHttpRequest"),
            String::new(),
        )
        .await
        .html;
    assert!(first.contains("data-next-page=\"1\""), "{first}");
    let second = user1
        .request(
            Request::get("/u/user0/activity/topics?per_page=1&page=1")
                .header("x-requested-with", "XMLHttpRequest"),
            String::new(),
        )
        .await
        .html;
    let first_row = |html: &str| {
        let start = html.find("<tr class=\"topic-list-item").unwrap();
        let id_at = start + html[start..].find("data-topic-id=\"").unwrap();
        html[id_at..id_at + 20].to_string()
    };
    assert_ne!(first_row(&first), first_row(&second));
}

#[tokio::test]
async fn created_by_is_a_json_list_and_ember_has_no_page() {
    let db = TestDb::new().await;
    let mut user1 = client(&db, "user1").await;
    let (status, doc) = user1.json("/topics/created-by/user0.json").await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<i64> = doc["topic_list"]["topics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect();
    assert!(ids.contains(&35), "{doc}");
    assert_eq!(
        user1.json("/topics/created-by/nobody.json").await.0,
        StatusCode::NOT_FOUND
    );
    let html = user1.page("/topics/created-by/user0").await;
    assert_eq!(html.status, StatusCode::OK);
    assert!(html.html.contains("page-not-found"));
}

#[tokio::test]
async fn the_read_tab_lists_what_the_viewer_read() {
    let db = TestDb::new().await;
    let mut user1 = client(&db, "user1").await;
    let page = user1.page("/u/user1/activity/read").await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.html.contains("<title>Read - Activity - user1 - "));
    assert!(
        page.html.contains("class=\"paginated-topics-list\"") || page.html.contains("empty-state")
    );
}

#[tokio::test]
async fn the_drafts_tab_lists_ones_drafts() {
    let db = TestDb::new().await;
    sqlx::query(
        "INSERT INTO drafts (user_id, draft_key, data, sequence, revisions, created_at, updated_at) VALUES \
         (3, 'new_topic', '{\"reply\":\"A **topic** draft with a [link](https://example.com) and ![img](https://example.com/a.png)\",\"title\":\"A draft title\",\"categoryId\":4,\"action\":\"createTopic\"}', 0, 1, now(), now()), \
         (3, 'topic_35', '{\"reply\":\"A reply draft\",\"action\":\"reply\"}', 0, 1, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE user_stats SET draft_count = 2 WHERE user_id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let mut user1 = client(&db, "user1").await;
    let page = user1.page("/u/user1/activity/drafts").await;
    assert_eq!(page.status, StatusCode::OK);
    let html = page.html;
    assert!(
        html.contains("<title>Drafts - Activity - user1 - "),
        "{html}"
    );
    assert!(html.contains("Drafts (2)"));
    assert!(html.contains("post-list post-list--bulk-select user-stream"));
    assert!(html.contains("data-draft-key=\"new_topic\""));
    assert!(html.contains(
        "<button class=\"btn no-text btn-transparent draft-title\" type=\"button\">A draft title</button>"
    ));
    assert!(html.contains("href=\"/t/parity-fixture-replies-and-posters/35\""));
    // The client's excerpt: text, links kept, images as [image].
    assert!(
        html.contains("A topic draft with a <a href=\"https://example.com\">link</a> and [image]"),
        "{html}"
    );
    assert!(html.contains("resume-draft"));
    assert!(html.contains("id=\"user-activity-strings\""));
    assert!(html.contains("/assets/user-activity.js"));
}

#[tokio::test]
async fn drafts_are_ones_own() {
    let db = TestDb::new().await;
    let mut user1 = client(&db, "user1").await;
    let other = user1.page("/u/user2/activity/drafts").await;
    assert_eq!(other.status, StatusCode::FOUND);
    assert!(
        other
            .location
            .unwrap()
            .ends_with("/u/user1/activity/drafts")
    );
    let empty = user1.page("/u/user1/activity/drafts").await.html;
    assert!(empty.contains("You haven’t started any drafts"), "{empty}");
    let mut anon = anonymous(&db).await;
    let page = anon.page("/u/user1/activity/drafts").await;
    assert_eq!(page.status, StatusCode::FOUND);
    assert!(page.location.unwrap().ends_with("/latest"));
}

#[tokio::test]
async fn failed_models_leave_embers_pages() {
    let db = TestDb::new().await;
    queue_post(&db).await;
    let mut anon = anonymous(&db).await;
    // /read.json and posts#pending answer 403: the exception page.
    for (path, json) in [
        ("/u/user1/activity/read", "/read.json"),
        ("/u/user1/activity/pending", "/posts/user1/pending.json"),
    ] {
        let page = anon.page(path).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(
            page.html.contains("<title>Profile - user1 - "),
            "{}",
            page.html
        );
        assert!(page.html.contains("error-page"));
        assert!(
            page.html.contains(&format!("href=\"{json}\"")),
            "{}",
            page.html
        );
    }
    // Another member's pending posts: posts#pending's 404, the /404 route.
    let mut user2 = client(&db, "user2").await;
    let page = user2.page("/u/user1/activity/pending").await;
    assert_eq!(page.status, StatusCode::FOUND);
    assert!(page.location.unwrap().ends_with("/404"));
}

#[tokio::test]
async fn the_pending_tab_lists_queued_posts() {
    let db = TestDb::new().await;
    queue_post(&db).await;
    let mut user1 = client(&db, "user1").await;
    let page = user1.page("/u/user1/activity/pending").await;
    assert_eq!(page.status, StatusCode::OK);
    let html = page.html;
    assert!(html.contains("<title>Activity - user1 - "), "{html}");
    assert!(html.contains("Pending (1)"));
    assert!(html.contains("data-pending-id=\"9321\""));
    assert!(
        html.contains("<p>A reply <b>waiting</b> for approval.</p>"),
        "{html}"
    );
    assert!(html.contains("reviewable-actions"));
    let (status, doc) = user1.json("/posts/user1/pending.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(doc["pending_posts"][0]["id"], 9321);
    // Staff see them, without the delete button; others get a 404.
    let mut admin = client(&db, "admin").await;
    let staff = admin.page("/u/user1/activity/pending").await.html;
    assert!(staff.contains("data-pending-id=\"9321\""));
    assert!(!staff.contains("reviewable-actions"));
    let mut user2 = client(&db, "user2").await;
    assert_eq!(
        user2.json("/posts/user1/pending.json").await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn the_bookmarks_tab_lists_and_searches() {
    let db = TestDb::new().await;
    let mut user1 = client(&db, "user1").await;
    let page = user1.page("/u/user1/activity/bookmarks").await;
    assert_eq!(page.status, StatusCode::OK);
    let html = page.html;
    assert!(
        html.contains("<title>Bookmarks - Activity - user1 - "),
        "{html}"
    );
    assert!(html.contains("user-activity-bookmarks-page"));
    assert!(html.contains("bookmark-search-form"));
    assert!(html.contains("table class=\"topic-list bookmark-list\""));
    assert!(html.contains("bookmark-actions-dropdown"));
    let nothing = user1
        .page("/u/user1/activity/bookmarks?q=zzzznothing")
        .await
        .html;
    assert!(nothing.contains("alert alert-info"), "{nothing}");
    assert!(nothing.contains("value=\"zzzznothing\""));
    // Another member's: the list request fails.
    let other = user1.page("/u/user2/activity/bookmarks").await.html;
    assert!(
        other.contains("You do not have permission to view this user"),
        "{other}"
    );
    // bookmarks-with-reminders goes to the bookmarks.
    let old = user1
        .page("/u/user1/activity/bookmarks-with-reminders?q=x")
        .await;
    assert_eq!(old.status, StatusCode::FOUND);
    assert!(
        old.location
            .unwrap()
            .ends_with("/u/user1/activity/bookmarks?q=x")
    );
}

/// Bookmark#formattedReminder in the member's timezone option, or the
/// browser's without one: a template for static/js/dates.js, the text
/// drawn here in that zone (UTC without one).
#[tokio::test]
async fn bookmark_reminders_are_drawn_in_the_members_zone() {
    let db = TestDb::new().await;
    let user1 = "(SELECT id FROM users WHERE username = 'user1')";
    sqlx::query(&format!(
        "UPDATE bookmarks SET reminder_at = '2099-03-04 05:06:00' WHERE user_id = {user1}"
    ))
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(&format!(
        "UPDATE user_options SET timezone = 'Asia/Kolkata' WHERE user_id = {user1}"
    ))
    .execute(&db.pool)
    .await
    .unwrap();
    let mut member = client(&db, "user1").await;
    let html = member.page("/u/user1/activity/bookmarks").await.html;
    assert!(
        html.contains(
            "bookmark-reminder\" data-tz-text=\"%{4076283960000:@reminder}\" data-tz-zone=\"Asia/Kolkata\">"
        ),
        "{html}"
    );
    assert!(
        html.contains("</svg>At Mar 4, 2099 10:36 am</span>"),
        "{html}"
    );
    sqlx::query(&format!(
        "UPDATE user_options SET timezone = NULL WHERE user_id = {user1}"
    ))
    .execute(&db.pool)
    .await
    .unwrap();
    let html = member.page("/u/user1/activity/bookmarks").await.html;
    assert!(html.contains("bookmark-reminder\" data-tz-text=\"%{4076283960000:@reminder}\">"));
    assert!(
        html.contains("</svg>At Mar 4, 2099 5:06 am</span>"),
        "{html}"
    );
}

#[tokio::test]
async fn no_bookmarks_is_the_empty_state() {
    let db = TestDb::new().await;
    let mut user2 = client(&db, "user2").await;
    let html = user2.page("/u/user2/activity/bookmarks").await.html;
    assert!(html.contains("empty-state__title"), "{html}");
    assert!(!html.contains("bookmark-search-form"));
}
