use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use mfs_index::SqliteSemanticIndex;
use mfs_metadata::{MetadataStore, TaskRecord};
use mfs_semantic::{SemanticPipeline, SemanticPipelineConfig};
use mfs_types::IdentityContext;
use mfs_uri::MfsUri;
use mfs_workspace::{ManagedResource, ResourceCatalog, SourceProvenance, WorkspaceLayout};

use super::rebuild::rebuild_metadata_entries_with_provenance;
use super::{ResourceIngestResult, ResourceSemanticCompletion, snapshot_record};

// ---------------------------------------------------------------------------
// Public API — ingest
// ---------------------------------------------------------------------------

pub async fn ingest_resource(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    source_kind: &str,
    source_path: &str,
    logical_name: Option<&str>,
) -> Result<ResourceIngestResult, Box<dyn std::error::Error>> {
    let task_key = format!(
        "semantic:{}:{}",
        identity.user_id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    );
    metadata.upsert_task(&TaskRecord {
        task_key: &task_key,
        account_id: identity.account_id(),
        user_id: identity.user_id(),
        agent_id: Some(identity.agent_id()),
        projection_view_id: Some(&format!(
            "tenant:{}:{}:resources",
            identity.account_id(),
            identity.user_id()
        )),
        state: "running",
        owner_space: Some("resources"),
        summary: Some("semantic resource ingest"),
        last_error: None,
        attempt_count: 1,
        max_attempts: 1,
        retry_state: "not_needed",
        processing_mode: None,
    })?;

    let result = ingest_resource_inner(
        metadata,
        workspace_root,
        identity,
        source_kind,
        source_path,
        logical_name,
    )
    .await;

    match result {
        Ok(result) => {
            metadata.upsert_task(&TaskRecord {
                task_key: &task_key,
                account_id: identity.account_id(),
                user_id: identity.user_id(),
                agent_id: Some(identity.agent_id()),
                projection_view_id: Some(&format!(
                    "tenant:{}:{}:resources",
                    identity.account_id(),
                    identity.user_id()
                )),
                state: "completed",
                owner_space: Some("resources"),
                summary: Some(&format!("indexed {}", result.root_uri)),
                last_error: None,
                attempt_count: 1,
                max_attempts: 1,
                retry_state: "not_needed",
                processing_mode: Some(&result.mode),
            })?;
            Ok(ResourceIngestResult { task_key, ..result })
        }
        Err(error) => {
            metadata.upsert_task(&TaskRecord {
                task_key: &task_key,
                account_id: identity.account_id(),
                user_id: identity.user_id(),
                agent_id: Some(identity.agent_id()),
                projection_view_id: Some(&format!(
                    "tenant:{}:{}:resources",
                    identity.account_id(),
                    identity.user_id()
                )),
                state: "failed",
                owner_space: Some("resources"),
                summary: Some("semantic resource ingest"),
                last_error: Some(&error.to_string()),
                attempt_count: 1,
                max_attempts: 1,
                retry_state: "exhausted",
                processing_mode: None,
            })?;
            Err(error)
        }
    }
}

async fn ingest_resource_inner(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    source_kind: &str,
    source_path: &str,
    logical_name: Option<&str>,
) -> Result<ResourceIngestResult, Box<dyn std::error::Error>> {
    let managed = prepare_resource_ingest(
        metadata,
        workspace_root,
        identity,
        source_kind,
        source_path,
        logical_name,
        None,
        None,
    )
    .await?;
    let completion =
        complete_prepared_resource_ingest(metadata, workspace_root, identity, &managed).await?;

    Ok(ResourceIngestResult {
        task_key: String::new(),
        resource_id: managed.resource_id,
        logical_name: managed.logical_name,
        root_uri: managed.root_uri,
        indexed_documents: completion.indexed_documents,
        mode: completion.mode,
    })
}

pub async fn prepare_resource_ingest(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    source_kind: &str,
    source_path: &str,
    logical_name: Option<&str>,
    branch: Option<&str>,
    revision: Option<&str>,
) -> Result<ManagedResource, Box<dyn std::error::Error>> {
    let catalog = ResourceCatalog::open(workspace_root)?;
    let managed = match source_kind {
        "localfs" | "git" | "inline" | "import" => {
            catalog
                .register_source_with_identifier_and_ref(
                    identity.account_id(),
                    identity.user_id(),
                    identity.agent_id(),
                    source_kind,
                    source_path,
                    source_path,
                    logical_name,
                    branch,
                    revision,
                )
                .await?
        }
        "url" => {
            let staged = stage_url_source(workspace_root, source_path).await?;
            catalog
                .register_source_with_identifier_and_ref(
                    identity.account_id(),
                    identity.user_id(),
                    identity.agent_id(),
                    "url",
                    staged.to_str().expect("staged url path utf-8"),
                    source_path,
                    logical_name,
                    None,
                    None,
                )
                .await?
        }
        "git_url" => {
            let staged = stage_git_url_source(workspace_root, source_path, branch).await?;
            catalog
                .register_source_with_identifier_and_ref(
                    identity.account_id(),
                    identity.user_id(),
                    identity.agent_id(),
                    "git_url",
                    staged.to_str().expect("staged git_url path utf-8"),
                    source_path,
                    logical_name,
                    branch,
                    revision,
                )
                .await?
        }
        other => {
            return Err(std::io::Error::other(format!(
                "ingest does not support source kind '{other}'"
            ))
            .into());
        }
    };
    metadata.register_resource_source(&mfs_metadata::ResourceSourceRecord {
        resource_id: &managed.resource_id,
        account_id: identity.account_id(),
        user_id: identity.user_id(),
        agent_id: Some(identity.agent_id()),
        logical_name: &managed.logical_name,
        source_kind: &managed.source_kind,
        source_identifier: &managed.source_identifier,
        canonical_root_uri: &managed.root_uri,
        projection_view_id: &provenance_projection_view_id(identity),
        resource_kind: &managed.resource_kind,
        source_host: managed.source_host.as_deref(),
        source_namespace: managed.source_namespace.as_deref(),
        source_repo: managed.source_repo.as_deref(),
        source_ref: managed.source_ref.as_deref(),
        canonical_strategy_version: &managed.canonical_strategy_version,
        status: "processing",
        last_snapshot_id: Some(&managed.source_snapshot_id),
    })?;

    Ok(managed)
}

pub async fn prepare_inline_resource_ingest(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    file_name: &str,
    content: &str,
    logical_name: Option<&str>,
) -> Result<ManagedResource, Box<dyn std::error::Error>> {
    let file_name = sanitize_inline_file_name(file_name)?;
    let upload_root = workspace_root.join("_system").join("uploads").join(format!(
        "inline-{}-{}",
        identity.user_id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));
    tokio::fs::create_dir_all(&upload_root).await?;
    tokio::fs::write(upload_root.join(&file_name), content).await?;
    prepare_resource_ingest(
        metadata,
        workspace_root,
        identity,
        "inline",
        upload_root.to_str().expect("upload root utf-8"),
        logical_name,
        None,
        None,
    )
    .await
}

pub async fn complete_prepared_resource_ingest(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    managed: &ManagedResource,
) -> Result<ResourceSemanticCompletion, Box<dyn std::error::Error>> {
    let ws = workspace_root.to_path_buf();
    let proj_view_id = provenance_projection_view_id(identity);
    let root_uri = managed.root_uri.clone();
    let resource_id = managed.resource_id.clone();
    let target_path = managed.target_path.clone();
    let semantic_index = tokio::task::spawn_blocking(move || {
        SqliteSemanticIndex::open_at(ws.join("_system").join("semantic.sqlite"))
    })
    .await
    .map_err(|e| -> Box<dyn std::error::Error> { format!("spawn_blocking: {e}").into() })?
    .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    let pipeline = SemanticPipeline::new(SemanticPipelineConfig::from_env(8));
    let report = pipeline
        .process_resource_root(
            &target_path,
            &proj_view_id,
            &root_uri,
            Some(&resource_id),
            &semantic_index,
        )
        .await
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    let indexed_documents = report.indexed_documents;
    let mode = format!("{:?}", report.mode).to_ascii_lowercase();

    let provenance = SourceProvenance {
        source_kind: managed.source_kind.clone(),
        source_identifier: managed.source_identifier.clone(),
        source_snapshot_id: managed.source_snapshot_id.clone(),
        projection_view_id: format!(
            "tenant:{}:{}:resources",
            identity.account_id(),
            identity.user_id()
        ),
        materialization_mode: "managed".to_owned(),
        target_uri: managed.root_uri.clone(),
    };
    let _ = rebuild_metadata_entries_with_provenance(
        metadata,
        identity,
        &managed.target_path,
        &managed.root_uri,
        Some(&provenance),
    )?;
    metadata.append_snapshot(&snapshot_record(&provenance))?;
    metadata.register_resource_source(&mfs_metadata::ResourceSourceRecord {
        resource_id: &managed.resource_id,
        account_id: identity.account_id(),
        user_id: identity.user_id(),
        agent_id: Some(identity.agent_id()),
        logical_name: &managed.logical_name,
        source_kind: &managed.source_kind,
        source_identifier: &managed.source_identifier,
        canonical_root_uri: &managed.root_uri,
        projection_view_id: &provenance.projection_view_id,
        resource_kind: &managed.resource_kind,
        source_host: managed.source_host.as_deref(),
        source_namespace: managed.source_namespace.as_deref(),
        source_repo: managed.source_repo.as_deref(),
        source_ref: managed.source_ref.as_deref(),
        canonical_strategy_version: &managed.canonical_strategy_version,
        status: "ready",
        last_snapshot_id: Some(&managed.source_snapshot_id),
    })?;

    Ok(ResourceSemanticCompletion {
        indexed_documents,
        mode,
    })
}

pub async fn complete_registered_resource_ingest(
    metadata: &MetadataStore,
    workspace_root: &Path,
    identity: &IdentityContext,
    resource_id: &str,
) -> Result<ResourceIngestResult, Box<dyn std::error::Error>> {
    let source = metadata
        .get_resource_source(resource_id)?
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "resource not found"))?;
    let target_uri = MfsUri::parse(&source.canonical_root_uri)?;
    let target_path = WorkspaceLayout::new(workspace_root).path_for_uri(identity, &target_uri)?;
    let managed = ManagedResource {
        resource_id: source.resource_id.clone(),
        logical_name: source.logical_name.clone(),
        root_uri: source.canonical_root_uri.clone(),
        target_path,
        resource_kind: source.resource_kind.clone(),
        source_kind: source.source_kind.clone(),
        source_identifier: source.source_identifier.clone(),
        source_snapshot_id: source
            .last_snapshot_id
            .clone()
            .unwrap_or_else(|| "ingest".to_owned()),
        source_host: source.source_host.clone(),
        source_namespace: source.source_namespace.clone(),
        source_repo: source.source_repo.clone(),
        source_ref: source.source_ref.clone(),
        canonical_strategy_version: source.canonical_strategy_version.clone(),
        alias_uris: metadata
            .list_resource_aliases(resource_id)?
            .into_iter()
            .map(|item| item.alias_uri)
            .collect(),
    };
    let completion =
        complete_prepared_resource_ingest(metadata, workspace_root, identity, &managed).await?;

    Ok(ResourceIngestResult {
        task_key: String::new(),
        resource_id: managed.resource_id,
        logical_name: managed.logical_name,
        root_uri: managed.root_uri,
        indexed_documents: completion.indexed_documents,
        mode: completion.mode,
    })
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn provenance_projection_view_id(identity: &IdentityContext) -> String {
    format!(
        "tenant:{}:{}:resources",
        identity.account_id(),
        identity.user_id()
    )
}

fn sanitize_inline_file_name(file_name: &str) -> Result<String, Box<dyn std::error::Error>> {
    if file_name.is_empty()
        || file_name.starts_with('/')
        || file_name.contains('\\')
        || file_name.contains("..")
    {
        return Err(std::io::Error::other("invalid inline resource file name").into());
    }
    Ok(file_name.to_owned())
}

pub(super) async fn stage_url_source(
    workspace_root: &Path,
    url: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // SSRF protection: validate URL target before fetching
    let allowed_domains: Vec<String> = std::env::var("MEMFUSE_ALLOWED_URL_DOMAINS")
        .ok()
        .map(|v| v.split(',').map(|s| s.trim().to_owned()).collect())
        .unwrap_or_default();
    let allowed_refs: Vec<&str> = allowed_domains.iter().map(|s| s.as_str()).collect();

    let ssrf_enabled = std::env::var("MEMFUSE_URL_SSRF_CHECK")
        .ok()
        .map(|v| v != "false")
        .unwrap_or(true);

    if ssrf_enabled {
        mfs_connectors::validate_url_target(url, &allowed_refs)?;
    }

    let response = reqwest::get(url).await?.error_for_status()?;
    let body = response.bytes().await?;
    let file_name = derive_url_file_name(url);
    let root = workspace_root.join("_system").join("uploads").join(format!(
        "url-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));
    tokio::fs::create_dir_all(&root).await?;
    tokio::fs::write(root.join(file_name), &body).await?;
    Ok(root)
}

fn derive_url_file_name(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| {
            parsed
                .path_segments()
                .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
                .map(str::to_owned)
        })
        .filter(|segment| !segment.is_empty())
        .unwrap_or_else(|| "index.md".to_owned())
}

// ---------------------------------------------------------------------------
// Git URL staging — ZIP-first strategy with git clone fallback
// ---------------------------------------------------------------------------

/// Parsed components of a git remote URL used to construct archive URLs.
struct GitUrlParts {
    host: String,
    owner: String, // namespace/organization (may contain '/')
    repo: String,  // repository name (without .git suffix)
}

fn parse_git_url(url: &str) -> Option<GitUrlParts> {
    let cleaned = url.trim().trim_end_matches(".git");
    let without_scheme: String = if let Some((_, rest)) = cleaned.split_once("://") {
        rest.to_owned()
    } else if let Some(rest) = cleaned.strip_prefix("git@") {
        rest.replace(':', "/")
    } else {
        return None;
    };

    let parts: Vec<&str> = without_scheme
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    if parts.len() < 2 {
        return None;
    }

    Some(GitUrlParts {
        host: parts[0].to_owned(),
        owner: if parts.len() > 2 {
            parts[1..parts.len() - 1].join("/")
        } else {
            "default".to_owned()
        },
        repo: parts[parts.len() - 1].to_owned(),
    })
}

/// Construct the ZIP archive URL for known git hosting platforms.
/// Returns None for unknown hosts (will fall back to git clone).
fn build_zip_archive_url(url: &str, branch: Option<&str>) -> Option<String> {
    let parts = parse_git_url(url)?;
    let branch = branch.unwrap_or("main");

    match parts.host.as_str() {
        "github.com" => Some(format!(
            "https://github.com/{}/{}/archive/refs/heads/{}.zip",
            parts.owner, parts.repo, branch
        )),
        "gitlab.com" | "gitlab.org" => Some(format!(
            "https://{}/{}/{}/-/archive/{}/{}-{}.zip",
            parts.host, parts.owner, parts.repo, branch, parts.repo, branch
        )),
        // Self-hosted GitLab instances (e.g. gitlab.internal.com)
        _ if parts.host.contains("gitlab") => Some(format!(
            "https://{}/{}/{}/-/archive/{}/{}-{}.zip",
            parts.host, parts.owner, parts.repo, branch, parts.repo, branch
        )),
        _ => None,
    }
}

/// Extract a ZIP archive to a target directory.
/// Handles GitHub's archive format which wraps content under a
/// `{repo}-{branch}/` prefix directory.
fn extract_zip_archive(
    zip_bytes: &[u8],
    target_dir: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let reader = io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader)?;

    // Detect the top-level prefix directory (e.g. "MemFuse-main/")
    // GitHub archives always wrap under such a prefix.
    let mut prefix_dirs: Vec<String> = Vec::new();
    for idx in 0..archive.len() {
        let entry = archive.by_index(idx)?;
        let name = entry.name().to_owned();
        if let Some(slash_pos) = name.find('/') {
            prefix_dirs.push(name[..slash_pos].to_owned());
        }
    }
    prefix_dirs.sort();
    prefix_dirs.dedup();

    // If there's exactly one top-level prefix dir, we strip it during extraction
    let strip_prefix = if prefix_dirs.len() == 1 {
        Some(prefix_dirs[0].clone())
    } else {
        None
    };

    fs::create_dir_all(target_dir)?;

    for idx in 0..archive.len() {
        let mut entry = archive.by_index(idx)?;
        let name = entry.name().to_owned();

        // Strip the top-level prefix if present
        let relative = match &strip_prefix {
            Some(prefix) => name.strip_prefix(prefix).unwrap_or(&name),
            None => &name,
        };

        if relative.is_empty() || relative == "/" {
            continue;
        }

        // Path traversal guard: reject entries containing ".." or that
        // would resolve outside the target directory.
        if relative.contains("..") {
            continue;
        }
        let destination = target_dir.join(relative);
        if !destination.starts_with(target_dir) {
            continue;
        }

        if entry.is_dir() {
            fs::create_dir_all(&destination)?;
        } else {
            // Zip bomb / oversized entry guard: skip entries larger than 100 MB.
            const MAX_ZIP_ENTRY_SIZE: u64 = 100_000_000;
            if entry.size() > MAX_ZIP_ENTRY_SIZE {
                tracing::warn!(
                    "skipping oversized ZIP entry '{}' ({} bytes > {} limit)",
                    relative,
                    entry.size(),
                    MAX_ZIP_ENTRY_SIZE
                );
                continue;
            }
            // Ensure parent directory exists
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut buf = Vec::with_capacity(entry.size().min(MAX_ZIP_ENTRY_SIZE) as usize);
            entry.read_to_end(&mut buf)?;
            fs::write(&destination, &buf)?;
        }
    }

    Ok(target_dir.to_path_buf())
}

/// Stage a git_url source: try ZIP archive download first, fall back to git clone.
///
/// ZIP strategy: construct platform-specific archive URL, download, extract.
/// Git clone fallback: shallow clone (`--depth 1`) to a temp directory.
pub(super) async fn stage_git_url_source(
    workspace_root: &Path,
    git_url: &str,
    branch: Option<&str>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let upload_root = workspace_root.join("_system").join("uploads").join(format!(
        "giturl-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));

    // Strategy 1: Try ZIP archive download for known hosting platforms
    let zip_url = build_zip_archive_url(git_url, branch);
    if let Some(zip_url) = zip_url {
        // SSRF protection for the constructed ZIP URL
        let allowed_domains: Vec<String> = std::env::var("MEMFUSE_ALLOWED_URL_DOMAINS")
            .ok()
            .map(|v| v.split(',').map(|s| s.trim().to_owned()).collect())
            .unwrap_or_default();
        let allowed_refs: Vec<&str> = allowed_domains.iter().map(|s| s.as_str()).collect();
        let ssrf_enabled = std::env::var("MEMFUSE_URL_SSRF_CHECK")
            .ok()
            .map(|v| v != "false")
            .unwrap_or(true);
        if ssrf_enabled {
            mfs_connectors::validate_url_target(&zip_url, &allowed_refs)?;
        }

        let response = reqwest::get(&zip_url).await;
        match response {
            Ok(resp) if resp.status().is_success() => {
                let body = resp.bytes().await?;
                let extracted = extract_zip_archive(&body, &upload_root)?;
                tracing::info!(
                    "git_url ZIP staging succeeded for '{}' → {}",
                    git_url,
                    extracted.display()
                );
                return Ok(extracted);
            }
            Ok(resp) => {
                tracing::warn!(
                    "git_url ZIP download for '{}' returned status {} — falling back to git clone",
                    zip_url,
                    resp.status()
                );
            }
            Err(err) => {
                tracing::warn!(
                    "git_url ZIP download for '{}' failed: {err} — falling back to git clone",
                    zip_url
                );
            }
        }
    } else {
        tracing::info!(
            "git_url '{}' is not a known ZIP-archive platform — using git clone",
            git_url
        );
    }

    // Strategy 2: Fall back to shallow git clone via CLI
    let branch_arg = branch.unwrap_or("main");
    let clone_dir = upload_root;
    fs::create_dir_all(&clone_dir)?;

    let clone_output = std::process::Command::new("git")
        .args(["clone", "--depth", "1", "--branch", branch_arg, git_url])
        .arg(&clone_dir)
        .output();

    match clone_output {
        Ok(output) if output.status.success() => {
            tracing::info!(
                "git_url git clone staging succeeded for '{}' → {}",
                git_url,
                clone_dir.display()
            );
            Ok(clone_dir)
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // First clone left partial content — clean before retry.
            if clone_dir.exists() {
                let _ = fs::remove_dir_all(&clone_dir);
            }
            fs::create_dir_all(&clone_dir)?;
            // If branch not found, try without --branch (let git pick default branch)
            let retry_output = std::process::Command::new("git")
                .args(["clone", "--depth", "1", git_url])
                .arg(&clone_dir)
                .output();
            match retry_output {
                Ok(retry) if retry.status.success() => {
                    tracing::info!(
                        "git_url git clone (no branch) succeeded for '{}' → {}",
                        git_url,
                        clone_dir.display()
                    );
                    Ok(clone_dir)
                }
                Ok(retry) => {
                    let retry_stderr = String::from_utf8_lossy(&retry.stderr);
                    Err(std::io::Error::other(format!(
                        "git clone failed for '{git_url}' (branch '{branch_arg}'): {stderr}\nRetry without branch: {retry_stderr}"
                    ))
                    .into())
                }
                Err(err) => Err(std::io::Error::other(format!(
                    "git clone retry failed for '{git_url}': {err}"
                ))
                .into()),
            }
        }
        Err(err) => Err(std::io::Error::other(format!(
            "git clone command unavailable for '{git_url}': {err} — ensure git is installed"
        ))
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parse_git_url_https() {
        let parts = parse_git_url("https://github.com/example-org/example-repo").unwrap();
        assert_eq!(parts.host, "github.com");
        assert_eq!(parts.owner, "example-org");
        assert_eq!(parts.repo, "example-repo");
    }

    #[test]
    fn parse_git_url_https_trailing_git() {
        let parts = parse_git_url("https://github.com/example-org/example-repo.git").unwrap();
        assert_eq!(parts.host, "github.com");
        assert_eq!(parts.owner, "example-org");
        assert_eq!(parts.repo, "example-repo");
    }

    #[test]
    fn parse_git_url_ssh() {
        let parts = parse_git_url("git@github.com:example-org/example-repo").unwrap();
        assert_eq!(parts.host, "github.com");
        assert_eq!(parts.owner, "example-org");
        assert_eq!(parts.repo, "example-repo");
    }

    #[test]
    fn parse_git_url_gitlab_nested() {
        let parts = parse_git_url("https://gitlab.com/org/sub/project").unwrap();
        assert_eq!(parts.host, "gitlab.com");
        assert_eq!(parts.owner, "org/sub");
        assert_eq!(parts.repo, "project");
    }

    #[test]
    fn parse_git_url_invalid_too_short() {
        assert!(parse_git_url("https://github.com").is_none());
        assert!(parse_git_url("just-a-word").is_none());
    }

    #[test]
    fn parse_git_url_ssh_short_two_parts() {
        // git@host:repo — after normalization becomes host/repo (2 segments)
        let parts = parse_git_url("git@internal-git:myrepo").unwrap();
        assert_eq!(parts.host, "internal-git");
        assert_eq!(parts.owner, "default");
        assert_eq!(parts.repo, "myrepo");
    }

    #[test]
    fn build_zip_url_github_main() {
        let url = build_zip_archive_url("https://github.com/example-org/example-repo", None).unwrap();
        assert_eq!(
            url,
            "https://github.com/example-org/example-repo/archive/refs/heads/main.zip"
        );
    }

    #[test]
    fn build_zip_url_github_branch() {
        let url = build_zip_archive_url("https://github.com/example-org/example-repo", Some("dev")).unwrap();
        assert_eq!(
            url,
            "https://github.com/example-org/example-repo/archive/refs/heads/dev.zip"
        );
    }

    #[test]
    fn build_zip_url_gitlab() {
        let url = build_zip_archive_url("https://gitlab.com/org/repo", Some("main")).unwrap();
        // GitLab format: host/owner/repo/-/archive/branch/repo-branch.zip
        assert_eq!(
            url,
            "https://gitlab.com/org/repo/-/archive/main/repo-main.zip"
        );
    }

    #[test]
    fn build_zip_url_self_hosted_gitlab() {
        let url =
            build_zip_archive_url("https://gitlab.internal.com/team/project", Some("v2")).unwrap();
        assert_eq!(
            url,
            "https://gitlab.internal.com/team/project/-/archive/v2/project-v2.zip"
        );
    }

    #[test]
    fn build_zip_url_unknown_host_fallback() {
        // Unknown hosting platform → returns None (will fall back to git clone)
        assert!(build_zip_archive_url("https://bitbucket.org/org/repo", Some("main")).is_none());
    }

    #[test]
    fn extract_zip_basic() {
        // Create a minimal ZIP archive in memory for testing ZIP parsing
        let mut buf = io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            writer.add_directory("repo-main/", options).unwrap();
            writer.start_file("repo-main/README.md", options).unwrap();
            writer.write_all(b"# Test Project\nHello world!").unwrap();
            writer.finish().unwrap();
        }
        let zip_bytes = buf.into_inner();

        // Verify the ZIP archive can be parsed and entries detected correctly
        let reader = io::Cursor::new(&zip_bytes);
        let mut archive = zip::ZipArchive::new(reader).expect("ZIP archive parsing should succeed");
        assert_eq!(archive.len(), 2); // 1 directory + 1 file

        // Verify prefix detection logic (GitHub format: repo-branch/)
        let mut names: Vec<String> = Vec::new();
        for idx in 0..archive.len() {
            let entry = archive.by_index(idx).unwrap();
            names.push(entry.name().to_owned());
        }
        assert!(names.iter().any(|n| n == "repo-main/"));
        assert!(names.iter().any(|n| n == "repo-main/README.md"));
    }
}
