//! Episodes module — episode chunking, building, search, and decay.
//!
//! Episodes are the primary unit of long-term episodic memory:
//! - Chunking: turns grouped by time gap (>15min) or token overflow (>1200)
//! - Building: each chunk → EpisodeChunk with summary (LLM or simple fallback)
//! - Search: embed query → vector search → rerank by score
//! - Decay: time-based salience decay + archival of cold episodes
//!
//! Summary strategy (signal灯塔 philosophy):
//! When LLM is available, generates L0 (one-line abstract) + L1 (structured overview)
//! + salience_hint + topic tags. These are directional signals telling the agent
//! "what happened and where to find more detail", not precise encyclopedic summaries.
//! When LLM is unavailable, falls back to `build_simple_summary` (concatenation + truncation).

pub(crate) mod annotate;
mod build;
mod chunk;
mod decay;
mod search;

pub use build::{build_episode, build_episode_with_summary, build_simple_summary};
pub use chunk::chunk_turns;
#[allow(deprecated)]
pub use decay::{EpisodeMaintenanceResult, compute_salience_decay, decay_episode_salience};
pub use search::{
    HOTNESS_ALPHA, MMR_LAMBDA, ScoredEpisode, VALENCE_BOOST_WEIGHT, compute_hotness,
    extract_keyword, limit_episodes, rerank_episodes, rerank_episodes_with_mmr,
    rerank_episodes_with_query,
};
