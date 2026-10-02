//! The jobs: `Jobs::<Name>#execute(args)` ports, by name.

use super::{Job, JobError};
use crate::AppState;

/// Runs a job by its Sidekiq name.
pub async fn run(state: &AppState, job: &Job) -> Result<(), JobError> {
    let _ = state;
    match job.name.as_str() {
        // Publishes the topic's tracking state on MessageBus, which is not
        // ported: there is no one to tell.
        "post_update_topic_tracking_state" => Ok(()),
        other => Err(JobError::Unported(format!(
            "not ported yet: the {other} job"
        ))),
    }
}
