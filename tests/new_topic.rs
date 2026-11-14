//! GET /new-topic and /new-message (new_topic#index) as the Ember routes
//! leave them: an anonymous visitor gets the login page, a member the page
//! the route replaces itself with (the category asked for, else the
//! homepage with the discovery query params kept) and, before the
//! composer, what static/js/composer.js opens it with.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const BOOL: i32 = 5;
const GROUP_LIST: i32 = 20;

struct Reply {
    status: StatusCode,
    location: Option<String>,
    cookies: Vec<String>,
    cache_control: Option<String>,
    body: String,
}

async fn send(st: &AppState, request: Request<Body>) -> Reply {
    let response = discourse_rs::app(st.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let header_value = |name| {
        response
            .headers()
            .get(name)
            .map(|v: &header::HeaderValue| v.to_str().unwrap().to_string())
    };
    let location = header_value(header::LOCATION);
    let cache_control = header_value(header::CACHE_CONTROL);
    let cookies = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|c| c.split(';').next().unwrap().to_string())
        .collect();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        location,
        cookies,
        cache_control,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

/// A page as a browser asks for it (no XHR header).
async fn page(st: &AppState, path: &str, cookie: Option<&str>) -> Reply {
    let mut builder = Request::get(path).header(header::HOST, "test.localhost");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    send(st, builder.body(Body::empty()).unwrap()).await
}

/// The `_t` cookie of `username`, logged in through POST /session.
async fn logged_in(st: &AppState, username: &str) -> String {
    let csrf_reply = send(
        st,
        Request::get("/session/csrf.json")
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let session = csrf_reply
        .cookies
        .iter()
        .find(|c| c.starts_with("_forum_session="))
        .unwrap()
        .clone();
    let csrf: Value = serde_json::from_str(&csrf_reply.body).unwrap();
    let reply = send(
        st,
        Request::post("/session")
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest")
            .header("x-csrf-token", csrf["csrf"].as_str().unwrap())
            .header(header::COOKIE, &session)
            .header(
                header::CONTENT_TYPE,
                "application/x-www-form-urlencoded; charset=UTF-8",
            )
            .body(Body::from(format!("login={username}&password=password")))
            .unwrap(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    reply
        .cookies
        .into_iter()
        .find(|c| c.starts_with("_t="))
        .expect("a login sets _t")
}

/// The `#composer-open` instruction the page carries, which must come
/// right before the composer.
fn instruction(html: &str) -> Value {
    const OPEN: &str = "<script type=\"application/json\" id=\"composer-open\">";
    let start = html.find(OPEN).expect("no #composer-open") + OPEN.len();
    let end = start + html[start..].find("</script>").unwrap();
    assert!(
        html[end..].starts_with("</script><div id=\"reply-control\""),
        "{}",
        &html[end..end + 200]
    );
    serde_json::from_str(&html[start..end]).unwrap()
}

#[tokio::test]
async fn anonymous_visitors_get_the_login_page() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    for path in [
        "/new-topic?title=Hello&body=World",
        "/new-message?username=user2",
    ] {
        let reply = page(&st, path, None).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}: {}", reply.body);
        let html = &reply.body;
        // login.js keeps this URL as the destination and takes /login.
        assert!(html.contains("id=\"login-data\""), "{path}: {html}");
        assert!(html.contains("\"page\":\"login\""), "{path}: {html}");
        assert!(html.contains("id=\"login-account-name\""), "{path}: {html}");
        assert!(html.contains(" login-page"), "{path}: {html}");
        assert!(!html.contains("composer-open"), "{path}: {html}");
    }

    // A site that requires login sends the visitor to /login first, with
    // the URL as the destination.
    set_setting(&db.pool, "login_required", BOOL, "t").await;
    let reply = page(&st, "/new-topic?title=Hello", None).await;
    assert_eq!(reply.status, StatusCode::FOUND, "{}", reply.body);
    assert_eq!(
        reply.location.as_deref(),
        Some("http://test.localhost/login")
    );
    assert!(
        reply.cookies.iter().any(|c| c
            == "destination_url=http%3A%2F%2Ftest.localhost%2Fnew-topic%3Ftitle%3DHello"),
        "{:?}",
        reply.cookies
    );
}

#[tokio::test]
async fn a_member_gets_the_homepage_with_the_composer_to_open() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let member = logged_in(&st, "user1").await;
    let reply = page(
        &st,
        "/new-topic?title=Prefilled+title&body=Some%20body&tags=howto,guide&page=2&order=views&category=nope&f=1&f=2",
        Some(&member),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.cache_control.as_deref(), Some("no-cache, no-store"));
    let html = &reply.body;
    // discovery.latest under it, as /latest draws it. Its `tags=howto,guide`
    // is one tag name to TopicQuery, which no topic has, so the list is
    // empty and draws no table, as on Rails.
    assert!(html.contains("<title>Discourse</title>"), "{html}");
    assert!(html.contains("list-container --topic-list"), "{html}");
    assert!(!html.contains("<table class=\"topic-list"), "{html}");
    // Only the discovery list's params go along, sorted and encoded as
    // Ember's router writes them; the params go to the composer as given
    // (the last of a repeated one).
    assert_eq!(
        instruction(html),
        json!({
            "route": "new-topic",
            "url": "/latest?f=2&order=views&tags=howto%2Cguide",
            "params": {
                "title": "Prefilled title",
                "body": "Some body",
                "tags": "howto,guide",
                "page": "2",
                "order": "views",
                "category": "nope",
                "f": "2",
            },
            "category": null,
            "sharedDraft": false,
            "open": true,
            "chooser": null,
        })
    );
    // What composer.js prefills it within.
    assert!(html.contains(" data-max-title-length=\"255\""), "{html}");
    assert!(
        html.contains(
            " data-label-cant-send-pm=\"Sorry, you can&#x27;t send a message to %{username}.\""
        ),
        "{html}"
    );
    assert!(
        html.contains(" data-dialog-src=\"/assets/dialog.js?v&#x3D;"),
        "{html}"
    );

    // The grippie is a DResizeSeparator resizing the composer's height
    // (composer.js measures and announces its size). The page's <html> is
    // drawn closed: composer.js sets html.composer-open and
    // --composer-height when it opens the composer, as Ember's composer
    // model and service do.
    assert!(
        html.contains(
            "<div class=\"d-resize-separator grippie\" aria-label=\"Resize composer\" \
             aria-orientation=\"horizontal\" data-pointer-drag=\"none\" \
             data-resize-axis=\"vertical\" role=\"separator\" tabindex=\"0\"></div>"
        ),
        "{html}"
    );
    let html_tag = &html[html.find("<html").unwrap()..];
    let html_tag = &html_tag[..html_tag.find('>').unwrap()];
    assert!(!html_tag.contains("composer-"), "{html_tag}");
    assert!(!html_tag.contains("--composer-height"), "{html_tag}");

    // Without params the homepage's URL is bare.
    let html = page(&st, "/new-topic", Some(&member)).await.body;
    assert_eq!(instruction(&html)["url"], "/latest");
    assert_eq!(instruction(&html)["params"], json!({}));
}

#[tokio::test]
async fn a_category_asked_for_is_the_page_under_the_composer() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let member = logged_in(&st, "user1").await;
    // By name or slug (any case), parent/child, or category_id, which wins.
    for (path, id, title) in [
        (
            "/new-topic?category=general&title=T",
            4,
            "Latest General topics",
        ),
        (
            "/new-topic?category=Site%20Feedback",
            2,
            "Latest Site Feedback topics",
        ),
        (
            "/new-topic?category=SUB-GENERAL",
            34,
            "Latest General/Sub General topics",
        ),
        (
            "/new-topic?category=general/sub-general",
            34,
            "Latest General/Sub General topics",
        ),
        (
            "/new-topic?category_id=34&body=sub",
            34,
            "Latest General/Sub General topics",
        ),
        (
            "/new-topic?category=general&category_id=2",
            2,
            "Latest Site Feedback topics",
        ),
    ] {
        let reply = page(&st, path, Some(&member)).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}: {}", reply.body);
        let html = &reply.body;
        assert!(
            html.contains(&format!("<title>{title} - ")),
            "{path}: {html}"
        );
        let open = instruction(html);
        assert_eq!(open["url"], format!("/c/{id}"), "{path}");
        assert_eq!(open["category"], id, "{path}");
        assert_eq!(open["open"], true, "{path}");
    }
    // Not among the site's categories: the homepage, no category.
    for path in [
        "/new-topic?category=nope&order=views",
        "/new-topic?category_id=999",
        "/new-topic?category_id=04",
    ] {
        let html = page(&st, path, Some(&member)).await.body;
        let open = instruction(&html);
        assert!(
            open["url"].as_str().unwrap().starts_with("/latest"),
            "{path}"
        );
        assert_eq!(open["category"], Value::Null, "{path}");
    }
    // A read-restricted category the member can't see is not found.
    sqlx::query("UPDATE categories SET read_restricted = true WHERE id = 2")
        .execute(&db.pool)
        .await
        .unwrap();
    let html = page(&st, "/new-topic?category_id=2", Some(&member))
        .await
        .body;
    assert_eq!(instruction(&html)["url"], "/latest");
}

#[tokio::test]
async fn the_composer_opens_only_for_who_may_create_topics() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let member = logged_in(&st, "user1").await;
    // Staff only (admins, moderators, staff).
    set_setting(&db.pool, "create_topic_allowed_groups", GROUP_LIST, "1|2|3").await;
    for path in ["/new-topic?title=T", "/new-topic?category=general"] {
        let html = page(&st, path, Some(&member)).await.body;
        assert_eq!(instruction(&html)["open"], false, "{path}");
    }
    // The message route opens whatever the member may do.
    let html = page(&st, "/new-message?username=user2", Some(&member))
        .await
        .body;
    assert_eq!(instruction(&html)["open"], true);
}

#[tokio::test]
async fn new_message_goes_to_the_homepage() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let member = logged_in(&st, "user1").await;
    let reply = page(
        &st,
        "/new-message?username=user2&title=Hi&body=There&order=views&group_name=admins&category=general",
        Some(&member),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    // No category for a message: group_name is a discovery param too.
    assert_eq!(
        instruction(&reply.body),
        json!({
            "route": "new-message",
            "url": "/latest?group_name=admins&order=views",
            "params": {
                "username": "user2",
                "title": "Hi",
                "body": "There",
                "order": "views",
                "group_name": "admins",
                "category": "general",
            },
            "category": null,
            "sharedDraft": false,
            "open": true,
            "chooser": null,
        })
    );
}

#[tokio::test]
async fn the_categories_homepage_keeps_only_filter() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    sqlx::query(
        "UPDATE user_options SET homepage_id = 2 WHERE user_id = \
         (SELECT id FROM users WHERE username = 'user1')",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let member = logged_in(&st, "user1").await;
    let reply = page(
        &st,
        "/new-topic?filter=tracked&order=views&title=T",
        Some(&member),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        instruction(&reply.body)["url"],
        "/categories?filter=tracked"
    );
}

#[tokio::test]
async fn array_params_go_on_as_arrays() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let member = logged_in(&st, "user1").await;
    // Collected under the bare name; the discovery list's are joined
    // with commas in the URL the route goes to.
    let reply = page(
        &st,
        "/new-topic?tags[]=howto&title=T&tags%5B%5D=guide&order[]=views&x[]=1&body[]=B",
        Some(&member),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        instruction(&reply.body),
        json!({
            "route": "new-topic",
            "url": "/latest?order=views&tags=howto%2Cguide",
            "params": {
                "tags": ["howto", "guide"],
                "title": "T",
                "order": ["views"],
                "x": ["1"],
                "body": ["B"],
            },
            "category": null,
            "sharedDraft": false,
            "open": true,
            "chooser": null,
        })
    );
    // An empty string before the array gives way to it, and a plain pair
    // after it replaces it.
    for (path, tags) in [
        ("/new-topic?tags=&tags[]=howto", json!(["howto"])),
        ("/new-topic?tags[]=howto&tags=guide", json!("guide")),
        ("/new-topic?tags[]=", json!([""])),
    ] {
        let html = page(&st, path, Some(&member)).await.body;
        assert_eq!(instruction(&html)["params"]["tags"], tags, "{path}");
    }
    // An array after a plain pair that isn't empty: route-recognizer
    // throws before the route is reached (refused, a 500).
    for path in [
        "/new-topic?tags=howto&tags[]=guide",
        "/new-message?username=user2&username[]=user3",
    ] {
        let reply = page(&st, path, Some(&member)).await;
        assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR, "{path}");
        assert!(!reply.body.contains("composer-open"), "{path}");
    }
    // No category has an array for an id, and `category` is not read
    // while `category_id` is given.
    let html = page(
        &st,
        "/new-topic?category_id[]=4&category=general",
        Some(&member),
    )
    .await
    .body;
    let open = instruction(&html);
    assert_eq!(open["url"], "/latest");
    assert_eq!(open["category"], Value::Null);
    // The route throws on an array `category` (its `split`).
    let reply = page(&st, "/new-topic?category[]=general", Some(&member)).await;
    assert_eq!(
        reply.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        reply.body
    );
    // The message route's too.
    let html = page(
        &st,
        "/new-message?username[]=user2&username[]=user3&group_name[]=admins",
        Some(&member),
    )
    .await
    .body;
    let open = instruction(&html);
    assert_eq!(open["url"], "/latest?group_name=admins");
    assert_eq!(open["params"]["username"], json!(["user2", "user3"]));
}

#[tokio::test]
async fn a_category_the_member_cannot_post_in_is_read_only_in_the_chooser() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    // Site Feedback: everyone may read, only staff create topics.
    sqlx::query(
        "INSERT INTO category_groups (category_id, group_id, permission_type, created_at, updated_at) \
         VALUES (2, 0, 3, now(), now()), (2, 3, 1, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let member = logged_in(&st, "user1").await;
    let reply = page(
        &st,
        "/new-topic?category=site-feedback&title=T",
        Some(&member),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let html = &reply.body;
    // The page's own chooser lists only where the member may post.
    assert!(
        html.contains("<title>Latest Site Feedback topics - "),
        "{html}"
    );
    let open = instruction(html);
    assert_eq!(open["url"], "/c/2");
    assert_eq!(open["category"], 2);
    assert_eq!(open["open"], true);
    // prioritizedCategoryId: its row first, then the others as listed
    // (uncategorized, empty, last).
    assert_eq!(open["chooser"]["order"], json!([2, 4, 34, 1]));
    assert_eq!(open["chooser"]["choose"], "Category…");
    let row = open["chooser"]["readOnlyRow"].as_str().unwrap();
    assert!(
        row.starts_with(
            "<div class=\"category-row select-kit-row\" data-name=\"Site Feedback\" \
             data-title=\"This category is read-only so you cannot choose it for this topic\" \
             data-value=\"2\" data-url=\"/c/site-feedback/2\" data-read-only=\"true\" \
             role=\"menuitemradio\" tabindex=\"-1\" \
             title=\"This category is read-only so you cannot choose it for this topic\">\
             <div class=\"category-status\"><span "
        ),
        "{row}"
    );
    assert!(
        row.contains(
            "<span class=\"topic-count\" aria-label=\"1 topic in this category\">× 1</span>\
             <span class=\"read-only\" aria-label=\"read-only\">read-only</span></span></div>"
        ),
        "{row}"
    );

    // A category the member may post in: no read-only row, its family
    // (the parent and the parent's subcategories) first.
    let html = page(&st, "/new-topic?category_id=34", Some(&member))
        .await
        .body;
    let open = instruction(&html);
    assert_eq!(open["chooser"]["order"], json!([4, 34, 1]));
    assert_eq!(open["chooser"]["readOnlyRow"], Value::Null);

    // Staff may post there.
    let admin = logged_in(&st, "admin").await;
    let html = page(&st, "/new-topic?category_id=2", Some(&admin))
        .await
        .body;
    let open = instruction(&html);
    assert_eq!(open["chooser"]["readOnlyRow"], Value::Null);
    assert_eq!(open["chooser"]["order"][0], 2);

    // The chooser is the composer's, which only opens for who may
    // create topics.
    set_setting(&db.pool, "create_topic_allowed_groups", GROUP_LIST, "1|2|3").await;
    let html = page(&st, "/new-topic?category_id=2", Some(&member))
        .await
        .body;
    let open = instruction(&html);
    assert_eq!(open["open"], false);
    assert_eq!(open["chooser"], Value::Null);
}
