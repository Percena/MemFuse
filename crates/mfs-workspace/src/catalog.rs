use std::collections::HashSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

use mfs_metadata::{
    MetadataStore, ResourceAliasRecord, ResourceSourceRecord, StoredResourceSource,
};
use mfs_types::IdentityContext;
use mfs_uri::short_hash_hex;

use crate::classify::infer_resource_kind_from_path;
use crate::materialize::{MaterializeError, Materializer};

pub struct ResourceCatalog {
    workspace_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedResource {
    pub resource_id: String,
    pub logical_name: String,
    pub root_uri: String,
    pub target_path: PathBuf,
    pub resource_kind: String,
    pub source_kind: String,
    pub source_identifier: String,
    pub source_snapshot_id: String,
    pub source_host: Option<String>,
    pub source_namespace: Option<String>,
    pub source_repo: Option<String>,
    pub source_ref: Option<String>,
    pub canonical_strategy_version: String,
    pub alias_uris: Vec<String>,
}

impl ResourceCatalog {
    pub fn open(workspace_root: impl AsRef<Path>) -> rusqlite::Result<Self> {
        let workspace_root = workspace_root.as_ref().to_path_buf();
        let _ = MetadataStore::open_at(metadata_path(&workspace_root), false)?;
        Ok(Self { workspace_root })
    }

    pub async fn register_localfs(
        &self,
        account_id: &str,
        user_id: &str,
        agent_id: &str,
        source_path: &str,
        logical_name: Option<&str>,
    ) -> Result<ManagedResource, CatalogError> {
        self.register_resource(
            IdentityContext::new(account_id, user_id, agent_id),
            "localfs",
            source_path,
            source_path,
            logical_name,
            None,
            None,
        )
        .await
    }

    pub async fn register_git(
        &self,
        account_id: &str,
        user_id: &str,
        agent_id: &str,
        source_path: &str,
        logical_name: Option<&str>,
    ) -> Result<ManagedResource, CatalogError> {
        self.register_resource(
            IdentityContext::new(account_id, user_id, agent_id),
            "git",
            source_path,
            source_path,
            logical_name,
            None,
            None,
        )
        .await
    }

    pub async fn register_source(
        &self,
        account_id: &str,
        user_id: &str,
        agent_id: &str,
        source_kind: &str,
        source_path: &str,
        logical_name: Option<&str>,
    ) -> Result<ManagedResource, CatalogError> {
        self.register_source_with_identifier_and_ref(
            account_id,
            user_id,
            agent_id,
            source_kind,
            source_path,
            source_path,
            logical_name,
            None,
            None,
        )
        .await
    }

    /// Register a source with separate materialize path and source identifier.
    /// Branch and revision defaults to None.
    #[deprecated(note = "Use register_source_with_identifier_and_ref instead")]
    pub async fn register_source_with_identifier(
        &self,
        account_id: &str,
        user_id: &str,
        agent_id: &str,
        source_kind: &str,
        materialize_source_path: &str,
        source_identifier: &str,
        logical_name: Option<&str>,
    ) -> Result<ManagedResource, CatalogError> {
        self.register_source_with_identifier_and_ref(
            account_id,
            user_id,
            agent_id,
            source_kind,
            materialize_source_path,
            source_identifier,
            logical_name,
            None,
            None,
        )
        .await
    }

    /// Register a source with optional branch/revision ref parameters.
    /// Materialize path and source identifier are the same (source_path).
    #[deprecated(note = "Use register_source_with_identifier_and_ref instead")]
    pub async fn register_source_with_ref(
        &self,
        account_id: &str,
        user_id: &str,
        agent_id: &str,
        source_kind: &str,
        source_path: &str,
        logical_name: Option<&str>,
        branch: Option<&str>,
        revision: Option<&str>,
    ) -> Result<ManagedResource, CatalogError> {
        self.register_source_with_identifier_and_ref(
            account_id,
            user_id,
            agent_id,
            source_kind,
            source_path,
            source_path,
            logical_name,
            branch,
            revision,
        )
        .await
    }

    /// Register a remote source (e.g. git_url) with separate identifier and ref parameters.
    /// `materialize_source_path` is the local staged path (for filesystem enumeration),
    /// `source_identifier` is the original URL/path (for provenance and family detection).
    pub async fn register_source_with_identifier_and_ref(
        &self,
        account_id: &str,
        user_id: &str,
        agent_id: &str,
        source_kind: &str,
        materialize_source_path: &str,
        source_identifier: &str,
        logical_name: Option<&str>,
        branch: Option<&str>,
        revision: Option<&str>,
    ) -> Result<ManagedResource, CatalogError> {
        self.register_resource(
            IdentityContext::new(account_id, user_id, agent_id),
            source_kind,
            materialize_source_path,
            source_identifier,
            logical_name,
            branch,
            revision,
        )
        .await
    }

    pub fn list_resources(
        &self,
        account_id: &str,
        user_id: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<StoredResourceSource>> {
        MetadataStore::open_at(metadata_path(&self.workspace_root), false)?
            .list_resource_sources(account_id, user_id, limit, None)
    }

    async fn register_resource(
        &self,
        identity: IdentityContext,
        source_kind: &str,
        materialize_source_path: &str,
        source_identifier: &str,
        logical_name: Option<&str>,
        branch: Option<&str>,
        revision: Option<&str>,
    ) -> Result<ManagedResource, CatalogError> {
        let metadata = MetadataStore::open_at(metadata_path(&self.workspace_root), false)?;

        // Family detection (git/git_url sources): if a resource with the same
        // canonical URI already exists for this tenant, refresh it instead of
        // creating a duplicate. Git URI is determined by origin URL (host/namespace/repo),
        // not by logical_name, so we can detect family before name resolution.
        if source_kind == "git" || source_kind == "git_url" {
            let sd_prelim = describe_source(source_kind, source_identifier, "");
            let prelim_uri = canonical_root_uri(source_kind, "", &sd_prelim);

            if let Some(existing) = metadata.get_resource_source_by_root_uri(
                identity.account_id(),
                identity.user_id(),
                &prelim_uri,
            )? {
                // Same family — update source_ref and re-materialize (refresh)
                let refreshed_source_ref = sd_prelim.source_ref;
                if refreshed_source_ref.is_some() {
                    let _ = metadata.register_resource_source(&ResourceSourceRecord {
                        resource_id: &existing.resource_id,
                        account_id: identity.account_id(),
                        user_id: identity.user_id(),
                        agent_id: existing.agent_id.as_deref(),
                        logical_name: &existing.logical_name,
                        source_kind: &existing.source_kind,
                        source_identifier: &existing.source_identifier,
                        canonical_root_uri: &existing.canonical_root_uri,
                        projection_view_id: &existing.projection_view_id,
                        resource_kind: &existing.resource_kind,
                        source_host: existing.source_host.as_deref(),
                        source_namespace: existing.source_namespace.as_deref(),
                        source_repo: existing.source_repo.as_deref(),
                        source_ref: refreshed_source_ref.as_deref(),
                        canonical_strategy_version: &existing.canonical_strategy_version,
                        status: "ready",
                        last_snapshot_id: existing.last_snapshot_id.as_deref(),
                    })?;
                }

                // Re-materialize to refresh file content
                let materializer = Materializer::new(&self.workspace_root);
                let materialized = if source_kind == "git" {
                    materializer
                        .materialize_git_with_ref(
                            &identity,
                            materialize_source_path,
                            &existing.canonical_root_uri,
                            branch,
                            revision,
                        )
                        .await?
                } else {
                    // git_url: staged content is already on local filesystem
                    materializer
                        .materialize_localfs_as(
                            &identity,
                            materialize_source_path,
                            &existing.canonical_root_uri,
                            source_kind,
                        )
                        .await?
                };

                return Ok(ManagedResource {
                    resource_id: existing.resource_id,
                    logical_name: existing.logical_name,
                    root_uri: existing.canonical_root_uri,
                    target_path: materialized.target_path,
                    resource_kind: existing.resource_kind,
                    source_kind: existing.source_kind,
                    source_identifier: existing.source_identifier,
                    source_snapshot_id: materialized.provenance.source_snapshot_id,
                    source_host: existing.source_host,
                    source_namespace: existing.source_namespace,
                    source_repo: existing.source_repo,
                    source_ref: refreshed_source_ref.or(existing.source_ref),
                    canonical_strategy_version: existing.canonical_strategy_version,
                    alias_uris: Vec::new(),
                });
            }
        }

        let existing_names = metadata
            .list_resource_sources(identity.account_id(), identity.user_id(), 256, None)?
            .into_iter()
            .map(|item| item.logical_name)
            .collect::<HashSet<_>>();

        let logical_name = unique_logical_name(
            logical_name
                .map(str::to_owned)
                .unwrap_or_else(|| derive_logical_name(source_identifier)),
            &existing_names,
        );
        let source_descriptor = describe_source(source_kind, source_identifier, &logical_name);
        let root_uri = canonical_root_uri(source_kind, &logical_name, &source_descriptor);
        let resource_id = build_resource_id(source_kind, &source_descriptor, &logical_name);
        let alias_uri = format!("mfs://resources/{logical_name}");
        let resource_kind = if source_kind == "git" || source_kind == "git_url" {
            "code_repo".to_owned()
        } else {
            infer_resource_kind_from_path(Path::new(materialize_source_path))
                .unwrap_or_else(|_| "generic_docs".to_owned())
        };

        let materializer = Materializer::new(&self.workspace_root);
        let materialized = match source_kind {
            "localfs" | "url" | "inline" | "import" => {
                materializer
                    .materialize_localfs_as(
                        &identity,
                        materialize_source_path,
                        &root_uri,
                        source_kind,
                    )
                    .await?
            }
            "git" => {
                materializer
                    .materialize_git_with_ref(
                        &identity,
                        materialize_source_path,
                        &root_uri,
                        branch,
                        revision,
                    )
                    .await?
            }
            "git_url" => {
                // git_url: staged content is on local filesystem after ZIP extraction or git clone
                materializer
                    .materialize_localfs_as(
                        &identity,
                        materialize_source_path,
                        &root_uri,
                        "git_url",
                    )
                    .await?
            }
            other => {
                return Err(CatalogError::UnsupportedSourceKind(other.to_owned()));
            }
        };

        metadata.register_resource_source(&ResourceSourceRecord {
            resource_id: &resource_id,
            account_id: identity.account_id(),
            user_id: identity.user_id(),
            agent_id: Some(identity.agent_id()),
            logical_name: &logical_name,
            source_kind,
            source_identifier,
            canonical_root_uri: &root_uri,
            projection_view_id: &format!(
                "tenant:{}:{}:resources",
                identity.account_id(),
                identity.user_id()
            ),
            resource_kind: &resource_kind,
            source_host: source_descriptor.host.as_deref(),
            source_namespace: source_descriptor.namespace.as_deref(),
            source_repo: source_descriptor.repo.as_deref(),
            source_ref: source_descriptor.source_ref.as_deref(),
            canonical_strategy_version: "v2",
            status: "ready",
            last_snapshot_id: Some(&materialized.provenance.source_snapshot_id),
        })?;
        if alias_uri != root_uri {
            let _ = metadata.upsert_resource_alias(&ResourceAliasRecord {
                alias_uri: &alias_uri,
                resource_id: &resource_id,
                canonical_root_uri: &root_uri,
            })?;
        }

        Ok(ManagedResource {
            resource_id,
            logical_name,
            root_uri: root_uri.clone(),
            target_path: materialized.target_path,
            resource_kind,
            source_kind: source_kind.to_owned(),
            source_identifier: source_identifier.to_owned(),
            source_snapshot_id: materialized.provenance.source_snapshot_id,
            source_host: source_descriptor.host,
            source_namespace: source_descriptor.namespace,
            source_repo: source_descriptor.repo,
            source_ref: source_descriptor.source_ref,
            canonical_strategy_version: "v2".to_owned(),
            alias_uris: if alias_uri == root_uri {
                Vec::new()
            } else {
                vec![alias_uri]
            },
        })
    }
}

fn metadata_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join("_system").join("metadata.sqlite")
}

fn derive_logical_name(source_path: &str) -> String {
    let raw = Path::new(source_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("resource");
    sanitize_logical_name(raw)
}

fn unique_logical_name(base: String, existing: &HashSet<String>) -> String {
    if !existing.contains(&base) {
        return base;
    }
    for index in 2..10_000 {
        let candidate = format!("{base}-{index}");
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    format!("{}-{}", base, existing.len() + 1)
}

fn sanitize_logical_name(raw: &str) -> String {
    let mut name = raw
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    while name.contains("--") {
        name = name.replace("--", "-");
    }
    name.trim_matches('-')
        .to_owned()
        .chars()
        .take(48)
        .collect::<String>()
}

fn build_resource_id(
    source_kind: &str,
    source_descriptor: &SourceDescriptor,
    logical_name: &str,
) -> String {
    // Use family_key content hash instead of timestamp XOR path.len()
    // Same repo → same resource_id (enables family detection)
    // Different repo → different resource_id (hash includes host+namespace+repo)
    let family_input = format!(
        "{}:{}:{}",
        source_descriptor.host.as_deref().unwrap_or("local"),
        source_descriptor.namespace.as_deref().unwrap_or("default"),
        source_descriptor.repo.as_deref().unwrap_or(logical_name)
    );
    let family_fingerprint = short_hash_hex(family_input.as_bytes(), 8);
    format!(
        "res-{}-{}-{}",
        sanitize_logical_name(source_kind),
        sanitize_logical_name(logical_name),
        family_fingerprint
    )
}

#[derive(Debug, Clone, Default)]
struct SourceDescriptor {
    host: Option<String>,
    namespace: Option<String>,
    repo: Option<String>,
    source_ref: Option<String>,
}

fn canonical_root_uri(source_kind: &str, logical_name: &str, source: &SourceDescriptor) -> String {
    match source_kind {
        "git" | "git_url" => {
            let host = source.host.as_deref().unwrap_or("local");
            let ns = source
                .namespace
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("workspace");
            let repo = source.repo.as_deref().unwrap_or(logical_name);
            format!("mfs://resources/git/{host}/{ns}/{repo}")
        }
        "localfs" => format!("mfs://resources/localfs/{logical_name}"),
        "url" => format!(
            "mfs://resources/url/{}/{}",
            source.host.as_deref().unwrap_or("unknown"),
            source.repo.as_deref().unwrap_or(logical_name)
        ),
        "inline" | "import" => format!("mfs://resources/inline/{logical_name}"),
        _ => format!("mfs://resources/{logical_name}"),
    }
}

fn describe_source(source_kind: &str, source_path: &str, logical_name: &str) -> SourceDescriptor {
    match source_kind {
        "git" => describe_git_source(source_path),
        "git_url" => describe_git_url_source(source_path),
        "url" => describe_url_source(source_path, logical_name),
        _ => SourceDescriptor::default(),
    }
}

/// Describe a git_url source by parsing the remote URL directly.
/// Unlike `describe_git_source()` which uses git2::Repository::discover(),
/// this only needs URL parsing since the source is a remote URL, not a local path.
fn describe_git_url_source(source_path: &str) -> SourceDescriptor {
    let parsed = parse_git_remote(source_path);
    if let Some(descriptor) = parsed {
        descriptor
    } else {
        // Fallback: derive repo name from URL path
        let repo_name = Path::new(source_path)
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| sanitize_repo_segment(name.trim_end_matches(".git")))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "repo".to_owned());
        SourceDescriptor {
            host: Some("remote".to_owned()),
            namespace: Some("default".to_owned()),
            repo: Some(repo_name),
            source_ref: None,
        }
    }
}

fn describe_git_source(source_path: &str) -> SourceDescriptor {
    let path = Path::new(source_path);
    let repo_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| sanitize_repo_segment(name.trim_end_matches(".git")))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "repo".to_owned());

    // Discover git repo and extract origin + source_ref in one pass
    let discovered = git2::Repository::discover(path).ok();
    let source_ref = discovered.as_ref().and_then(extract_source_ref);

    let origin = discovered.as_ref().and_then(|repo| {
        repo.find_remote("origin")
            .ok()
            .and_then(|remote| remote.url().map(str::to_owned))
    });

    if let Some(origin) = origin {
        if let Some(parsed) = parse_git_remote(&origin) {
            return SourceDescriptor {
                source_ref,
                ..parsed
            };
        }
    }

    // No origin (or parse failed): stable namespace that doesn't depend on path
    // URI: mfs://resources/git/local/local/{repo_name}
    // No canonicalize(path) hash — stable regardless of directory moves
    SourceDescriptor {
        host: Some("local".to_owned()),
        namespace: Some("local".to_owned()),
        repo: Some(repo_name),
        source_ref,
    }
}

/// Extract branch:commit_short from a git repository's HEAD reference.
fn extract_source_ref(repo: &git2::Repository) -> Option<String> {
    let head = repo.head().ok()?;
    let branch = head.shorthand().unwrap_or("HEAD");
    let commit_id = head.target().map(|oid| oid.to_string()).unwrap_or_default();
    let short_id = &commit_id[..7.min(commit_id.len())];
    Some(format!("{}:{}", branch, short_id))
}

fn describe_url_source(source_path: &str, logical_name: &str) -> SourceDescriptor {
    let trimmed = source_path.trim();
    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let mut parts = without_scheme.split('/');
    let host = parts.next().unwrap_or("unknown");
    let path = parts
        .collect::<Vec<_>>()
        .join("-")
        .replace('.', "-")
        .trim_matches('-')
        .to_owned();
    SourceDescriptor {
        host: Some(compact_repo_segment(host)),
        namespace: None,
        repo: Some(if path.is_empty() {
            compact_repo_segment(logical_name)
        } else {
            compact_repo_segment(&path)
        }),
        source_ref: None,
    }
}

fn parse_git_remote(remote: &str) -> Option<SourceDescriptor> {
    let cleaned = remote.trim().trim_end_matches(".git");
    let without_scheme = if let Some((_, rest)) = cleaned.split_once("://") {
        rest
    } else if let Some(rest) = cleaned.strip_prefix("git@") {
        rest
    } else {
        cleaned
    };

    let normalized = without_scheme.replace(':', "/");
    let parts = normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    // Require at least host + repo (2 segments).
    // SSH URLs like git@internal-git:myrepo produce only 2 parts after normalization.
    if parts.len() < 2 {
        return None;
    }

    let host = normalize_host_segment(parts[0]);
    let repo = compact_repo_segment(parts[parts.len() - 1]);
    // Flatten namespace segments into a single segment (e.g. "org/sub/deep" → "org-sub-deep")
    // This keeps URI depth fixed at 4 layers: git/host/namespace/repo
    // If no namespace segments exist (e.g. host+repo only), use "default"
    let flat_namespace = if parts.len() > 2 {
        parts[1..parts.len() - 1]
            .iter()
            .map(|segment| compact_repo_segment(segment))
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>()
            .join("-")
    } else {
        "default".to_owned()
    };
    if host.is_empty() || flat_namespace.is_empty() || repo.is_empty() {
        return None;
    }

    Some(SourceDescriptor {
        host: Some(host),
        namespace: Some(flat_namespace),
        repo: Some(repo),
        source_ref: None,
    })
}

fn sanitize_repo_segment(raw: &str) -> String {
    sanitize_logical_name(raw)
}

fn compact_repo_segment(raw: &str) -> String {
    let normalized = sanitize_repo_segment(raw);
    if normalized.len() <= 48 {
        return normalized;
    }
    let suffix = short_hash_hex(normalized.as_bytes(), 8);
    let prefix_len = 48usize.saturating_sub(suffix.len() + 1);
    format!("{}-{}", &normalized[..prefix_len], suffix)
}

fn normalize_host_segment(raw: &str) -> String {
    let normalized = raw.trim().to_ascii_lowercase();
    let mut host = normalized
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    while host.contains("--") {
        host = host.replace("--", "-");
    }
    host.trim_matches('-').to_owned()
}

#[derive(Debug)]
pub enum CatalogError {
    Metadata(rusqlite::Error),
    Materialize(MaterializeError),
    UnsupportedSourceKind(String),
}

impl Display for CatalogError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Metadata(source) => write!(f, "metadata error: {source}"),
            Self::Materialize(source) => write!(f, "materialization error: {source}"),
            Self::UnsupportedSourceKind(kind) => write!(f, "unsupported source kind '{kind}'"),
        }
    }
}

impl Error for CatalogError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Metadata(source) => Some(source),
            Self::Materialize(source) => Some(source),
            Self::UnsupportedSourceKind(_) => None,
        }
    }
}

impl From<rusqlite::Error> for CatalogError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Metadata(value)
    }
}

impl From<MaterializeError> for CatalogError {
    fn from(value: MaterializeError) -> Self {
        Self::Materialize(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_git_url_source_github() {
        let sd = describe_git_url_source("https://github.com/example-org/example-repo");
        assert_eq!(sd.host.as_deref(), Some("github.com"));
        assert_eq!(sd.namespace.as_deref(), Some("example-org"));
        assert_eq!(sd.repo.as_deref(), Some("example-repo"));
    }

    #[test]
    fn describe_git_url_source_ssh() {
        let sd = describe_git_url_source("git@github.com:org/repo.git");
        assert_eq!(sd.host.as_deref(), Some("github.com"));
        assert_eq!(sd.namespace.as_deref(), Some("org"));
        assert_eq!(sd.repo.as_deref(), Some("repo"));
    }

    #[test]
    fn describe_git_url_source_gitlab_nested() {
        let sd = describe_git_url_source("https://gitlab.com/org/sub/deep-repo");
        assert_eq!(sd.host.as_deref(), Some("gitlab.com"));
        // parse_git_remote flattens namespace segments with "-"
        assert_eq!(sd.namespace.as_deref(), Some("org-sub"));
        assert_eq!(sd.repo.as_deref(), Some("deep-repo"));
    }

    #[test]
    fn canonical_root_uri_git_url() {
        let sd = SourceDescriptor {
            host: Some("github.com".to_owned()),
            namespace: Some("example-org".to_owned()),
            repo: Some("example-repo".to_owned()),
            source_ref: None,
        };
        let uri = canonical_root_uri("git_url", "fallback", &sd);
        assert_eq!(uri, "mfs://resources/git/github.com/example-org/example-repo");
    }

    #[test]
    fn canonical_root_uri_git_url_same_as_git() {
        let sd = SourceDescriptor {
            host: Some("github.com".to_owned()),
            namespace: Some("example-org".to_owned()),
            repo: Some("example-repo".to_owned()),
            source_ref: None,
        };
        let git_uri = canonical_root_uri("git", "fallback", &sd);
        let git_url_uri = canonical_root_uri("git_url", "fallback", &sd);
        // git_url and git share the same URI namespace (enables family detection)
        assert_eq!(git_uri, git_url_uri);
    }
}
