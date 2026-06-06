//! Verifies route registration works with multiple modules mounted at the
//! same prefix. The empty-`scope("")` pattern in the prior code prevented
//! multiple writable scopes from coexisting; this regression test guards
//! against that returning.

mod common;

use actix_web::{test, App, web};

#[actix_web::test]
async fn protected_endpoints_return_401_without_auth() {
    let pool = common::setup().pool();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool))
            .configure(discourse_rs::routes::config),
    ).await;

    // Hit several protected endpoints across different modules. They
    // should all return 401 (extractor refused), not 404 (route not
    // registered because a sibling scope ate the request).
    let endpoints = [
        ("POST", "/users"),
        ("POST", "/topics"),
        ("POST", "/posts"),
        ("PUT", "/settings/foo"),
    ];

    for (method, path) in endpoints {
        let req = match method {
            "POST" => test::TestRequest::post().uri(path),
            "PUT" => test::TestRequest::put().uri(path).set_json(serde_json::json!({"value": "x"})),
            _ => unreachable!(),
        }.to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status().as_u16(),
            401,
            "expected 401 for {method} {path}, got {}",
            resp.status()
        );
    }
}

#[actix_web::test]
async fn public_get_endpoints_return_200_without_auth() {
    let pool = common::setup().pool();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool))
            .configure(discourse_rs::routes::config),
    ).await;

    let endpoints = ["/users", "/topics", "/posts", "/settings"];

    for path in endpoints {
        let req = test::TestRequest::get().uri(path).to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status().as_u16(),
            200,
            "expected 200 for GET {path}, got {}",
            resp.status()
        );
    }
}
