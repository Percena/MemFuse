mod catalog;
mod classify;
mod fs_ops;
mod layout;
mod materialize;
mod summaries;

pub use catalog::{CatalogError, ManagedResource, ResourceCatalog};
pub use classify::{
    ClassifiedPath, classify_path, content_digest_for_bytes, content_digest_for_path,
    directory_metadata_digest, infer_resource_kind_from_path, is_summary_sidecar, should_skip_path,
};
pub use fs_ops::{DirEntry, FileStat, FsError, TreeNode, WorkspaceFs};
pub use layout::{WorkspaceLayout, WorkspaceLayoutError};
pub use materialize::{MaterializationResult, MaterializeError, Materializer, SourceProvenance};
pub use summaries::{AdaptiveBudget, LayeredSummaries, SummaryError, write_layered_summaries};
