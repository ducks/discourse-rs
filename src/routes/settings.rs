use actix_web::{get, put, web, HttpResponse, Responder};
use crate::{readable, writable};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::models::{SiteSetting, UpdateSiteSetting};
use crate::schema::site_settings;
use crate::DbPool;

#[derive(Deserialize)]
struct UpdateSettingRequest {
    value: String,
}

#[derive(Serialize)]
struct SettingsResponse {
    settings: Vec<SiteSetting>,
}

#[get("/settings")]
async fn list_settings(pool: web::Data<DbPool>) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to get database connection"
            }))
        }
    };

    let results = web::block(move || {
        site_settings::table
            .select(SiteSetting::as_select())
            .load(&mut conn)
    })
    .await;

    match results {
        Ok(Ok(settings)) => HttpResponse::Ok().json(SettingsResponse { settings }),
        Ok(Err(_)) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to load settings"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

#[get("/settings/{key}")]
async fn get_setting(pool: web::Data<DbPool>, key: web::Path<String>) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to get database connection"
            }))
        }
    };

    let key = key.into_inner();

    let result = web::block(move || {
        site_settings::table
            .find(&key)
            .select(SiteSetting::as_select())
            .first(&mut conn)
    })
    .await;

    match result {
        Ok(Ok(setting)) => HttpResponse::Ok().json(setting),
        Ok(Err(diesel::NotFound)) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Setting not found"
        })),
        Ok(Err(_)) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to load setting"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

#[put("/settings/{key}")]
async fn update_setting(
    pool: web::Data<DbPool>,
    key: web::Path<String>,
    update_request: web::Json<UpdateSettingRequest>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to get database connection"
            }))
        }
    };

    let key = key.into_inner();
    let update = UpdateSiteSetting {
        value: update_request.value.clone(),
    };

    let result = web::block(move || {
        diesel::update(site_settings::table.find(&key))
            .set(&update)
            .returning(SiteSetting::as_returning())
            .get_result(&mut conn)
    })
    .await;

    match result {
        Ok(Ok(setting)) => HttpResponse::Ok().json(setting),
        Ok(Err(diesel::NotFound)) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "Setting not found"
        })),
        Ok(Err(_)) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Failed to update setting"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(readable!(list_settings, get_setting));
    // PUT endpoint - always require authentication (admin only ideally, but we'll use basic auth for now)
    cfg.service(writable!(update_setting));
}
