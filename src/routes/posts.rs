use actix_web::{get, post, web, HttpResponse, Responder};
use diesel::prelude::*;

use crate::models::{NewPost, Post};
use crate::schema::posts;
use crate::DbPool;

#[get("/posts")]
async fn list_posts(pool: web::Data<DbPool>) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to get database connection"
        })),
    };

    let results = web::block(move || {
        posts::table
            .select(Post::as_select())
            .order(posts::created_at.desc())
            .limit(50)
            .load(&mut conn)
    })
    .await;

    match results {
        Ok(Ok(posts)) => HttpResponse::Ok().json(posts),
        Ok(Err(_)) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to load posts"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

#[get("/topics/{topic_id}/posts")]
async fn list_topic_posts(
    pool: web::Data<DbPool>,
    topic_id: web::Path<i32>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to get database connection"
        })),
    };

    let topic_id = topic_id.into_inner();

    let results = web::block(move || {
        posts::table
            .filter(posts::topic_id.eq(topic_id))
            .select(Post::as_select())
            .order(posts::post_number.asc())
            .load(&mut conn)
    })
    .await;

    match results {
        Ok(Ok(posts)) => HttpResponse::Ok().json(posts),
        Ok(Err(_)) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to load posts"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

#[post("/posts")]
async fn create_post(pool: web::Data<DbPool>, new_post: web::Json<NewPost>) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to get database connection"
        })),
    };

    let new_post = new_post.into_inner();

    let result = web::block(move || {
        diesel::insert_into(posts::table)
            .values(&new_post)
            .returning(Post::as_returning())
            .get_result(&mut conn)
    })
    .await;

    match result {
        Ok(Ok(post)) => HttpResponse::Created().json(post),
        Ok(Err(_)) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Failed to create post"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(list_posts)
        .service(list_topic_posts)
        .service(create_post);
}
