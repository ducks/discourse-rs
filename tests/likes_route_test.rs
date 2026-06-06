//! Route-level tests for POST/DELETE /api/posts/:id/like. These exercise
//! the HTTP contract: status codes, auth requirement, and JSON shape. The
//! deep business-rule coverage lives in `likes_test.rs`; here we just
//! confirm the service is wired to the route correctly.

mod common;

use actix_web::test;

#[actix_web::test]
async fn like_post_returns_201_with_like_json() {
    let mut ctx = common::setup();

    let author = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let liker = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(author.id));
    let post = common::create_post(
        &mut ctx.conn,
        common::PostOpts::for_topic(topic.id, author.id),
    );

    let app = test::init_service(common::test_app_factory()).await;
    let (hk, hv) = common::auth_header_for(&liker);
    let req = test::TestRequest::post()
        .uri(&format!("/api/posts/{}/like", post.id))
        .insert_header((hk, hv))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 201);

    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["user_id"], liker.id);
    assert_eq!(body["post_id"], post.id);
    // ctx stays alive until here, so its Drop (TRUNCATE) runs after the
    // request completes. We don't pass ctx to the handler — the handler
    // gets its own pool connection.
    drop(ctx);
}

#[actix_web::test]
async fn like_post_without_auth_is_401() {
    let mut ctx = common::setup();
    let author = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(author.id));
    let post = common::create_post(
        &mut ctx.conn,
        common::PostOpts::for_topic(topic.id, author.id),
    );

    let app = test::init_service(common::test_app_factory()).await;
    let req = test::TestRequest::post()
        .uri(&format!("/api/posts/{}/like", post.id))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 401);
    drop(ctx);
}

#[actix_web::test]
async fn like_own_post_is_422() {
    let mut ctx = common::setup();
    let author = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(author.id));
    let post = common::create_post(
        &mut ctx.conn,
        common::PostOpts::for_topic(topic.id, author.id),
    );

    let app = test::init_service(common::test_app_factory()).await;
    let (hk, hv) = common::auth_header_for(&author);
    let req = test::TestRequest::post()
        .uri(&format!("/api/posts/{}/like", post.id))
        .insert_header((hk, hv))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 422);
    drop(ctx);
}

#[actix_web::test]
async fn like_missing_post_is_404() {
    let mut ctx = common::setup();
    let liker = common::create_user(&mut ctx.conn, common::UserOpts::default());

    let app = test::init_service(common::test_app_factory()).await;
    let (hk, hv) = common::auth_header_for(&liker);
    let req = test::TestRequest::post()
        .uri("/api/posts/999999/like")
        .insert_header((hk, hv))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 404);
    drop(ctx);
}

#[actix_web::test]
async fn unlike_post_returns_204() {
    let mut ctx = common::setup();
    let author = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let liker = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(author.id));
    let post = common::create_post(
        &mut ctx.conn,
        common::PostOpts::for_topic(topic.id, author.id),
    );

    let app = test::init_service(common::test_app_factory()).await;
    let (hk, hv) = common::auth_header_for(&liker);

    // Like first
    let req = test::TestRequest::post()
        .uri(&format!("/api/posts/{}/like", post.id))
        .insert_header((hk, hv.clone()))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 201);

    // Then unlike
    let req = test::TestRequest::delete()
        .uri(&format!("/api/posts/{}/like", post.id))
        .insert_header((hk, hv))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 204);
    drop(ctx);
}

#[actix_web::test]
async fn unlike_never_liked_post_is_still_204() {
    let mut ctx = common::setup();
    let author = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let liker = common::create_user(&mut ctx.conn, common::UserOpts::default());
    let topic = common::create_topic(&mut ctx.conn, common::TopicOpts::for_user(author.id));
    let post = common::create_post(
        &mut ctx.conn,
        common::PostOpts::for_topic(topic.id, author.id),
    );

    let app = test::init_service(common::test_app_factory()).await;
    let (hk, hv) = common::auth_header_for(&liker);
    let req = test::TestRequest::delete()
        .uri(&format!("/api/posts/{}/like", post.id))
        .insert_header((hk, hv))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 204);
    drop(ctx);
}
