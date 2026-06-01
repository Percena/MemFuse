use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mfs_metadata::{AuditEventRecord, MetadataStore, TaskRecord};

use crate::Cli;

/// Long-lived state shared across all CLI subcommands.
/// Eliminates per-command MetadataStore::open_at overhead by opening once at startup
/// and reusing the same Arc<MetadataStore> throughout the process lifetime.
pub struct CliState {
    pub cli: Cli,
    pub metadata: Arc<MetadataStore>,
}

impl CliState {
    pub fn new(cli: Cli) -> Result<Self, Box<dyn std::error::Error>> {
        let metadata = Arc::new(MetadataStore::open_at(
            metadata_path(&cli.workspace_root),
            false,
        )?);
        Ok(Self { cli, metadata })
    }
}

pub fn metadata_path(workspace_root: &std::path::Path) -> PathBuf {
    workspace_root.join("_system").join("metadata.sqlite")
}

pub async fn configured_fs(
    cli: &Cli,
) -> Result<mfs_workspace::WorkspaceFs, Box<dyn std::error::Error>> {
    let source_path = cli
        .source_path
        .as_ref()
        .ok_or_else(|| std::io::Error::other("--source-path is required"))?;
    let target_uri = cli
        .target_uri
        .as_deref()
        .ok_or_else(|| std::io::Error::other("--target-uri is required"))?;
    Ok(match cli.source_kind.as_str() {
        "localfs" => {
            mfs_workspace::WorkspaceFs::from_localfs_source(
                &cli.workspace_root,
                &cli.account_id,
                &cli.user_id,
                &cli.agent_id,
                source_path.to_str().expect("source path utf-8"),
                target_uri,
            )
            .await?
        }
        "git" => {
            mfs_workspace::WorkspaceFs::from_git_source(
                &cli.workspace_root,
                &cli.account_id,
                &cli.user_id,
                &cli.agent_id,
                source_path.to_str().expect("source path utf-8"),
                target_uri,
            )
            .await?
        }
        other => {
            return Err(std::io::Error::other(format!("unsupported source kind '{other}'")).into());
        }
    })
}

pub async fn resolved_fs(
    cli: &Cli,
    uri: Option<&str>,
) -> Result<mfs_workspace::WorkspaceFs, Box<dyn std::error::Error>> {
    if let Some(uri) = uri {
        let scoped_existing = mfs_workspace::WorkspaceFs::open_existing_for_uri(
            &cli.workspace_root,
            &cli.account_id,
            &cli.user_id,
            &cli.agent_id,
            Some(uri),
        )?;
        if scoped_existing.stat(uri).await.is_ok() {
            return Ok(scoped_existing);
        }
    }
    configured_fs(cli).await
}

pub fn resource_projection_view_id(account_id: &str, user_id: &str) -> String {
    format!("tenant:{account_id}:{user_id}:resources")
}

pub fn semantic_task_key(operation: &str, identifier: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("semantic:{operation}:{identifier}:{nanos}")
}

pub fn upsert_semantic_task(
    metadata: &MetadataStore,
    cli: &Cli,
    task_key: &str,
    state: &str,
    summary: Option<&str>,
    last_error: Option<&str>,
    attempt_count: u32,
    max_attempts: u32,
    retry_state: &str,
    processing_mode: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    metadata.upsert_task(&TaskRecord {
        task_key,
        account_id: &cli.account_id,
        user_id: &cli.user_id,
        agent_id: Some(&cli.agent_id),
        projection_view_id: Some(&resource_projection_view_id(&cli.account_id, &cli.user_id)),
        state,
        owner_space: Some("resources"),
        summary,
        last_error,
        attempt_count,
        max_attempts,
        retry_state,
        processing_mode,
    })?;
    Ok(())
}

pub fn update_resource_status(
    metadata: &MetadataStore,
    resource_id: &str,
    status: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let resource = metadata
        .get_resource_source(resource_id)?
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "resource not found"))?;
    metadata.register_resource_source(&mfs_metadata::ResourceSourceRecord {
        resource_id: &resource.resource_id,
        account_id: &resource.account_id,
        user_id: &resource.user_id,
        agent_id: resource.agent_id.as_deref(),
        logical_name: &resource.logical_name,
        source_kind: &resource.source_kind,
        source_identifier: &resource.source_identifier,
        canonical_root_uri: &resource.canonical_root_uri,
        projection_view_id: &resource.projection_view_id,
        resource_kind: &resource.resource_kind,
        source_host: resource.source_host.as_deref(),
        source_namespace: resource.source_namespace.as_deref(),
        source_repo: resource.source_repo.as_deref(),
        source_ref: resource.source_ref.as_deref(),
        canonical_strategy_version: &resource.canonical_strategy_version,
        status,
        last_snapshot_id: resource.last_snapshot_id.as_deref(),
    })?;
    Ok(())
}

pub fn append_cli_audit(
    metadata: &MetadataStore,
    cli: &Cli,
    event_type: &str,
    subject_uri: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    metadata.append_audit(&AuditEventRecord {
        account_id: &cli.account_id,
        user_id: &cli.user_id,
        agent_id: Some(&cli.agent_id),
        projection_view_id: Some(&resource_projection_view_id(&cli.account_id, &cli.user_id)),
        event_type,
        subject_uri: Some(subject_uri),
        actor: Some("cli"),
        details_json: Some("{\"result\":\"ok\"}"),
    })?;
    Ok(())
}

pub fn spawn_resource_worker(
    cli: &Cli,
    command: &str,
    task_key: &str,
    resource_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut child = std::process::Command::new(std::env::current_exe()?);
    child
        .arg("--workspace-root")
        .arg(&cli.workspace_root)
        .arg("--account-id")
        .arg(&cli.account_id)
        .arg("--user-id")
        .arg(&cli.user_id)
        .arg("--agent-id")
        .arg(&cli.agent_id)
        .arg(command)
        .arg("--task-key")
        .arg(task_key)
        .arg("--resource-id")
        .arg(resource_id)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    child.spawn()?;
    Ok(())
}

pub async fn execute_resource_task<F, Fut>(
    metadata: &Arc<MetadataStore>,
    cli: &Cli,
    identity: &mfs_types::IdentityContext,
    task_key: &str,
    resource_id: &str,
    event_type: &str,
    worker: F,
) -> Result<(), Box<dyn std::error::Error>>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<(String, String), Box<dyn std::error::Error>>>,
{
    let max_attempts = 2_u32;
    for attempt in 1..=max_attempts {
        upsert_semantic_task(
            metadata,
            cli,
            task_key,
            "running",
            Some(resource_id),
            None,
            attempt,
            max_attempts,
            if attempt < max_attempts {
                "retrying"
            } else {
                "not_needed"
            },
            None,
        )?;

        match worker().await {
            Ok((root_uri, mode)) => {
                upsert_semantic_task(
                    metadata,
                    cli,
                    task_key,
                    "completed",
                    Some(resource_id),
                    None,
                    attempt,
                    max_attempts,
                    "not_needed",
                    Some(&mode),
                )?;
                update_resource_status(metadata, resource_id, "ready")?;
                append_cli_audit(metadata, cli, event_type, &root_uri)?;
                return Ok(());
            }
            Err(error) => {
                let error_text = error.to_string();
                upsert_semantic_task(
                    metadata,
                    cli,
                    task_key,
                    if attempt < max_attempts {
                        "retrying"
                    } else {
                        "failed"
                    },
                    Some(resource_id),
                    Some(&error_text),
                    attempt,
                    max_attempts,
                    if attempt < max_attempts {
                        "retryable"
                    } else {
                        "exhausted"
                    },
                    None,
                )?;
                if attempt >= max_attempts {
                    update_resource_status(metadata, resource_id, "failed")?;
                    return Err(error);
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        }
    }

    let _ = identity;
    Ok(())
}
