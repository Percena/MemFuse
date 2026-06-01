//! Memory candidate extraction, merge, and schema types.
//!
//! This module owns the "what to remember" and "how to merge it" logic
//! for memory candidates — the precursor stage before facts/episodes
//! consolidation.
//!
//! Previously these types and functions lived in mfs-session, violating
//! the architecture boundary (mfs-memory owns "what/how to remember",
//! mfs-session owns "where to persist"). This module restores that
//! boundary.

pub mod extract;
pub(crate) mod extract_llm;
pub(crate) mod extract_rules;
pub mod merge;
pub mod schema;

// Re-export key types for convenient access.
pub use extract::{deterministic_extract, extract_memory_candidates};
pub use merge::{decide_memory_merge, deterministic_merge, llm_merge_bundle};
pub use schema::{
    MemoryCandidate, MemoryCategory, MemoryDecision, MemoryMergeDecision, MemoryOwnership,
    MemoryRecord,
};
