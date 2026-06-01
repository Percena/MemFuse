use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::SessionError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedoMarker {
    pub task_id: String,
    pub archive_uri: String,
    pub archive_path: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: String,
    pub session_id: String,
}

pub async fn write_redo_marker(
    workspace_root: &Path,
    marker: &RedoMarker,
) -> Result<PathBuf, SessionError> {
    let redo_dir = workspace_root.join("_system").join("redo");
    fs::create_dir_all(&redo_dir)
        .await
        .map_err(|source| SessionError::io("create redo directory", &redo_dir, source))?;
    let redo_path = redo_dir.join(format!("{}.json", marker.task_id));
    fs::write(
        &redo_path,
        serde_json::to_vec_pretty(marker).map_err(SessionError::Serde)?,
    )
    .await
    .map_err(|source| SessionError::io("write redo marker", &redo_path, source))?;
    Ok(redo_path)
}

pub async fn load_redo_markers(workspace_root: &Path) -> Result<Vec<RedoMarker>, SessionError> {
    let redo_dir = workspace_root.join("_system").join("redo");
    if !fs::try_exists(&redo_dir)
        .await
        .map_err(|source| SessionError::io("check redo directory", &redo_dir, source))?
    {
        return Ok(Vec::new());
    }

    let mut entries = fs::read_dir(&redo_dir)
        .await
        .map_err(|source| SessionError::io("read redo directory", &redo_dir, source))?;
    let mut markers = Vec::new();

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|source| SessionError::io("iterate redo directory", &redo_dir, source))?
    {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let content = fs::read(&path)
            .await
            .map_err(|source| SessionError::io("read redo marker", &path, source))?;
        let marker = serde_json::from_slice::<RedoMarker>(&content).map_err(SessionError::Serde)?;
        markers.push(marker);
    }

    markers.sort_by(|left, right| left.task_id.cmp(&right.task_id));
    Ok(markers)
}
