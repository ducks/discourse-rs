use actix_web::{post, web, HttpResponse, Responder};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::auth::{generate_token, hash_password, verify_password};
use crate::models::{NewUser, User};
use crate::schema::users;
use crate::DbPool;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: UserResponse,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: i32,
    pub username: String,
    pub email: String,
    pub admin: bool,
    pub moderator: bool,
    pub trust_level: i32,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        UserResponse {
            id: user.id,
            username: user.username,
            email: user.email,
            admin: user.admin,
            moderator: user.moderator,
            trust_level: user.trust_level,
        }
    }
}

#[post("/auth/login")]
async fn login(pool: web::Data<DbPool>, credentials: web::Json<LoginRequest>) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to get database connection"
            }))
        }
    };

    let credentials = credentials.into_inner();

    let result = web::block(move || {
        users::table
            .filter(users::username.eq(&credentials.username))
            .select(User::as_select())
            .first(&mut conn)
    })
    .await;

    match result {
        Ok(Ok(user)) => {
            match verify_password(&credentials.password, &user.password_hash) {
                Ok(true) => {
                    match generate_token(user.id, user.username.clone()) {
                        Ok(token) => HttpResponse::Ok().json(AuthResponse {
                            token,
                            user: user.into(),
                        }),
                        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
                            "error": "Failed to generate token"
                        })),
                    }
                }
                Ok(false) => HttpResponse::Unauthorized().json(serde_json::json!({
                    "error": "Invalid username or password"
                })),
                Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
                    "error": "Failed to verify password"
                })),
            }
        }
        Ok(Err(diesel::NotFound)) => HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Invalid username or password"
        })),
        Ok(Err(_)) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to load user"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

#[post("/auth/register")]
async fn register(
    pool: web::Data<DbPool>,
    new_user: web::Json<RegisterRequest>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to get database connection"
            }))
        }
    };

    let new_user = new_user.into_inner();

    let password_hash = match hash_password(&new_user.password) {
        Ok(hash) => hash,
        Err(_) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to hash password"
            }))
        }
    };

    let new_user_data = NewUser {
        username: new_user.username,
        email: new_user.email,
        password_hash,
        admin: false,
        moderator: false,
        trust_level: 0,
    };

    let result = web::block(move || {
        diesel::insert_into(users::table)
            .values(&new_user_data)
            .returning(User::as_returning())
            .get_result(&mut conn)
    })
    .await;

    match result {
        Ok(Ok(user)) => match generate_token(user.id, user.username.clone()) {
            Ok(token) => HttpResponse::Created().json(AuthResponse {
                token,
                user: user.into(),
            }),
            Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to generate token"
            })),
        },
        Ok(Err(_)) => HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Failed to create user (username or email may already exist)"
        })),
        Err(_) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Blocking error"
        })),
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(login).service(register);
}
