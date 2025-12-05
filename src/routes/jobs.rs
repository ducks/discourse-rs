use actix_web::{post, web, HttpResponse, Responder};
use serde::Deserialize;

use crate::jobs::{Job, JobQueue, ProcessTopicJob, WelcomeEmailJob};

#[derive(Deserialize)]
struct EnqueueWelcomeEmailRequest {
    user_id: i32,
    username: String,
    email: String,
}

#[derive(Deserialize)]
struct EnqueueProcessTopicRequest {
    topic_id: i32,
    action: String,
}

#[post("/jobs/welcome_email")]
async fn enqueue_welcome_email(
    queue: web::Data<JobQueue>,
    req: web::Json<EnqueueWelcomeEmailRequest>,
) -> impl Responder {
    let job = WelcomeEmailJob {
        user_id: req.user_id,
        username: req.username.clone(),
        email: req.email.clone(),
    };

    match queue.enqueue(job) {
        Ok(task_hash) => HttpResponse::Ok().json(serde_json::json!({
            "message": "Welcome email job enqueued successfully",
            "task_hash": task_hash
        })),
        Err(e) => {
            log::error!("Failed to enqueue welcome email job: {:?}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to enqueue job: {}", e)
            }))
        }
    }
}

#[post("/jobs/process_topic")]
async fn enqueue_process_topic(
    queue: web::Data<JobQueue>,
    req: web::Json<EnqueueProcessTopicRequest>,
) -> impl Responder {
    let job = ProcessTopicJob {
        topic_id: req.topic_id,
        action: req.action.clone(),
    };

    match queue.enqueue(job) {
        Ok(task_hash) => HttpResponse::Ok().json(serde_json::json!({
            "message": "Process topic job enqueued successfully",
            "task_hash": task_hash
        })),
        Err(e) => {
            log::error!("Failed to enqueue process topic job: {:?}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to enqueue job: {}", e)
            }))
        }
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(enqueue_welcome_email)
        .service(enqueue_process_topic);
}
