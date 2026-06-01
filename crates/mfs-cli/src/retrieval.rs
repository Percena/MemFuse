use crate::format::print_retrieval_result;
use crate::helpers::CliState;
use mfs_retrieval::RetrievalEngine;

pub async fn handle_find(
    state: &CliState,
    query: &str,
    target: &Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let fs = crate::helpers::resolved_fs(cli, target.as_deref()).await?;
    let identity = mfs_types::IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let retrieval = RetrievalEngine::from_workspace(
        &cli.workspace_root,
        &identity,
        fs.projection_root(),
        fs.projection_uri(),
    )
    .await?;
    let result = retrieval.find(query, target.as_deref()).await?;
    print_retrieval_result(&result);
    Ok(())
}

pub async fn handle_grep(
    state: &CliState,
    query: &str,
    target: &Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let fs = crate::helpers::resolved_fs(cli, target.as_deref()).await?;
    let retrieval =
        RetrievalEngine::from_projection(fs.projection_root(), fs.projection_uri()).await?;
    let result = retrieval.grep(query, target.as_deref(), None).await?;
    print_retrieval_result(&result);
    Ok(())
}

pub async fn handle_search(
    state: &CliState,
    query: &str,
    target: &Option<String>,
    session_context: &Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let fs = crate::helpers::resolved_fs(cli, target.as_deref()).await?;
    let identity = mfs_types::IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let retrieval = RetrievalEngine::from_workspace(
        &cli.workspace_root,
        &identity,
        fs.projection_root(),
        fs.projection_uri(),
    )
    .await?;
    let result = retrieval
        .search(query, target.as_deref(), session_context.as_deref())
        .await?;
    print_retrieval_result(&result);
    Ok(())
}
