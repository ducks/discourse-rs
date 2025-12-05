use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use diesel::prelude::*;
use diesel::r2d2::{self, ConnectionManager};
use std::env;

mod auth;
mod config;
mod middleware;
mod models;
mod pagination;
mod route_helpers;
mod routes;
mod schema;

pub type DbPool = r2d2::Pool<ConnectionManager<PgConnection>>;

#[get("/")]
async fn index() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "message": "Welcome to Discourse-rs!",
        "version": "0.1.0"
    }))
}

#[get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "ok"
    }))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenvy::dotenv().ok();
    env_logger::init();

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let manager = ConnectionManager::<PgConnection>::new(database_url);
    let pool = r2d2::Pool::builder()
        .build(manager)
        .expect("Failed to create pool");

    log::info!("Starting server at http://127.0.0.1:8080");

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .service(index)
            .service(health)
            .service(web::scope("/api").configure(routes::config))
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}
