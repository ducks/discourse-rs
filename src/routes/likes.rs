use actix_web::{HttpResponse, Responder, delete, post, web};
use serde_json::json;

use crate::DbPool;
use crate::middleware::AuthUser;
use crate::services::likes::{LikeError, LikeOutcome, UnlikeOutcome, like_post, unlike_post};

/// POST /posts/:id/like
///
/// Like a post. Constraints:
/// - Caller must be authenticated.
/// - Post must exist and not be deleted/hidden.
/// - Caller cannot like their own post.
/// - Liking the same post twice is a no-op (returns 200 with the existing
///   like rather than 409, so clients don't have to handle the race
///   between two tabs).
#[post("/posts/{id}/like")]
async fn like_post_route(
    pool: web::Data<DbPool>,
    auth: AuthUser,
    path: web::Path<i32>,
) -> impl Responder {
    let post_id = path.into_inner();
    let user_id = auth.0.user_id;

    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => {
            return HttpResponse::InternalServerError()
                .json(json!({ "error": "Failed to get database connection" }));
        }
    };

    let result = web::block(move || like_post(&mut conn, user_id, post_id)).await;

    match result {
        Ok(Ok(LikeOutcome::Created(like))) => HttpResponse::Created().json(like),
        Ok(Ok(LikeOutcome::AlreadyLiked(like))) => HttpResponse::Ok().json(like),
        Ok(Err(LikeError::PostNotFound)) => {
            HttpResponse::NotFound().json(json!({ "error": "Post not found" }))
        }
        Ok(Err(LikeError::SelfLike)) => HttpResponse::UnprocessableEntity()
            .json(json!({ "error": "You cannot like your own post" })),
        Ok(Err(LikeError::Db(e))) => HttpResponse::InternalServerError()
            .json(json!({ "error": format!("Database error: {e}") })),
        Err(e) => HttpResponse::InternalServerError()
            .json(json!({ "error": format!("Blocking error: {e}") })),
    }
}

/// DELETE /posts/:id/like
///
/// Unlike a post. Reverses the counter updates from `like_post`. Deleting
/// a non-existent like is a no-op (returns 204) so clients don't have to
/// distinguish "never liked" from "already unliked."
#[delete("/posts/{id}/like")]
async fn unlike_post_route(
    pool: web::Data<DbPool>,
    auth: AuthUser,
    path: web::Path<i32>,
) -> impl Responder {
    let post_id = path.into_inner();
    let user_id = auth.0.user_id;

    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => {
            return HttpResponse::InternalServerError()
                .json(json!({ "error": "Failed to get database connection" }));
        }
    };

    let result = web::block(move || unlike_post(&mut conn, user_id, post_id)).await;

    match result {
        Ok(Ok(UnlikeOutcome::Removed)) | Ok(Ok(UnlikeOutcome::NothingToRemove)) => {
            HttpResponse::NoContent().finish()
        }
        Ok(Err(LikeError::Db(e))) => HttpResponse::InternalServerError()
            .json(json!({ "error": format!("Database error: {e}") })),
        Ok(Err(_)) => HttpResponse::InternalServerError()
            .json(json!({ "error": "Unexpected error" })),
        Err(e) => HttpResponse::InternalServerError()
            .json(json!({ "error": format!("Blocking error: {e}") })),
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(like_post_route).service(unlike_post_route);
}
