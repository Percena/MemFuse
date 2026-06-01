use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::SessionError;
use mfs_index::SqliteSemanticIndex;
use mfs_memory::{
    ArchiveMemoryCommitInput, LlmAssist, MemoryCandidate, MemoryCategory, UsageRecord,
    build_agent_memory_content, build_agent_skill_record,
    build_append_only_category_memory_content, build_archive_abstract, build_archive_overview,
    build_fact_backed_memory_content, build_mergeable_category_memory_content,
    build_profile_memory_content, build_user_memory_content, entity_slug_from_fact,
    extract_memory_candidates, is_entity_fact, is_preference_fact, is_profile_fact,
    run_archive_memory_commit, sanitize_memory_slug, write_memory_file,
};
use mfs_metadata::{
    AuditEventRecord, MetadataStore, PathEntryRecord, RelationRecord, SnapshotRecord,
};
use mfs_semantic::{ProcessingMode, SemanticPipeline, SemanticPipelineConfig};
use mfs_uri::short_hash_hex;
use mfs_workspace::write_layered_summaries;
use tokio::fs;

/// Adapter: wraps `write_memory_file` (which now returns `io::Error`) into
/// `Result<(), SessionError>` for use in the session-domain pipeline.
async fn session_write_memory_file(path: &Path, content: &str) -> Result<(), SessionError> {
    write_memory_file(path, content)
        .await
        .map_err(|source| SessionError::io("write memory file", path, source))
}

/// Adapter: wraps `build_mergeable_category_memory_content` (which now returns
/// `Result<String, String>`) into `Result<String, SessionError>`.
async fn session_build_mergeable(
    path: &Path,
    category: MemoryCategory,
    title: &str,
    candidates: &[MemoryCandidate],
) -> Result<String, SessionError> {
    build_mergeable_category_memory_content(path, category, title, candidates)
        .await
        .map_err(|e| SessionError::IoRaw(std::io::Error::other(e)))
}

/// Adapter: wraps `build_profile_memory_content` (which now returns
/// `Result<String, String>`) into `Result<String, SessionError>`.
async fn session_build_profile(
    path: &Path,
    candidates: &[MemoryCandidate],
) -> Result<String, SessionError> {
    build_profile_memory_content(path, candidates)
        .await
        .map_err(|e| SessionError::IoRaw(std::io::Error::other(e)))
}

#[derive(Debug, Clone)]
pub struct MemoryPipelineJob {
    pub task_id: String,
    pub workspace_root: PathBuf,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: String,
    pub session_id: String,
    pub archive_uri: String,
    pub archive_path: PathBuf,
    pub redo_marker_path: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryPipelineResult {
    pub memories_extracted: HashMap<String, usize>,
    pub artifacts_written: HashMap<String, usize>,
    pub processing_mode: String,
}

pub async fn write_usage_snapshot(
    archive_path: &Path,
    usage: &[UsageRecord],
) -> Result<PathBuf, SessionError> {
    let usage_path = archive_path.join("usage.json");
    fs::write(
        &usage_path,
        serde_json::to_vec_pretty(usage).map_err(SessionError::Serde)?,
    )
    .await
    .map_err(|source| SessionError::io("write archive usage", &usage_path, source))?;
    Ok(usage_path)
}

pub async fn run_background_memory_pipeline(
    job: MemoryPipelineJob,
) -> Result<MemoryPipelineResult, SessionError> {
    let abstract_path = job.archive_path.join(".abstract.md");
    let overview_path = job.archive_path.join(".overview.md");
    let done_path = job.archive_path.join(".done");
    let messages = read_messages(&job.archive_path.join("messages.jsonl")).await?;
    let usage = read_usage_snapshot(&job.archive_path.join("usage.json")).await?;
    let abstract_text = build_archive_abstract(&job.archive_uri, &messages, &usage);
    let overview_text = build_archive_overview(&job.archive_uri, &messages, &usage);
    let metadata_path = job.workspace_root.join("_system").join("metadata.sqlite");
    let metadata_path_for_blocking = metadata_path.clone();
    let metadata = tokio::task::spawn_blocking(move || {
        MetadataStore::open_at(&metadata_path_for_blocking, false)
    })
    .await
    .map_err(|e| {
        SessionError::io(
            "spawn_blocking for metadata store open",
            &metadata_path,
            std::io::Error::other(e.to_string()),
        )
    })?
    .map_err(|source| {
        SessionError::io(
            "open metadata store",
            &metadata_path,
            std::io::Error::other(source.to_string()),
        )
    })?;

    fs::write(&abstract_path, abstract_text)
        .await
        .map_err(|source| SessionError::io("write archive abstract", &abstract_path, source))?;
    fs::write(&overview_path, overview_text)
        .await
        .map_err(|source| SessionError::io("write archive overview", &overview_path, source))?;

    let mut memories_extracted = HashMap::new();
    let mut artifacts_written = HashMap::new();
    let mut processing_mode = ProcessingMode::Full;
    let usage_records = usage
        .iter()
        .map(|record| (record.kind.clone(), record.uri.clone(), record.success))
        .collect::<Vec<_>>();
    let memory_candidates = extract_memory_candidates(&messages, &usage_records).await;
    if !messages.is_empty() || usage.iter().any(|record| record.kind == "context") {
        let user_memory_path = job
            .workspace_root
            .join("tenants")
            .join(&job.account_id)
            .join(&job.user_id)
            .join("user")
            .join("memories")
            .join("session")
            .join(&job.agent_id)
            .join(&job.session_id)
            .join(archive_file_name(&job.archive_path));
        session_write_memory_file(
            &user_memory_path,
            &build_user_memory_content(&job.archive_uri, &messages, &usage),
        )
        .await?;
        artifacts_written.insert("user_session".to_owned(), 1);
        let mode = semantic_index_root(
            &job.workspace_root,
            &format!("tenant:{}:{}:user", job.account_id, job.user_id),
            user_memory_path.parent().expect("user memory parent"),
            &format!(
                "mfs://user/memories/session/{}/{}",
                job.agent_id, job.session_id
            ),
            "memory",
        )
        .await?;
        if mode == ProcessingMode::Degraded {
            processing_mode = ProcessingMode::Degraded;
        }
        record_derived_projection(
            &metadata,
            &job,
            &format!("tenant:{}:{}:user", job.account_id, job.user_id),
            user_memory_path.parent().expect("user memory parent"),
            &format!(
                "mfs://user/memories/session/{}/{}",
                job.agent_id, job.session_id
            ),
            "memory.writeback",
        )?;
    }

    let metadata_path = job.workspace_root.join("_system").join("metadata.sqlite");
    let metadata_path_for_blocking = metadata_path.clone();
    let metadata = tokio::task::spawn_blocking(move || {
        MetadataStore::open_at(&metadata_path_for_blocking, false)
    })
    .await
    .map_err(|e| {
        SessionError::io(
            "spawn_blocking for consolidation metadata open",
            &metadata_path,
            std::io::Error::other(e.to_string()),
        )
    })?
    .map_err(|e| {
        SessionError::io(
            "open metadata store",
            &metadata_path,
            std::io::Error::other(e),
        )
    })?;
    let llm = LlmAssist::from_env();
    let archive_name = archive_segment_name(&job.archive_path);
    let commit_output = run_archive_memory_commit(
        &metadata,
        &ArchiveMemoryCommitInput {
            account_id: &job.account_id,
            user_id: &job.user_id,
            agent_id: &job.agent_id,
            session_id: &job.session_id,
            archive_name: &archive_name,
            messages: &messages,
        },
        &llm,
    )
    .await
    .map_err(|source| {
        SessionError::io(
            "persist mfs-memory metadata pipeline",
            &job.archive_path.join("messages.jsonl"),
            std::io::Error::other(source),
        )
    })?;
    let consolidation_result = commit_output.consolidation_result;
    let t2h_result = commit_output.t2h_result;
    if consolidation_result.episode_count > 0 {
        artifacts_written.insert(
            "memory_metadata".to_owned(),
            consolidation_result.episode_count,
        );
    }
    if consolidation_result.fact_count > 0 {
        memories_extracted.insert("facts".to_owned(), consolidation_result.fact_count);
    }
    if consolidation_result.assertion_count > 0 {
        artifacts_written.insert(
            "fact_assertions".to_owned(),
            consolidation_result.assertion_count,
        );
    }

    if t2h_result.instances_created > 0 {
        memories_extracted.insert(
            "heuristic_instances".to_owned(),
            t2h_result.instances_created,
        );
    }
    if t2h_result.rules_distilled > 0 || t2h_result.rules_promoted > 0 {
        artifacts_written.insert(
            "heuristic_rules".to_owned(),
            t2h_result.rules_distilled + t2h_result.rules_promoted,
        );
    }
    tracing::info!(
        signals = t2h_result.signals_detected,
        instances = t2h_result.instances_created,
        distilled = t2h_result.rules_distilled,
        promoted = t2h_result.rules_promoted,
        evidence = t2h_result.evidence_added,
        "T2H pipeline step completed"
    );

    let active_facts = metadata
        .get_active_facts(&job.account_id, &job.user_id)
        .map_err(|source| {
            SessionError::io(
                "read active facts",
                &job.archive_path.join("messages.jsonl"),
                std::io::Error::other(source.to_string()),
            )
        })?;

    let profile_candidates = memory_candidates
        .iter()
        .filter(|candidate| candidate.category == MemoryCategory::Profile)
        .cloned()
        .collect::<Vec<_>>();
    let profile_facts = active_facts
        .iter()
        .filter(|fact| is_profile_fact(fact))
        .cloned()
        .collect::<Vec<_>>();
    if !profile_facts.is_empty() || !profile_candidates.is_empty() {
        let user_memories_root = job
            .workspace_root
            .join("tenants")
            .join(&job.account_id)
            .join(&job.user_id)
            .join("user")
            .join("memories");
        let profile_memory_path = user_memories_root.join("profile.md");
        let profile_content = if profile_facts.is_empty() {
            session_build_profile(&profile_memory_path, &profile_candidates).await?
        } else {
            build_fact_backed_memory_content(
                "Profile",
                &profile_facts,
                profile_candidates
                    .iter()
                    .map(|candidate| candidate.content.clone())
                    .collect(),
            )
        };
        session_write_memory_file(&profile_memory_path, &profile_content).await?;
        memories_extracted.insert("profile".to_owned(), 1);
        let mode = semantic_index_root(
            &job.workspace_root,
            &format!("tenant:{}:{}:user", job.account_id, job.user_id),
            &user_memories_root,
            "mfs://user/memories",
            "memory",
        )
        .await?;
        if mode == ProcessingMode::Degraded {
            processing_mode = ProcessingMode::Degraded;
        }
        record_derived_projection(
            &metadata,
            &job,
            &format!("tenant:{}:{}:user", job.account_id, job.user_id),
            &user_memories_root,
            "mfs://user/memories",
            "memory.writeback",
        )?;
    }

    let preference_candidates = memory_candidates
        .iter()
        .filter(|candidate| candidate.category == MemoryCategory::Preferences)
        .cloned()
        .collect::<Vec<_>>();
    let preference_facts = active_facts
        .iter()
        .filter(|fact| is_preference_fact(fact))
        .cloned()
        .collect::<Vec<_>>();
    if !preference_facts.is_empty() || !preference_candidates.is_empty() {
        let preferences_root = job
            .workspace_root
            .join("tenants")
            .join(&job.account_id)
            .join(&job.user_id)
            .join("user")
            .join("memories")
            .join("preferences");
        let preference_memory_path = preferences_root.join("general.md");
        let preference_content = if preference_facts.is_empty() {
            session_build_mergeable(
                &preference_memory_path,
                MemoryCategory::Preferences,
                "Preferences",
                &preference_candidates,
            )
            .await?
        } else {
            build_fact_backed_memory_content(
                "Preferences",
                &preference_facts,
                preference_candidates
                    .iter()
                    .map(|candidate| candidate.content.clone())
                    .collect(),
            )
        };
        session_write_memory_file(&preference_memory_path, &preference_content).await?;
        memories_extracted.insert("preferences".to_owned(), 1);
        let mode = semantic_index_root(
            &job.workspace_root,
            &format!("tenant:{}:{}:user", job.account_id, job.user_id),
            &preferences_root,
            "mfs://user/memories/preferences",
            "memory",
        )
        .await?;
        if mode == ProcessingMode::Degraded {
            processing_mode = ProcessingMode::Degraded;
        }
        record_derived_projection(
            &metadata,
            &job,
            &format!("tenant:{}:{}:user", job.account_id, job.user_id),
            &preferences_root,
            "mfs://user/memories/preferences",
            "memory.writeback",
        )?;
    }

    let event_candidates = memory_candidates
        .iter()
        .filter(|candidate| candidate.category == MemoryCategory::Events)
        .cloned()
        .collect::<Vec<_>>();
    if !event_candidates.is_empty() {
        let events_root = job
            .workspace_root
            .join("tenants")
            .join(&job.account_id)
            .join(&job.user_id)
            .join("user")
            .join("memories")
            .join("events");
        for candidate in &event_candidates {
            // Use title slug + short content hash to keep filename under 255 bytes.
            let title_slug = sanitize_memory_slug(&candidate.title);
            let content_hash = short_hash_hex(candidate.content.as_bytes(), 8);
            let event_path = events_root.join(format!("{title_slug}-{content_hash}.md"));
            session_write_memory_file(
                &event_path,
                &build_append_only_category_memory_content(
                    "Event Memory",
                    &job.archive_uri,
                    candidate,
                ),
            )
            .await?;
        }
        memories_extracted.insert("events".to_owned(), 1);
        let mode = semantic_index_root(
            &job.workspace_root,
            &format!("tenant:{}:{}:user", job.account_id, job.user_id),
            &events_root,
            "mfs://user/memories/events",
            "memory",
        )
        .await?;
        if mode == ProcessingMode::Degraded {
            processing_mode = ProcessingMode::Degraded;
        }
        record_derived_projection(
            &metadata,
            &job,
            &format!("tenant:{}:{}:user", job.account_id, job.user_id),
            &events_root,
            "mfs://user/memories/events",
            "memory.writeback",
        )?;
    }

    let entity_candidates = memory_candidates
        .iter()
        .filter(|candidate| candidate.category == MemoryCategory::Entities)
        .cloned()
        .collect::<Vec<_>>();
    let entity_facts = active_facts
        .iter()
        .filter(|fact| is_entity_fact(fact))
        .cloned()
        .collect::<Vec<_>>();
    if !entity_facts.is_empty() || !entity_candidates.is_empty() {
        let entities_root = job
            .workspace_root
            .join("tenants")
            .join(&job.account_id)
            .join(&job.user_id)
            .join("user")
            .join("memories")
            .join("entities");
        for fact in &entity_facts {
            let entity_slug = entity_slug_from_fact(fact);
            let entity_path = entities_root.join(format!("{entity_slug}.md"));
            session_write_memory_file(
                &entity_path,
                &build_fact_backed_memory_content(
                    "Entity Memory",
                    std::slice::from_ref(fact),
                    Vec::new(),
                ),
            )
            .await?;
        }
        for candidate in &entity_candidates {
            let entity_path =
                entities_root.join(format!("{}.md", sanitize_memory_slug(&candidate.title)));
            session_write_memory_file(
                &entity_path,
                &session_build_mergeable(
                    &entity_path,
                    MemoryCategory::Entities,
                    "Entity Memory",
                    std::slice::from_ref(candidate),
                )
                .await?,
            )
            .await?;
        }
        memories_extracted.insert("entities".to_owned(), 1);
        let mode = semantic_index_root(
            &job.workspace_root,
            &format!("tenant:{}:{}:user", job.account_id, job.user_id),
            &entities_root,
            "mfs://user/memories/entities",
            "memory",
        )
        .await?;
        if mode == ProcessingMode::Degraded {
            processing_mode = ProcessingMode::Degraded;
        }
        record_derived_projection(
            &metadata,
            &job,
            &format!("tenant:{}:{}:user", job.account_id, job.user_id),
            &entities_root,
            "mfs://user/memories/entities",
            "memory.writeback",
        )?;
    }

    let pattern_candidates = memory_candidates
        .iter()
        .filter(|candidate| candidate.category == MemoryCategory::Patterns)
        .cloned()
        .collect::<Vec<_>>();
    if !pattern_candidates.is_empty() {
        let agent_space_name = format!("{}__{}", job.user_id, job.agent_id);
        let patterns_root = job
            .workspace_root
            .join("tenants")
            .join(&job.account_id)
            .join(&job.user_id)
            .join("agent")
            .join(&agent_space_name)
            .join("memories")
            .join("patterns");
        for candidate in &pattern_candidates {
            let pattern_path =
                patterns_root.join(format!("{}.md", sanitize_memory_slug(&candidate.title)));
            session_write_memory_file(
                &pattern_path,
                &session_build_mergeable(
                    &pattern_path,
                    MemoryCategory::Patterns,
                    "Pattern Memory",
                    std::slice::from_ref(candidate),
                )
                .await?,
            )
            .await?;
        }
        memories_extracted.insert("patterns".to_owned(), 1);
        let mode = semantic_index_root(
            &job.workspace_root,
            &format!(
                "tenant:{}:{}:agent:{}__{}",
                job.account_id, job.user_id, job.user_id, job.agent_id
            ),
            &patterns_root,
            "mfs://agent/memories/patterns",
            "memory",
        )
        .await?;
        if mode == ProcessingMode::Degraded {
            processing_mode = ProcessingMode::Degraded;
        }
        record_derived_projection(
            &metadata,
            &job,
            &format!(
                "tenant:{}:{}:agent:{}__{}",
                job.account_id, job.user_id, job.user_id, job.agent_id
            ),
            &patterns_root,
            "mfs://agent/memories/patterns",
            "memory.writeback",
        )?;
    }

    let case_candidates = memory_candidates
        .iter()
        .filter(|candidate| candidate.category == MemoryCategory::Cases)
        .cloned()
        .collect::<Vec<_>>();
    if !case_candidates.is_empty() {
        let agent_space_name = format!("{}__{}", job.user_id, job.agent_id);
        let cases_root = job
            .workspace_root
            .join("tenants")
            .join(&job.account_id)
            .join(&job.user_id)
            .join("agent")
            .join(&agent_space_name)
            .join("memories")
            .join("cases");
        for candidate in &case_candidates {
            let title_slug = sanitize_memory_slug(&candidate.title);
            let content_hash = short_hash_hex(candidate.content.as_bytes(), 8);
            let case_path = cases_root.join(format!("{title_slug}-{content_hash}.md"));
            session_write_memory_file(
                &case_path,
                &build_append_only_category_memory_content(
                    "Case Memory",
                    &job.archive_uri,
                    candidate,
                ),
            )
            .await?;
        }
        memories_extracted.insert("cases".to_owned(), 1);
        let mode = semantic_index_root(
            &job.workspace_root,
            &format!(
                "tenant:{}:{}:agent:{}__{}",
                job.account_id, job.user_id, job.user_id, job.agent_id
            ),
            &cases_root,
            "mfs://agent/memories/cases",
            "memory",
        )
        .await?;
        if mode == ProcessingMode::Degraded {
            processing_mode = ProcessingMode::Degraded;
        }
        record_derived_projection(
            &metadata,
            &job,
            &format!(
                "tenant:{}:{}:agent:{}__{}",
                job.account_id, job.user_id, job.user_id, job.agent_id
            ),
            &cases_root,
            "mfs://agent/memories/cases",
            "memory.writeback",
        )?;
    }

    if usage.iter().any(|record| record.kind == "skill") {
        let agent_space_name = format!("{}__{}", job.user_id, job.agent_id);
        let agent_memory_path = job
            .workspace_root
            .join("tenants")
            .join(&job.account_id)
            .join(&job.user_id)
            .join("agent")
            .join(&agent_space_name)
            .join("memories")
            .join("skills")
            .join(&job.session_id)
            .join(archive_file_name(&job.archive_path));
        session_write_memory_file(
            &agent_memory_path,
            &build_agent_memory_content(&job.archive_uri, &usage),
        )
        .await?;
        memories_extracted.insert("skills".to_owned(), 1);
        artifacts_written.insert("agent_skill".to_owned(), 1);
        let mode = semantic_index_root(
            &job.workspace_root,
            &format!(
                "tenant:{}:{}:agent:{}__{}",
                job.account_id, job.user_id, job.user_id, job.agent_id
            ),
            agent_memory_path.parent().expect("agent memory parent"),
            &format!("mfs://agent/memories/skills/{}", job.session_id),
            "memory",
        )
        .await?;
        if mode == ProcessingMode::Degraded {
            processing_mode = ProcessingMode::Degraded;
        }
        record_derived_projection(
            &metadata,
            &job,
            &format!(
                "tenant:{}:{}:agent:{}__{}",
                job.account_id, job.user_id, job.user_id, job.agent_id
            ),
            agent_memory_path.parent().expect("agent memory parent"),
            &format!("mfs://agent/memories/skills/{}", job.session_id),
            "memory.writeback",
        )?;

        let skill_candidates = memory_candidates
            .iter()
            .filter(|candidate| candidate.category == MemoryCategory::Skills)
            .cloned()
            .collect::<Vec<_>>();
        if !skill_candidates.is_empty() {
            let stable_skill_root = job
                .workspace_root
                .join("tenants")
                .join(&job.account_id)
                .join(&job.user_id)
                .join("agent")
                .join(&agent_space_name)
                .join("memories")
                .join("skills");
            for candidate in &skill_candidates {
                let stable_skill_path = stable_skill_root
                    .join(format!("{}.md", sanitize_memory_slug(&candidate.title)));
                session_write_memory_file(
                    &stable_skill_path,
                    &session_build_mergeable(
                        &stable_skill_path,
                        MemoryCategory::Skills,
                        "Skill Memory",
                        std::slice::from_ref(candidate),
                    )
                    .await?,
                )
                .await?;
            }
            let mode = semantic_index_root(
                &job.workspace_root,
                &format!(
                    "tenant:{}:{}:agent:{}__{}",
                    job.account_id, job.user_id, job.user_id, job.agent_id
                ),
                &stable_skill_root,
                "mfs://agent/memories/skills",
                "memory",
            )
            .await?;
            if mode == ProcessingMode::Degraded {
                processing_mode = ProcessingMode::Degraded;
            }
            record_derived_projection(
                &metadata,
                &job,
                &format!(
                    "tenant:{}:{}:agent:{}__{}",
                    job.account_id, job.user_id, job.user_id, job.agent_id
                ),
                &stable_skill_root,
                "mfs://agent/memories/skills",
                "memory.writeback",
            )?;
        }

        let agent_skill_path = job
            .workspace_root
            .join("tenants")
            .join(&job.account_id)
            .join(&job.user_id)
            .join("agent")
            .join(&agent_space_name)
            .join("skills")
            .join("used")
            .join(&job.session_id)
            .join(archive_file_name(&job.archive_path));
        session_write_memory_file(
            &agent_skill_path,
            &build_agent_skill_record(&job.archive_uri, &usage),
        )
        .await?;
        artifacts_written.insert("skill_record".to_owned(), 1);
        let mode = semantic_index_root(
            &job.workspace_root,
            &format!(
                "tenant:{}:{}:agent:{}__{}",
                job.account_id, job.user_id, job.user_id, job.agent_id
            ),
            agent_skill_path.parent().expect("agent skill parent"),
            &format!("mfs://agent/skills/used/{}", job.session_id),
            "skill",
        )
        .await?;
        if mode == ProcessingMode::Degraded {
            processing_mode = ProcessingMode::Degraded;
        }
        record_derived_projection(
            &metadata,
            &job,
            &format!(
                "tenant:{}:{}:agent:{}__{}",
                job.account_id, job.user_id, job.user_id, job.agent_id
            ),
            agent_skill_path.parent().expect("agent skill parent"),
            &format!("mfs://agent/skills/used/{}", job.session_id),
            "skill.writeback",
        )?;
    }

    let tool_candidates = memory_candidates
        .iter()
        .filter(|candidate| candidate.category == MemoryCategory::Tools)
        .cloned()
        .collect::<Vec<_>>();
    if !tool_candidates.is_empty() {
        let agent_space_name = format!("{}__{}", job.user_id, job.agent_id);
        let stable_tool_root = job
            .workspace_root
            .join("tenants")
            .join(&job.account_id)
            .join(&job.user_id)
            .join("agent")
            .join(&agent_space_name)
            .join("memories")
            .join("tools");
        for candidate in &tool_candidates {
            let stable_tool_path =
                stable_tool_root.join(format!("{}.md", sanitize_memory_slug(&candidate.title)));
            session_write_memory_file(
                &stable_tool_path,
                &session_build_mergeable(
                    &stable_tool_path,
                    MemoryCategory::Tools,
                    "Tool Memory",
                    std::slice::from_ref(candidate),
                )
                .await?,
            )
            .await?;
        }
        memories_extracted.insert("tools".to_owned(), 1);
        let mode = semantic_index_root(
            &job.workspace_root,
            &format!(
                "tenant:{}:{}:agent:{}__{}",
                job.account_id, job.user_id, job.user_id, job.agent_id
            ),
            &stable_tool_root,
            "mfs://agent/memories/tools",
            "memory",
        )
        .await?;
        if mode == ProcessingMode::Degraded {
            processing_mode = ProcessingMode::Degraded;
        }
        record_derived_projection(
            &metadata,
            &job,
            &format!(
                "tenant:{}:{}:agent:{}__{}",
                job.account_id, job.user_id, job.user_id, job.agent_id
            ),
            &stable_tool_root,
            "mfs://agent/memories/tools",
            "memory.writeback",
        )?;
    }

    // OV-P1: Semantic change propagation — rebuild L0/L1 for affected directories
    let tenant_root = job
        .workspace_root
        .join("tenants")
        .join(&job.account_id)
        .join(&job.user_id);
    let mut affected_dirs = HashSet::new();

    if memories_extracted.contains_key("profile") {
        affected_dirs.insert(tenant_root.join("user/memories"));
    }
    if memories_extracted.contains_key("preferences") {
        affected_dirs.insert(tenant_root.join("user/memories/preferences"));
        affected_dirs.insert(tenant_root.join("user/memories"));
    }
    if memories_extracted.contains_key("events") {
        affected_dirs.insert(tenant_root.join("user/memories/events"));
        affected_dirs.insert(tenant_root.join("user/memories"));
    }
    if memories_extracted.contains_key("entities") {
        affected_dirs.insert(tenant_root.join("user/memories/entities"));
        affected_dirs.insert(tenant_root.join("user/memories"));
    }
    if memories_extracted.contains_key("patterns") {
        let agent_memories = tenant_root
            .join("agent")
            .join(format!("{}__{}", job.user_id, job.agent_id))
            .join("memories");
        affected_dirs.insert(agent_memories.join("patterns"));
        affected_dirs.insert(agent_memories);
    }
    if memories_extracted.contains_key("cases") {
        let agent_memories = tenant_root
            .join("agent")
            .join(format!("{}__{}", job.user_id, job.agent_id))
            .join("memories");
        affected_dirs.insert(agent_memories.join("cases"));
        affected_dirs.insert(agent_memories);
    }
    if memories_extracted.contains_key("tools") {
        let agent_memories = tenant_root
            .join("agent")
            .join(format!("{}__{}", job.user_id, job.agent_id))
            .join("memories");
        affected_dirs.insert(agent_memories.join("tools"));
        affected_dirs.insert(agent_memories);
    }
    if memories_extracted.contains_key("skills") {
        let agent_memories = tenant_root
            .join("agent")
            .join(format!("{}__{}", job.user_id, job.agent_id))
            .join("memories");
        affected_dirs.insert(agent_memories.join("skills"));
        affected_dirs.insert(agent_memories);
    }
    if artifacts_written.contains_key("user_session") {
        affected_dirs.insert(
            tenant_root
                .join("user/memories/session")
                .join(&job.agent_id)
                .join(&job.session_id),
        );
        affected_dirs.insert(
            tenant_root
                .join("user/memories/session")
                .join(&job.agent_id),
        );
        affected_dirs.insert(tenant_root.join("user/memories/session"));
        affected_dirs.insert(tenant_root.join("user/memories"));
    }

    // Rebuild summaries bottom-up: sort affected dirs by depth (deepest first)
    let mut sorted_dirs: Vec<PathBuf> = affected_dirs.into_iter().collect();
    sorted_dirs.sort_by(|left, right| {
        let left_depth = left.components().count();
        let right_depth = right.components().count();
        right_depth.cmp(&left_depth)
    });

    for dir_path in &sorted_dirs {
        if tokio::fs::try_exists(dir_path).await.unwrap_or(false) {
            let dir_uri = local_path_to_mfs_uri(dir_path, &job);
            if let Err(error) = write_layered_summaries(dir_path, &dir_uri) {
                tracing::warn!(
                    dir_uri = %dir_uri,
                    error = %error,
                    "failed to rebuild summaries for directory"
                );
            }
        }
    }

    // OV-P0: Auto-link session to consumed resources/skills
    let session_uri = format!("mfs://session/{}/{}", job.agent_id, job.session_id);
    for record in &usage {
        match record.kind.as_str() {
            "context" | "skill" if record.uri.starts_with("mfs://") => {
                metadata
                    .upsert_relation(&RelationRecord {
                        account_id: &job.account_id,
                        user_id: &job.user_id,
                        agent_id: Some(&job.agent_id),
                        from_uri: &session_uri,
                        to_uri: &record.uri,
                        relation_type: "accessed",
                    })
                    .map_err(|source| {
                        SessionError::io(
                            "upsert session-resource relation",
                            &job.archive_path.join("messages.jsonl"),
                            std::io::Error::other(source.to_string()),
                        )
                    })?;
            }
            _ => {}
        }
    }

    fs::write(&done_path, b"done\n")
        .await
        .map_err(|source| SessionError::io("write done marker", &done_path, source))?;

    if fs::try_exists(&job.redo_marker_path)
        .await
        .map_err(|source| SessionError::io("check redo marker", &job.redo_marker_path, source))?
    {
        fs::remove_file(&job.redo_marker_path)
            .await
            .map_err(|source| {
                SessionError::io("remove redo marker", &job.redo_marker_path, source)
            })?;
    }

    Ok(MemoryPipelineResult {
        memories_extracted,
        artifacts_written,
        processing_mode: format!("{processing_mode:?}").to_ascii_lowercase(),
    })
}

async fn read_messages(path: &Path) -> Result<Vec<(String, String)>, SessionError> {
    let content = fs::read_to_string(path)
        .await
        .map_err(|source| SessionError::io("read archive messages", path, source))?;
    let mut messages = Vec::new();

    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        let value = serde_json::from_str::<serde_json::Value>(line).map_err(SessionError::Serde)?;
        let role = value
            .get("role")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .to_owned();
        let message = value
            .get("content")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_owned();
        messages.push((role, message));
    }

    Ok(messages)
}

async fn read_usage_snapshot(path: &Path) -> Result<Vec<UsageRecord>, SessionError> {
    if !fs::try_exists(path)
        .await
        .map_err(|source| SessionError::io("check usage snapshot", path, source))?
    {
        return Ok(Vec::new());
    }

    let content = fs::read(path)
        .await
        .map_err(|source| SessionError::io("read usage snapshot", path, source))?;
    serde_json::from_slice(&content).map_err(SessionError::Serde)
}

fn archive_file_name(archive_path: &Path) -> String {
    archive_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!("{name}.md"))
        .unwrap_or_else(|| "archive.md".to_owned())
}

fn archive_segment_name(archive_path: &Path) -> String {
    archive_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("archive_000")
        .to_owned()
}

/// Async semantic indexing root — processes a directory tree through the
/// semantic pipeline (summary + embedding) and indexes documents into
/// `SqliteSemanticIndex`.
///
/// SQLite open + prefix delete run via `spawn_blocking` to avoid blocking
/// the tokio worker thread.  The semantic pipeline itself is async (HTTP
/// calls to LLM/embedding providers), so it runs directly on the async
/// runtime.  Individual `upsert_document` calls inside the pipeline are
/// fast (<1ms each) and tolerable on the worker thread.
async fn semantic_index_root(
    workspace_root: &Path,
    projection_view_id: &str,
    root: &Path,
    root_uri: &str,
    context_type: &str,
) -> Result<ProcessingMode, SessionError> {
    let semantic_sqlite_path = workspace_root.join("_system").join("semantic.sqlite");
    let pv = projection_view_id.to_owned();
    let ru = root_uri.to_owned();
    let semantic_sqlite_for_blocking = semantic_sqlite_path.clone();
    let semantic_index = tokio::task::spawn_blocking(move || {
        SqliteSemanticIndex::open_at(&semantic_sqlite_for_blocking).and_then(|idx| {
            idx.delete_prefix_in_projection(Some(&pv), Some(&ru))?;
            Ok(idx)
        })
    })
    .await
    .map_err(|e| {
        SessionError::io(
            "spawn_blocking for semantic index open+delete",
            &semantic_sqlite_path,
            std::io::Error::other(e.to_string()),
        )
    })?
    .map_err(|e| {
        SessionError::io(
            "open+delete semantic index",
            &semantic_sqlite_path,
            std::io::Error::other(e.to_string()),
        )
    })?;
    let pipeline = SemanticPipeline::new(SemanticPipelineConfig::from_env(8));
    let report = pipeline
        .process_root(
            root,
            projection_view_id,
            root_uri,
            context_type,
            None,
            &semantic_index,
        )
        .await
        .map_err(|e| {
            SessionError::io(
                "process semantic root",
                root,
                std::io::Error::other(e.to_string()),
            )
        })?;
    Ok(report.mode)
}

fn record_derived_projection(
    metadata: &MetadataStore,
    job: &MemoryPipelineJob,
    projection_view_id: &str,
    root_path: &Path,
    root_uri: &str,
    event_type: &str,
) -> Result<(), SessionError> {
    let snapshot_id = format!("session:{}:{}", job.task_id, root_uri);
    metadata
        .append_snapshot(&SnapshotRecord {
            snapshot_id: &snapshot_id,
            account_id: &job.account_id,
            user_id: &job.user_id,
            agent_id: Some(&job.agent_id),
            projection_view_id,
            root_uri,
            manifest_digest: Some(&snapshot_id),
            created_by: Some("session.commit"),
            notes: Some("derived session writeback"),
        })
        .map_err(|source| {
            SessionError::io(
                "append derived snapshot",
                root_path,
                std::io::Error::other(source.to_string()),
            )
        })?;

    upsert_projection_entries(
        metadata,
        &job.account_id,
        &job.user_id,
        Some(&job.agent_id),
        projection_view_id,
        root_path,
        root_uri,
        "session",
        &job.archive_uri,
        &snapshot_id,
    )?;

    append_projection_audit(
        metadata,
        &job.account_id,
        &job.user_id,
        Some(&job.agent_id),
        projection_view_id,
        root_path,
        root_uri,
        event_type,
    )?;

    Ok(())
}

fn upsert_projection_entries(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    agent_id: Option<&str>,
    projection_view_id: &str,
    root_path: &Path,
    root_uri: &str,
    source_kind: &str,
    source_identifier: &str,
    source_snapshot_id: &str,
) -> Result<(), SessionError> {
    let mut stack = vec![root_path.to_path_buf()];

    while let Some(path) = stack.pop() {
        let canonical_uri = if path == root_path {
            root_uri.to_owned()
        } else {
            let relative = path
                .strip_prefix(root_path)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            format!("{}/{}", root_uri.trim_end_matches('/'), relative)
        };
        let entry_kind = if path.is_dir() { "directory" } else { "file" };
        metadata
            .upsert_path_entry(&PathEntryRecord {
                account_id,
                user_id,
                agent_id,
                projection_view_id,
                canonical_uri: &canonical_uri,
                workspace_path: &path.to_string_lossy(),
                entry_kind,
                source_kind: Some(source_kind),
                source_identifier: Some(source_identifier),
                source_snapshot_id: Some(source_snapshot_id),
                content_kind: None,
                language: None,
                relative_resource_path: None,
                repo_root_uri: None,
                is_text: None,
                is_generated: None,
                content_digest: None,
                metadata_digest: None,
                size_bytes: None,
            })
            .map_err(|source| {
                SessionError::io(
                    "upsert derived path entry",
                    &path,
                    std::io::Error::other(source.to_string()),
                )
            })?;

        if path.is_dir() {
            for entry in std::fs::read_dir(&path).map_err(SessionError::IoRaw)? {
                let entry = entry.map_err(SessionError::IoRaw)?;
                stack.push(entry.path());
            }
        }
    }

    Ok(())
}

fn append_projection_audit(
    metadata: &MetadataStore,
    account_id: &str,
    user_id: &str,
    agent_id: Option<&str>,
    projection_view_id: &str,
    root_path: &Path,
    root_uri: &str,
    event_type: &str,
) -> Result<(), SessionError> {
    let mut stack = vec![root_path.to_path_buf()];

    while let Some(path) = stack.pop() {
        let canonical_uri = if path == root_path {
            root_uri.to_owned()
        } else {
            let relative = path
                .strip_prefix(root_path)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            format!("{}/{}", root_uri.trim_end_matches('/'), relative)
        };
        metadata
            .append_audit(&AuditEventRecord {
                account_id,
                user_id,
                agent_id,
                projection_view_id: Some(projection_view_id),
                event_type,
                subject_uri: Some(&canonical_uri),
                actor: Some("session"),
                details_json: Some("{\"result\":\"ok\"}"),
            })
            .map_err(|source| {
                SessionError::io(
                    "append derived audit",
                    &path,
                    std::io::Error::other(source.to_string()),
                )
            })?;

        if path.is_dir() {
            for entry in std::fs::read_dir(&path).map_err(SessionError::IoRaw)? {
                let entry = entry.map_err(SessionError::IoRaw)?;
                stack.push(entry.path());
            }
        }
    }

    Ok(())
}

fn local_path_to_mfs_uri(dir_path: &Path, job: &MemoryPipelineJob) -> String {
    let tenant_prefix = job
        .workspace_root
        .join("tenants")
        .join(&job.account_id)
        .join(&job.user_id);
    let relative = dir_path.strip_prefix(&tenant_prefix).unwrap_or(dir_path);
    let agent_space_prefix = format!("agent/{}__{}/", job.user_id, job.agent_id);
    let relative_str = relative.to_string_lossy();

    if let Some(after_agent) = relative_str.strip_prefix("agent/") {
        // Strip the per-agent namespace (user__agent) to get canonical URI
        let stripped = relative_str
            .strip_prefix(&agent_space_prefix)
            .unwrap_or(after_agent);
        format!("mfs://agent/{stripped}")
    } else {
        format!("mfs://{}", relative_str)
    }
}
