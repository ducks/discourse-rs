use actix_web::{post, web, HttpResponse, Responder};
use diesel::prelude::*;
use serde::Deserialize;

use crate::middleware::AuthUser;
use crate::moderation::{
    is_moderator, log_moderation_action, NewModerationAction, NewUserSuspension,
};
use crate::schema::{posts, topics, user_suspensions, users};
use crate::DbPool;

// Topic moderation

#[derive(Deserialize)]
struct TopicModerationRequest {
    topic_id: i32,
}

#[post("/moderation/topics/lock")]
async fn lock_topic(
    pool: web::Data<DbPool>,
    auth: AuthUser,
    req: web::Json<TopicModerationRequest>,
) -> impl Responder {
    // Check if user is moderator
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    let user_trust_level: i32 = match users::table
        .find(auth.0.user_id)
        .select(users::trust_level)
        .first(&mut conn)
    {
        Ok(level) => level,
        Err(_) => {
            return HttpResponse::Forbidden().json(serde_json::json!({
                "error": "User not found"
            }))
        }
    };

    if !is_moderator(user_trust_level) {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Only moderators can lock topics"
        }));
    }

    // Lock the topic
    match diesel::update(topics::table)
        .filter(topics::id.eq(req.topic_id))
        .set(topics::locked.eq(true))
        .execute(&mut conn)
    {
        Ok(_) => {
            // Log moderation action
            let _ = log_moderation_action(
                &pool,
                NewModerationAction {
                    action_type: "lock_topic".to_string(),
                    moderator_id: auth.0.user_id,
                    target_user_id: None,
                    target_topic_id: Some(req.topic_id),
                    target_post_id: None,
                    details: None,
                },
            );

            HttpResponse::Ok().json(serde_json::json!({
                "message": "Topic locked successfully"
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to lock topic: {}", e)
        })),
    }
}

#[post("/moderation/topics/unlock")]
async fn unlock_topic(
    pool: web::Data<DbPool>,
    auth: AuthUser,
    req: web::Json<TopicModerationRequest>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    let user_trust_level: i32 = match users::table
        .find(auth.0.user_id)
        .select(users::trust_level)
        .first(&mut conn)
    {
        Ok(level) => level,
        Err(_) => {
            return HttpResponse::Forbidden().json(serde_json::json!({
                "error": "User not found"
            }))
        }
    };

    if !is_moderator(user_trust_level) {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Only moderators can unlock topics"
        }));
    }

    match diesel::update(topics::table)
        .filter(topics::id.eq(req.topic_id))
        .set(topics::locked.eq(false))
        .execute(&mut conn)
    {
        Ok(_) => {
            let _ = log_moderation_action(
                &pool,
                NewModerationAction {
                    action_type: "unlock_topic".to_string(),
                    moderator_id: auth.0.user_id,
                    target_user_id: None,
                    target_topic_id: Some(req.topic_id),
                    target_post_id: None,
                    details: None,
                },
            );

            HttpResponse::Ok().json(serde_json::json!({
                "message": "Topic unlocked successfully"
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to unlock topic: {}", e)
        })),
    }
}

#[post("/moderation/topics/pin")]
async fn pin_topic(
    pool: web::Data<DbPool>,
    auth: AuthUser,
    req: web::Json<TopicModerationRequest>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    let user_trust_level: i32 = match users::table
        .find(auth.0.user_id)
        .select(users::trust_level)
        .first(&mut conn)
    {
        Ok(level) => level,
        Err(_) => {
            return HttpResponse::Forbidden().json(serde_json::json!({
                "error": "User not found"
            }))
        }
    };

    if !is_moderator(user_trust_level) {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Only moderators can pin topics"
        }));
    }

    let now = chrono::Utc::now().naive_utc();

    match diesel::update(topics::table)
        .filter(topics::id.eq(req.topic_id))
        .set((topics::pinned.eq(true), topics::pinned_at.eq(Some(now))))
        .execute(&mut conn)
    {
        Ok(_) => {
            let _ = log_moderation_action(
                &pool,
                NewModerationAction {
                    action_type: "pin_topic".to_string(),
                    moderator_id: auth.0.user_id,
                    target_user_id: None,
                    target_topic_id: Some(req.topic_id),
                    target_post_id: None,
                    details: None,
                },
            );

            HttpResponse::Ok().json(serde_json::json!({
                "message": "Topic pinned successfully"
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to pin topic: {}", e)
        })),
    }
}

#[post("/moderation/topics/unpin")]
async fn unpin_topic(
    pool: web::Data<DbPool>,
    auth: AuthUser,
    req: web::Json<TopicModerationRequest>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    let user_trust_level: i32 = match users::table
        .find(auth.0.user_id)
        .select(users::trust_level)
        .first(&mut conn)
    {
        Ok(level) => level,
        Err(_) => {
            return HttpResponse::Forbidden().json(serde_json::json!({
                "error": "User not found"
            }))
        }
    };

    if !is_moderator(user_trust_level) {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Only moderators can unpin topics"
        }));
    }

    match diesel::update(topics::table)
        .filter(topics::id.eq(req.topic_id))
        .set((topics::pinned.eq(false), topics::pinned_at.eq(None::<chrono::NaiveDateTime>)))
        .execute(&mut conn)
    {
        Ok(_) => {
            let _ = log_moderation_action(
                &pool,
                NewModerationAction {
                    action_type: "unpin_topic".to_string(),
                    moderator_id: auth.0.user_id,
                    target_user_id: None,
                    target_topic_id: Some(req.topic_id),
                    target_post_id: None,
                    details: None,
                },
            );

            HttpResponse::Ok().json(serde_json::json!({
                "message": "Topic unpinned successfully"
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to unpin topic: {}", e)
        })),
    }
}

#[post("/moderation/topics/close")]
async fn close_topic(
    pool: web::Data<DbPool>,
    auth: AuthUser,
    req: web::Json<TopicModerationRequest>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    let user_trust_level: i32 = match users::table
        .find(auth.0.user_id)
        .select(users::trust_level)
        .first(&mut conn)
    {
        Ok(level) => level,
        Err(_) => {
            return HttpResponse::Forbidden().json(serde_json::json!({
                "error": "User not found"
            }))
        }
    };

    if !is_moderator(user_trust_level) {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Only moderators can close topics"
        }));
    }

    let now = chrono::Utc::now().naive_utc();

    match diesel::update(topics::table)
        .filter(topics::id.eq(req.topic_id))
        .set((topics::closed.eq(true), topics::closed_at.eq(Some(now))))
        .execute(&mut conn)
    {
        Ok(_) => {
            let _ = log_moderation_action(
                &pool,
                NewModerationAction {
                    action_type: "close_topic".to_string(),
                    moderator_id: auth.0.user_id,
                    target_user_id: None,
                    target_topic_id: Some(req.topic_id),
                    target_post_id: None,
                    details: None,
                },
            );

            HttpResponse::Ok().json(serde_json::json!({
                "message": "Topic closed successfully"
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to close topic: {}", e)
        })),
    }
}

#[post("/moderation/topics/open")]
async fn open_topic(
    pool: web::Data<DbPool>,
    auth: AuthUser,
    req: web::Json<TopicModerationRequest>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    let user_trust_level: i32 = match users::table
        .find(auth.0.user_id)
        .select(users::trust_level)
        .first(&mut conn)
    {
        Ok(level) => level,
        Err(_) => {
            return HttpResponse::Forbidden().json(serde_json::json!({
                "error": "User not found"
            }))
        }
    };

    if !is_moderator(user_trust_level) {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Only moderators can open topics"
        }));
    }

    match diesel::update(topics::table)
        .filter(topics::id.eq(req.topic_id))
        .set((topics::closed.eq(false), topics::closed_at.eq(None::<chrono::NaiveDateTime>)))
        .execute(&mut conn)
    {
        Ok(_) => {
            let _ = log_moderation_action(
                &pool,
                NewModerationAction {
                    action_type: "open_topic".to_string(),
                    moderator_id: auth.0.user_id,
                    target_user_id: None,
                    target_topic_id: Some(req.topic_id),
                    target_post_id: None,
                    details: None,
                },
            );

            HttpResponse::Ok().json(serde_json::json!({
                "message": "Topic opened successfully"
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to open topic: {}", e)
        })),
    }
}

// Post moderation

#[derive(Deserialize)]
struct PostModerationRequest {
    post_id: i32,
}

#[post("/moderation/posts/hide")]
async fn hide_post(
    pool: web::Data<DbPool>,
    auth: AuthUser,
    req: web::Json<PostModerationRequest>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    let user_trust_level: i32 = match users::table
        .find(auth.0.user_id)
        .select(users::trust_level)
        .first(&mut conn)
    {
        Ok(level) => level,
        Err(_) => {
            return HttpResponse::Forbidden().json(serde_json::json!({
                "error": "User not found"
            }))
        }
    };

    if !is_moderator(user_trust_level) {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Only moderators can hide posts"
        }));
    }

    let now = chrono::Utc::now().naive_utc();

    match diesel::update(posts::table)
        .filter(posts::id.eq(req.post_id))
        .set((
            posts::hidden.eq(true),
            posts::hidden_at.eq(Some(now)),
            posts::hidden_by_user_id.eq(Some(auth.0.user_id)),
        ))
        .execute(&mut conn)
    {
        Ok(_) => {
            let _ = log_moderation_action(
                &pool,
                NewModerationAction {
                    action_type: "hide_post".to_string(),
                    moderator_id: auth.0.user_id,
                    target_user_id: None,
                    target_topic_id: None,
                    target_post_id: Some(req.post_id),
                    details: None,
                },
            );

            HttpResponse::Ok().json(serde_json::json!({
                "message": "Post hidden successfully"
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to hide post: {}", e)
        })),
    }
}

#[post("/moderation/posts/unhide")]
async fn unhide_post(
    pool: web::Data<DbPool>,
    auth: AuthUser,
    req: web::Json<PostModerationRequest>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    let user_trust_level: i32 = match users::table
        .find(auth.0.user_id)
        .select(users::trust_level)
        .first(&mut conn)
    {
        Ok(level) => level,
        Err(_) => {
            return HttpResponse::Forbidden().json(serde_json::json!({
                "error": "User not found"
            }))
        }
    };

    if !is_moderator(user_trust_level) {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Only moderators can unhide posts"
        }));
    }

    match diesel::update(posts::table)
        .filter(posts::id.eq(req.post_id))
        .set((
            posts::hidden.eq(false),
            posts::hidden_at.eq(None::<chrono::NaiveDateTime>),
            posts::hidden_by_user_id.eq(None::<i32>),
        ))
        .execute(&mut conn)
    {
        Ok(_) => {
            let _ = log_moderation_action(
                &pool,
                NewModerationAction {
                    action_type: "unhide_post".to_string(),
                    moderator_id: auth.0.user_id,
                    target_user_id: None,
                    target_topic_id: None,
                    target_post_id: Some(req.post_id),
                    details: None,
                },
            );

            HttpResponse::Ok().json(serde_json::json!({
                "message": "Post unhidden successfully"
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to unhide post: {}", e)
        })),
    }
}

#[post("/moderation/posts/delete")]
async fn delete_post(
    pool: web::Data<DbPool>,
    auth: AuthUser,
    req: web::Json<PostModerationRequest>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    let user_trust_level: i32 = match users::table
        .find(auth.0.user_id)
        .select(users::trust_level)
        .first(&mut conn)
    {
        Ok(level) => level,
        Err(_) => {
            return HttpResponse::Forbidden().json(serde_json::json!({
                "error": "User not found"
            }))
        }
    };

    if !is_moderator(user_trust_level) {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Only moderators can delete posts"
        }));
    }

    let now = chrono::Utc::now().naive_utc();

    match diesel::update(posts::table)
        .filter(posts::id.eq(req.post_id))
        .set((
            posts::deleted_at.eq(Some(now)),
            posts::deleted_by_user_id.eq(Some(auth.0.user_id)),
        ))
        .execute(&mut conn)
    {
        Ok(_) => {
            let _ = log_moderation_action(
                &pool,
                NewModerationAction {
                    action_type: "delete_post".to_string(),
                    moderator_id: auth.0.user_id,
                    target_user_id: None,
                    target_topic_id: None,
                    target_post_id: Some(req.post_id),
                    details: None,
                },
            );

            HttpResponse::Ok().json(serde_json::json!({
                "message": "Post deleted successfully"
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to delete post: {}", e)
        })),
    }
}

// User suspension

#[derive(Deserialize)]
struct SuspendUserRequest {
    user_id: i32,
    reason: String,
    duration_days: i64,
}

#[post("/moderation/users/suspend")]
async fn suspend_user(
    pool: web::Data<DbPool>,
    auth: AuthUser,
    req: web::Json<SuspendUserRequest>,
) -> impl Responder {
    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Database connection failed: {}", e)
            }))
        }
    };

    let user_trust_level: i32 = match users::table
        .find(auth.0.user_id)
        .select(users::trust_level)
        .first(&mut conn)
    {
        Ok(level) => level,
        Err(_) => {
            return HttpResponse::Forbidden().json(serde_json::json!({
                "error": "User not found"
            }))
        }
    };

    if !is_moderator(user_trust_level) {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Only moderators can suspend users"
        }));
    }

    let suspended_until =
        chrono::Utc::now().naive_utc() + chrono::Duration::days(req.duration_days);

    let new_suspension = NewUserSuspension {
        user_id: req.user_id,
        suspended_by_user_id: auth.0.user_id,
        reason: req.reason.clone(),
        suspended_until,
    };

    match diesel::insert_into(user_suspensions::table)
        .values(&new_suspension)
        .execute(&mut conn)
    {
        Ok(_) => {
            let _ = log_moderation_action(
                &pool,
                NewModerationAction {
                    action_type: "suspend_user".to_string(),
                    moderator_id: auth.0.user_id,
                    target_user_id: Some(req.user_id),
                    target_topic_id: None,
                    target_post_id: None,
                    details: Some(serde_json::json!({
                        "reason": req.reason,
                        "duration_days": req.duration_days
                    })),
                },
            );

            HttpResponse::Ok().json(serde_json::json!({
                "message": "User suspended successfully",
                "suspended_until": suspended_until.to_string()
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Failed to suspend user: {}", e)
        })),
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        crate::writable!(
            lock_topic,
            unlock_topic,
            pin_topic,
            unpin_topic,
            close_topic,
            open_topic,
            hide_post,
            unhide_post,
            delete_post,
            suspend_user
        ),
    );
}
