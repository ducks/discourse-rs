use actix_web::{get, post, put, web, HttpResponse, Responder};
use diesel::prelude::*;

use crate::models::{NewUser, UpdateUser, User};
use crate::schema::users;
use crate::DbPool;

#[get("/users")]
async fn list_users(pool: web::Data<DbPool>) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to get database connection"
        })),
    };

    let results = web::block(move || {
        users::table
            .select(User::as_select())
            .limit(50)
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
    user_id: web::Path<i32>,
    update_user: web::Json<UpdateUser>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to get database connection"
        })),
    };

    let user_id = user_id.into_inner();
    let update_user = update_user.into_inner();

    let result = web::block(move || {
        diesel::update(users::table.find(user_id))
            .set(&update_user)
            .returning(User::as_returning())
            .get_result(&mut conn)
    })
    .await;

    match result {
        Ok(Ok(user)) => HttpResponse::Ok().json(user),
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

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(list_users)
        .service(get_user)
        .service(create_user)
        .service(update_user);
}
