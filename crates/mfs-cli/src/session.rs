use crate::format::{print_session_archive, print_session_context, print_session_summary};
use crate::helpers::CliState;
use mfs_session::SessionEngine;

pub async fn handle_sessions_list(state: &CliState) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let engine = SessionEngine::open(&cli.workspace_root).await?;
    let sessions = engine
        .list_sessions(&cli.account_id, &cli.user_id, &cli.agent_id)
        .await?;
    for session in &sessions {
        print_session_summary(session);
    }
    Ok(())
}

pub async fn handle_session_get(
    state: &CliState,
    session_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let engine = SessionEngine::open(&cli.workspace_root).await?;
    let session = engine.get_session(session_id).await?;
    print_session_summary(&session);
    Ok(())
}

pub async fn handle_session_context(
    state: &CliState,
    session_id: &str,
    token_budget: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let engine = SessionEngine::open(&cli.workspace_root).await?;
    let context = engine.get_session_context(session_id, token_budget).await?;
    print_session_context(&context);
    Ok(())
}

pub async fn handle_session_archive(
    state: &CliState,
    session_id: &str,
    archive_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let engine = SessionEngine::open(&cli.workspace_root).await?;
    let archive = engine.get_session_archive(session_id, archive_id).await?;
    print_session_archive(&archive);
    Ok(())
}

pub async fn handle_session_delete(
    state: &CliState,
    session_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let engine = SessionEngine::open(&cli.workspace_root).await?;
    engine.delete_session(session_id).await?;
    println!("deleted=true");
    Ok(())
}
