use actix_web::{get, post, put, web, HttpResponse, Responder};
use diesel::prelude::*;
use serde::Deserialize;

use crate::guardian::AuthenticatedUser;
use crate::models::Notification;
use crate::pagination::PaginationParams;
use crate::schema::notifications;
use crate::DbPool;

#[derive(Debug, Deserialize)]
pub struct NotificationFilters {
    pub unread_only: Option<bool>,
}

#[get("/notifications")]
async fn list_notifications(
    pool: web::Data<DbPool>,
    auth: AuthenticatedUser,
    pagination: web::Query<PaginationParams>,
    filters: web::Query<NotificationFilters>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    let user_id = auth.0.user_id;
    let per_page = pagination.per_page();
    let offset = pagination.offset();
    let unread_only = filters.unread_only.unwrap_or(false);

    let result = if unread_only {
        notifications::table
            .filter(notifications::user_id.eq(user_id))
            .filter(notifications::read.eq(false))
            .order(notifications::created_at.desc())
            .limit(per_page)
            .offset(offset)
            .select(Notification::as_select())
            .load(&mut conn)
    } else {
        notifications::table
            .filter(notifications::user_id.eq(user_id))
            .order(notifications::created_at.desc())
            .limit(per_page)
            .offset(offset)
            .select(Notification::as_select())
            .load(&mut conn)
    };

    match result {
        Ok(notifs) => HttpResponse::Ok().json(notifs),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to load notifications: {}", e)
        })),
    }
}

#[get("/notifications/unread-count")]
async fn unread_count(pool: web::Data<DbPool>, auth: AuthenticatedUser) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    let user_id = auth.0.user_id;

    match notifications::table
        .filter(notifications::user_id.eq(user_id))
        .filter(notifications::read.eq(false))
        .count()
        .get_result::<i64>(&mut conn)
    {
        Ok(count) => HttpResponse::Ok().json(serde_json::json!({ "unread_count": count })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to count notifications: {}", e)
        })),
    }
}

#[put("/notifications/{id}/read")]
async fn mark_as_read(
    pool: web::Data<DbPool>,
    auth: AuthenticatedUser,
    notification_id: web::Path<i64>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    let user_id = auth.0.user_id;
    let notif_id = notification_id.into_inner();

    // Only allow marking your own notifications as read
    match diesel::update(
        notifications::table
            .filter(notifications::id.eq(notif_id))
            .filter(notifications::user_id.eq(user_id)),
    )
    .set(notifications::read.eq(true))
    .get_result::<Notification>(&mut conn)
    {
        Ok(notif) => HttpResponse::Ok().json(notif),
        Err(diesel::NotFound) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Notification not found"
        })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to mark notification as read: {}", e)
        })),
    }
}

#[post("/notifications/mark-all-read")]
async fn mark_all_read(pool: web::Data<DbPool>, auth: AuthenticatedUser) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    let user_id = auth.0.user_id;

    match diesel::update(
        notifications::table
            .filter(notifications::user_id.eq(user_id))
            .filter(notifications::read.eq(false)),
    )
    .set(notifications::read.eq(true))
    .execute(&mut conn)
    {
        Ok(count) => HttpResponse::Ok().json(serde_json::json!({
            "marked_read": count
        })),
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to mark notifications as read: {}", e)
        })),
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(list_notifications)
        .service(unread_count)
        .service(mark_as_read)
        .service(mark_all_read);
}
