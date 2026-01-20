use actix_web::{delete, get, post, put, web, HttpResponse, Responder};
use crate::{readable, writable};
use diesel::prelude::*;

use crate::jobs::{JobQueue, PropagateUsernameJob};
use crate::models::{NewUser, UpdateUser, User};
use crate::pagination::PaginationParams;
use crate::schema::users;
use crate::DbPool;

#[get("/users")]
async fn list_users(
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
        users::table
            .select(User::as_select())
            .limit(per_page)
            .offset(offset)
            .load(&mut conn)
    })
    .await;

    match results {
        Ok(Ok(users)) => HttpResponse::Ok().json(users),
        Ok(Err(_)) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to load users"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

#[get("/users/{id}")]
async fn get_user(pool: web::Data<DbPool>, user_id: web::Path<i32>) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to get database connection"
        })),
    };

    let user_id = user_id.into_inner();

    let result = web::block(move || {
        users::table
            .find(user_id)
            .select(User::as_select())
            .first(&mut conn)
    })
    .await;

    match result {
        Ok(Ok(user)) => HttpResponse::Ok().json(user),
        Ok(Err(diesel::NotFound)) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "User not found"
        })),
        Ok(Err(_)) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to load user"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

#[post("/users")]
async fn create_user(
    pool: web::Data<DbPool>,
    new_user: web::Json<NewUser>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to get database connection"
        })),
    };

    let new_user = new_user.into_inner();

    let result = web::block(move || {
        diesel::insert_into(users::table)
            .values(&new_user)
            .returning(User::as_returning())
            .get_result(&mut conn)
    })
    .await;

    match result {
        Ok(Ok(user)) => HttpResponse::Created().json(user),
        Ok(Err(_)) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Failed to create user"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

#[put("/users/{id}")]
async fn update_user(
    pool: web::Data<DbPool>,
    job_queue: web::Data<JobQueue>,
    user_id: web::Path<i32>,
    update_user: web::Json<UpdateUser>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to get database connection"
        })),
    };

    let user_id_val = user_id.into_inner();
    let update_data = update_user.into_inner();

    // Get the current username before updating (for propagation job)
    let old_username: Option<String> = if update_data.username.is_some() {
        users::table
            .find(user_id_val)
            .select(users::username)
            .first(&mut conn)
            .ok()
    } else {
        None
    };

    let new_username = update_data.username.clone();

    let result = web::block(move || {
        diesel::update(users::table.find(user_id_val))
            .set(&update_data)
            .returning(User::as_returning())
            .get_result(&mut conn)
    })
    .await;

    match result {
        Ok(Ok(user)) => {
            // If username changed, enqueue propagation job
            if let (Some(old), Some(new)) = (old_username, new_username) {
                if old != new {
                    let job = PropagateUsernameJob {
                        user_id: user_id_val,
                        old_username: old,
                        new_username: new,
                    };
                    if let Err(e) = job_queue.enqueue(job) {
                        log::error!("Failed to enqueue username propagation job: {}", e);
                    }
                }
            }
            HttpResponse::Ok().json(user)
        }
        Ok(Err(diesel::NotFound)) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "User not found"
        })),
        Ok(Err(_)) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Failed to update user"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

#[delete("/users/{id}")]
async fn delete_user(pool: web::Data<DbPool>, user_id: web::Path<i32>) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to get database connection"
            }))
        }
    };

    let user_id = user_id.into_inner();

    let result = web::block(move || diesel::delete(users::table.find(user_id)).execute(&mut conn))
        .await;

    match result {
        Ok(Ok(1)) => HttpResponse::NoContent().finish(),
        Ok(Ok(0)) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "User not found"
        })),
        Ok(Ok(_)) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Multiple users deleted"
        })),
        Ok(Err(_)) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Failed to delete user"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(readable!(list_users, get_user));
    cfg.service(writable!(create_user, update_user, delete_user));
}
