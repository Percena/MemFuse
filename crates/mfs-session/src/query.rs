use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::{SessionError, StoredMessage};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub account_id: String,
    pub user_id: String,
    pub agent_id: String,
    pub message_count: usize,
    pub commit_count: u32,
    pub last_commit_archive_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveAbstractView {
    pub archive_id: String,
    pub abstract_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMessageView {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionContextView {
    pub latest_archive_overview: String,
    pub pre_archive_abstracts: Vec<ArchiveAbstractView>,
    pub messages: Vec<SessionMessageView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionArchiveView {
    pub archive_id: String,
    pub abstract_text: String,
    pub overview_text: String,
    pub messages: Vec<SessionMessageView>,
}

pub async fn list_sessions(
    workspace_root: &Path,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
    pending_sessions: &[(String, usize)],
) -> Result<Vec<SessionSummary>, SessionError> {
    let session_root = workspace_root
        .join("tenants")
        .join(account_id)
        .join(user_id)
        .join("session")
        .join(agent_id);

    if !fs::try_exists(&session_root)
        .await
        .map_err(|source| SessionError::io("check session root", &session_root, source))?
    {
        return Ok(Vec::new());
    }

    let mut entries = fs::read_dir(&session_root)
        .await
        .map_err(|source| SessionError::io("read session root", &session_root, source))?;
    let mut sessions = Vec::new();

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|source| SessionError::io("iterate session root", &session_root, source))?
    {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .await
            .map_err(|source| SessionError::io("inspect session root entry", &path, source))?;
        if !file_type.is_dir() {
            continue;
        }

        let session_id = entry.file_name().to_string_lossy().into_owned();
        let pending_messages = pending_sessions
            .iter()
            .find_map(|(candidate, count)| (candidate == &session_id).then_some(*count))
            .unwrap_or_default();
        sessions.push(
            load_session_summary(
                &path,
                account_id,
                user_id,
                agent_id,
                &session_id,
                pending_messages,
            )
            .await?,
        );
    }

    sessions.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    Ok(sessions)
}

pub async fn load_session_summary(
    session_root: &Path,
    account_id: &str,
    user_id: &str,
    agent_id: &str,
    session_id: &str,
    pending_messages: usize,
) -> Result<SessionSummary, SessionError> {
    let history_root = session_root.join("history");
    let (commit_count, last_commit_archive_id) = read_archive_summary(&history_root).await?;

    Ok(SessionSummary {
        session_id: session_id.to_owned(),
        account_id: account_id.to_owned(),
        user_id: user_id.to_owned(),
        agent_id: agent_id.to_owned(),
        message_count: pending_messages,
        commit_count,
        last_commit_archive_id,
    })
}

async fn read_archive_summary(history_root: &Path) -> Result<(u32, Option<String>), SessionError> {
    if !fs::try_exists(history_root)
        .await
        .map_err(|source| SessionError::io("check history root", history_root, source))?
    {
        return Ok((0, None));
    }

    let mut entries = fs::read_dir(history_root)
        .await
        .map_err(|source| SessionError::io("read history root", history_root, source))?;
    let mut archive_ids = Vec::new();

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|source| SessionError::io("iterate history root", history_root, source))?
    {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .await
            .map_err(|source| SessionError::io("inspect history root entry", &path, source))?;
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("archive_") {
            archive_ids.push(name);
        }
    }

    archive_ids.sort();
    let commit_count = archive_ids.len() as u32;
    let last_commit_archive_id = archive_ids.pop();
    Ok((commit_count, last_commit_archive_id))
}

pub async fn load_session_context(
    session_root: &Path,
    pending_messages: &[StoredMessage],
    token_budget: usize,
) -> Result<SessionContextView, SessionError> {
    let history_root = session_root.join("history");
    let completed_archives = completed_archive_ids(&history_root).await?;

    let latest_archive_overview = if let Some(latest_archive_id) = completed_archives.last() {
        let overview =
            fs::read_to_string(history_root.join(latest_archive_id).join(".overview.md"))
                .await
                .map_err(|source| {
                    SessionError::io(
                        "read archive overview",
                        &history_root.join(latest_archive_id).join(".overview.md"),
                        source,
                    )
                })?;
        if overview.len() <= token_budget {
            overview
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let latest_archive_id = completed_archives.last().cloned();
    let mut pre_archive_abstracts = Vec::new();
    for archive_id in completed_archives
        .into_iter()
        .filter(|archive_id| Some(archive_id.clone()) != latest_archive_id)
        .rev()
    {
        let abstract_text = fs::read_to_string(history_root.join(&archive_id).join(".abstract.md"))
            .await
            .map_err(|source| {
                SessionError::io(
                    "read archive abstract",
                    &history_root.join(&archive_id).join(".abstract.md"),
                    source,
                )
            })?;
        pre_archive_abstracts.push(ArchiveAbstractView {
            archive_id,
            abstract_text,
        });
    }

    let messages = pending_messages
        .iter()
        .map(|message| SessionMessageView {
            role: message.role.clone(),
            content: message.content.clone(),
        })
        .collect();

    Ok(SessionContextView {
        latest_archive_overview,
        pre_archive_abstracts,
        messages,
    })
}

pub async fn load_session_archive(
    session_root: &Path,
    archive_id: &str,
) -> Result<SessionArchiveView, SessionError> {
    let archive_root = session_root.join("history").join(archive_id);
    if !fs::try_exists(&archive_root)
        .await
        .map_err(|source| SessionError::io("check archive root", &archive_root, source))?
    {
        return Err(SessionError::NotFound(archive_id.to_owned()));
    }

    let abstract_text = fs::read_to_string(archive_root.join(".abstract.md"))
        .await
        .map_err(|source| {
            SessionError::io(
                "read archive abstract",
                &archive_root.join(".abstract.md"),
                source,
            )
        })?;
    let overview_text = fs::read_to_string(archive_root.join(".overview.md"))
        .await
        .map_err(|source| {
            SessionError::io(
                "read archive overview",
                &archive_root.join(".overview.md"),
                source,
            )
        })?;
    let messages = read_archive_messages(&archive_root.join("messages.jsonl")).await?;

    Ok(SessionArchiveView {
        archive_id: archive_id.to_owned(),
        abstract_text,
        overview_text,
        messages,
    })
}

pub async fn completed_archive_ids(history_root: &Path) -> Result<Vec<String>, SessionError> {
    if !fs::try_exists(history_root)
        .await
        .map_err(|source| SessionError::io("check history root", history_root, source))?
    {
        return Ok(Vec::new());
    }

    let mut entries = fs::read_dir(history_root)
        .await
        .map_err(|source| SessionError::io("read history root", history_root, source))?;
    let mut archive_ids = Vec::new();

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|source| SessionError::io("iterate history root", history_root, source))?
    {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .await
            .map_err(|source| SessionError::io("inspect history root entry", &path, source))?;
        if !file_type.is_dir() {
            continue;
        }
        let archive_id = entry.file_name().to_string_lossy().into_owned();
        if !archive_id.starts_with("archive_") {
            continue;
        }
        let done_path = path.join(".done");
        if fs::try_exists(&done_path)
            .await
            .map_err(|source| SessionError::io("check archive done marker", &done_path, source))?
        {
            archive_ids.push(archive_id);
        }
    }

    archive_ids.sort();
    Ok(archive_ids)
}

async fn read_archive_messages(path: &Path) -> Result<Vec<SessionMessageView>, SessionError> {
    let content = fs::read_to_string(path)
        .await
        .map_err(|source| SessionError::io("read archive messages", path, source))?;
    let mut messages = Vec::new();

    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        let value = serde_json::from_str::<serde_json::Value>(line).map_err(SessionError::Serde)?;
        messages.push(SessionMessageView {
            role: value
                .get("role")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown")
                .to_owned(),
            content: value
                .get("content")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_owned(),
        });
    }

    Ok(messages)
}

pub async fn read_live_messages(session_root: &Path) -> Result<Vec<StoredMessage>, SessionError> {
    let messages_path = session_root.join("messages.jsonl");
    if !fs::try_exists(&messages_path)
        .await
        .map_err(|source| SessionError::io("check live messages", &messages_path, source))?
    {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&messages_path)
        .await
        .map_err(|source| SessionError::io("read live messages", &messages_path, source))?;
    let mut messages = Vec::new();

    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        messages.push(serde_json::from_str::<StoredMessage>(line).map_err(SessionError::Serde)?);
    }

    Ok(messages)
}
