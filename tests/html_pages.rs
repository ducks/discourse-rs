//! The server-rendered pages for anonymous readers.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;

async fn get(pool: &PgPool, path: &str) -> (StatusCode, String, String) {
    let app = discourse_rs::app(state(pool.clone(), config(RailsEnv::Test, &[])).await);
    let response = app
        .oneshot(
            Request::get(path)
                .header(header::HOST, "test.localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        content_type,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

#[tokio::test]
async fn home_and_latest_render_the_topic_list() {
    let db = TestDb::new().await;
    for path in ["/", "/latest"] {
        let (status, content_type, html) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(
            content_type.starts_with("text/html"),
            "{path}: {content_type}"
        );
        assert!(html.contains("<!DOCTYPE html>"));
        // Latest is the homepage: no title token (BuildTopicRoute).
        assert!(html.contains("<title>Discourse</title>"), "{path}");
        assert!(
            html.contains(r#"href="/t/welcome-to-discourse/5/1""#),
            "{path}"
        );
        assert!(
            html.contains(
                r#"Welcome to Discourse! <img width="20" height="20" src='/images/emoji/twitter/wave.png?v=15' title='wave' alt='wave' class='emoji'>"#
            ),
            "emoji title"
        );
        assert!(html.contains(r#"<span class="badge-category__name">General</span>"#));
        assert!(html.contains(
            r#"<a href='/tag/howto/1'  data-tag-name=howto class='discourse-tag simple'>howto</a>"#
        ));
        assert!(html.contains(r#"<table class="topic-list">"#));
        // Dates in the viewer's zone: dates.js in the head with the
        // formats it draws with, and the activity cells' titles as
        // templates beside their UTC text (crate::viewer_dates).
        let head = &html[..html.find("</head>").unwrap()];
        assert!(
            head.contains(r#"<script src="/assets/dates.js?v="#),
            "{path}"
        );
        assert!(head.contains("data-relative-date-duration=\"30\""));
        assert!(head.contains("&quot;dates.long_with_year&quot;:&quot;MMM D, YYYY h:mm a&quot;"));
        assert!(html.contains(
            r#" title="Created: Jul 25, 2026 5:58 am" data-tz-title="Created: %{1784959108320:MMM D, YYYY h:mm a}"><a class="post-activity" href="/t/welcome-to-discourse/5/1"><span class='relative-date' data-time='1784959108405' data-format='tiny'>"#
        ), "{path}");
        assert!(!html.contains("Parity fixture: unlisted topic"));
        assert!(!html.contains("Parity fixture: deleted topic"));
    }
}

#[tokio::test]
async fn latest_pages_link_to_the_next_html_page() {
    let db = TestDb::new().await;
    let (_, _, html) = get(&db.pool, "/latest?per_page=2").await;
    assert!(
        html.contains(r#"href="/latest?page=1&#38;per_page=2" rel="next""#),
        "{html}"
    );
    assert!(!html.contains("no_definitions"));
    let (_, _, last) = get(&db.pool, "/latest?page=2&per_page=2").await;
    assert!(!last.contains(r#"rel="next""#));
}

#[tokio::test]
async fn topic_page_renders_posts_and_small_actions() {
    let db = TestDb::new().await;
    let (status, content_type, html) =
        get(&db.pool, "/t/parity-fixture-pinned-and-closed/37").await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type.starts_with("text/html"));
    assert!(html.contains("<title>Parity fixture: pinned and closed - Discourse</title>"));
    assert!(
        html.contains(r#"<link rel="canonical" href="http://test.localhost/t/parity-fixture-pinned-and-closed/37">"#)
    );
    assert!(html.contains(r#"<span class="badge-category__name">Site Feedback</span>"#));
    assert!(html.contains(r#"<span class="topic-status --closed""#));
    assert!(html.contains(r#"id="post_1""#));
    assert!(html.contains(r#"<div class="cooked"><p>A pinned, closed topic in Site Feedback"#));
    // The close is a small action: its icon, the actor, what they did.
    assert!(html.contains(r#"class="small-action onscreen-post""#));
    assert!(html.contains(r#"<p aria-hidden="true">Closed <span class="relative-date""#));
    assert!(html.contains(r#"data-user-card="admin" href="/u/admin""#));
}

#[tokio::test]
async fn topic_page_shows_subcategory_breadcrumbs_and_likes() {
    let db = TestDb::new().await;
    let (_, _, html) = get(&db.pool, "/t/parity-fixture-tagged-in-a-subcategory/38").await;
    // The subcategory's badge, marked with its parent.
    assert!(html.contains(
        r#"href="/c/general/sub-general/34"><span data-category-id="34" data-parent-category-id="4" data-drop-close="true" class="badge-category --has-parent --style-square">"#
    ));
    assert!(html.contains(
        r#"<li><a href='/tag/howto/1'  data-tag-name=howto class='discourse-tag simple'>howto</a>"#
    ));

    // Core's like button, where discourse-reactions doesn't replace it.
    common::set_setting(&db.pool, "discourse_reactions_enabled", 5, "f").await;
    let (_, _, html) = get(&db.pool, "/t/parity-fixture-liked-and-archived/41").await;
    // The like count, and the like button an anonymous reader may press
    // (it asks them to log in), disabled in an archived topic.
    assert!(
        html.contains(
            r#"aria-label="1 person liked this post." class="btn btn-flat no-text post-action-menu__like-count like-count button-count highlight-action regular-likes btn-flat""#
        ),
        "{html}"
    );
    assert!(html.contains(r#"title="like this post" type="button" disabled>"#));
}

#[tokio::test]
async fn topic_page_shows_reactions_in_place_of_the_like_button() {
    let db = TestDb::new().await;
    let (_, _, html) = get(&db.pool, "/t/parity-fixture-liked-and-archived/41").await;
    // The liked post's summary: the like as the main reaction.
    assert!(
        html.contains(
            r#"<nav class="post-controls expanded" role="none"><div class="reactions-actions-summary"><div class="discourse-reactions-actions has-reactions" id="discourse-reactions-actions-46-left""#
        ),
        "{html}"
    );
    assert!(html.contains(
        r#"<button aria-expanded="false" aria-haspopup="dialog" aria-label="1 reaction" class="discourse-reactions-counter only-like" id="discourse-reactions-counter-46-left" type="button""#
    ));
    assert!(html.contains(
        r#"<span class="discourse-reactions-list-emoji" id="discourse-reactions-list-emoji-46-heart"><img width="20" height="20" src="/images/emoji/twitter/heart.png?v=15" title="heart" alt="heart" class="emoji"></span>"#
    ));
    // The button: no reacting in an archived topic (no
    // can-toggle-reaction), titled for an anonymous reader.
    assert!(html.contains(
        r#"<div class="discourse-reactions-reaction-button" title="Please sign up or log in to like this post"><button class="btn no-text btn-icon btn-toggle-reaction-like btn-flat btn-icon no-text reaction-button" title="Please sign up or log in to like this post" type="button">"#
    ));
    assert!(html.contains(r#"class="discourse-reactions-actions has-reactions" id="discourse-reactions-actions-46-right""#));
    assert!(!html.contains("post-action-menu__like"));
    assert!(html.contains("/assets/discourse-reactions.js"));
}

#[tokio::test]
async fn html_requests_without_a_slug_redirect() {
    let db = TestDb::new().await;
    let (status, _, _) = get(&db.pool, "/t/35").await;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    // A browser gets the not-found page (tests/not_found_page.rs).
    let (status, _, body) = get(&db.pool, "/t/999999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains(r#"<div class="page-not-found">"#));
}

#[tokio::test]
async fn stylesheet_is_served() {
    let db = TestDb::new().await;
    let (status, content_type, css) = get(&db.pool, "/assets/site.css").await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type.starts_with("text/css"));
    assert!(css.contains("topic-pagination"));
    let (status, content_type, css) = get(&db.pool, "/assets/discourse.css").await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type.starts_with("text/css"));
    assert!(css.contains(".topic-list .posters"));
}

/// Each bundled asset has its route: a page that links one without it
/// gets a 404 for the script.
#[tokio::test]
async fn every_asset_is_served() {
    let db = TestDb::new().await;
    for name in discourse_rs::assets::names() {
        let (status, _, _) = get(&db.pool, &format!("/assets/{name}")).await;
        assert_eq!(status, StatusCode::OK, "/assets/{name}");
    }
}

/// Each page says where its live updates start, as a pg-bus position, and
/// polling from there delivers what changed after the page was read.
#[tokio::test(flavor = "multi_thread")]
async fn pages_carry_where_their_live_updates_start() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let position_of = |path: &'static str| {
        let st = st.clone();
        async move {
            let response = discourse_rs::app(st)
                .oneshot(
                    Request::get(path)
                        .header(header::HOST, "test.localhost")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let body = String::from_utf8_lossy(&bytes).into_owned();
            let marker = "<meta name=\"bus-position\" content=\"";
            let start = body
                .find(marker)
                .unwrap_or_else(|| panic!("{path} has no bus position"))
                + marker.len();
            let end = start + body[start..].find('"').unwrap();
            body[start..end]
                .parse::<pg_bus::Position>()
                .unwrap_or_else(|e| panic!("{path}: {e}"))
        }
    };
    for path in [
        "/",
        "/categories",
        "/t/parity-fixture-replies-and-posters/35",
        "/u/user1",
        "/search?q=fixture",
    ] {
        position_of(path).await;
    }

    let from = position_of("/latest").await;
    let mut conn = db.pool.acquire().await.unwrap();
    let settings = discourse_rs::site_settings::SiteSettings::load(
        &mut conn,
        &st.site_setting_defs,
        &st.config.globals,
    )
    .await
    .unwrap();
    drop(conn);
    let mut tx = db.pool.begin().await.unwrap();
    discourse_rs::topic_tracking_state::publish_latest(&st.bus, &settings, &mut tx, 35)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let messages = common::bus_messages(&st, from, &["/latest"]).await;
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0].data["topic_id"], 35);
}

/// The site's color schemes as stylesheets: the default theme has no
/// scheme of its own (the base light palette) and dark mode uses the Dark
/// scheme. parity/writes/stylesheets compares them with the reference's
/// byte for byte.
#[tokio::test]
async fn color_definitions_are_the_sites_schemes() {
    let db = TestDb::new().await;
    for (path, primary) in [
        ("/assets/color_definitions_light.css", "--primary: #222;"),
        ("/assets/color_definitions_dark.css", "--primary: #dddddd;"),
        // The same scheme by id, as a member's own palette is linked.
        (
            "/assets/color_definitions_scheme/13.css",
            "--primary: #dddddd;",
        ),
    ] {
        let (status, content_type, css) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::OK, "{path}: {css}");
        assert_eq!(content_type, "text/css; charset=utf-8");
        assert!(
            css.starts_with(":root{--csstools-color-scheme--light:"),
            "{path}"
        );
        assert!(css.contains(primary), "{path}");
    }
    for path in [
        "/assets/color_definitions_scheme/999.css",
        "/assets/color_definitions_scheme/dark.css",
    ] {
        let (status, _, _) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
}

/// Around the posts: the topic map in the first post and at the bottom,
/// the timeline, an anonymous reader's reply and the suggested topics.
#[tokio::test]
async fn topic_page_has_its_map_timeline_footer_and_suggestions() {
    let db = TestDb::new().await;
    let (_, _, html) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35").await;
    let first = html.find(r#"id="post_1""#).unwrap();
    let second = html.find(r#"id="post_2""#).unwrap();
    let op_map = html
        .find(r#"<div class="post__topic-map topic-map --op">"#)
        .expect("the first post's topic map");
    assert!(first < op_map && op_map < second);
    assert!(html.contains(r#"<div class="topic-map --bottom">"#));
    assert!(html.contains(r#"<div class="topic-map__users-list --users-summary">"#));
    assert!(html.contains(r#"<span class="post-count">2</span>"#));
    // The timeline at the first of five posts.
    assert!(html.contains(r#"<div class="with-timeline topic-navigation">"#));
    assert!(html.contains(r#"<div class="timeline-replies">1 / 5</div>"#));
    assert!(html.contains(r#"<a class="start-date" href="/t/parity-fixture-replies-and-posters/35/1" title="Jump to the first post">"#));
    assert!(html.contains(
        r#"<a class="now-date" href="/t/parity-fixture-replies-and-posters/35/5"><span><span class="relative-date" title="Jump to the last post""#
    ));
    // Its dates as templates static/js/dates.js draws in the viewer's
    // zone: timelineDate, the last post's age with ago, the scroller's
    // label from the days ago, which the lookup carries for topic.js.
    let start = html.find(r#"title="Jump to the first post"><span data-tz-text="%{"#);
    assert!(start.is_some(), "{html}");
    assert!(html[start.unwrap()..].contains(r#":@timeline}">"#));
    assert!(html.contains(r#"data-format="tiny" data-tz-text="%{"#));
    assert!(html.contains(r#":@timeline-ago}">"#));
    assert!(html.contains(r#"<div class="timeline-ago" data-tz-text="%{"#));
    assert!(html.contains(r#":@timeline-days-ago}">"#));
    assert!(html.contains(r#"data-lookup="[[1,"#));
    // The posts' dates: their longDate titles and screen reader texts.
    assert!(html.contains(r#"data-tz-title="%{"#));
    assert!(html.contains(r#":MMM D, YYYY h:mm a}" data-time=""#));
    assert!(html.contains(r#":@medium-with-ago}" class="post-date""#));
    assert!(html.contains(r#":@medium-with-ago-and-on}">post by "#));
    // The footer's reply asks an anonymous reader to log in.
    assert!(html.contains(
        r#"<div id="topic-footer-buttons" role="region"><div class="topic-footer-main-buttons"><button class="btn btn-icon-text btn-primary" data-login-url="/login" type="button">"#
    ));
    // The suggested topics, and where to read more.
    assert!(html.contains(r#"<h3 class="more-topics__list-title" id="suggested-topics-title">New &amp; Unread Topics</h3>"#));
    assert!(html.contains(r#"<thead class="topic-list-header --has-tabs">"#));
    assert!(
        !html.contains(r#"<td class="posters topic-list-data">"#),
        "no posters column"
    );
    assert!(html.contains(r#"<h3 class="more-topics__browse-more">Want to read more? Browse other topics in <a class="badge-category__wrapper ""#));
    assert!(html.contains(&format!(
        r#"<script src="{}"></script>"#,
        discourse_rs::assets::url("", "topic.js")
    )));
}

/// /p/:post_id(/:user_id) (posts#short_link): a 302 to the post's URL,
/// where the topic page jumps to it; a missing post is not found and one
/// the viewer may not see (a message's) is forbidden.
#[tokio::test]
async fn a_short_link_redirects_to_its_post() {
    let db = TestDb::new().await;
    for path in ["/p/37", "/p/37/3", "/p/37.json"] {
        let app = discourse_rs::app(state(db.pool.clone(), config(RailsEnv::Test, &[])).await);
        let response = app
            .oneshot(
                Request::get(path)
                    .header(header::HOST, "test.localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND, "{path}");
        assert_eq!(
            response.headers()[header::LOCATION],
            "http://test.localhost/t/parity-fixture-replies-and-posters/35/3",
            "{path}"
        );
    }
    for path in ["/p/abc", "/p/99999"] {
        let (status, _, _) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
    sqlx::query("UPDATE posts SET deleted_at = now() WHERE id = 38")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, _, _) = get(&db.pool, "/p/38").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a trashed post");
    let (status, content_type, _) = get(&db.pool, "/p/48").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a message's post");
    assert_eq!(content_type, "text/html; charset=utf-8");
}

/// What a browser gets that accepts compression.
async fn get_compressed(pool: &PgPool, path: &str) -> axum::response::Response {
    let app = discourse_rs::app(state(pool.clone(), config(RailsEnv::Test, &[])).await);
    app.oneshot(
        Request::get(path)
            .header(header::HOST, "test.localhost")
            .header(header::ACCEPT_ENCODING, "br, gzip")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn pages_and_assets_compress_for_browsers_that_accept_it() {
    let db = TestDb::new().await;
    for path in ["/latest", "/latest.json", "/assets/discourse.css"] {
        let response = get_compressed(&db.pool, path).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(response.headers()[header::CONTENT_ENCODING], "br", "{path}");
    }
    // Small bodies stay as they are.
    let response = get_compressed(&db.pool, "/srv/status").await;
    assert!(response.headers().get(header::CONTENT_ENCODING).is_none());
}

/// A handler that holds a connection while asking for another starves
/// once requests outnumber the pool, and every page hung at 16 concurrent
/// requests when the page chrome did so. Each page here is served from a
/// pool of one connection: a second checkout times out into a 500.
#[tokio::test]
async fn pages_take_one_connection_at_a_time() {
    let db = TestDb::new().await;
    let one = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(2))
        .connect_with((*db.pool.connect_options()).clone())
        .await
        .unwrap();
    let st = common::state_with_bus_on(db.pool.clone(), one, config(RailsEnv::Test, &[])).await;
    for path in [
        "/latest",
        "/categories",
        "/c/general/4",
        "/c/general/sub-general/34",
        "/t/parity-fixture-replies-and-posters/35",
        "/t/999999",
        "/u/user1/summary",
        "/tags",
        "/tag/guide",
        "/search?q=fixture",
    ] {
        let response = discourse_rs::app(st.clone())
            .oneshot(
                Request::get(path)
                    .header(header::HOST, "test.localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{path} took a second connection"
        );
    }
}

#[tokio::test]
async fn links_swap_pages_in() {
    let db = TestDb::new().await;
    let (_, _, html) = get(&db.pool, "/latest").await;
    // The body's links are boosted; page.js, in the head, runs the page
    // scripts across swaps, and back and forward ask the server again.
    assert!(html.contains(r#"<body hx-boost="true" class="#));
    assert!(html.contains(r#"<meta name="htmx-config" content='{"historyCacheSize": 0}'>"#));
    let page_js = html.find("/assets/page.js").expect("page.js");
    let htmx = html.find("/assets/htmx.min.js").expect("htmx");
    assert!(page_js < htmx && htmx < html.find("<body").unwrap());
}

/// The templates' client strings come from the locale, so an admin's site
/// text reaches the page as it does in the Ember client.
#[tokio::test]
async fn site_texts_reach_the_templates_client_strings() {
    let db = TestDb::new().await;
    for (key, value) in [
        ("js.sign_up", "Join us"),
        ("js.log_in", "Enter"),
        ("js.sr_topic_list_caption", "All the topics"),
        ("js.replies", "Answers"),
        ("js.filters.top.title", "Best"),
        ("js.user.card", "Member card"),
        ("js.tagging.tags", "Labels"),
    ] {
        sqlx::query(
            "INSERT INTO translation_overrides (locale, translation_key, value, \
             original_translation, status, created_at, updated_at) \
             VALUES ('en', $1, $2, '', 0, now(), now())",
        )
        .bind(key)
        .bind(value)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    let (_, _, top) = get(&db.pool, "/top").await;
    assert!(
        top.contains("<title>Best topics - Discourse</title>"),
        "{top}"
    );
    let (_, _, html) = get(&db.pool, "/latest").await;
    assert!(html.contains(r#"<span class="d-button-label">Join us</span>"#));
    assert!(html.contains(r#"<span class="d-button-label">Enter</span>"#));
    assert!(html.contains(r#"<caption class="sr-only">All the topics</caption>"#));
    assert!(html.contains("<button>Answers</button>"));
    assert!(html.contains(r#"data-label="Member card""#));
    assert!(!html.contains(">Sign Up<"));
    let (_, _, tags) = get(&db.pool, "/tags").await;
    assert!(tags.contains("<title>Labels - Discourse</title>"), "{tags}");
}

/// The `<title>` of each list: what Ember sets once the route loads
/// (BuildTopicRoute, BuildCategoryRoute, TagShowRoute and
/// DiscoveryCategoriesRoute titleTokens, then the site title).
#[tokio::test]
async fn list_titles_follow_the_filter_category_and_tag() {
    let db = TestDb::new().await;
    for (path, title) in [
        ("/", "Discourse"),
        ("/latest", "Discourse"),
        ("/latest?page=1", "Discourse"),
        ("/top", "Top topics - Discourse"),
        ("/top?period=weekly", "Top topics - Discourse"),
        ("/hot", "Hot topics - Discourse"),
        ("/categories", "Categories - Discourse"),
        ("/c/general/4", "Latest General topics - Discourse"),
        ("/c/general/4/none", "Latest General topics - Discourse"),
        ("/c/general/4/l/top", "Top General topics - Discourse"),
        ("/c/general/4/l/hot", "Hot General topics - Discourse"),
        (
            "/c/general/sub-general/34",
            "Latest General/Sub General topics - Discourse",
        ),
        (
            "/c/general/sub-general/34/l/top",
            "Top General/Sub General topics - Discourse",
        ),
        ("/tag/howto/1", "Latest howto topics - Discourse"),
        ("/tag/howto/1/l/top", "Top howto topics - Discourse"),
        (
            "/tags/c/general/4/howto/1",
            "Latest howto topics in General - Discourse",
        ),
        (
            "/tags/c/general/4/howto/1/l/hot",
            "Hot howto topics in General - Discourse",
        ),
        ("/tag/none", "Latest untagged topics - Discourse"),
        (
            "/tags/c/general/4/none",
            "Latest untagged topics in General - Discourse",
        ),
    ] {
        let (status, _, html) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(
            html.contains(&format!("<title>{title}</title>")),
            "{path}: {}",
            &html[..html.find("</title>").unwrap_or(0)]
        );
    }

    // The short site description follows the site title on the front
    // page only.
    common::set_setting(&db.pool, "short_site_description", 1, "Talk shop").await;
    let (_, _, html) = get(&db.pool, "/").await;
    assert!(html.contains("<title>Discourse - Talk shop</title>"));
    let (_, _, html) = get(&db.pool, "/latest").await;
    assert!(html.contains("<title>Discourse</title>"));

    // Another homepage: its list loses the token, latest gains one.
    common::set_setting(&db.pool, "default_homepage", 7, "top").await;
    let (_, _, html) = get(&db.pool, "/top").await;
    assert!(html.contains("<title>Discourse</title>"));
    let (_, _, html) = get(&db.pool, "/latest").await;
    assert!(html.contains("<title>Latest topics - Discourse</title>"));
    common::set_setting(&db.pool, "default_homepage", 7, "categories").await;
    let (_, _, html) = get(&db.pool, "/categories").await;
    assert!(html.contains("<title>Discourse</title>"));
    // A category's list keeps its token whatever the homepage.
    let (_, _, html) = get(&db.pool, "/c/general/4/l/top").await;
    assert!(html.contains("<title>Top General topics - Discourse</title>"));

    // The member filters, and a member's own homepage.
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Member::logged_in(st, "user1").await;
    for (path, title) in [
        ("/new", "New topics - Discourse"),
        ("/unread", "Unread topics - Discourse"),
        ("/unseen", "Unseen topics - Discourse"),
        ("/read", "Read topics - Discourse"),
        ("/posted", "My posts topics - Discourse"),
        ("/bookmarks", "Bookmarks topics - Discourse"),
        ("/c/general/4/l/new", "New General topics - Discourse"),
        ("/c/general/4/l/unread", "Unread General topics - Discourse"),
        ("/latest", "Latest topics - Discourse"),
        ("/categories", "Discourse"),
    ] {
        let html = user1.page(path).await;
        assert!(
            html.contains(&format!("<title>{title}</title>")),
            "{path}: {}",
            &html[..html.find("</title>").unwrap_or(0)]
        );
    }
    sqlx::query("UPDATE user_options SET homepage_id = 4 FROM users WHERE users.id = user_id AND username = 'user1'")
        .execute(&db.pool)
        .await
        .unwrap();
    let html = user1.page("/new").await;
    assert!(html.contains("<title>Discourse</title>"));
    let html = user1.page("/categories").await;
    assert!(html.contains("<title>Categories - Discourse</title>"));
}

// The categories page with a member's tracking state: CategoryUnread's
// unread and new counts beside each category and subcategory, and the
// latest column's rows as Ember draws them from the read state (visited,
// the unread count, links to the first unread post).
#[tokio::test]
async fn categories_page_draws_the_members_read_state() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Member::logged_in(st, "user1").await;
    let html = user1.page("/categories").await;
    // Navigation's body class, which spaces the muted categories.
    assert!(html.contains(" navigation-categories categories-list"));
    // General: the replies-and-posters topic, read to post 2 of 5.
    assert!(html.contains(
        r#"<div class="unread-new"><div class="category__badges unread-new"><a class="badge new-posts badge-notification" href="/c/general/4/l/unread" title="1 unread topic">1 unread</a></div></div>"#
    ));
    // Sub General's definition topic is new, counted for it alone
    // (is_category_topic).
    assert!(html.contains(
        r#"<span class="category__badges"><a class="badge new-posts badge-notification" href="/c/general/sub-general/34/l/new" title="1 new topic">1 new</a></span></span>"#
    ));
    // The latest column: the unread topic links to its first unread post
    // with the count; the read one is visited.
    assert!(html.contains(
        r#"<a href="/t/parity-fixture-replies-and-posters/35/3" class="title" data-topic-id="35">Parity fixture: replies and posters</a><span class="topic-post-badges"> &nbsp;<a aria-description="you have 3 unread posts in this topic" class="badge badge-notification unread-posts" href="/t/parity-fixture-replies-and-posters/35/3" title="you have 3 unread posts in this topic">3</a></span>"#
    ));
    assert!(html.contains(
        r#"<div class="latest-topic-list-item tag-howto category-general-sub-general visited" data-topic-id="38">"#
    ));
    assert!(html.contains(
        r#"<a href="/t/parity-fixture-tagged-in-a-subcategory/38/1" class="title" data-topic-id="38">Parity fixture: tagged in a subcategory<span class="sr-only">&nbsp;read</span></a><span class="topic-post-badges"></span>"#
    ));

    // A visitor: the badges' elements stay, empty, and nothing is read.
    let (_, _, html) = get(&db.pool, "/categories").await;
    assert!(html.contains(
        r#"<div class="unread-new"><div class="category__badges unread-new"></div></div>"#
    ));
    assert!(html.contains(r#"<span class="category__badges"></span></span>"#));
    assert!(!html.contains("badge new-posts"));
    assert!(!html.contains(" visited\""));
    assert!(html.contains(
        r#"<a href="/t/parity-fixture-replies-and-posters/35" class="title" data-topic-id="35">Parity fixture: replies and posters</a><span class="topic-post-badges"></span>"#
    ));
}

/// A member's session on the in-process router.
struct Member {
    state: discourse_rs::AppState,
    cookies: Vec<String>,
}

impl Member {
    async fn send(&mut self, request: axum::http::request::Builder, body: &str) -> String {
        let request = request
            .header(header::HOST, "test.localhost")
            .header(header::COOKIE, self.cookies.join("; "));
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let pair = v.to_str().unwrap().split(';').next().unwrap().to_string();
            let name = pair.split('=').next().unwrap().to_string();
            self.cookies.retain(|c| !c.starts_with(&format!("{name}=")));
            if !pair.ends_with('=') {
                self.cookies.push(pair);
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    async fn logged_in(state: discourse_rs::AppState, username: &str) -> Member {
        let mut member = Member {
            state,
            cookies: Vec::new(),
        };
        let xhr = |r: axum::http::request::Builder| r.header("x-requested-with", "XMLHttpRequest");
        let csrf = member
            .send(xhr(Request::get("/session/csrf.json")), "")
            .await;
        let csrf: serde_json::Value = serde_json::from_str(&csrf).unwrap();
        let login = Request::post("/session")
            .header("x-csrf-token", csrf["csrf"].as_str().unwrap())
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        member
            .send(xhr(login), &format!("login={username}&password=password"))
            .await;
        member
    }

    async fn page(&mut self, path: &str) -> String {
        self.send(Request::get(path), "").await
    }
}

impl Member {
    async fn json(&mut self, path: &str) -> serde_json::Value {
        let body = self
            .send(
                Request::get(path).header(header::ACCEPT, "application/json"),
                "",
            )
            .await;
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("{path}: {e}: {body}"))
    }
}

async fn get_json(pool: &PgPool, path: &str) -> (StatusCode, serde_json::Value) {
    let app = discourse_rs::app(state(pool.clone(), config(RailsEnv::Test, &[])).await);
    let response = app
        .oneshot(
            Request::get(path)
                .header(header::HOST, "test.localhost")
                .header(header::ACCEPT, "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&bytes);
    let json = serde_json::from_str(&body).unwrap_or_else(|e| panic!("{path}: {e}: {body}"));
    (status, json)
}

/// The filter of the list's active navigation item.
fn active_nav(html: &str) -> Option<&str> {
    let start = html.find("<li class=\"active ")?;
    let rest = &html[start..];
    let item = &rest[rest.find("nav-item_")? + "nav-item_".len()..];
    Some(&item[..item.find('"')?])
}

/// `/` is the current homepage's page (config/routes.rb's root routes under
/// HomePageConstraint), as HTML and as JSON: SiteSetting.homepage, the
/// anonymous homepage and a member's own.
#[tokio::test]
async fn front_page_serves_the_current_homepage() {
    let db = TestDb::new().await;
    common::set_setting(&db.pool, "short_site_description", 1, "Talk shop").await;

    for homepage in ["latest", "top", "hot", "categories"] {
        common::set_setting(&db.pool, "default_homepage", 7, homepage).await;
        let (status, _, html) = get(&db.pool, "/").await;
        assert_eq!(status, StatusCode::OK, "{homepage}");
        // No title token on the homepage's own route; the short site
        // description on the front page.
        assert!(
            html.contains("<title>Discourse - Talk shop</title>"),
            "{homepage}: {}",
            &html[..html.find("</title>").unwrap_or(0)]
        );
        let (status, json) = get_json(&db.pool, "/").await;
        assert_eq!(status, StatusCode::OK, "{homepage}");
        let (_, twin) = get_json(&db.pool, &format!("/{homepage}.json")).await;
        assert_eq!(json, twin, "{homepage}");
        if homepage == "categories" {
            assert!(html.contains("category-list"), "{homepage}");
            assert!(json["category_list"].is_object());
        } else if homepage == "top" {
            // Not in the default top menu: no pill (DNavigation).
            assert_eq!(active_nav(&html), None, "{homepage}");
            assert!(html.contains(r#"<div class="top-lists">"#));
            assert!(json["topic_list"].is_object());
        } else {
            assert_eq!(active_nav(&html), Some(homepage), "{homepage}");
            assert!(json["topic_list"].is_object());
        }
    }

    // `?format=json` asks for JSON too.
    let (_, _, body) = get(&db.pool, "/?format=json").await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json["category_list"].is_object());

    // A homepage only members have: anonymous readers get the first top
    // menu item they may have (SiteSetting.anonymous_homepage).
    common::set_setting(&db.pool, "top_menu", 8, "latest|new|top|categories").await;
    common::set_setting(&db.pool, "default_homepage", 7, "new").await;
    let (_, _, html) = get(&db.pool, "/").await;
    assert_eq!(active_nav(&html), Some("latest"));
    common::set_setting(&db.pool, "top_menu", 8, "top|latest|new|categories").await;
    let (_, _, html) = get(&db.pool, "/").await;
    assert_eq!(active_nav(&html), Some("top"));
    let (_, json) = get_json(&db.pool, "/").await;
    let (_, top) = get_json(&db.pool, "/top.json").await;
    assert_eq!(json, top);

    // Members get SiteSetting.homepage, unless they picked their own.
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Member::logged_in(st, "user1").await;
    let html = user1.page("/").await;
    assert_eq!(active_nav(&html), Some("new"));
    assert!(html.contains("<title>Discourse - Talk shop</title>"));
    assert_eq!(user1.json("/").await, user1.json("/new.json").await);
    for (homepage_id, homepage) in [(5, "top"), (6, "bookmarks"), (2, "categories")] {
        sqlx::query(
            "UPDATE user_options SET homepage_id = $1 FROM users \
             WHERE users.id = user_id AND username = 'user1'",
        )
        .bind(homepage_id)
        .execute(&db.pool)
        .await
        .unwrap();
        let html = user1.page("/").await;
        assert!(
            html.contains("<title>Discourse - Talk shop</title>"),
            "{homepage}"
        );
        assert_eq!(
            user1.json("/").await,
            user1.json(&format!("/{homepage}.json")).await,
            "{homepage}"
        );
        if homepage == "categories" {
            assert!(html.contains("category-list"), "{homepage}");
        } else {
            // top is in the menu here; bookmarks is not, so has no pill.
            let pill = (homepage == "top").then_some(homepage);
            assert_eq!(active_nav(&html), pill, "{homepage}");
        }
    }
    // hot is a member's homepage only while it is in the top menu.
    sqlx::query(
        "UPDATE user_options SET homepage_id = 8 FROM users \
         WHERE users.id = user_id AND username = 'user1'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    common::set_setting(&db.pool, "top_menu", 8, "latest|new|hot|categories").await;
    let html = user1.page("/").await;
    assert_eq!(active_nav(&html), Some("hot"));
    common::set_setting(&db.pool, "top_menu", 8, "latest|new|categories").await;
    let html = user1.page("/").await;
    assert_eq!(active_nav(&html), Some("new"));
}

/// The homepages `/` has no page for: a theme's custom homepage, and the
/// installation page while has_login_hint is on. Login required answers
/// the front page's JSON as Rails does (home_page#blank's
/// redirect_to_login_if_required).
#[tokio::test]
async fn front_page_refuses_custom_and_finish_installation() {
    let db = TestDb::new().await;
    // The default theme's modifiers (theme -1, the seed's default_theme_id).
    sqlx::query("UPDATE theme_modifier_sets SET custom_homepage = TRUE WHERE theme_id = -1")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, _, _) = get(&db.pool, "/").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    sqlx::query("UPDATE theme_modifier_sets SET custom_homepage = NULL")
        .execute(&db.pool)
        .await
        .unwrap();

    let (status, _, _) = get(&db.pool, "/?format=rss").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

    common::set_setting(&db.pool, "has_login_hint", 5, "t").await;
    let (status, _, _) = get(&db.pool, "/").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    common::set_setting(&db.pool, "has_login_hint", 5, "f").await;

    common::set_setting(&db.pool, "login_required", 5, "t").await;
    let (status, json) = get_json(&db.pool, "/").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(json["error_type"], "not_logged_in");
    let (status, content_type, html) = get(&db.pool, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type.starts_with("text/html"));
    assert!(html.contains("login"), "the login page");
}

/// A category the viewer may see under a parent they may not: Site's
/// categories leave it out, so BuildCategoryRoute finds nothing and goes
/// to /404; its JSON is served as before. One whose parent the site's
/// categories leave out (a hidden grandparent) has Ember's titleToken
/// throw on the missing parent: refused.
#[tokio::test]
async fn category_list_under_a_hidden_parent() {
    let db = TestDb::new().await;
    common::set_setting(&db.pool, "max_category_nesting", 3, "3").await;
    let grandchild: i32 = sqlx::query_scalar(
        "INSERT INTO categories (name, name_lower, slug, color, text_color, user_id, \
         parent_category_id, read_restricted, position, created_at, updated_at) \
         VALUES ('Deep', 'deep', 'deep', '0088CC', 'FFFFFF', -1, 34, false, \
                 (SELECT max(position) + 1 FROM categories), now(), now()) RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let deep = format!("/c/general/sub-general/deep/{grandchild}");
    let (status, _, html) = get(&db.pool, &deep).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<title>Latest Sub General/Deep topics - Discourse</title>"));

    sqlx::query("UPDATE categories SET read_restricted = TRUE WHERE id = 4")
        .execute(&db.pool)
        .await
        .unwrap();
    let app = discourse_rs::app(state(db.pool.clone(), config(RailsEnv::Test, &[])).await);
    let response = app
        .oneshot(
            Request::get("/c/general/sub-general/34")
                .header(header::HOST, "test.localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        response.headers()[header::LOCATION],
        "http://test.localhost/404"
    );
    let (status, json) = get_json(&db.pool, "/c/general/sub-general/34.json").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["topic_list"].is_object());

    let (status, _, _) = get(&db.pool, &deep).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

/// A top list draws DiscoveryTopics' PeriodChooser above its topics: the
/// list's period (`for_period`) with its dates in the header, and a row
/// for each period for top-period.js to open. Other lists have none.
#[tokio::test]
async fn top_lists_draw_the_period_chooser() {
    let db = TestDb::new().await;
    for (path, period, title) in [
        ("/top", "yearly", "Year"),
        ("/top?period=weekly", "weekly", "Week"),
        ("/top?period=all", "all", "All time"),
        ("/c/general/4/l/top?period=monthly", "monthly", "Month"),
        ("/c/general/4/none/l/top?period=daily", "daily", "Today"),
        (
            "/tag/howto/1/l/top?period=quarterly",
            "quarterly",
            "Quarter",
        ),
    ] {
        let (status, _, html) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(
            html.contains(&format!(
                r#"<div class="top-lists"><details class="select-kit single-select dropdown-select-box period-chooser has-selection"><summary aria-label="Filter by period" name="Filter by: {period}" data-name="{period}" data-value="{period}" tabindex="0" class="select-kit-header single-select-header dropdown-select-box-header period-chooser-header btn-flat btn btn-icon-text"><h2 class="selected-name"><span class="date-section">{title}</span>"#
            )),
            "{path}"
        );
        assert_eq!(html.matches("period-chooser-row").count(), 6, "{path}");
        assert!(
            html.contains(&format!(
                r#"data-name="{period}" data-value="{period}" tabindex="0" class="select-kit-row dropdown-select-box-row period-chooser-row is-highlighted is-selected">"#
            )),
            "{path}"
        );
        assert!(html.contains("/top-period.js"), "{path}");
    }
    for path in ["/latest", "/hot", "/c/general/4"] {
        let (_, _, html) = get(&db.pool, path).await;
        assert!(!html.contains("top-lists"), "{path}");
        assert!(!html.contains("top-period.js"), "{path}");
    }
}

/// A list with no topics draws no topic list (DiscoveryTopics#hasTopics):
/// a top period nothing scored in is the chooser alone.
#[tokio::test]
async fn an_empty_list_draws_no_topic_list() {
    let db = TestDb::new().await;
    let (_, _, html) = get(&db.pool, "/top?period=weekly").await;
    assert!(html.contains(r#"<table class="topic-list">"#));
    sqlx::query("DELETE FROM top_topics")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, _, html) = get(&db.pool, "/top?period=weekly").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains(r#"<div class="top-lists">"#));
    assert!(!html.contains(r#"<table class="topic-list">"#));
}

/// The navigation pills are top_menu's alone: DNavigation hands buildList
/// `filterType`, not the `filterMode` it reads to add the current list, so
/// a list the menu lacks gets no pill (and the mobile menu's trigger the
/// first pill's label). A list the menu has is active as before.
#[tokio::test]
async fn a_list_missing_from_the_top_menu_gets_no_pill() {
    let db = TestDb::new().await;
    common::set_setting(&db.pool, "top_menu", 8, "latest|new|hot|categories").await;
    let (_, _, html) = get(&db.pool, "/top").await;
    assert!(!html.contains("nav-item_top"));
    assert_eq!(active_nav(&html), None);
    let (_, _, html) = get(&db.pool, "/c/general/4/l/top").await;
    assert!(!html.contains("nav-item_top"));
    assert_eq!(active_nav(&html), None);
    common::set_setting(&db.pool, "top_menu", 8, "latest|top|categories").await;
    let (_, _, html) = get(&db.pool, "/top?period=weekly").await;
    assert_eq!(active_nav(&html), Some("top"));
    let (_, _, html) = get(&db.pool, "/c/general/4/l/top").await;
    assert_eq!(active_nav(&html), Some("top"));
}

/// /new's `subset` with unified new (TopicQuery#list_new): topics is
/// new_results, replies unread_results, anything else both; the next page
/// keeps it.
#[tokio::test]
async fn the_new_list_subset_picks_topics_or_replies() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Member::logged_in(st, "user1").await;
    let ids = |v: &serde_json::Value| -> Vec<i64> {
        v["topic_list"]["topics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_i64().unwrap())
            .collect()
    };
    let all = ids(&user1.json("/new.json").await);
    let topics = ids(&user1.json("/new.json?subset=topics").await);
    let replies = ids(&user1.json("/new.json?subset=replies").await);
    assert_eq!(topics, [34]);
    assert_eq!(replies, [35]);
    assert_eq!(all, [35, 34]);
    assert_eq!(ids(&user1.json("/new.json?subset=other").await), all);
    let page = user1.json("/new.json?subset=topics&per_page=1").await;
    assert_eq!(
        page["topic_list"]["more_topics_url"],
        "/new?page=1&per_page=1&subset=topics"
    );
}

/// The docked header's template (header_topic_view): what
/// static/js/header-topic-info.js swaps into the header once the topic's
/// title scrolls out of view.
fn header_topic_template(html: &str) -> &str {
    let start = html
        .find("<template id=\"header-topic-info\">")
        .expect("the docked header's template");
    let rest = &html[start..];
    &rest[..rest.find("</template>").unwrap()]
}

#[tokio::test]
async fn topic_page_carries_the_docked_header_topic_info() {
    let db = TestDb::new().await;
    let (_, _, html) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35").await;
    let template = header_topic_template(&html);
    // HomeLogo minimized: the small logo, 36 wide.
    assert!(
        template.contains(
            "<div class=\"title--minimized title\"><a href=\"/\"><img alt=\"Discourse\" class=\"logo-small\" id=\"site-logo\" src=\""
        ),
        "{template}"
    );
    assert!(
        template.contains("/images/discourse-logo-sketch-small.png\" width=\"36\"></a></div>"),
        "{template}"
    );
    // Header::Topic::Info: two rows (topic_featured_link_enabled), the
    // title linked to the topic, General's badge and the tag.
    assert!(
        template.contains(
            "<div class=\"two-rows extra-info-wrapper\"><div class=\"two-rows extra-info\"><div class=\"title-wrapper\"><h1 class=\"header-title\"><span class=\"topic-statuses\"></span><a class=\"topic-link\" data-topic-id=\"35\" href=\"/t/parity-fixture-replies-and-posters/35\"><span>Parity fixture: replies and posters</span></a><span class=\"header-topic-title-suffix\"></span></h1><div class=\"categories-wrapper\"><a class=\"badge-category__wrapper \""
        ),
        "{template}"
    );
    assert!(
        template.contains("<span class=\"badge-category__name\">General</span></span></a></div><div class=\"topic-header-extra\"><ul class='discourse-tags' aria-label=Tags><li><a href='/tag/guide/2'  data-tag-name=guide class='discourse-tag simple'>guide</a></li></ul></div></div></div></div>"),
        "{template}"
    );
    assert!(!template.contains("private-message-glyph"));
    assert!(html.contains(&format!(
        r#"<script src="{}"></script>"#,
        discourse_rs::assets::url("", "header-topic-info.js")
    )));
    // What the script finds: the header's logo it swaps, the title it
    // watches and the first post it scrolls back to.
    assert!(html.contains("<div class=\"home-logo-wrapper-outlet\">"));
    assert!(html.contains("<div class=\"container\" id=\"topic-title\"><div class=\"title-wrapper\"><h1 data-topic-id=\"35\">"));
    assert!(html.contains("id=\"post_1\""));
    // Only a topic docks it.
    let (_, _, latest) = get(&db.pool, "/latest").await;
    assert!(!latest.contains("header-topic-info"));
}

#[tokio::test]
async fn the_mobile_docked_header_leaves_out_the_grandparent_badge_only() {
    let db = TestDb::new().await;
    let app = discourse_rs::app(state(db.pool.clone(), config(RailsEnv::Test, &[])).await);
    let response = app
        .oneshot(
            Request::get("/t/parity-fixture-tagged-in-a-subcategory/38")
                .header(header::HOST, "test.localhost")
                .header(header::COOKIE, "mobile_view=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&bytes).into_owned();
    assert!(html.contains(" mobile-view "), "the mobile layout");
    let template = header_topic_template(&html);
    // A subcategory: the parent's badge, then its own, neither with its
    // parent's class.
    let general = template.find("data-category-id=\"4\"").expect("General");
    let sub = template
        .find("data-category-id=\"34\"")
        .expect("Sub General");
    assert!(general < sub, "{template}");
    assert!(!template.contains("--has-parent"), "{template}");
    assert!(template.contains(">howto</a>"), "{template}");
}
