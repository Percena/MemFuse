use std::error::Error;
use std::path::Path;

use mfs_index::SqliteSemanticIndex;
use mfs_metadata::MetadataStore;
use mfs_semantic::{SemanticRuntimeConfig, current_runtime_config};
use mfs_session::{SessionEngine, TaskRecord as SessionTaskRecord, TaskStatus};
use mfs_types::IdentityContext;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResourceStatusCounts {
    pub total: usize,
    pub ready: usize,
    pub processing: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskStateCounts {
    pub total: usize,
    pub pending: usize,
    pub running: usize,
    pub completed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SystemStatusSummary {
    pub workspace_root: String,
    pub resources: ResourceStatusCounts,
    pub metadata_tasks: TaskStateCounts,
    pub session_tasks: TaskStateCounts,
    pub snapshots_total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SemanticObserverStats {
    pub total_documents: usize,
    pub resource_documents: usize,
    pub memory_documents: usize,
    pub skill_documents: usize,
    pub embedding_dimension: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObserverStatusSummary {
    pub runtime: SemanticRuntimeConfig,
    pub semantic: SemanticObserverStats,
}

pub async fn system_status(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
) -> Result<SystemStatusSummary, Box<dyn Error>> {
    let resources = ResourceStatusCounts {
        total: metadata.count_resource_sources(identity.account_id(), identity.user_id(), None)?,
        ready: metadata.count_resource_sources(
            identity.account_id(),
            identity.user_id(),
            Some("ready"),
        )?,
        processing: metadata.count_resource_sources(
            identity.account_id(),
            identity.user_id(),
            Some("processing"),
        )?,
        failed: metadata.count_resource_sources(
            identity.account_id(),
            identity.user_id(),
            Some("failed"),
        )?,
    };
    let metadata_tasks = TaskStateCounts {
        total: metadata.count_tasks(identity.account_id(), identity.user_id(), None)?,
        pending: metadata.count_tasks(
            identity.account_id(),
            identity.user_id(),
            Some("pending"),
        )?,
        running: metadata.count_tasks(
            identity.account_id(),
            identity.user_id(),
            Some("running"),
        )?,
        completed: metadata.count_tasks(
            identity.account_id(),
            identity.user_id(),
            Some("completed"),
        )?,
        failed: metadata.count_tasks(identity.account_id(), identity.user_id(), Some("failed"))?,
    };
    let session_engine = SessionEngine::open(workspace_root).await?;
    let session_task_records = session_engine.list_tasks(10_000).await?;
    let session_tasks = summarize_session_tasks(&session_task_records);

    Ok(SystemStatusSummary {
        workspace_root: workspace_root.display().to_string(),
        resources,
        metadata_tasks,
        session_tasks,
        snapshots_total: metadata.count_snapshots(
            identity.account_id(),
            identity.user_id(),
            None,
        )?,
    })
}

pub fn observer_status(workspace_root: &Path) -> Result<ObserverStatusSummary, Box<dyn Error>> {
    let runtime = current_runtime_config();
    let semantic_index =
        SqliteSemanticIndex::open_at(workspace_root.join("_system").join("semantic.sqlite"))?;
    let semantic = SemanticObserverStats {
        total_documents: semantic_index.count_documents()?,
        resource_documents: semantic_index.count_documents_by_context_type("resource")?,
        memory_documents: semantic_index.count_documents_by_context_type("memory")?,
        skill_documents: semantic_index.count_documents_by_context_type("skill")?,
        embedding_dimension: semantic_index.embedding_dimension()?.unwrap_or_default(),
    };
    Ok(ObserverStatusSummary { runtime, semantic })
}

fn summarize_session_tasks(tasks: &[SessionTaskRecord]) -> TaskStateCounts {
    let mut counts = TaskStateCounts {
        total: tasks.len(),
        pending: 0,
        running: 0,
        completed: 0,
        failed: 0,
    };
    for task in tasks {
        match task.status {
            TaskStatus::Pending => counts.pending += 1,
            TaskStatus::Running => counts.running += 1,
            TaskStatus::Completed => counts.completed += 1,
            TaskStatus::Failed => counts.failed += 1,
        }
    }
    counts
}
