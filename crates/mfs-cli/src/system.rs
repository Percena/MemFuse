use mfs_ops::{observer_status, rebuild_metadata_entries, refresh_projection, system_status};
use mfs_types::IdentityContext;

use crate::format::print_audit_record;
use crate::helpers::CliState;

pub async fn handle_observer_status(state: &CliState) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let summary = observer_status(&cli.workspace_root)?;
    println!(
        "runtime.summary_provider={}",
        summary.runtime.summary_provider
    );
    println!(
        "runtime.embedding_provider={}",
        summary.runtime.embedding_provider
    );
    println!("runtime.summary_model={}", summary.runtime.summary_model);
    println!("runtime.chat_model={}", summary.runtime.chat_model);
    println!(
        "runtime.embedding_model={}",
        summary.runtime.embedding_model
    );
    println!(
        "runtime.summary_concurrency={}",
        summary.runtime.summary_concurrency
    );
    println!(
        "runtime.openai_compatible_env={}",
        summary.runtime.openai_compatible_env
    );
    println!(
        "semantic.total_documents={}",
        summary.semantic.total_documents
    );
    println!(
        "semantic.resource_documents={}",
        summary.semantic.resource_documents
    );
    println!(
        "semantic.memory_documents={}",
        summary.semantic.memory_documents
    );
    println!(
        "semantic.skill_documents={}",
        summary.semantic.skill_documents
    );
    println!(
        "semantic.embedding_dimension={}",
        summary.semantic.embedding_dimension
    );
    Ok(())
}

pub async fn handle_system_status(state: &CliState) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let summary = system_status(&state.metadata, &cli.workspace_root, &identity).await?;
    println!("workspace_root={}", summary.workspace_root);
    println!("resources.total={}", summary.resources.total);
    println!("resources.ready={}", summary.resources.ready);
    println!("resources.processing={}", summary.resources.processing);
    println!("resources.failed={}", summary.resources.failed);
    println!("metadata_tasks.total={}", summary.metadata_tasks.total);
    println!("metadata_tasks.pending={}", summary.metadata_tasks.pending);
    println!("metadata_tasks.running={}", summary.metadata_tasks.running);
    println!(
        "metadata_tasks.completed={}",
        summary.metadata_tasks.completed
    );
    println!("metadata_tasks.failed={}", summary.metadata_tasks.failed);
    println!("session_tasks.total={}", summary.session_tasks.total);
    println!("session_tasks.pending={}", summary.session_tasks.pending);
    println!("session_tasks.running={}", summary.session_tasks.running);
    println!(
        "session_tasks.completed={}",
        summary.session_tasks.completed
    );
    println!("session_tasks.failed={}", summary.session_tasks.failed);
    println!("snapshots.total={}", summary.snapshots_total);
    Ok(())
}

pub async fn handle_rebuild(state: &CliState) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let fs = crate::helpers::configured_fs(cli).await?;
    let rebuild = rebuild_metadata_entries(
        &state.metadata,
        &identity,
        fs.projection_root(),
        fs.projection_uri(),
    )?;
    println!("indexed_paths={}", rebuild.indexed_paths);
    println!("projection_uri={}", rebuild.projection_uri);
    Ok(())
}

pub async fn handle_refresh(state: &CliState) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let source_path = cli
        .source_path
        .as_ref()
        .ok_or_else(|| std::io::Error::other("--source-path is required for refresh"))?;
    let target_uri = cli
        .target_uri
        .as_deref()
        .ok_or_else(|| std::io::Error::other("--target-uri is required for refresh"))?;
    let refresh = refresh_projection(
        &state.metadata,
        &cli.workspace_root,
        &identity,
        &cli.source_kind,
        source_path.to_str().expect("source path utf-8"),
        target_uri,
    )
    .await?;
    println!("snapshot_id={}", refresh.snapshot_id);
    println!("indexed_paths={}", refresh.indexed_paths);
    println!("projection_uri={}", refresh.projection_uri);
    Ok(())
}

pub fn handle_snapshot_list(state: &CliState) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    for snapshot in state.metadata.list_snapshots(
        &cli.account_id,
        &cli.user_id,
        Some(&crate::helpers::resource_projection_view_id(
            &cli.account_id,
            &cli.user_id,
        )),
        50,
    )? {
        println!("snapshot_id={}", snapshot.snapshot_id);
        println!("root_uri={}", snapshot.root_uri);
    }
    Ok(())
}

pub fn handle_audit(state: &CliState, limit: usize) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    for record in state
        .metadata
        .list_audit(&cli.account_id, &cli.user_id, limit)?
    {
        print_audit_record(&record);
    }
    Ok(())
}

pub fn handle_link(
    state: &CliState,
    from_uri: &str,
    to_uri: &str,
    relation_type: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    state
        .metadata
        .upsert_relation(&mfs_metadata::RelationRecord {
            account_id: &cli.account_id,
            user_id: &cli.user_id,
            agent_id: Some(&cli.agent_id),
            from_uri,
            to_uri,
            relation_type,
        })?;
    crate::helpers::append_cli_audit(&state.metadata, cli, "relation.link", from_uri)?;
    println!("ok=true");
    Ok(())
}

pub fn handle_unlink(
    state: &CliState,
    from_uri: &str,
    to_uri: &str,
    relation_type: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    state.metadata.remove_relation(
        &cli.account_id,
        &cli.user_id,
        from_uri,
        to_uri,
        relation_type,
    )?;
    crate::helpers::append_cli_audit(&state.metadata, cli, "relation.unlink", from_uri)?;
    println!("ok=true");
    Ok(())
}

pub fn handle_relations(
    state: &CliState,
    uri: &str,
    limit: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    for relation in state
        .metadata
        .list_relations(&cli.account_id, &cli.user_id, uri, limit)?
    {
        println!("relation_type={}", relation.relation_type);
        if relation.from_uri == *uri {
            println!("direction=outbound");
            println!("peer_uri={}", relation.to_uri);
        } else {
            println!("direction=inbound");
            println!("peer_uri={}", relation.from_uri);
        }
    }
    Ok(())
}
