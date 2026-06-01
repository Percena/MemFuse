use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mfs_metadata::MetadataStore;
use mfs_ops::run_due_resource_watches;
use mfs_types::IdentityContext;

use crate::format::WatchDaemonStatus;

pub async fn start_watch_daemon(
    cli: &crate::Cli,
    poll_ms: u64,
) -> Result<u32, Box<dyn std::error::Error>> {
    if let Some(status) = load_watch_daemon_status(&cli.workspace_root).await? {
        if process_alive(status.pid) {
            return Ok(status.pid);
        }
    }

    remove_stop_sentinel(&cli.workspace_root).await?;
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
        .arg("__watch-daemon-run")
        .arg("--poll-ms")
        .arg(poll_ms.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = child.spawn()?;
    let pid = child.id();

    for _ in 0..100 {
        if let Some(status) = load_watch_daemon_status(&cli.workspace_root).await? {
            if status.running && status.pid == pid {
                return Ok(pid);
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    Err(std::io::Error::other("watch daemon did not report ready").into())
}

pub async fn stop_watch_daemon(
    workspace_root: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(status) = load_watch_daemon_status(workspace_root).await? else {
        return Ok(());
    };
    if !process_alive(status.pid) {
        write_watch_daemon_status(
            workspace_root,
            &WatchDaemonStatus {
                running: false,
                stopped_at_ms: Some(now_millis()),
                ..status
            },
        )
        .await?;
        return Ok(());
    }

    create_stop_sentinel(workspace_root).await?;
    for _ in 0..200 {
        if !process_alive(status.pid) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let _ = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(status.pid.to_string())
        .status();
    Ok(())
}

/// Daemon loop using a pre-shared Arc<MetadataStore> — avoids per-tick open_at.
pub async fn run_watch_daemon_loop_with_metadata(
    metadata: &Arc<MetadataStore>,
    workspace_root: &std::path::Path,
    identity: &IdentityContext,
    poll_ms: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut status = WatchDaemonStatus {
        pid: std::process::id(),
        running: true,
        poll_ms,
        started_at_ms: now_millis(),
        stopped_at_ms: None,
        last_tick_at_ms: None,
        total_ticks: 0,
        total_runs: 0,
        last_run_count: 0,
    };
    remove_stop_sentinel(workspace_root).await?;
    write_watch_daemon_status(workspace_root, &status).await?;

    loop {
        if watch_daemon_stop_path(workspace_root).exists() {
            remove_stop_sentinel(workspace_root).await?;
            status.running = false;
            status.stopped_at_ms = Some(now_millis());
            write_watch_daemon_status(workspace_root, &status).await?;
            return Ok(());
        }

        let runs = run_due_resource_watches(metadata, workspace_root, identity, 100).await?;
        status.last_tick_at_ms = Some(now_millis());
        status.total_ticks += 1;
        status.last_run_count = runs.len() as u64;
        status.total_runs += runs.len() as u64;
        write_watch_daemon_status(workspace_root, &status).await?;
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    }
}

pub async fn load_watch_daemon_status(
    workspace_root: &std::path::Path,
) -> Result<Option<WatchDaemonStatus>, Box<dyn std::error::Error>> {
    let path = watch_daemon_status_path(workspace_root);
    if !tokio::fs::try_exists(&path).await? {
        return Ok(None);
    }
    let raw = tokio::fs::read_to_string(&path).await?;
    Ok(WatchDaemonStatus::parse(&raw).map(|mut status| {
        if status.running && !process_alive(status.pid) {
            status.running = false;
        }
        status
    }))
}

async fn write_watch_daemon_status(
    workspace_root: &std::path::Path,
    status: &WatchDaemonStatus,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = watch_daemon_status_path(workspace_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, status.render()).await?;
    Ok(())
}

async fn create_stop_sentinel(
    workspace_root: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = watch_daemon_stop_path(workspace_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, "stop\n").await?;
    Ok(())
}

async fn remove_stop_sentinel(
    workspace_root: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = watch_daemon_stop_path(workspace_root);
    if tokio::fs::try_exists(&path).await? {
        tokio::fs::remove_file(path).await?;
    }
    Ok(())
}

fn watch_daemon_status_path(workspace_root: &std::path::Path) -> PathBuf {
    workspace_root
        .join("_system")
        .join("watch_daemon")
        .join("status.txt")
}

fn watch_daemon_stop_path(workspace_root: &std::path::Path) -> PathBuf {
    workspace_root
        .join("_system")
        .join("watch_daemon")
        .join("stop")
}

fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}
