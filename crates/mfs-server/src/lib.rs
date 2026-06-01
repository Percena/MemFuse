// ---------------------------------------------------------------------------
// Module declarations
// ---------------------------------------------------------------------------
pub mod auth;
pub mod dream;
pub mod error_handler;
pub mod http;
pub mod metrics;
mod pid_lock;
pub mod runtime_config;

// ---------------------------------------------------------------------------
// Re-exports — preserve the public API surface at mfs_server::xxx
// ---------------------------------------------------------------------------
pub use pid_lock::PidLockGuard;

pub use mfs_ops::{
    ManagedResourceRebuildResult, ManagedResourceRefreshResult, ObserverStatusSummary,
    OwnedPathMutationResult, RebuildResult, RefreshResult, ResourceIngestResult,
    ResourcePackManifest, ResourceSemanticCompletion, ResourceStatusCounts,
    ResourceWatchLoopResult, ResourceWatchRunResult, ResourceWatchStatus, SemanticObserverStats,
    SkillIngestResult, SkillSummary, SystemStatusSummary, TaskStateCounts, WaitTaskOutcome,
    complete_prepared_resource_ingest, complete_registered_resource_ingest, disable_resource_watch,
    export_resource_pack, import_resource_pack, ingest_resource, ingest_skill,
    list_resource_watch_statuses, list_resource_watches, list_skills, mkdir_owned_path,
    move_owned_path, observer_status, prepare_inline_resource_ingest, prepare_resource_ingest,
    rebuild_metadata_entries, rebuild_projection, rebuild_registered_resource, refresh_projection,
    refresh_registered_resource, register_resource_watch, remove_owned_path,
    run_due_resource_watches, run_resource_watch, run_resource_watch_loop, system_status,
    wait_for_task_completion, write_owned_path,
};

// ---------------------------------------------------------------------------
// Runtime environment loader
// ---------------------------------------------------------------------------
pub fn load_runtime_env() {
    let _ = dotenvy::dotenv();
}

pub fn load_runtime_env_from(path: &std::path::Path) {
    let _ = dotenvy::from_path(path);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupRuntimeProviderNames {
    pub summary_provider: String,
    pub embedding_provider: String,
}

pub fn startup_runtime_provider_names() -> StartupRuntimeProviderNames {
    let runtime = mfs_semantic::current_runtime_config();
    StartupRuntimeProviderNames {
        summary_provider: runtime.summary_provider,
        embedding_provider: runtime.embedding_provider,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn startup_runtime_provider_names_auto_detect_llm_keys() {
        let _guard = mfs_test_util::env_isolated();
        unsafe {
            std::env::set_var("MEMFUSE_OPENAI_API_KEY", "test-openai-key");
            std::env::set_var("MEMFUSE_JINA_API_KEY", "test-jina-key");
        }

        let providers = crate::startup_runtime_provider_names();

        assert_eq!(providers.summary_provider, "openai");
        assert_eq!(providers.embedding_provider, "jina");
    }
}
