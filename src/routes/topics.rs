use actix_web::{delete, get, post, put, web, HttpResponse, Responder};
use crate::{readable, writable};
use diesel::prelude::*;

use crate::models::{NewTopic, Topic, UpdateTopic};
use crate::pagination::PaginationParams;
use crate::schema::topics;
use crate::DbPool;

#[get("/topics")]
async fn list_topics(
    pool: web::Data<DbPool>,
    pagination: web::Query<PaginationParams>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to get database connection"
        })),
    };

    let per_page = pagination.per_page();
    let offset = pagination.offset();

    let results = web::block(move || {
        topics::table
            .select(Topic::as_select())
            .order(topics::created_at.desc())
            .limit(per_page)
            .offset(offset)
            .load(&mut conn)
    })
    .await;

    match results {
        Ok(Ok(topics)) => HttpResponse::Ok().json(topics),
        Ok(Err(_)) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to load topics"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

#[get("/topics/{id}")]
async fn get_topic(pool: web::Data<DbPool>, topic_id: web::Path<i32>) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to get database connection"
        })),
    };

    let topic_id = topic_id.into_inner();

    let result = web::block(move || {
        topics::table
            .find(topic_id)
            .select(Topic::as_select())
            .first(&mut conn)
    })
    .await;

    match result {
        Ok(Ok(topic)) => HttpResponse::Ok().json(topic),
        Ok(Err(diesel::NotFound)) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Topic not found"
        })),
        Ok(Err(_)) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to load topic"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

#[post("/topics")]
async fn create_topic(
    pool: web::Data<DbPool>,
    new_topic: web::Json<NewTopic>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to get database connection"
        })),
    };

    let new_topic = new_topic.into_inner();

    let result = web::block(move || {
        diesel::insert_into(topics::table)
            .values(&new_topic)
            .returning(Topic::as_returning())
            .get_result(&mut conn)
    })
    .await;

    match result {
        Ok(Ok(topic)) => HttpResponse::Created().json(topic),
        Ok(Err(_)) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Failed to create topic"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

#[put("/topics/{id}")]
async fn update_topic(
    pool: web::Data<DbPool>,
    topic_id: web::Path<i32>,
    update_topic: web::Json<UpdateTopic>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to get database connection"
        })),
    };

    let topic_id = topic_id.into_inner();
    let update_topic = update_topic.into_inner();

    let result = web::block(move || {
        diesel::update(topics::table.find(topic_id))
            .set(&update_topic)
            .returning(Topic::as_returning())
            .get_result(&mut conn)
    })
    .await;

    match result {
        Ok(Ok(topic)) => HttpResponse::Ok().json(topic),
        Ok(Err(diesel::NotFound)) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Topic not found"
        })),
        Ok(Err(_)) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Failed to update topic"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

#[delete("/topics/{id}")]
async fn delete_topic(pool: web::Data<DbPool>, topic_id: web::Path<i32>) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to get database connection"
            }))
        }
    };

    let topic_id = topic_id.into_inner();

    let result =
        web::block(move || diesel::delete(topics::table.find(topic_id)).execute(&mut conn)).await;

    match result {
        Ok(Ok(1)) => HttpResponse::NoContent().finish(),
        Ok(Ok(0)) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Topic not found"
        })),
        Ok(Ok(_)) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Multiple topics deleted"
        })),
        Ok(Err(_)) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Failed to delete topic"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(readable!(list_topics, get_topic));
    cfg.service(writable!(create_topic, update_topic, delete_topic));
}
