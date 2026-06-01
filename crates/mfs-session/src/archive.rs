use std::path::{Path, PathBuf};

use tokio::fs;

use crate::{SessionError, StoredMessage};

pub async fn archive_messages(
    workspace_root: &Path,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
    session_id: &str,
    archive_index: u32,
    messages: &[StoredMessage],
) -> Result<(String, PathBuf), SessionError> {
    let archive_uri =
        format!("mfs://session/{agent_id}/{session_id}/history/archive_{archive_index:03}");
    let archive_path = workspace_root
        .join("tenants")
        .join(account_id)
        .join(user_id)
        .join("session")
        .join(agent_id)
        .join(session_id)
        .join("history")
        .join(format!("archive_{archive_index:03}"));

    fs::create_dir_all(&archive_path)
        .await
        .map_err(|source| SessionError::io("create archive directory", &archive_path, source))?;

    let body = messages
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .map_err(SessionError::Serde)?
        .join("\n");
    let messages_path = archive_path.join("messages.jsonl");
    fs::write(&messages_path, format!("{body}\n"))
        .await
        .map_err(|source| SessionError::io("write archive messages", &messages_path, source))?;

    Ok((archive_uri, archive_path))
}
