mod archive;
mod lock;
mod memory;
mod query;
mod redo;

use std::collections::HashMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::memory::{MemoryPipelineJob, MemoryPipelineResult, write_usage_snapshot};
use mfs_memory::UsageRecord;

/// Validate that an identity field is safe for use as a filesystem path segment.
fn validate_identity_segment(value: &str, field: &str) -> Result<(), SessionError> {
    mfs_types::sanitize_path_segment(value, field).map_err(|e: mfs_types::MfsError| match e {
        mfs_types::MfsError::InvalidArgument { field, reason } => {
            SessionError::InvalidArgument { field, reason }
        }
        other => SessionError::InvalidArgument {
            field: field.to_owned(),
            reason: other.to_string(),
        },
    })?;
    Ok(())
}
// Re-export candidate types from mfs-memory (backward compatibility for downstream consumers).
pub use crate::query::{
    ArchiveAbstractView, SessionArchiveView, SessionContextView, SessionMessageView, SessionSummary,
};
use crate::redo::RedoMarker;
pub use mfs_memory::{
    MemoryCandidate, MemoryCategory, MemoryDecision, MemoryMergeDecision, MemoryOwnership,
    MemoryRecord, decide_memory_merge, deterministic_extract, deterministic_merge,
    extract_memory_candidates, llm_merge_bundle,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    role: String,
    content: String,
}

/// Result of adding a message to a session. If `auto_committed` is true,
/// the session was automatically committed because it exceeded the
/// `auto_commit_threshold` estimated token budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddMessageResult {
    pub auto_committed: bool,
    pub archive_uri: Option<String>,
    pub task_id: Option<String>,
}

#[derive(Debug, Clone)]
struct SessionState {
    account_id: String,
    user_id: String,
    agent_id: String,
    messages: Vec<StoredMessage>,
    usage: Vec<UsageRecord>,
    archive_count: u32,
    estimated_tokens: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub task_id: String,
    pub archive_uri: String,
    pub status: TaskStatus,
    pub retry_state: Option<String>,
    pub processing_mode: Option<String>,
    pub used_contexts: usize,
    pub used_skills: usize,
    pub used_tools: usize,
    pub memories_extracted: HashMap<String, usize>,
    pub artifacts_written: HashMap<String, usize>,
    pub error: Option<String>,
}

/// Default auto-commit threshold (estimated tokens). 0 means disabled.
pub const AUTO_COMMIT_THRESHOLD_DEFAULT: usize = 8000;

pub struct SessionEngine {
    workspace_root: PathBuf,
    sessions: Arc<Mutex<HashMap<String, SessionState>>>,
    tasks: Arc<Mutex<HashMap<String, TaskRecord>>>,
    background_tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    auto_commit_threshold: usize,
    #[cfg(feature = "test-support")]
    _guard: Option<Arc<tempfile::TempDir>>,
}

impl SessionEngine {
    pub fn from_workspace_root(workspace_root: impl Into<PathBuf>) -> Self {
        Self::from_workspace_root_with_threshold(workspace_root, AUTO_COMMIT_THRESHOLD_DEFAULT)
    }

    pub fn from_workspace_root_with_threshold(
        workspace_root: impl Into<PathBuf>,
        auto_commit_threshold: usize,
    ) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            tasks: Arc::new(Mutex::new(HashMap::new())),
            background_tasks: Arc::new(Mutex::new(Vec::new())),
            auto_commit_threshold,
            #[cfg(feature = "test-support")]
            _guard: None,
        }
    }

    #[cfg(feature = "test-support")]
    pub async fn for_tests() -> Result<Self, SessionError> {
        Self::for_tests_with_threshold(AUTO_COMMIT_THRESHOLD_DEFAULT).await
    }

    #[cfg(feature = "test-support")]
    pub async fn for_tests_with_threshold(
        auto_commit_threshold: usize,
    ) -> Result<Self, SessionError> {
        let guard = Arc::new(tempfile::tempdir().map_err(SessionError::IoRaw)?);
        let workspace_root = guard.path().to_path_buf();
        tokio::fs::create_dir_all(&workspace_root)
            .await
            .map_err(|source| SessionError::io("create workspace root", &workspace_root, source))?;
        Ok(Self {
            workspace_root,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            tasks: Arc::new(Mutex::new(HashMap::new())),
            background_tasks: Arc::new(Mutex::new(Vec::new())),
            auto_commit_threshold,
            _guard: Some(guard),
        })
    }

    pub async fn open(workspace_root: impl AsRef<Path>) -> Result<Self, SessionError> {
        Self::open_with_threshold(workspace_root, AUTO_COMMIT_THRESHOLD_DEFAULT).await
    }

    pub async fn open_with_threshold(
        workspace_root: impl AsRef<Path>,
        auto_commit_threshold: usize,
    ) -> Result<Self, SessionError> {
        let workspace_root = workspace_root.as_ref();
        tokio::fs::create_dir_all(workspace_root)
            .await
            .map_err(|source| SessionError::io("create workspace root", workspace_root, source))?;
        Ok(Self {
            workspace_root: workspace_root.to_path_buf(),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            tasks: Arc::new(Mutex::new(HashMap::new())),
            background_tasks: Arc::new(Mutex::new(Vec::new())),
            auto_commit_threshold,
            #[cfg(feature = "test-support")]
            _guard: None,
        })
    }

    pub async fn new_session(
        &self,
        account_id: &str,
        user_id: &str,
        agent_id: &str,
    ) -> Result<String, SessionError> {
        let session_id = Uuid::new_v4().to_string();
        self.new_session_with_id(account_id, user_id, agent_id, &session_id)
            .await
    }

    pub async fn new_session_with_id(
        &self,
        account_id: &str,
        user_id: &str,
        agent_id: &str,
        session_id: &str,
    ) -> Result<String, SessionError> {
        validate_identity_segment(account_id, "account_id")?;
        validate_identity_segment(user_id, "user_id")?;
        validate_identity_segment(agent_id, "agent_id")?;
        let root = self
            .workspace_root
            .join("tenants")
            .join(account_id)
            .join(user_id)
            .join("session")
            .join(agent_id)
            .join(session_id);
        tokio::fs::create_dir_all(root.join("history"))
            .await
            .map_err(|source| SessionError::io("create session root", &root, source))?;
        let messages = query::read_live_messages(&root).await?;
        let estimated_tokens = messages
            .iter()
            .map(|message| (message.content.len() / 4).max(1))
            .sum();

        self.sessions.lock().await.insert(
            session_id.to_owned(),
            SessionState {
                account_id: account_id.to_owned(),
                user_id: user_id.to_owned(),
                agent_id: agent_id.to_owned(),
                messages,
                usage: Vec::new(),
                archive_count: 0,
                estimated_tokens,
            },
        );

        let metadata_path = self.workspace_root.join("_system").join("metadata.sqlite");
        let metadata =
            mfs_metadata::MetadataStore::open_at(&metadata_path, false).map_err(|source| {
                SessionError::io(
                    "open metadata store",
                    &metadata_path,
                    std::io::Error::other(source.to_string()),
                )
            })?;
        if metadata
            .get_session(session_id)
            .map_err(|source| {
                SessionError::io(
                    "read metadata session",
                    &metadata_path,
                    std::io::Error::other(source.to_string()),
                )
            })?
            .is_none()
        {
            metadata
                .insert_session(session_id, account_id, user_id, agent_id, "active", None)
                .map_err(|source| {
                    SessionError::io(
                        "insert metadata session",
                        &metadata_path,
                        std::io::Error::other(source.to_string()),
                    )
                })?;
        }
        Ok(session_id.to_owned())
    }

    pub async fn add_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
    ) -> Result<AddMessageResult, SessionError> {
        let new_tokens = (content.len() / 4).max(1);
        let should_auto_commit = {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .get_mut(session_id)
                .ok_or_else(|| SessionError::NotFound(session_id.to_owned()))?;
            session.messages.push(StoredMessage {
                role: role.to_owned(),
                content: content.to_owned(),
            });
            session.estimated_tokens += new_tokens;
            self.auto_commit_threshold > 0 && session.estimated_tokens >= self.auto_commit_threshold
        };

        // Persist messages regardless of auto-commit decision.
        {
            let sessions = self.sessions.lock().await;
            let session = sessions
                .get(session_id)
                .ok_or_else(|| SessionError::NotFound(session_id.to_owned()))?;
            let account_id = session.account_id.clone();
            let user_id = session.user_id.clone();
            let agent_id = session.agent_id.clone();
            let messages = session.messages.clone();
            drop(sessions);
            persist_live_messages(
                &self.workspace_root,
                &account_id,
                &user_id,
                &agent_id,
                session_id,
                &messages,
            )
            .await?;
        }

        if should_auto_commit {
            let commit_result = self.commit(session_id).await?;
            Ok(AddMessageResult {
                auto_committed: true,
                archive_uri: Some(commit_result.archive_uri),
                task_id: commit_result.task_id,
            })
        } else {
            Ok(AddMessageResult {
                auto_committed: false,
                archive_uri: None,
                task_id: None,
            })
        }
    }

    pub async fn used_context(&self, session_id: &str, uri: &str) -> Result<(), SessionError> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| SessionError::NotFound(session_id.to_owned()))?;
        session.usage.push(UsageRecord {
            kind: "context".to_owned(),
            uri: uri.to_owned(),
            success: None,
        });
        Ok(())
    }

    pub async fn used_skill(
        &self,
        session_id: &str,
        skill_uri: &str,
        success: bool,
    ) -> Result<(), SessionError> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| SessionError::NotFound(session_id.to_owned()))?;
        session.usage.push(UsageRecord {
            kind: "skill".to_owned(),
            uri: skill_uri.to_owned(),
            success: Some(success),
        });
        Ok(())
    }

    pub async fn used_tool(
        &self,
        session_id: &str,
        tool_uri: &str,
        success: bool,
    ) -> Result<(), SessionError> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| SessionError::NotFound(session_id.to_owned()))?;
        session.usage.push(UsageRecord {
            kind: "tool".to_owned(),
            uri: tool_uri.to_owned(),
            success: Some(success),
        });
        Ok(())
    }

    pub async fn task_status(&self, task_id: &str) -> Option<TaskRecord> {
        if let Some(task) = self.tasks.lock().await.get(task_id).cloned() {
            return Some(task);
        }
        load_persisted_task_record(&self.workspace_root, task_id)
            .await
            .ok()
            .flatten()
    }

    pub async fn list_tasks(&self, limit: usize) -> Result<Vec<TaskRecord>, SessionError> {
        let mut tasks = {
            let tasks = self.tasks.lock().await;
            tasks.clone()
        };

        for task in load_persisted_task_records(&self.workspace_root).await? {
            tasks.entry(task.task_id.clone()).or_insert(task);
        }

        let mut records = tasks.into_values().collect::<Vec<_>>();
        records.sort_by(|left, right| right.task_id.cmp(&left.task_id));
        if records.len() > limit {
            records.truncate(limit);
        }
        Ok(records)
    }

    pub async fn list_sessions(
        &self,
        account_id: &str,
        user_id: &str,
        agent_id: &str,
    ) -> Result<Vec<SessionSummary>, SessionError> {
        validate_identity_segment(account_id, "account_id")?;
        validate_identity_segment(user_id, "user_id")?;
        validate_identity_segment(agent_id, "agent_id")?;
        let pending_sessions = {
            let sessions = self.sessions.lock().await;
            sessions
                .iter()
                .filter(|(_, session)| {
                    session.account_id == account_id
                        && session.user_id == user_id
                        && session.agent_id == agent_id
                })
                .map(|(session_id, session)| (session_id.clone(), session.messages.len()))
                .collect::<Vec<_>>()
        };

        query::list_sessions(
            &self.workspace_root,
            account_id,
            user_id,
            agent_id,
            &pending_sessions,
        )
        .await
    }

    pub async fn get_session(&self, session_id: &str) -> Result<SessionSummary, SessionError> {
        let (account_id, user_id, agent_id, session_root, pending_messages, _) =
            self.resolve_session(session_id).await?;
        query::load_session_summary(
            &session_root,
            &account_id,
            &user_id,
            &agent_id,
            session_id,
            pending_messages,
        )
        .await
    }

    pub async fn get_session_context(
        &self,
        session_id: &str,
        token_budget: usize,
    ) -> Result<SessionContextView, SessionError> {
        let (_, _, _, session_root, _, pending_messages) = self.resolve_session(session_id).await?;
        query::load_session_context(&session_root, &pending_messages, token_budget).await
    }

    pub async fn get_session_archive(
        &self,
        session_id: &str,
        archive_id: &str,
    ) -> Result<SessionArchiveView, SessionError> {
        let (_, _, _, session_root, _, _) = self.resolve_session(session_id).await?;
        query::load_session_archive(&session_root, archive_id).await
    }

    /// List all completed archive IDs for a session, sorted chronologically.
    pub async fn list_session_archives(
        &self,
        session_id: &str,
    ) -> Result<Vec<String>, SessionError> {
        let (_, _, _, session_root, _, _) = self.resolve_session(session_id).await?;
        let history_root = session_root.join("history");
        query::completed_archive_ids(&history_root).await
    }

    /// Write a structured observation to a session as a message.
    /// Observations are stored as JSON-formatted messages with role "observation".
    pub async fn add_observation(
        &self,
        session_id: &str,
        tool_name: &str,
        tool_input: &str,
        tool_output: &str,
        content: &str,
        platform: &str,
        source_trust: Option<&str>,
        metadata_json: Option<&str>,
    ) -> Result<AddMessageResult, SessionError> {
        let mut observation = serde_json::json!({
            "tool_name": tool_name,
            "tool_input": tool_input,
            "tool_output": tool_output,
            "content": content,
            "platform": platform,
        });
        if let Some(trust) = source_trust {
            observation["source_trust"] = serde_json::Value::String(trust.to_owned());
        }
        if let Some(meta) = metadata_json {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(meta) {
                observation["metadata"] = parsed;
            }
        }
        self.add_message(session_id, "observation", &observation.to_string())
            .await
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<(), SessionError> {
        let (account_id, user_id, agent_id, session_root, _, _) =
            self.resolve_session(session_id).await?;
        let _path_lock = lock::acquire_path_lock(
            &session_root.join(".session.commit.lock"),
            Duration::from_secs(5),
        )
        .await?;

        {
            let mut sessions = self.sessions.lock().await;
            sessions.remove(session_id);
        }
        prune_session_tasks(&self.workspace_root, &self.tasks, &agent_id, session_id).await?;

        if tokio::fs::try_exists(&session_root)
            .await
            .map_err(|source| SessionError::io("check session root", &session_root, source))?
        {
            tokio::fs::remove_dir_all(&session_root)
                .await
                .map_err(|source| SessionError::io("remove session root", &session_root, source))?;
        }

        let agent_root = self
            .workspace_root
            .join("tenants")
            .join(&account_id)
            .join(&user_id)
            .join("session")
            .join(&agent_id);
        remove_empty_directories_upward(&agent_root, &session_root).await?;
        Ok(())
    }

    pub async fn recover_pending_redo(&self) -> Result<usize, SessionError> {
        let markers = redo::load_redo_markers(&self.workspace_root).await?;
        let mut recovered = 0;

        for marker in markers {
            let task_id = marker.task_id.clone();
            self.set_task_running(&task_id).await;
            let job = marker_to_job(&self.workspace_root, &marker);
            match memory::run_background_memory_pipeline(job).await {
                Ok(result) => {
                    self.complete_task(&task_id, result).await;
                    recovered += 1;
                }
                Err(error) => {
                    self.fail_task(&task_id, error.to_string()).await;
                }
            }
        }

        Ok(recovered)
    }

    pub async fn commit(&self, session_id: &str) -> Result<CommitResult, SessionError> {
        let (account_id, user_id, agent_id) = {
            let sessions = self.sessions.lock().await;
            let session = sessions
                .get(session_id)
                .ok_or_else(|| SessionError::NotFound(session_id.to_owned()))?;
            (
                session.account_id.clone(),
                session.user_id.clone(),
                session.agent_id.clone(),
            )
        };
        let session_root = session_root_path(
            &self.workspace_root,
            &account_id,
            &user_id,
            &agent_id,
            session_id,
        );
        let _path_lock = lock::acquire_path_lock(
            &session_root.join(".session.commit.lock"),
            Duration::from_secs(5),
        )
        .await?;

        let (messages, usage, archive_index) = {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .get_mut(session_id)
                .ok_or_else(|| SessionError::NotFound(session_id.to_owned()))?;
            if session.messages.is_empty() && session.usage.is_empty() {
                return Ok(CommitResult {
                    archive_uri: String::new(),
                    task_id: None,
                });
            }
            let archive_index = next_archive_index(&session_root).await?;
            session.archive_count = archive_index;
            session.estimated_tokens = 0;
            (
                std::mem::take(&mut session.messages),
                std::mem::take(&mut session.usage),
                archive_index,
            )
        };

        let (archive_uri, archive_path) = archive::archive_messages(
            &self.workspace_root,
            &account_id,
            &user_id,
            &agent_id,
            session_id,
            archive_index,
            &messages,
        )
        .await?;
        clear_live_messages(
            &self.workspace_root,
            &account_id,
            &user_id,
            &agent_id,
            session_id,
        )
        .await?;
        let _ = write_usage_snapshot(&archive_path, &usage).await?;

        let task_id = Uuid::new_v4().to_string();
        let redo_marker = RedoMarker {
            task_id: task_id.clone(),
            archive_uri: archive_uri.clone(),
            archive_path: archive_path.to_string_lossy().into_owned(),
            account_id: account_id.clone(),
            user_id: user_id.clone(),
            agent_id: agent_id.clone(),
            session_id: session_id.to_owned(),
        };
        let redo_marker_path = redo::write_redo_marker(&self.workspace_root, &redo_marker).await?;
        self.tasks.lock().await.insert(
            task_id.clone(),
            TaskRecord {
                task_id: task_id.clone(),
                archive_uri: archive_uri.clone(),
                status: TaskStatus::Pending,
                retry_state: Some("queued".to_owned()),
                processing_mode: None,
                used_contexts: usage
                    .iter()
                    .filter(|record| record.kind == "context")
                    .count(),
                used_skills: usage.iter().filter(|record| record.kind == "skill").count(),
                used_tools: usage.iter().filter(|record| record.kind == "tool").count(),
                memories_extracted: HashMap::new(),
                artifacts_written: HashMap::new(),
                error: None,
            },
        );
        if let Some(task) = self.tasks.lock().await.get(&task_id).cloned() {
            persist_task_record(&self.workspace_root, &task).await?;
        }

        let tasks = Arc::clone(&self.tasks);
        let workspace_root = self.workspace_root.clone();
        let job = MemoryPipelineJob {
            task_id: task_id.clone(),
            workspace_root: self.workspace_root.clone(),
            account_id: account_id.clone(),
            user_id: user_id.clone(),
            agent_id: agent_id.clone(),
            session_id: session_id.to_owned(),
            archive_uri: archive_uri.clone(),
            archive_path,
            redo_marker_path,
        };

        let spawned_task_id = task_id.clone();
        let handle = tokio::spawn(async move {
            set_task_status(
                &workspace_root,
                &tasks,
                &spawned_task_id,
                TaskStatus::Running,
                None,
                None,
            )
            .await;
            match memory::run_background_memory_pipeline(job).await {
                Ok(result) => {
                    set_task_status(
                        &workspace_root,
                        &tasks,
                        &spawned_task_id,
                        TaskStatus::Completed,
                        Some(result),
                        None,
                    )
                    .await;
                }
                Err(error) => {
                    set_task_status(
                        &workspace_root,
                        &tasks,
                        &spawned_task_id,
                        TaskStatus::Failed,
                        None,
                        Some(error.to_string()),
                    )
                    .await;
                }
            }
        });
        self.background_tasks.lock().await.push(handle);

        Ok(CommitResult {
            archive_uri,
            task_id: Some(task_id),
        })
    }

    pub async fn drain_background_tasks(&self, timeout: Duration) -> Result<usize, SessionError> {
        let mut handles = {
            let mut guard = self.background_tasks.lock().await;
            guard.drain(..).collect::<Vec<_>>()
        };
        let deadline = tokio::time::Instant::now() + timeout;
        let mut drained = 0;

        for handle in &mut handles {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                handle.abort();
                continue;
            }
            match tokio::time::timeout(remaining, &mut *handle).await {
                Ok(Ok(())) => drained += 1,
                Ok(Err(error)) => {
                    return Err(SessionError::IoRaw(std::io::Error::other(
                        error.to_string(),
                    )));
                }
                Err(_) => handle.abort(),
            }
        }

        Ok(drained)
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    async fn set_task_running(&self, task_id: &str) {
        let mut tasks = self.tasks.lock().await;
        let task = tasks
            .entry(task_id.to_owned())
            .or_insert_with(|| TaskRecord {
                task_id: task_id.to_owned(),
                archive_uri: String::new(),
                status: TaskStatus::Pending,
                retry_state: Some("queued".to_owned()),
                processing_mode: None,
                used_contexts: 0,
                used_skills: 0,
                used_tools: 0,
                memories_extracted: HashMap::new(),
                artifacts_written: HashMap::new(),
                error: None,
            });
        task.status = TaskStatus::Running;
        task.retry_state = Some("not_needed".to_owned());
        let task_snapshot = task.clone();
        drop(tasks);
        let _ = persist_task_record(&self.workspace_root, &task_snapshot).await;
    }

    async fn complete_task(&self, task_id: &str, result: MemoryPipelineResult) {
        set_task_status(
            &self.workspace_root,
            &self.tasks,
            task_id,
            TaskStatus::Completed,
            Some(result),
            None,
        )
        .await;
    }

    async fn fail_task(&self, task_id: &str, error: String) {
        set_task_status(
            &self.workspace_root,
            &self.tasks,
            task_id,
            TaskStatus::Failed,
            None,
            Some(error),
        )
        .await;
    }

    async fn resolve_session(
        &self,
        session_id: &str,
    ) -> Result<(String, String, String, PathBuf, usize, Vec<StoredMessage>), SessionError> {
        let live_session = {
            let sessions = self.sessions.lock().await;
            sessions.get(session_id).map(|session| {
                (
                    session.account_id.clone(),
                    session.user_id.clone(),
                    session.agent_id.clone(),
                    session.messages.clone(),
                )
            })
        };

        if let Some((account_id, user_id, agent_id, pending_messages)) = live_session {
            let session_root = session_root_path(
                &self.workspace_root,
                &account_id,
                &user_id,
                &agent_id,
                session_id,
            );
            let pending_count = pending_messages.len();
            return Ok((
                account_id,
                user_id,
                agent_id,
                session_root,
                pending_count,
                pending_messages,
            ));
        }

        let tenants_root = self.workspace_root.join("tenants");
        if !tokio::fs::try_exists(&tenants_root)
            .await
            .map_err(|source| SessionError::io("check tenants root", &tenants_root, source))?
        {
            return Err(SessionError::NotFound(session_id.to_owned()));
        }

        let mut accounts = tokio::fs::read_dir(&tenants_root)
            .await
            .map_err(|source| SessionError::io("read tenants root", &tenants_root, source))?;
        while let Some(account_entry) = accounts
            .next_entry()
            .await
            .map_err(|source| SessionError::io("iterate tenants root", &tenants_root, source))?
        {
            let account_id = account_entry.file_name().to_string_lossy().into_owned();
            let users_root = account_entry.path();
            if !account_entry
                .file_type()
                .await
                .map_err(|source| SessionError::io("inspect tenant entry", &users_root, source))?
                .is_dir()
            {
                continue;
            }

            let mut users = tokio::fs::read_dir(&users_root)
                .await
                .map_err(|source| SessionError::io("read user root", &users_root, source))?;
            while let Some(user_entry) = users
                .next_entry()
                .await
                .map_err(|source| SessionError::io("iterate user root", &users_root, source))?
            {
                let user_id = user_entry.file_name().to_string_lossy().into_owned();
                let session_agents_root = user_entry.path().join("session");
                if !tokio::fs::try_exists(&session_agents_root)
                    .await
                    .map_err(|source| {
                        SessionError::io("check session agent root", &session_agents_root, source)
                    })?
                {
                    continue;
                }

                let mut agents =
                    tokio::fs::read_dir(&session_agents_root)
                        .await
                        .map_err(|source| {
                            SessionError::io(
                                "read session agent root",
                                &session_agents_root,
                                source,
                            )
                        })?;
                while let Some(agent_entry) = agents.next_entry().await.map_err(|source| {
                    SessionError::io("iterate session agent root", &session_agents_root, source)
                })? {
                    let agent_id = agent_entry.file_name().to_string_lossy().into_owned();
                    let session_root = agent_entry.path().join(session_id);
                    if tokio::fs::try_exists(&session_root)
                        .await
                        .map_err(|source| {
                            SessionError::io("check session root", &session_root, source)
                        })?
                    {
                        let pending_messages = query::read_live_messages(&session_root).await?;
                        let pending_count = pending_messages.len();
                        return Ok((
                            account_id,
                            user_id,
                            agent_id,
                            session_root,
                            pending_count,
                            pending_messages,
                        ));
                    }
                }
            }
        }

        Err(SessionError::NotFound(session_id.to_owned()))
    }
}

pub struct CommitResult {
    pub archive_uri: String,
    pub task_id: Option<String>,
}

#[derive(Debug)]
pub enum SessionError {
    NotFound(String),
    InvalidArgument {
        field: String,
        reason: String,
    },
    LockTimeout(PathBuf),
    Serde(serde_json::Error),
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    IoRaw(io::Error),
}

impl SessionError {
    fn io(action: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.to_path_buf(),
            source,
        }
    }
}

impl Display for SessionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(session_id) => write!(f, "session not found: {session_id}"),
            Self::InvalidArgument { field, reason } => {
                write!(f, "invalid argument '{field}': {reason}")
            }
            Self::LockTimeout(path) => {
                write!(f, "timed out waiting for path lock '{}'", path.display())
            }
            Self::Serde(source) => write!(f, "serialization error: {source}"),
            Self::Io {
                action,
                path,
                source,
            } => write!(f, "failed to {action} '{}': {source}", path.display()),
            Self::IoRaw(source) => write!(f, "io error: {source}"),
        }
    }
}

impl Error for SessionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::NotFound(_) | Self::InvalidArgument { .. } => None,
            Self::LockTimeout(_) => None,
            Self::Serde(source) => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::IoRaw(source) => Some(source),
        }
    }
}

fn marker_to_job(workspace_root: &Path, marker: &RedoMarker) -> MemoryPipelineJob {
    MemoryPipelineJob {
        task_id: marker.task_id.clone(),
        workspace_root: workspace_root.to_path_buf(),
        account_id: marker.account_id.clone(),
        user_id: marker.user_id.clone(),
        agent_id: marker.agent_id.clone(),
        session_id: marker.session_id.clone(),
        archive_uri: marker.archive_uri.clone(),
        archive_path: PathBuf::from(&marker.archive_path),
        redo_marker_path: workspace_root
            .join("_system")
            .join("redo")
            .join(format!("{}.json", marker.task_id)),
    }
}

fn session_root_path(
    workspace_root: &Path,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
    session_id: &str,
) -> PathBuf {
    workspace_root
        .join("tenants")
        .join(account_id)
        .join(user_id)
        .join("session")
        .join(agent_id)
        .join(session_id)
}

async fn next_archive_index(session_root: &Path) -> Result<u32, SessionError> {
    let history_root = session_root.join("history");
    tokio::fs::create_dir_all(&history_root)
        .await
        .map_err(|source| SessionError::io("create history directory", &history_root, source))?;
    let mut entries = tokio::fs::read_dir(&history_root)
        .await
        .map_err(|source| SessionError::io("read history directory", &history_root, source))?;
    let mut max_index = 0;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|source| SessionError::io("iterate history directory", &history_root, source))?
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(index) = name
            .strip_prefix("archive_")
            .and_then(|value| value.parse::<u32>().ok())
        {
            max_index = max_index.max(index);
        }
    }

    Ok(max_index + 1)
}

async fn set_task_status(
    workspace_root: &Path,
    tasks: &Arc<Mutex<HashMap<String, TaskRecord>>>,
    task_id: &str,
    status: TaskStatus,
    result: Option<MemoryPipelineResult>,
    error: Option<String>,
) {
    let mut tasks = tasks.lock().await;
    let entry = tasks
        .entry(task_id.to_owned())
        .or_insert_with(|| TaskRecord {
            task_id: task_id.to_owned(),
            archive_uri: String::new(),
            status: TaskStatus::Pending,
            retry_state: Some("queued".to_owned()),
            processing_mode: None,
            used_contexts: 0,
            used_skills: 0,
            used_tools: 0,
            memories_extracted: HashMap::new(),
            artifacts_written: HashMap::new(),
            error: None,
        });
    entry.status = status;
    if let Some(result) = result {
        entry.memories_extracted = result.memories_extracted;
        entry.artifacts_written = result.artifacts_written;
        entry.processing_mode = Some(result.processing_mode);
        entry.retry_state = Some("not_needed".to_owned());
        entry.error = None;
    }
    if let Some(error) = error {
        entry.error = Some(error);
        entry.retry_state = Some("exhausted".to_owned());
    }
    let task_snapshot = entry.clone();
    drop(tasks);
    let _ = persist_task_record(workspace_root, &task_snapshot).await;
}

async fn persist_live_messages(
    workspace_root: &Path,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
    session_id: &str,
    messages: &[StoredMessage],
) -> Result<(), SessionError> {
    let session_root = session_root_path(workspace_root, account_id, user_id, agent_id, session_id);
    tokio::fs::create_dir_all(&session_root)
        .await
        .map_err(|source| SessionError::io("create session root", &session_root, source))?;
    let messages_path = session_root.join("messages.jsonl");
    let body = messages
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .map_err(SessionError::Serde)?
        .join("\n");
    tokio::fs::write(&messages_path, format!("{body}\n"))
        .await
        .map_err(|source| SessionError::io("write live messages", &messages_path, source))?;
    Ok(())
}

async fn clear_live_messages(
    workspace_root: &Path,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
    session_id: &str,
) -> Result<(), SessionError> {
    let messages_path =
        session_root_path(workspace_root, account_id, user_id, agent_id, session_id)
            .join("messages.jsonl");
    if tokio::fs::try_exists(&messages_path)
        .await
        .map_err(|source| SessionError::io("check live messages", &messages_path, source))?
    {
        tokio::fs::remove_file(&messages_path)
            .await
            .map_err(|source| SessionError::io("remove live messages", &messages_path, source))?;
    }
    Ok(())
}

fn persisted_task_record_path(workspace_root: &Path, task_id: &str) -> PathBuf {
    workspace_root
        .join("_system")
        .join("session_tasks")
        .join(format!("{task_id}.json"))
}

async fn persist_task_record(workspace_root: &Path, task: &TaskRecord) -> Result<(), SessionError> {
    let path = persisted_task_record_path(workspace_root, &task.task_id);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|source| SessionError::io("create session task directory", parent, source))?;
    }
    tokio::fs::write(
        &path,
        serde_json::to_vec_pretty(task).map_err(SessionError::Serde)?,
    )
    .await
    .map_err(|source| SessionError::io("write session task record", &path, source))?;
    Ok(())
}

async fn load_persisted_task_record(
    workspace_root: &Path,
    task_id: &str,
) -> Result<Option<TaskRecord>, SessionError> {
    let path = persisted_task_record_path(workspace_root, task_id);
    if !tokio::fs::try_exists(&path)
        .await
        .map_err(|source| SessionError::io("check session task record", &path, source))?
    {
        return Ok(None);
    }
    let content = tokio::fs::read(&path)
        .await
        .map_err(|source| SessionError::io("read session task record", &path, source))?;
    Ok(Some(
        serde_json::from_slice(&content).map_err(SessionError::Serde)?,
    ))
}

async fn load_persisted_task_records(
    workspace_root: &Path,
) -> Result<Vec<TaskRecord>, SessionError> {
    let directory = workspace_root.join("_system").join("session_tasks");
    if !tokio::fs::try_exists(&directory)
        .await
        .map_err(|source| SessionError::io("check session task directory", &directory, source))?
    {
        return Ok(Vec::new());
    }

    let mut tasks = Vec::new();
    let mut entries = tokio::fs::read_dir(&directory)
        .await
        .map_err(|source| SessionError::io("read session task directory", &directory, source))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|source| SessionError::io("iterate session task directory", &directory, source))?
    {
        let path = entry.path();
        if entry
            .file_type()
            .await
            .map_err(|source| SessionError::io("inspect session task entry", &path, source))?
            .is_dir()
        {
            continue;
        }
        let content = tokio::fs::read(&path)
            .await
            .map_err(|source| SessionError::io("read session task record", &path, source))?;
        tasks.push(serde_json::from_slice(&content).map_err(SessionError::Serde)?);
    }

    Ok(tasks)
}

async fn prune_session_tasks(
    workspace_root: &Path,
    tasks: &Arc<Mutex<HashMap<String, TaskRecord>>>,
    agent_id: &str,
    session_id: &str,
) -> Result<(), SessionError> {
    let needle = format!("/{agent_id}/{session_id}/history/");
    {
        let mut in_memory = tasks.lock().await;
        in_memory.retain(|_, task| !task.archive_uri.contains(&needle));
    }

    for task in load_persisted_task_records(workspace_root).await? {
        if task.archive_uri.contains(&needle) {
            let path = persisted_task_record_path(workspace_root, &task.task_id);
            if tokio::fs::try_exists(&path)
                .await
                .map_err(|source| SessionError::io("check session task record", &path, source))?
            {
                tokio::fs::remove_file(&path).await.map_err(|source| {
                    SessionError::io("remove session task record", &path, source)
                })?;
            }
        }
    }

    Ok(())
}

async fn remove_empty_directories_upward(
    agent_root: &Path,
    leaf: &Path,
) -> Result<(), SessionError> {
    let mut current = leaf.parent().map(Path::to_path_buf);
    while let Some(path) = current {
        if path == agent_root {
            break;
        }
        match tokio::fs::remove_dir(&path).await {
            Ok(()) => {
                current = path.parent().map(Path::to_path_buf);
            }
            Err(source) if source.kind() == io::ErrorKind::DirectoryNotEmpty => break,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                current = path.parent().map(Path::to_path_buf);
            }
            Err(source) => {
                return Err(SessionError::io(
                    "remove empty session directory",
                    &path,
                    source,
                ));
            }
        }
    }
    Ok(())
}
