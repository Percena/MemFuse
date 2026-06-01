use std::error::Error;
use std::time::Duration;

use mfs_metadata::StoredTask;
use mfs_session::{SessionEngine, TaskRecord as SessionTaskRecord, TaskStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitTaskOutcome {
    Session(SessionTaskRecord),
    Metadata(StoredTask),
    Timeout { task_id: String },
}

pub async fn wait_for_task_completion(
    metadata: &mfs_metadata::MetadataStore,
    session_engine: &SessionEngine,
    task_id: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<WaitTaskOutcome, Box<dyn Error>> {
    let started = std::time::Instant::now();
    loop {
        if let Some(task) = session_engine.task_status(task_id).await {
            if matches!(task.status, TaskStatus::Completed | TaskStatus::Failed) {
                return Ok(WaitTaskOutcome::Session(task));
            }
        }

        if let Some(task) = metadata.get_task(task_id)? {
            if matches!(task.state.as_str(), "completed" | "failed") {
                return Ok(WaitTaskOutcome::Metadata(task));
            }
        }

        if started.elapsed() >= timeout {
            return Ok(WaitTaskOutcome::Timeout {
                task_id: task_id.to_owned(),
            });
        }

        tokio::time::sleep(poll_interval).await;
    }
}
