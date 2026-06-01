use mfs_metadata::MetadataStore;
use mfs_types::MfsError;

use crate::{
    ConversationTurn, LlmAssist, TurnRole,
    consolidation::{ConsolidationResult, consolidate_and_persist},
    t2h::{T2hPipelineResult, run_t2h_pipeline},
};

pub struct ArchiveMemoryCommitInput<'a> {
    pub account_id: &'a str,
    pub user_id: &'a str,
    pub agent_id: &'a str,
    pub session_id: &'a str,
    pub archive_name: &'a str,
    pub messages: &'a [(String, String)],
}

pub struct ArchiveMemoryCommitOutput {
    pub conversation_turns: Vec<ConversationTurn>,
    pub consolidation_result: ConsolidationResult,
    pub t2h_result: T2hPipelineResult,
}

/// Normalize archived session messages and run the canonical memory-domain
/// commit steps: metadata consolidation followed by T2H.
pub async fn run_archive_memory_commit(
    metadata: &MetadataStore,
    input: &ArchiveMemoryCommitInput<'_>,
    llm: &LlmAssist,
) -> Result<ArchiveMemoryCommitOutput, MfsError> {
    let existing_turns = metadata
        .get_turns_by_session(input.session_id)
        .map_err(|source| MfsError::Internal {
            message: format!("load existing metadata turns: {source}"),
        })?;

    let conversation_turns = build_archive_conversation_turns(
        input.session_id,
        input.user_id,
        input.archive_name,
        existing_turns.len() as i64,
        input.messages,
    );

    let consolidation_result = consolidate_and_persist(
        metadata,
        input.account_id,
        input.user_id,
        input.agent_id,
        input.session_id,
        None,
        &conversation_turns,
        llm,
    )
    .await?;

    let t2h_result = run_t2h_pipeline(
        metadata,
        input.account_id,
        input.user_id,
        input.agent_id,
        input.session_id,
        &conversation_turns,
        llm,
    )
    .await;

    Ok(ArchiveMemoryCommitOutput {
        conversation_turns,
        consolidation_result,
        t2h_result,
    })
}

pub fn build_archive_conversation_turns(
    session_id: &str,
    user_id: &str,
    archive_name: &str,
    seq_offset: i64,
    messages: &[(String, String)],
) -> Vec<ConversationTurn> {
    messages
        .iter()
        .enumerate()
        .map(|(index, (role, content))| ConversationTurn {
            turn_id: format!("{session_id}:{archive_name}:turn_{:03}", index + 1),
            turn_seq: seq_offset + index as i64 + 1,
            session_id: session_id.to_owned(),
            user_id: user_id.to_owned(),
            role: TurnRole::from_str(role),
            content_text: content.clone(),
            token_count: (content.len() / 4).max(1),
            created_at: format!(
                "2026-01-01T00:{:02}:{:02}Z",
                ((seq_offset as usize + index) / 60) % 60,
                (seq_offset as usize + index) % 60
            ),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_archive_conversation_turns_with_existing_sequence_offset() {
        let messages = vec![
            ("user".to_owned(), "please use the shared router".to_owned()),
            ("assistant".to_owned(), "implemented".to_owned()),
            ("observation".to_owned(), "ran tests".to_owned()),
        ];

        let turns =
            build_archive_conversation_turns("session-1", "alice", "archive_002", 7, &messages);

        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0].turn_id, "session-1:archive_002:turn_001");
        assert_eq!(turns[0].turn_seq, 8);
        assert_eq!(turns[0].role, crate::TurnRole::User);
        assert_eq!(turns[1].role, crate::TurnRole::Assistant);
        assert_eq!(turns[2].role, crate::TurnRole::Tool);
        assert_eq!(turns[2].created_at, "2026-01-01T00:00:09Z");
    }
}
