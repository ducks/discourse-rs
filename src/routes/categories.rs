use actix_web::{delete, get, post, put, web, HttpResponse, Responder};
use diesel::prelude::*;

use crate::guardian::ModeratorGuard;
use crate::models::{Category, NewCategory, UpdateCategory};
use crate::schema::categories;
use crate::DbPool;

// Public endpoints - anyone can read categories

#[get("/categories")]
async fn list_categories(pool: web::Data<DbPool>) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    match categories::table
        .select(Category::as_select())
        .order(categories::position.asc())
        .load(&mut conn)
    {
        Ok(cats) => HttpResponse::Ok().json(cats),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to load categories: {}", e)
        })),
    }
}

#[get("/categories/{id}")]
async fn get_category(pool: web::Data<DbPool>, category_id: web::Path<i32>) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    match categories::table
        .find(category_id.into_inner())
        .select(Category::as_select())
        .first(&mut conn)
    {
        Ok(category) => HttpResponse::Ok().json(category),
        Err(diesel::NotFound) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Category not found"
        })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to load category: {}", e)
        })),
    }
}

// Moderator-only endpoints - create, update, delete

#[post("/categories")]
async fn create_category(
    pool: web::Data<DbPool>,
    _guard: ModeratorGuard,
    new_category: web::Json<NewCategory>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    match diesel::insert_into(categories::table)
        .values(new_category.into_inner())
        .get_result::<Category>(&mut conn)
    {
        Ok(category) => HttpResponse::Created().json(category),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to create category: {}", e)
        })),
    }
}

#[put("/categories/{id}")]
async fn update_category(
    pool: web::Data<DbPool>,
    _guard: ModeratorGuard,
    category_id: web::Path<i32>,
    update_data: web::Json<UpdateCategory>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    match diesel::update(categories::table.find(category_id.into_inner()))
        .set(update_data.into_inner())
        .get_result::<Category>(&mut conn)
    {
        Ok(category) => HttpResponse::Ok().json(category),
        Err(diesel::NotFound) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Category not found"
        })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to update category: {}", e)
        })),
    }
}

#[delete("/categories/{id}")]
async fn delete_category(
    pool: web::Data<DbPool>,
    _guard: ModeratorGuard,
    category_id: web::Path<i32>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    match diesel::delete(categories::table.find(category_id.into_inner())).execute(&mut conn) {
        Ok(0) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Category not found"
        })),
        Ok(_) => HttpResponse::Ok().json(serde_json::json!({
            "message": "Category deleted successfully"
        })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to delete category: {}", e)
        })),
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(list_categories)
        .service(get_category)
        .service(create_category)
        .service(update_category)
        .service(delete_category);
}
