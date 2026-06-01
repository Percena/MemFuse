//! Facts module — LLM-assisted + rule-based fact extraction + projection lifecycle.
//!
//! Extraction strategy (signal灯塔 philosophy):
//! 1. When LLM is available, use it to extract facts across the full 8-category
//!    taxonomy (profile, preferences, entities, events, cases, patterns, tools, skills).
//!    The LLM provides **directional signals** — telling the agent what kind of
//!    information exists and where to find more detail — not encyclopedic precision.
//! 2. When LLM is unavailable or returns unparseable output, fall back to the
//!    45-rule regex system covering 21 predicates (13 life-category + 1 architecture_decision + 7 procedural).
//!
//! Projection lifecycle unchanged: scalar (single active), set (multiple with dedup),
//! temporal (retract supersedes).

mod decay;
mod extract;
mod project;

pub use decay::{FactExpiryResult, expire_stale_facts};
pub use extract::{extract_facts, extract_facts_from_text};
pub use project::{filter_facts_for_injection, format_display_value, project_assertion};
