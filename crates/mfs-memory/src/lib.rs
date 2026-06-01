//! MemFuse Memory Business Logic
//!
//! This crate owns: what to remember, how to remember it, and how to present it.
//! Storage crates (mfs-session, mfs-metadata, etc.) own: where to store it.
//!
//! Modules:
//! - `llm` — LLM-assisted operations with deterministic fallback (signal灯塔 philosophy)
//! - `overlay` — unconsolidated turn filtering for context injection
//! - `budget` — token budget allocation across overlay/facts/episodes
//! - `intent` — query intent classification for predicate routing (LLM + keyword fallback)
//! - `facts` — fact extraction + projection (LLM + regex fallback, assert/update/retract lifecycle)
//! - `episodes` — episode chunking, building, search, and decay (LLM summary + simple fallback)
//! - `consolidation` — consolidation pipeline: window resolution → chunk → build → project
//! - `briefs` — cross-thread memory brief generation
//! - `render` — markdown rendering for memory context injection
//! - `candidates` — memory candidate extraction, merge, and schema (previously in mfs-session)
//! - `service` — MemoryService facade for context resolution and search (thin handler support)

pub mod briefs;
pub mod budget;
pub mod candidates;
pub mod commit_service;
pub mod consolidation;
pub mod consolidation_sleep;
pub mod dream_phases;
pub mod episodes;
pub mod facts;
pub mod feedback_signal;
pub mod heuristics;
pub mod intent;
pub mod llm;
pub mod overlay;
pub mod render;
pub mod service;
pub mod t2h;
pub mod writeback;

// Re-export writeback types for backward compatibility (mfs-session used to own these).
pub use writeback::UsageRecord;
pub use writeback::{
    build_agent_memory_content, build_agent_skill_record,
    build_append_only_category_memory_content, build_archive_abstract, build_archive_overview,
    build_fact_backed_memory_content, build_mergeable_category_memory_content,
    build_profile_memory_content, build_user_memory_content, entity_slug_from_fact, is_entity_fact,
    is_preference_fact, is_profile_fact, sanitize_memory_slug, write_memory_file,
};

// Re-export candidate types for backward compatibility (mfs-session used to export these).
pub use candidates::{
    MemoryCandidate, MemoryCategory, MemoryDecision, MemoryMergeDecision, MemoryOwnership,
    MemoryRecord, decide_memory_merge, deterministic_extract, deterministic_merge,
    extract_memory_candidates, llm_merge_bundle,
};
// Re-export consolidation, LLM assist, and T2H pipeline so downstream crates
// (mfs-session) do not need to reach into internal submodules.
pub use commit_service::{
    ArchiveMemoryCommitInput, ArchiveMemoryCommitOutput, run_archive_memory_commit,
};
pub use consolidation::consolidate_and_persist;
pub use heuristics::{build_deterministic_prediction, build_simulate_reaction_prompt};
pub use llm::LlmAssist;
pub use service::{ResolveContextInput, ResolveContextOutput, resolve_context};
pub use service::{compute_staleness_note, fact_is_procedural, format_noted_date, parse_timestamp};
pub use t2h::run_t2h_pipeline;

use serde::{Deserialize, Serialize};

/// Search strategy presets.
/// Controls how facts and episodes are ranked and budgeted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SearchStrategy {
    /// Pure relevance sorting — current behavior unchanged. (default)
    #[default]
    Precision,
    /// Relevance + MMR post-processing (lambda=0.7).
    /// Diversity reranking penalizes episodes similar to already-selected ones.
    Diverse,
    /// Enhanced recency boost: 24h 2.0×, 7d 1.3×.
    Recent,
    /// Budget ×2 for maximum recall (thresholds unchanged).
    Comprehensive,
}

impl SearchStrategy {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "diverse" => SearchStrategy::Diverse,
            "recent" => SearchStrategy::Recent,
            "comprehensive" => SearchStrategy::Comprehensive,
            _ => SearchStrategy::Precision,
        }
    }
}

/// Domain types shared across modules.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationTurn {
    pub turn_id: String,
    pub turn_seq: i64,
    pub session_id: String,
    pub user_id: String,
    pub role: TurnRole,
    pub content_text: String,
    pub token_count: usize,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TurnRole {
    #[default]
    User,
    Assistant,
    System,
    Tool,
}

impl TurnRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            TurnRole::User => "user",
            TurnRole::Assistant => "assistant",
            TurnRole::System => "system",
            TurnRole::Tool => "tool",
        }
    }

    /// Parse a role string into a TurnRole, with fallback to User for unknown values.
    /// Maps "observation" to Tool (legacy alias from session engine).
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "assistant" => TurnRole::Assistant,
            "system" => TurnRole::System,
            "observation" | "tool" => TurnRole::Tool,
            _ => TurnRole::User,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactAssertion {
    pub assertion_id: String,
    pub user_id: String,
    pub subject: String,
    pub predicate: String,
    pub raw_value_text: String,
    pub value_type: String, // scalar, set, temporal
    pub operation: FactOperation,
    pub confidence: f64,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub source_turn_id: Option<String>,
    pub source_episode_ids: Option<Vec<String>>,
    pub extractor_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FactOperation {
    Assert,
    Update,
    Retract,
}

impl FactOperation {
    pub fn as_str(&self) -> &'static str {
        match self {
            FactOperation::Assert => "assert",
            FactOperation::Update => "update",
            FactOperation::Retract => "retract",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub fact_id: String,
    pub user_id: String,
    pub subject: String,
    pub predicate: String,
    pub display_value: String,
    pub confidence: f64,
    pub status: FactStatus,
    pub source_assertion_id: String,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_episode_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FactStatus {
    Active,
    Superseded,
    Retracted,
    Expired,
}

impl FactStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            FactStatus::Active => "active",
            FactStatus::Superseded => "superseded",
            FactStatus::Retracted => "retracted",
            FactStatus::Expired => "expired",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodeChunk {
    pub episode_id: String,
    pub user_id: String,
    pub session_id: String,
    pub resource_id: Option<String>,
    pub summary: String,
    pub salience_score: f64,
    pub strength_score: f64,
    pub recall_count: usize,
    pub last_recalled_at: Option<String>,
    pub source_start_turn_id: String,
    pub source_end_turn_id: String,
    pub created_at: String,
    pub embedding: Option<Vec<f32>>,
    /// Emotional valence of the episode (-1.0 = negative, 0.0 = neutral, 1.0 = positive).
    /// §9.3: activated from schema vestige, populated via keyword heuristics + LLM fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotional_valence: Option<f64>,
    /// Emotional intensity (0.0 = low, 1.0 = high).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotional_intensity: Option<f64>,
    /// Context tags as JSON array string (e.g., ["error_encountered", "fix_applied"]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tags_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationCursor {
    pub cursor_id: String,
    pub user_id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub last_consolidated_turn_id: String,
    pub dedupe_key: Option<String>,
    pub lease_owner: Option<String>,
    pub lease_expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBrief {
    pub brief_id: String,
    pub user_id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub summary: String,
    pub source_thread_ids: Vec<String>,
    pub anchor_episode_ids: Vec<String>,
    pub updated_at: Option<String>,
}

/// Memory context request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryContextRequest {
    pub user_id: String,
    pub thread_id: String,
    pub resource_id: Option<String>,
    pub query_text: String,
    pub budget: usize,
}

/// Memory context response sections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryContextSections {
    pub current_facts: Vec<FactEntry>,
    pub recent_updates: Vec<OverlayEntry>,
    pub relevant_history: Vec<EpisodeSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub behavioral_heuristics: Vec<heuristics::HeuristicEntry>,
}

/// Memory context artifacts (side-channel data).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryContextArtifacts {
    pub cross_thread_briefs: Vec<MemoryBrief>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryContextResponse {
    pub sections: MemoryContextSections,
    pub artifacts: MemoryContextArtifacts,
    pub detail_handles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactEntry {
    pub fact_id: String,
    pub predicate: String,
    pub display_value: String,
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staleness_note: Option<String>,
    /// ISO-8601/RFC3339 timestamp when this fact became valid.
    /// Used for validity-period staleness annotation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayEntry {
    pub turn_id: String,
    pub role: TurnRole,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodeSummary {
    pub episode_id: String,
    pub summary: String,
    pub salience: f64,
    pub strength: f64,
    pub recall_count: usize,
    /// Emotional valence: -1.0 (negative) to 1.0 (positive). None if not computed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotional_valence: Option<f64>,
    /// Emotional intensity: 0.0 (neutral) to 1.0 (highly intense). None if not computed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotional_intensity: Option<f64>,
    /// JSON-serialized context tags. None if not available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tags_json: Option<String>,
    /// JSON-serialized embedding vector. None if not computed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_json: Option<String>,
    /// ISO-8601 creation timestamp. None if not available (older episodes).
    /// Used for recency boost in reranking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

// Scope type constants
pub const SCOPE_TYPE_THREAD: &str = "thread";
pub const SCOPE_TYPE_RESOURCE: &str = "resource";
pub const SCOPE_TYPE_USER: &str = "user";

// Default injection budget
pub const DEFAULT_INJECTION_BUDGET: usize = 1200;

// Min confidence for injected facts
pub const MIN_INJECTED_FACT_CONFIDENCE: f64 = 0.5;

// Default generic fact limit when no intent match
pub const DEFAULT_GENERIC_FACT_LIMIT: usize = 2;

// Episodic search constants
pub const DEFAULT_EPISODIC_CANDIDATE_K: usize = 8;
pub const DEFAULT_EPISODIC_TOP_K: usize = 5;

// Overlay constants (aligned with Go constants)
pub const OVERLAY_CANDIDATE_LIMIT: usize = 20;
pub const MAX_OVERLAY_ENTRIES: usize = 6;
pub const MAX_OVERLAY_TOKENS: usize = 350;

// Episode budget denominator (1/3 to episodes)
pub const EPISODE_BUDGET_DENOMINATOR: usize = 3;
pub const MIN_EPISODE_BUDGET_TOKENS: usize = 150;

// Brief constants
pub const MAX_BRIEF_EPISODES: usize = 5;
pub const BRIEF_SUMMARY_TRUNCATE: usize = 160;

// Consolidation constants
pub const TIME_GAP_THRESHOLD_SECS: i64 = 900; // 15 minutes
pub const MAX_EPISODE_TOKENS: usize = 1200;

// Re-export unified contains_any from mfs-types
pub use mfs_types::text::contains_any;
