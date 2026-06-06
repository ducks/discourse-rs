use actix_web::{HttpResponse, Responder, post, web};
use serde::Deserialize;
use serde_json::json;

use crate::DbPool;
use crate::middleware::AuthUser;
use crate::services::reads::{ReadError, record_topic_view};

#[derive(Debug, Deserialize)]
pub struct TopicReadInput {
    pub topic_id: i32,
    /// Seconds spent on the topic since the last call. Server caps this
    /// (see `services::reads::MAX_SECONDS_PER_CALL`), so a client that
    /// reports 9999 won't inflate the counter.
    pub seconds: i32,
}

/// POST /api/read/topic
///
/// Records that the authenticated user just viewed a topic. Idempotent:
/// repeated calls update `last_viewed_at` but don't double-count the
/// topic in the user's `topics_entered` stat.
///
/// - 204 No Content on success (first view or revisit, no distinction
///   exposed to the client)
/// - 404 if the topic doesn't exist
#[post("/read/topic")]
async fn record_topic_read(
    pool: web::Data<DbPool>,
    auth: AuthUser,
    input: web::Json<TopicReadInput>,
) -> impl Responder {
    let user_id = auth.0.user_id;
    let TopicReadInput { topic_id, seconds } = input.into_inner();

    let mut conn = match pool.get() {
        Ok(c) => c,
        Err(_) => {
            return HttpResponse::InternalServerError()
                .json(json!({ "error": "Failed to get database connection" }));
        }
    };

    let result = web::block(move || record_topic_view(&mut conn, user_id, topic_id, seconds)).await;

    match result {
        Ok(Ok(_)) => HttpResponse::NoContent().finish(),
        Ok(Err(ReadError::TopicNotFound)) => {
            HttpResponse::NotFound().json(json!({ "error": "Topic not found" }))
        }
        Ok(Err(ReadError::Db(e))) => HttpResponse::InternalServerError()
            .json(json!({ "error": format!("Database error: {e}") })),
        Err(e) => HttpResponse::InternalServerError()
            .json(json!({ "error": format!("Blocking error: {e}") })),
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(record_topic_read);
}
