pub mod owned_path_ops;
pub mod resource;
pub mod skill_ingest;
pub mod status_ops;
pub mod wait_ops;
pub mod watch_ops;

use mfs_types::IdentityContext;

// ---------------------------------------------------------------------------
// Shared helpers used across the ops modules
// ---------------------------------------------------------------------------

pub(crate) fn projection_view_id_for_uri(
    identity: &IdentityContext,
    projection_uri: &str,
) -> String {
    if projection_uri.starts_with("mfs://resources") {
        return format!(
            "tenant:{}:{}:resources",
            identity.account_id(),
            identity.user_id()
        );
    }
    if projection_uri.starts_with("mfs://user") {
        return format!(
            "tenant:{}:{}:user",
            identity.account_id(),
            identity.user_id()
        );
    }
    format!(
        "tenant:{}:{}:agent:{}",
        identity.account_id(),
        identity.user_id(),
        identity.agent_space_name()
    )
}

pub(crate) fn projection_component(projection_view_id: &str, index: usize) -> &str {
    projection_view_id.split(':').nth(index).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// pub(crate) re-exports used by skill_ingest and owned_path_ops
// ---------------------------------------------------------------------------
pub(crate) use resource::{rebuild_metadata_entries_with_provenance, snapshot_record};

// ---------------------------------------------------------------------------
// Public API — re-exported for consumers (mfs-server, mfs-cli, mfs-mcp)
// ---------------------------------------------------------------------------
pub use resource::{
    ManagedResourceRebuildResult, ManagedResourceRefreshResult, RebuildResult, RefreshResult,
    ResourceIngestResult, ResourcePackManifest, ResourceSemanticCompletion,
    complete_prepared_resource_ingest, complete_registered_resource_ingest, export_resource_pack,
    import_resource_pack, ingest_resource, prepare_inline_resource_ingest, prepare_resource_ingest,
    rebuild_metadata_entries, rebuild_projection, rebuild_registered_resource, refresh_projection,
    refresh_registered_resource,
};

pub use watch_ops::{
    ResourceWatchLoopResult, ResourceWatchRunResult, ResourceWatchStatus, disable_resource_watch,
    list_resource_watch_statuses, list_resource_watches, register_resource_watch,
    run_due_resource_watches, run_resource_watch, run_resource_watch_loop,
};

pub use owned_path_ops::{
    OwnedPathMutationResult, mkdir_owned_path, move_owned_path, remove_owned_path, write_owned_path,
};

pub use status_ops::{
    ObserverStatusSummary, ResourceStatusCounts, SemanticObserverStats, SystemStatusSummary,
    TaskStateCounts, observer_status, system_status,
};

pub use wait_ops::{WaitTaskOutcome, wait_for_task_completion};

pub use skill_ingest::{SkillIngestResult, SkillSummary, ingest_skill, list_skills};
