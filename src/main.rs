use actix_governor::{Governor, GovernorConfigBuilder};
use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use diesel::prelude::*;
use diesel::r2d2::{self, ConnectionManager};
use std::env;

mod auth;
mod config;
mod guardian;
mod jobs;
mod middleware;
mod moderation;
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
    let manager = ConnectionManager::<PgConnection>::new(&database_url);
    let pool = r2d2::Pool::builder()
        .build(manager)
        .expect("Failed to create pool");

    // Set up background job queue and worker pool
    let pool_arc = std::sync::Arc::new(pool.clone());
    let job_queue = jobs::JobQueue::new(pool_arc.clone());
    let worker_pool = jobs::WorkerPool::new(pool_arc, 4);

    // Start worker pool in background
    tokio::spawn(async move {
        worker_pool.run().await;
    });

    log::info!("Starting server at http://127.0.0.1:8080");

    let job_queue_data = web::Data::new(job_queue);

    // Rate limiting: 60 requests per minute per IP
    let governor_conf = GovernorConfigBuilder::default()
        .per_second(1)
        .burst_size(60)
        .finish()
        .unwrap();

    HttpServer::new(move || {
        App::new()
            .wrap(Governor::new(&governor_conf))
            .app_data(web::Data::new(pool.clone()))
            .app_data(job_queue_data.clone())
            .service(index)
            .service(health)
            .service(web::scope("/api").configure(routes::config))
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}
