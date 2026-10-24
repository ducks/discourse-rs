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

/// DLoadMore's sentinel carries more_topics_url for
/// static/js/discovery-topic-list.js, which loads that page as the reader
/// nears it; no "Next page" link. The page footer waits in its templates
/// while there is more to load.
#[tokio::test]
async fn latest_pages_carry_the_next_page_on_the_sentinel() {
    let db = TestDb::new().await;
    let (_, _, html) = get(&db.pool, "/latest?per_page=2").await;
    assert!(
        html.contains(r#"<div aria-hidden="true" class="load-more-sentinel" data-more-url="/latest?page=1&#38;per_page=2"></div>"#),
        "{html}"
    );
    assert!(!html.contains("no_definitions"));
    assert!(!html.contains(r#"rel="next""#));
    assert!(
        html.contains(r#"<template class="deferred-footer"><div class="custom-footer-content">"#)
    );
    assert!(html.contains("/assets/discovery-topic-list.js?v="));
    let (_, _, last) = get(&db.pool, "/latest?page=2&per_page=2").await;
    assert!(!last.contains("data-more-url"));
    assert!(!last.contains("deferred-footer"));
    assert!(last.contains(r#"<div class="custom-footer-content">"#));
    // DiscoveryTopics' footer after the list, its spinner's container
    // empty either way; no education for a visitor.
    for page in [&html, &last] {
        assert!(page.contains(
            r#"></div>
   </div>
   <footer class="topic-list-bottom"><div class="loading-container"></div></footer>
  </div>"#
        ));
    }
}

#[tokio::test]
async fn latest_pages_apply_the_list_filters() {
    let db = TestDb::new().await;
    let (status, _, html) = get(&db.pool, "/latest?status=closed").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Parity fixture: pinned and closed"));
    assert!(!html.contains("Parity fixture: replies and posters"));
    let (_, _, html) = get(&db.pool, "/latest?tags=howto&per_page=1").await;
    assert!(
        html.contains("Parity fixture: liked and archived"),
        "{html}"
    );
    assert!(
        html.contains(r#"data-more-url="/latest?page=1&#38;per_page=1&#38;tags=howto""#),
        "{html}"
    );
    let (status, _, body) = get(&db.pool, "/latest?before=soon").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, "before");
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

/// A topic whose first post's image process_post took (image_upload_id
/// and the share-size thumbnail): the page draws, its og:image the
/// thumbnail with the original's size and type (crawlable_meta_data).
#[tokio::test]
async fn topic_page_with_a_topic_image() {
    let db = TestDb::new().await;
    let sha = "0000000000000000000000000000000000009741";
    for sql in [
        format!(
            "INSERT INTO uploads (id, user_id, original_filename, filesize, width, height, url, sha1, \
             extension, created_at, updated_at) VALUES (9741, 3, 'pond.png', 120000, 2048, 1536, \
             '/uploads/default/original/1X/{sha}.png', '{sha}', 'png', now(), now())"
        ),
        format!(
            "INSERT INTO optimized_images (id, sha1, extension, width, height, upload_id, url, \
             filesize, version, created_at, updated_at) VALUES (9741, '{sha}', '.png', 1024, 768, \
             9741, '/uploads/default/optimized/1X/{sha}_2_1024x768.png', 4000, 2, now(), now())"
        ),
        "INSERT INTO topic_thumbnails (upload_id, optimized_image_id, max_width, max_height) \
         VALUES (9741, 9741, 1024, 1024)"
            .to_string(),
        "UPDATE topics SET image_upload_id = 9741 WHERE id = 35".to_string(),
        "UPDATE posts SET image_upload_id = 9741 WHERE id = 35".to_string(),
    ] {
        sqlx::query(&sql).execute(&db.pool).await.unwrap();
    }
    let (status, _, html) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    let image = format!("http://test.localhost/uploads/default/optimized/1X/{sha}_2_1024x768.png");
    for meta in [
        format!(r#"<meta name="twitter:image" content="{image}">"#),
        format!(r#"<meta property="og:image" content="{image}">"#),
        r#"<meta property="og:image:width" content="2048">"#.to_string(),
        r#"<meta property="og:image:height" content="1536">"#.to_string(),
        r#"<meta property="og:image:type" content="image/png">"#.to_string(),
    ] {
        assert!(html.contains(&meta), "{meta}");
    }
    // Another post's page has that post's image, none here: the site's.
    let (status, _, html) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35/3").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(!html.contains("og:image:width"));
    assert!(!html.contains(sha));
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
    // The Ace editors' wrapper (components/ace-editor.scss) and the cooked
    // rules' editor preview selectors, which the admin's email editors use.
    assert!(css.contains(".ace-wrapper {\n  border: 1px solid var(--primary-400);"));
    assert!(css.contains(".cooked>*:first-child,\n.d-editor-preview>*:first-child {"));
}

/// The stylesheets' @font-face URLs are absolute (Discourse.base_url, or
/// the CDN), so a page on another origin than the base URL's (a CDN,
/// 127.0.0.1 against localhost) loads the fonts only with CORS, as
/// nginx.sample.conf's font location allows. Without it the page falls
/// back to Arial, whose shorter line boxes pull the list up.
#[tokio::test]
async fn fonts_allow_any_origin() {
    let db = TestDb::new().await;
    let app = discourse_rs::app(state(db.pool.clone(), config(RailsEnv::Test, &[])).await);
    let response = app
        .oneshot(
            Request::get("/fonts/InterVariable.woff2?v=0.0.19")
                .header(header::HOST, "test.localhost")
                .header(header::ORIGIN, "http://cdn.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "font/woff2");
    assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
}

/// The gem's fonts the port doesn't embed (the admin's font previews name
/// them all) come from DISCOURSE_SRC's discourse-fonts gem; only the gem's
/// own font files, and nothing without a checkout.
#[tokio::test]
async fn the_gems_other_fonts_come_from_the_checkout() {
    let db = TestDb::new().await;
    let src = std::env::temp_dir().join(format!("discourse-rs-fonts-{}", std::process::id()));
    let fonts = src.join(".bundle/ruby/3.4.0/gems/discourse-fonts-0.0.19/vendor/assets/fonts");
    std::fs::create_dir_all(&fonts).unwrap();
    std::fs::write(fonts.join("Lato-Bold.woff2"), b"lato").unwrap();
    std::fs::write(fonts.join("notes.txt"), b"not a font").unwrap();
    let fetch = |discourse_src: Option<std::path::PathBuf>, path: &'static str| {
        let pool = db.pool.clone();
        async move {
            let mut config = config(RailsEnv::Test, &[]);
            config.discourse_src = discourse_src;
            let response = discourse_rs::app(state(pool, config).await)
                .oneshot(
                    Request::get(path)
                        .header(header::HOST, "test.localhost")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            (status, body)
        }
    };
    let (status, body) = fetch(Some(src.clone()), "/fonts/Lato-Bold.woff2?v=0.0.19").await;
    assert_eq!((status, &body[..]), (StatusCode::OK, &b"lato"[..]));
    let (status, _) = fetch(Some(src.clone()), "/fonts/notes.txt").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = fetch(Some(src.clone()), "/fonts/Lora-Bold.woff2").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = fetch(None, "/fonts/Lato-Bold.woff2").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    std::fs::remove_dir_all(&src).unwrap();
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

/// Entering a topic at a post (/t/:slug/:id/:post_number, where
/// notifications, search results and replies link): the page is served
/// at that address, whose post number static/js/topic.js reads
/// (TopicFromParams' nearPost) and later replaces as the reader scrolls,
/// and each post is drawn as Ember's stream draws it: the
/// `[data-post-number]` wrapper the closest post is picked from, the
/// `article#post_N` jumpToPost locks on, and the `.topic-body`
/// highlightPost focuses.
#[tokio::test]
async fn a_topic_entered_at_a_post_draws_what_the_jump_needs() {
    let db = TestDb::new().await;
    let path = "/t/parity-fixture-replies-and-posters/35/3";
    let (status, _, html) = get(&db.pool, path).await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains(r#"<section class="topic-area" data-topic-id="35" id="topic">"#));
    for n in 1..=5 {
        let wrapper = html
            .find(&format!(r#" data-post-number="{n}"><h2 "#))
            .unwrap_or_else(|| panic!("post {n}'s wrapper"));
        let post = &html[wrapper..];
        let article = post
            .find(&format!(r#" id="post_{n}">"#))
            .unwrap_or_else(|| panic!("post {n}'s article"));
        let body = post
            .find(r#"class="post__body topic-body clearfix""#)
            .unwrap_or_else(|| panic!("post {n}'s body"));
        assert!(article < body, "post {n}");
        if n < 5 {
            let next = post
                .find(&format!(r#" data-post-number="{}"><h2 "#, n + 1))
                .unwrap();
            assert!(body < next, "post {n}'s body is its own");
        }
    }
    // A post past the last is entered at the nearest (the last).
    let (status, _, html) = get(&db.pool, "/t/parity-fixture-replies-and-posters/35/99").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains(r#" id="post_5">"#));
}

/// PostStream#loadedAllPosts: a topic entered at a post near its end is
/// drawn to the end, so it has the footer and the suggested topics, and
/// new posts land on it (the live stream's tail), where the composer's
/// save waits for its reply; entered nearer the start, neither.
#[tokio::test]
async fn a_topic_entered_near_its_end_is_loaded_to_the_end() {
    let db = TestDb::new().await;
    let state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Member::logged_in(state, "admin").await;
    let xhr = |r: axum::http::request::Builder| r.header("x-requested-with", "XMLHttpRequest");
    let csrf = admin
        .send(xhr(Request::get("/session/csrf.json")), "")
        .await;
    let csrf: serde_json::Value = serde_json::from_str(&csrf).unwrap();
    for n in 0..20 {
        let reply = Request::post("/posts")
            .header("x-csrf-token", csrf["csrf"].as_str().unwrap())
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        let body = admin
            .send(
                xhr(reply),
                &format!("topic_id=35&raw=Reply+{n}+making+the+topic+longer+than+a+page"),
            )
            .await;
        assert!(body.contains("\"post_number\""), "{body}");
    }
    let end = admin
        .page("/t/parity-fixture-replies-and-posters/35/24")
        .await;
    assert!(end.contains(r#" id="post_25">"#));
    assert!(!end.contains(r#" id="post_1">"#));
    assert!(end.contains(r#" id="topic-footer-buttons""#), "{end}");
    assert!(end.contains("&amp;tail=1"));
    let start = admin
        .page("/t/parity-fixture-replies-and-posters/35/3")
        .await;
    assert!(start.contains(r#" id="post_3">"#));
    assert!(!start.contains(r#" id="post_25">"#));
    // The footer waits for static/js/post-stream.js to load the rest.
    let deferred = start
        .find(r#"<template class="loaded-all-posts">"#)
        .unwrap();
    let footer = start.find(r#"id="topic-footer-buttons""#).unwrap();
    assert!(deferred < footer, "{start}");
    assert!(!start.contains("&amp;tail=1"));
    assert!(
        start.contains(r#"<template class="deferred-footer"><div class="custom-footer-content">"#)
    );
    assert!(!end.contains("loaded-all-posts"));
    assert!(!end.contains("deferred-footer"));
}

/// What static/js/post-stream.js loads more from: the stream's ids and
/// chunk size on #posts, DLoadMore's sentinels with their headings where
/// there is more above or below (the bottom boundary once the last post
/// is drawn), whether the view had its suggested topics, and the chunks
/// /live/posts draws with the params GET /t/:id/posts.json takes.
#[tokio::test]
async fn a_long_topic_draws_what_the_post_stream_loads_more_from() {
    let db = TestDb::new().await;
    let state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Member::logged_in(state, "admin").await;
    let xhr = |r: axum::http::request::Builder| r.header("x-requested-with", "XMLHttpRequest");
    let csrf = admin
        .send(xhr(Request::get("/session/csrf.json")), "")
        .await;
    let csrf: serde_json::Value = serde_json::from_str(&csrf).unwrap();
    for n in 0..40 {
        let reply = Request::post("/posts")
            .header("x-csrf-token", csrf["csrf"].as_str().unwrap())
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        let body = admin
            .send(
                xhr(reply),
                &format!("topic_id=35&raw=Reply+{n}+making+the+topic+several+chunks+long"),
            )
            .await;
        assert!(body.contains("\"post_number\""), "{body}");
    }
    let view = admin.json("/t/35.json").await;
    let stream: Vec<i64> = view["post_stream"]["stream"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_i64().unwrap())
        .collect();
    assert_eq!(stream.len(), 45);
    let ids = format!(
        "[{}]",
        stream
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    let sentinel = |direction: &str| {
        let label = if direction == "above" {
            "above"
        } else {
            "below"
        };
        format!(
            r#"<div aria-hidden="true" class="load-more-sentinel"></div><div class="post-stream-load-more-accessible sr-only"><h2 class="post-stream-load-more-accessible__heading" id="post-stream-load-more-heading__{direction}">Load more posts {label}</h2></div>"#
        )
    };

    // The first page: more below only, no suggested topics yet.
    let first = admin.page("/t/parity-fixture-replies-and-posters/35").await;
    assert!(
        first.contains(&format!(
            r#"<div class="posts-wrapper"><span></span><div class="loading-container"></div><div class="post-stream" id="posts" data-stream="{ids}" data-chunk-size="20">"#
        )),
        "{first}"
    );
    assert!(first.contains(&format!(
        r#"{}</div><div class="loading-container"></div>"#,
        sentinel("below")
    )));
    assert!(!first.contains("post-stream-load-more-heading__above"));
    assert!(!first.contains("post-stream__bottom-boundary"));
    assert!(first.contains("/assets/post-stream.js?v="));
    assert!(!first.contains(r#"rel="next""#));

    // Entered mid-topic: more both ways.
    let middle = admin
        .page("/t/parity-fixture-replies-and-posters/35/25")
        .await;
    assert!(
        middle.contains(&format!(
            r#"data-chunk-size="20">{}<div"#,
            sentinel("above")
        )),
        "{middle}"
    );
    assert!(middle.contains(&sentinel("below")));
    assert!(!middle.contains(r#" id="post_1">"#));

    // Entered at the end: more above, the bottom boundary, the suggested
    // topics with the view.
    let end = admin
        .page("/t/parity-fixture-replies-and-posters/35/45")
        .await;
    assert!(
        end.contains(r#"data-chunk-size="20" data-has-suggested>"#),
        "{end}"
    );
    assert!(end.contains(&sentinel("above")));
    assert!(end.contains(r#"<div class="post-stream__bottom-boundary"></div></div><div class="loading-container"></div>"#));
    assert!(!end.contains("post-stream-load-more-heading__below"));

    // The next chunk below the first page, as appendMore asks for it.
    let query: String = stream[20..40]
        .iter()
        .map(|id| format!("post_ids%5B%5D={id}&"))
        .collect();
    let chunk = admin
        .page(&format!("/live/posts/35?{query}include_suggested=true"))
        .await;
    assert!(chunk.starts_with(r#"<div class="topic-post"#), "{chunk}");
    assert!(chunk.contains(r#" id="post_21">"#));
    assert!(chunk.contains(r#" id="post_40">"#));
    assert!(!chunk.contains(r#" id="post_20">"#));
    assert!(!chunk.contains(r#" id="post_41">"#));
    assert!(chunk.contains(
        r#"<template class="post-stream-more-topics"><div class="more-topics__container">"#
    ));
    let later = admin
        .page(&format!(
            "/live/posts/35?post_ids%5B%5D={}&include_suggested=false",
            stream[44]
        ))
        .await;
    assert!(later.contains(r#" id="post_45">"#));
    assert!(!later.contains("post-stream-more-topics"));
}

/// What composer.js's save draws from a member's topic page: the topic
/// as the reply-where dialog shows it (TopicReplyChoiceDialog's button:
/// its statuses, title, unlinked category badge and tags) and whether the
/// dialog is offered there (an open topic, or a closed one to staff),
/// with the composer's labels for it and for PostEnqueued's modal.
#[tokio::test]
async fn a_members_topic_page_draws_what_the_composers_save_shows() {
    let db = TestDb::new().await;
    let state = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Member::logged_in(state.clone(), "admin").await;
    let html = admin
        .page("/t/parity-fixture-tagged-in-a-subcategory/38")
        .await;
    assert!(html.contains(TAGGED_REPLY_WHERE), "{html}");
    assert!(html.contains(r#" data-label-posting-not-on-topic="Which topic do you want to reply to?" data-label-cancel="Cancel" data-approval="{&quot;title&quot;:&quot;Post Needs Approval&quot;,&quot;description&quot;:&quot;We&#x27;ve received your new post but it needs to be approved by a moderator before it will appear. Please be patient.&quot;,&quot;pending_one&quot;:&quot;You have &lt;strong&gt;%{count}&lt;/strong&gt; post pending.&quot;,&quot;pending_other&quot;:&quot;You have &lt;strong&gt;%{count}&lt;/strong&gt; posts pending.&quot;,&quot;ok&quot;:&quot;OK&quot;}""#), "{html}");
    // A closed topic: offered to staff, with its statuses.
    let closed = admin.page("/t/parity-fixture-pinned-and-closed/37").await;
    assert!(closed.contains(CLOSED_REPLY_WHERE), "{closed}");
    let mut member = Member::logged_in(state, "user1").await;
    let closed = member.page("/t/parity-fixture-pinned-and-closed/37").await;
    assert!(
        closed.contains(
            r#"<template class="reply-where-topic" data-topic-id="37" data-offer="false">"#
        )
    );
    let (_, _, visitor) = get(&db.pool, "/t/parity-fixture-tagged-in-a-subcategory/38").await;
    assert!(!visitor.contains("reply-where-topic"));
}

const TAGGED_REPLY_WHERE: &str = r#"<template class="reply-where-topic" data-topic-id="38" data-offer="true"><div class="topic-title"><div class="topic-title__top-line"><span class="topic-statuses"></span><span class="fancy-title">Parity fixture: tagged in a subcategory</span></div><div class="topic-title__bottom-line"><span class="badge-category__wrapper" style="--category-badge-color: #AB9364;--category-badge-text-color: #FFFFFF;--parent-category-badge-color: #25AAE2;--parent-category-badge-text-color: #FFFFFF;"><span data-category-id="34" data-parent-category-id="4" data-drop-close="true" class="badge-category --has-parent --style-square"><span class="badge-category__name">Sub General</span></span></span><ul class='discourse-tags' aria-label=Tags><li><a href='/tag/howto/1'  data-tag-name=howto class='discourse-tag simple'>howto</a></li></ul></div></div></template>"#;
const CLOSED_REPLY_WHERE: &str = r##"<template class="reply-where-topic" data-topic-id="37" data-offer="true"><div class="topic-title"><div class="topic-title__top-line"><span class="topic-statuses"><span class="topic-status --closed"><svg class="fa d-icon d-icon-lock svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#lock"></use></svg></span><span class="topic-status --pinned"><svg class="fa d-icon d-icon-thumbtack svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#thumbtack"></use></svg></span></span><span class="fancy-title">Parity fixture: pinned and closed</span></div><div class="topic-title__bottom-line"><span class="badge-category__wrapper" style="--category-badge-color: #808281;--category-badge-text-color: #FFFFFF;"><span data-category-id="2" data-drop-close="true" class="badge-category --style-emoji"><img width="20" height="20" src='/images/emoji/twitter/thought_balloon.png?v=15' title='thought_balloon' alt='thought_balloon' class='emoji'><span class="badge-category__name">Site Feedback</span></span></span></div></div></template>"##;

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

/// Avatars are drawn for a device pixel ratio of 1 and redrawn at the
/// screen's by static/js/avatars.js, as Ember's avatarImg draws them
/// (getRawAvatarSize): every page's head loads it with avatar_sizes, and
/// every avatar it should redraw is one it recognizes, an img with a
/// width whose avatar URL is at that width or that width's size at a
/// ratio of 1. The redraw itself was measured with playwright against
/// Rails (ROADMAP.md item 272).
#[tokio::test]
async fn avatars_are_drawn_for_avatars_js_to_scale() {
    let db = TestDb::new().await;
    let avatar = regex::Regex::new(
        r#"<img [^>]*\bsrc=['"][^'"]*/(?:user_avatar/[^/]+/[^/]+|letter_avatar/[^/]+|letter_avatar_proxy/v\d+/letter/[^/]+/[^/]+)/(\d+)(?:/|\.png)[^>]*>"#,
    )
    .unwrap();
    let width = regex::Regex::new(r#"\bwidth=['"](\d+)['"]"#).unwrap();
    // getRawAvatarSize at a ratio of 1 with the default avatar_sizes.
    let raw = |size: u32| {
        [24, 48, 72, 96, 144, 288]
            .into_iter()
            .find(|s| *s >= size)
            .unwrap_or(288)
    };
    for mobile in [false, true] {
        for path in [
            "/latest",
            "/t/parity-fixture-replies-and-posters/35",
            "/u/user1/summary",
            "/u",
            "/about",
        ] {
            let app = discourse_rs::app(state(db.pool.clone(), config(RailsEnv::Test, &[])).await);
            let mut request = Request::get(path).header(header::HOST, "test.localhost");
            if mobile {
                request = request.header(header::COOKIE, "mobile_view=1");
            }
            let response = app
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let html = String::from_utf8_lossy(&bytes).into_owned();
            let head = &html[..html.find("</head>").unwrap()];
            assert!(
                head.contains(r#"<script src="/assets/avatars.js?v="#)
                    && head.contains(r#" data-sizes="24|48|72|96|144|288"></script>"#),
                "{path}"
            );
            let mut seen = 0;
            for m in avatar.captures_iter(&html) {
                let img = &m[0];
                let drawn: u32 = m[1].parse().unwrap();
                let w: u32 = width
                    .captures(img)
                    .unwrap_or_else(|| panic!("{path}: no width: {img}"))[1]
                    .parse()
                    .unwrap();
                assert!(
                    drawn == w || drawn == raw(w),
                    "{path} (mobile {mobile}): {img}"
                );
                seen += 1;
            }
            assert!(seen > 0, "{path} (mobile {mobile}): no avatars");
        }
    }
}

/// avatars.js picks among the site's avatar_sizes, as getRawAvatarSize
/// reads the setting.
#[tokio::test]
async fn avatars_js_takes_the_sites_avatar_sizes() {
    let db = TestDb::new().await;
    // 8: SiteSettings::TypeSupervisor's list.
    common::set_setting(&db.pool, "avatar_sizes", 8, "24|48|120|240").await;
    let (_, _, html) = get(&db.pool, "/latest").await;
    assert!(
        html.contains(r#" data-sizes="24|48|120|240"></script>"#),
        "{}",
        &html[..html.find("</head>").unwrap()]
    );
}

/// The body's classes, as the page draws them.
fn body_classes(html: &str) -> &str {
    let open = "<body hx-boost=\"true\" class=\"";
    let start = html.find(open).expect("body") + open.len();
    &html[start..start + html[start..].find('"').unwrap()]
}

async fn mobile_page(pool: &PgPool, path: &str) -> String {
    let app = discourse_rs::app(state(pool.clone(), config(RailsEnv::Test, &[])).await);
    let response = app
        .oneshot(
            Request::get(path)
                .header(header::HOST, "test.localhost")
                .header(header::COOKIE, "mobile_view=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The topic list's rows (its body), for what they draw.
fn list_rows<'a>(html: &'a str, path: &str) -> &'a str {
    let start = html
        .find("<tbody class=\"topic-list-body\">")
        .unwrap_or_else(|| panic!("{path}: no rows"));
    &html[start..start + html[start..].find("</tbody>").unwrap()]
}

// Each discovery list's body classes, as rs-ref-b ends up with them:
// Rails' layout's `category-<slug_path>` first on a category's list, then
// the chrome's, WelcomeBanner's around Navigation's and
// AddCategoryTagClasses'.
#[tokio::test]
async fn discovery_lists_carry_embers_body_classes() {
    let db = TestDb::new().await;
    let uc = "uc-enable-new-checkbox-style uc-floating-dismiss-topics-on-mobile uc-modernize-foundation-theme";
    for (path, classes) in [
        (
            "/latest",
            "has-sidebar-page welcome-banner--enabled navigation-topics welcome-banner--visible",
        ),
        ("/top", "has-sidebar-page navigation-topics"),
        (
            "/categories",
            "has-sidebar-page welcome-banner--enabled navigation-categories categories-list welcome-banner--visible",
        ),
        ("/tag/guide/2", "has-sidebar-page tag-guide tags-page"),
        (
            "/tags/c/general/4/guide/2",
            "has-sidebar-page tags-page category category-general tag-guide",
        ),
    ] {
        let (status, _, html) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(body_classes(&html), format!("{uc} {classes}"), "{path}");
    }
    for (path, slug) in [
        ("/c/general/4", "general"),
        ("/c/general/4/none", "general"),
        ("/c/general/4/l/top", "general"),
        ("/c/general/sub-general/34", "general-sub-general"),
        ("/c/site-feedback/2", "site-feedback"),
    ] {
        let (status, _, html) = get(&db.pool, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(
            body_classes(&html),
            format!("category-{slug} {uc} has-sidebar-page category navigation-category"),
            "{path}"
        );
    }
    // The mobile view: no sidebar, and the banner's search is hidden, so
    // it isn't visible.
    let html = mobile_page(&db.pool, "/latest").await;
    assert_eq!(
        body_classes(&html),
        format!("{uc} welcome-banner--enabled navigation-topics")
    );
    assert!(html.contains("/assets/welcome-banner.js"));
    let html = mobile_page(&db.pool, "/c/general/sub-general/34").await;
    assert_eq!(
        body_classes(&html),
        format!("category-general-sub-general {uc} category navigation-category")
    );
    // A staff member's: GlimmerSiteHeader's `staff`.
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Member::logged_in(st, "admin").await;
    let html = admin.page("/c/general/4").await;
    assert_eq!(
        body_classes(&html),
        format!(
            "category-general chat-enabled {uc} staff has-sidebar-page category navigation-category"
        )
    );
    // No banner, no banner classes or script.
    common::set_setting(&db.pool, "enable_welcome_banner", 5, "f").await;
    let (_, _, html) = get(&db.pool, "/latest").await;
    assert_eq!(
        body_classes(&html),
        format!("{uc} has-sidebar-page navigation-topics")
    );
    assert!(!html.contains("welcome-banner.js"));
}

// TopicList.hideUniformCategory: the list of a category without
// subcategories goes without the rows' category badges, desktop and
// mobile; a parent's, its /none list and a tag's keep them.
#[tokio::test]
async fn a_leaf_categorys_rows_hide_the_category_badge() {
    let db = TestDb::new().await;
    let badge = "badge-category__wrapper";
    for path in [
        "/c/general/4",
        "/c/general/4/none",
        "/tag/guide/2",
        "/latest",
    ] {
        let (_, _, html) = get(&db.pool, path).await;
        assert!(list_rows(&html, path).contains(badge), "{path}");
        assert!(
            list_rows(&mobile_page(&db.pool, path).await, path).contains(badge),
            "{path} mobile"
        );
    }
    for path in ["/c/general/sub-general/34", "/c/site-feedback/2"] {
        let (_, _, html) = get(&db.pool, path).await;
        assert!(!list_rows(&html, path).contains(badge), "{path}");
        let html = mobile_page(&db.pool, path).await;
        assert!(!list_rows(&html, path).contains(badge), "{path} mobile");
        assert!(
            list_rows(&html, path).contains("topic-item-stats__category-tags"),
            "{path}"
        );
    }
    // A secret subcategory leaves its parent a leaf for a reader who can't
    // see it, not for an admin.
    sqlx::query("UPDATE categories SET read_restricted = TRUE WHERE id = 34")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, _, html) = get(&db.pool, "/c/general/4").await;
    assert!(!list_rows(&html, "/c/general/4 secret sub").contains(badge));
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Member::logged_in(st, "admin").await;
    let html = admin.page("/c/general/4").await;
    assert!(list_rows(&html, "/c/general/4 admin").contains(badge));
}

// The breadcrumbs' selected drops take category-drop.scss's and
// tag-drop.scss's border (--d-nav-border-color--active) and the badge's
// normal weight at --font-0.
#[tokio::test]
async fn the_breadcrumb_drops_are_styled() {
    let db = TestDb::new().await;
    let (_, _, css) = get(&db.pool, "/assets/discourse.css").await;
    for rule in [
        ".select-kit.combo-box.category-drop.has-selection .category-drop-header {\n  border-color: var(--d-nav-border-color--active);",
        ".select-kit.combo-box.category-drop .badge-category__wrapper {\n  font-size: var(--font-0);\n  font-weight: normal;",
        ".select-kit.combo-box.tag-drop.has-selection .tag-drop-header {\n  color: var(--d-nav-border-color--active);",
    ] {
        assert!(css.contains(rule), "{rule}");
    }
}

/// The template's content, `<template class="{class}"...>` to its end.
fn template<'a>(html: &'a str, class: &str) -> &'a str {
    let open = format!("<template class=\"{class}\"");
    let start = html
        .find(&open)
        .unwrap_or_else(|| panic!("no {class} template"));
    let rest = &html[start..];
    let inner = &rest[rest.find('>').unwrap() + 1..];
    &inner[..inner.find("</template>").unwrap()]
}

// The staff menus as Ember draws them for an admin (parity/menus, from
// the reference): TopicAdminMenu's wrench in the footer and the
// timeline's controls with the page's menu, and a post's wrench with its
// AdminPostMenu. A member who manages nothing gets neither.
#[tokio::test]
async fn staff_get_the_topic_and_post_admin_menus() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Member::logged_in(st.clone(), "admin").await;
    let html = admin.page("/t/parity-fixture-replies-and-posters/35").await;
    let wrench = r##"<svg class="fa d-icon d-icon-wrench svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#wrench"></use></svg>"##;
    assert!(html.contains(&format!(
        r#"<div class="topic-footer-main-buttons__actions"><button aria-expanded="false" class="btn no-text fk-d-menu__trigger topic-admin-menu-trigger btn-default btn-icon toggle-admin-menu topic-footer-button" data-identifier="topic-admin-menu" data-trigger="" type="button">{wrench}</button><button class="btn btn-icon-text btn-default topic-footer-button share-and-invite""#
    )));
    // The timeline's: @buttonClasses unset leaves Ember's trailing space.
    assert!(html.contains(&format!(
        r#"<div class="timeline-controls"><button aria-expanded="false" class="btn no-text fk-d-menu__trigger topic-admin-menu-trigger btn-default btn-icon toggle-admin-menu " data-identifier="topic-admin-menu" data-trigger="" type="button">{wrench}</button></div>"#
    )));
    assert_eq!(
        template(&html, "topic-admin-menu-template"),
        include_str!("../parity/menus/topic-admin-menu-t35-admin.html").trim_end()
    );
    assert!(html.contains("/assets/topic-admin-menu.js?v="));
    // Post 2: the wrench among the collapsed buttons, before show more
    // (Ember draws them once it is pressed), its menu after the buttons.
    let post = &html[html.find("id=\"post_2\"").unwrap()..];
    let post = &post[..post.find("</article>").unwrap()];
    assert!(post.contains(&format!(
        r#"<button hidden aria-label="post admin actions" class="btn no-text btn-icon post-action-menu__admin show-post-admin-menu btn-flat" title="post admin actions" type="button">{wrench}<span aria-hidden="true">&#8203;</span></button><button aria-label="show more""#
    )));
    assert_eq!(
        template(post, "admin-post-menu-content"),
        include_str!("../parity/menus/admin-post-menu-post36-admin.html").trim_end()
    );

    // The mobile view: the wrench before the progress bar too.
    admin.cookies.push("mobile_view=1".to_string());
    let html = admin.page("/t/parity-fixture-replies-and-posters/35").await;
    assert!(html.contains(&format!(
        r#"<div id="topic-progress-wrapper"><button aria-expanded="false" class="btn no-text fk-d-menu__trigger topic-admin-menu-trigger btn-default btn-icon toggle-admin-menu " data-identifier="topic-admin-menu" data-trigger="" type="button">{wrench}</button><nav "#
    )));
    assert!(html.contains("toggle-admin-menu topic-footer-button\""));

    let mut user1 = Member::logged_in(st, "user1").await;
    let html = user1.page("/t/parity-fixture-replies-and-posters/35").await;
    assert!(!html.contains("topic-admin-menu-trigger"));
    assert!(!html.contains("topic-admin-menu-template"));
    assert!(!html.contains("show-post-admin-menu"));
    assert!(!html.contains("admin-post-menu-content"));
}
