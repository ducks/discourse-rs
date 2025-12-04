use actix_web::{get, post, put, web, HttpResponse, Responder};
use diesel::prelude::*;

use crate::models::{NewPost, Post, UpdatePost};
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

#[put("/posts/{id}")]
async fn update_post(
    pool: web::Data<DbPool>,
    post_id: web::Path<i32>,
    update_post: web::Json<UpdatePost>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to get database connection"
        })),
    };

    let post_id = post_id.into_inner();
    let update_post = update_post.into_inner();

    let result = web::block(move || {
        diesel::update(posts::table.find(post_id))
            .set(&update_post)
            .returning(Post::as_returning())
            .get_result(&mut conn)
    })
    .await;

    match result {
        Ok(Ok(post)) => HttpResponse::Ok().json(post),
        Ok(Err(diesel::NotFound)) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Post not found"
        })),
        Ok(Err(_)) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Failed to update post"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(list_posts)
        .service(list_topic_posts)
        .service(create_post)
        .service(update_post);
}
