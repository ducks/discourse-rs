use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

use crate::schema::backie_tasks;
use crate::DbPool;

// Job trait - implement this for each job type
pub trait Job: Send + Sync + 'static {
    fn job_name(&self) -> &'static str;
    fn execute(&self) -> Result<(), String>;
    fn to_json(&self) -> Result<serde_json::Value, String>;
}

// Example job: Welcome email
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WelcomeEmailJob {
    pub user_id: i32,
    pub username: String,
    pub email: String,
}

impl Job for WelcomeEmailJob {
    fn job_name(&self) -> &'static str {
        "welcome_email"
    }

    fn execute(&self) -> Result<(), String> {
        log::info!(
            "Sending welcome email to user {} ({}) at {}",
            self.user_id,
            self.username,
            self.email
        );

        // Simulate email sending
        std::thread::sleep(Duration::from_secs(2));

        log::info!("Welcome email sent successfully to {}", self.email);
        Ok(())
    }

    fn to_json(&self) -> Result<serde_json::Value, String> {
        serde_json::to_value(self).map_err(|e| e.to_string())
    }
}

// Example job: Process topic
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessTopicJob {
    pub topic_id: i32,
    pub action: String,
}

impl Job for ProcessTopicJob {
    fn job_name(&self) -> &'static str {
        "process_topic"
    }

    fn execute(&self) -> Result<(), String> {
        log::info!(
            "Processing topic {} with action: {}",
            self.topic_id,
            self.action
        );

        // Simulate processing
        std::thread::sleep(Duration::from_secs(1));

        log::info!("Topic {} processed successfully", self.topic_id);
        Ok(())
    }

    fn to_json(&self) -> Result<serde_json::Value, String> {
        serde_json::to_value(self).map_err(|e| e.to_string())
    }
}

// Database model for jobs
#[derive(Queryable, Selectable)]
#[diesel(table_name = backie_tasks)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct JobRecord {
    pub id: uuid::Uuid,
    pub task_name: String,
    pub task_hash: String,
    pub payload: serde_json::Value,
    pub timeout_msecs: i64,
    pub max_retries: i32,
    pub retries: i32,
    pub created_at: chrono::NaiveDateTime,
    pub scheduled_at: chrono::NaiveDateTime,
    pub running_at: Option<chrono::NaiveDateTime>,
    pub done_at: Option<chrono::NaiveDateTime>,
    pub error: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = backie_tasks)]
pub struct NewJob {
    pub task_name: String,
    pub task_hash: String,
    pub payload: serde_json::Value,
    pub timeout_msecs: i64,
    pub max_retries: i32,
    pub scheduled_at: chrono::NaiveDateTime,
}

// Job queue for enqueueing jobs
#[derive(Clone)]
pub struct JobQueue {
    pool: Arc<DbPool>,
}

impl JobQueue {
    pub fn new(pool: Arc<DbPool>) -> Self {
        Self { pool }
    }

    pub fn enqueue<J: Job>(&self, job: J) -> Result<String, String> {
        let mut conn = self.pool.get().map_err(|e| e.to_string())?;

        let payload = job.to_json()?;
        let task_hash = format!("{:x}", md5::compute(format!("{:?}", payload)));

        let new_job = NewJob {
            task_name: job.job_name().to_string(),
            task_hash: task_hash.clone(),
            payload,
            timeout_msecs: 30000, // 30 seconds
            max_retries: 3,
            scheduled_at: chrono::Utc::now().naive_utc(),
        };

        diesel::insert_into(backie_tasks::table)
            .values(&new_job)
            .execute(&mut conn)
            .map_err(|e| e.to_string())?;

        log::info!("Enqueued job {} with hash {}", job.job_name(), task_hash);
        Ok(task_hash)
    }
}

// Worker pool for processing jobs
pub struct WorkerPool {
    pool: Arc<DbPool>,
    worker_count: usize,
}

impl WorkerPool {
    pub fn new(pool: Arc<DbPool>, worker_count: usize) -> Self {
        Self { pool, worker_count }
    }

    pub async fn run(self) {
        log::info!("Starting job worker pool with {} workers", self.worker_count);

        let mut handles = vec![];

        for worker_id in 0..self.worker_count {
            let pool = Arc::clone(&self.pool);
            let handle = tokio::spawn(async move {
                Self::worker_loop(worker_id, pool).await;
            });
            handles.push(handle);
        }

        // Wait for all workers
        for handle in handles {
            let _ = handle.await;
        }
    }

    async fn worker_loop(worker_id: usize, pool: Arc<DbPool>) {
        log::info!("Worker {} started", worker_id);

        loop {
            // Poll for jobs every 5 seconds
            sleep(Duration::from_secs(5)).await;

            match Self::claim_and_execute_job(worker_id, &pool).await {
                Ok(executed) => {
                    if executed {
                        log::debug!("Worker {} executed a job", worker_id);
                    }
                }
                Err(e) => {
                    log::error!("Worker {} error: {}", worker_id, e);
                }
            }
        }
    }

    async fn claim_and_execute_job(worker_id: usize, pool: &DbPool) -> Result<bool, String> {
        use crate::schema::backie_tasks::dsl::*;

        let mut conn = pool.get().map_err(|e| e.to_string())?;

        // Claim a job using FOR UPDATE SKIP LOCKED
        let job: Option<JobRecord> = conn.transaction(|conn| {
            backie_tasks
                .filter(done_at.is_null())
                .filter(running_at.is_null())
                .filter(scheduled_at.le(chrono::Utc::now().naive_utc()))
                .order(scheduled_at.asc())
                .limit(1)
                .for_update()
                .skip_locked()
                .select(JobRecord::as_select())
                .first(conn)
                .optional()
        }).map_err(|e| e.to_string())?;

        if let Some(job_record) = job {
            log::info!("Worker {} claimed job {} ({})", worker_id, job_record.task_name, job_record.id);

            // Mark as running
            diesel::update(backie_tasks)
                .filter(id.eq(job_record.id))
                .set(running_at.eq(Some(chrono::Utc::now().naive_utc())))
                .execute(&mut conn)
                .map_err(|e| e.to_string())?;

            // Execute the job based on task_name
            let result = Self::execute_job_by_name(&job_record.task_name, &job_record.payload);

            // Update job status
            match result {
                Ok(_) => {
                    diesel::update(backie_tasks)
                        .filter(id.eq(job_record.id))
                        .set(done_at.eq(Some(chrono::Utc::now().naive_utc())))
                        .execute(&mut conn)
                        .map_err(|e| e.to_string())?;
                    log::info!("Worker {} completed job {}", worker_id, job_record.id);
                }
                Err(e) => {
                    diesel::update(backie_tasks)
                        .filter(id.eq(job_record.id))
                        .set((
                            error.eq(Some(e.clone())),
                            done_at.eq(Some(chrono::Utc::now().naive_utc())),
                        ))
                        .execute(&mut conn)
                        .map_err(|e| e.to_string())?;
                    log::error!("Worker {} failed job {}: {}", worker_id, job_record.id, e);
                }
            }

            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn execute_job_by_name(task_name: &str, payload: &serde_json::Value) -> Result<(), String> {
        match task_name {
            "welcome_email" => {
                let job: WelcomeEmailJob = serde_json::from_value(payload.clone())
                    .map_err(|e| format!("Failed to deserialize job: {}", e))?;
                job.execute()
            }
            "process_topic" => {
                let job: ProcessTopicJob = serde_json::from_value(payload.clone())
                    .map_err(|e| format!("Failed to deserialize job: {}", e))?;
                job.execute()
            }
            _ => Err(format!("Unknown job type: {}", task_name)),
        }
    }
}
