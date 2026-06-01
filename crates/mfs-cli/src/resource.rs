use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use mfs_ops::{
    WaitTaskOutcome, complete_registered_resource_ingest, disable_resource_watch,
    export_resource_pack, import_resource_pack, ingest_skill, list_resource_watch_statuses,
    list_skills, mkdir_owned_path, move_owned_path, prepare_inline_resource_ingest,
    prepare_resource_ingest, rebuild_registered_resource, refresh_registered_resource,
    register_resource_watch, remove_owned_path, run_due_resource_watches, run_resource_watch,
    run_resource_watch_loop, wait_for_task_completion, write_owned_path,
};
use mfs_session::SessionEngine;
use mfs_types::IdentityContext;

use crate::format::{
    print_metadata_task_record, print_session_task_record, print_task_list_metadata_record,
    print_task_list_session_record, print_watch_daemon_status,
};
use crate::helpers::CliState;

pub async fn handle_ls(state: &CliState, uri: &str) -> Result<(), Box<dyn std::error::Error>> {
    let fs = crate::helpers::configured_fs(&state.cli).await?;
    for entry in fs.ls(uri).await? {
        println!("{}", entry.name);
    }
    Ok(())
}

pub async fn handle_tree(
    state: &CliState,
    uri: &str,
    depth: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let fs = crate::helpers::configured_fs(&state.cli).await?;
    let tree = fs.tree(uri, depth).await?;
    crate::format::print_tree(&tree, 0);
    Ok(())
}

pub async fn handle_stat(state: &CliState, uri: &str) -> Result<(), Box<dyn std::error::Error>> {
    let fs = crate::helpers::configured_fs(&state.cli).await?;
    let stat = fs.stat(uri).await?;
    println!("path={}", stat.path.display());
    println!("is_dir={}", stat.is_dir);
    println!("size_bytes={}", stat.size_bytes);
    Ok(())
}

pub async fn handle_abstract(
    state: &CliState,
    uri: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let fs = crate::helpers::configured_fs(&state.cli).await?;
    print!("{}", fs.abstract_text(uri).await?);
    Ok(())
}

pub async fn handle_overview(
    state: &CliState,
    uri: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let fs = crate::helpers::configured_fs(&state.cli).await?;
    print!("{}", fs.overview_text(uri).await?);
    Ok(())
}

pub async fn handle_read(state: &CliState, uri: &str) -> Result<(), Box<dyn std::error::Error>> {
    let fs = crate::helpers::configured_fs(&state.cli).await?;
    print!("{}", fs.read(uri).await?);
    Ok(())
}

pub async fn handle_glob(
    state: &CliState,
    uri: &str,
    pattern: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let fs = crate::helpers::resolved_fs(&state.cli, Some(uri)).await?;
    for matched_uri in fs.glob(uri, pattern).await? {
        println!("{matched_uri}");
    }
    Ok(())
}

pub async fn handle_mkdir(state: &CliState, uri: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let result = mkdir_owned_path(&state.metadata, &cli.workspace_root, &identity, uri).await?;
    crate::helpers::append_cli_audit(&state.metadata, cli, "mkdir", uri)?;
    println!("uri={}", result.primary_uri);
    println!("indexed_paths={}", result.indexed_paths);
    Ok(())
}

pub async fn handle_write(
    state: &CliState,
    uri: &str,
    content: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let result = write_owned_path(
        &state.metadata,
        &cli.workspace_root,
        &identity,
        uri,
        content,
    )
    .await?;
    crate::helpers::append_cli_audit(&state.metadata, cli, "write", uri)?;
    println!("uri={}", result.primary_uri);
    println!("indexed_paths={}", result.indexed_paths);
    Ok(())
}

pub async fn handle_mv(
    state: &CliState,
    from_uri: &str,
    to_uri: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let result = move_owned_path(
        &state.metadata,
        &cli.workspace_root,
        &identity,
        from_uri,
        to_uri,
    )
    .await?;
    crate::helpers::append_cli_audit(&state.metadata, cli, "mv", to_uri)?;
    println!("uri={}", result.primary_uri);
    println!("indexed_paths={}", result.indexed_paths);
    Ok(())
}

pub async fn handle_rm(state: &CliState, uri: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let result = remove_owned_path(&state.metadata, &cli.workspace_root, &identity, uri).await?;
    crate::helpers::append_cli_audit(&state.metadata, cli, "rm", uri)?;
    println!("uri={}", result.primary_uri);
    println!("indexed_paths={}", result.indexed_paths);
    Ok(())
}

pub async fn handle_skills_list(state: &CliState) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let skills = list_skills(&cli.workspace_root, &identity).await?;
    for skill in &skills {
        println!("skill_name={}", skill.skill_name);
        println!("skill_uri={}", skill.skill_uri);
        if let Some(description) = &skill.description {
            println!("description={description}");
        }
    }
    Ok(())
}

pub async fn handle_add_skill(
    state: &CliState,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let result = ingest_skill(&state.metadata, &cli.workspace_root, &identity, path).await?;
    println!("skill_name={}", result.skill_name);
    println!("skill_uri={}", result.skill_uri);
    println!("indexed_documents={}", result.indexed_documents);
    println!("mode={}", result.mode);
    Ok(())
}

pub async fn handle_add_resource(
    state: &CliState,
    name: &Option<String>,
    file_name: &Option<String>,
    content: &Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);

    let prepared = match (file_name.as_deref(), content.as_deref()) {
        (Some(file_name), Some(content)) => {
            prepare_inline_resource_ingest(
                &state.metadata,
                &cli.workspace_root,
                &identity,
                file_name,
                content,
                name.as_deref(),
            )
            .await?
        }
        (None, None) => {
            let source_path = cli.source_path.as_ref().ok_or_else(|| {
                std::io::Error::other(
                    "--source-path is required when --file-name/--content are not set",
                )
            })?;
            prepare_resource_ingest(
                &state.metadata,
                &cli.workspace_root,
                &identity,
                &cli.source_kind,
                source_path.to_str().expect("source path utf-8"),
                name.as_deref(),
                None,
                None,
            )
            .await?
        }
        _ => {
            return Err(std::io::Error::other(
                "--file-name and --content must be provided together",
            )
            .into());
        }
    };
    let task_key = crate::helpers::semantic_task_key("ingest", &prepared.resource_id);
    crate::helpers::upsert_semantic_task(
        &state.metadata,
        cli,
        &task_key,
        "pending",
        Some(&format!("ingest {}", prepared.root_uri)),
        None,
        0,
        2,
        "queued",
        None,
    )?;
    crate::helpers::spawn_resource_worker(
        cli,
        "__complete-resource-ingest",
        &task_key,
        &prepared.resource_id,
    )?;
    println!("task_key={task_key}");
    println!("resource_id={}", prepared.resource_id);
    println!("logical_name={}", prepared.logical_name);
    println!("root_uri={}", prepared.root_uri);
    println!("state=pending");
    Ok(())
}

pub async fn handle_add_resources_batch(
    state: &CliState,
    paths: &[String],
    name: &Option<String>,
    branch: Option<&str>,
    revision: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let mut results = Vec::with_capacity(paths.len());

    for (index, source_path) in paths.iter().enumerate() {
        let logical_name = name.as_ref().map(|base| {
            if paths.len() == 1 {
                base.clone()
            } else {
                format!("{base}-{seq}", seq = index + 1)
            }
        });

        match prepare_resource_ingest(
            &state.metadata,
            &cli.workspace_root,
            &identity,
            &cli.source_kind,
            source_path,
            logical_name.as_deref(),
            branch,
            revision,
        )
        .await
        {
            Ok(prepared) => {
                let task_key = crate::helpers::semantic_task_key("ingest", &prepared.resource_id);
                crate::helpers::upsert_semantic_task(
                    &state.metadata,
                    cli,
                    &task_key,
                    "pending",
                    Some(&format!("ingest {}", prepared.root_uri)),
                    None,
                    0,
                    2,
                    "queued",
                    None,
                )?;
                crate::helpers::spawn_resource_worker(
                    cli,
                    "__complete-resource-ingest",
                    &task_key,
                    &prepared.resource_id,
                )?;
                results.push(format!(
                    "[{}] resource_id={} logical_name={} root_uri={} state=pending",
                    index + 1,
                    prepared.resource_id,
                    prepared.logical_name,
                    prepared.root_uri,
                ));
            }
            Err(err) => {
                results.push(format!("[{}] error={}", index + 1, err));
            }
        }
    }

    println!("batch_count={}", paths.len());
    println!(
        "succeeded={}",
        results.iter().filter(|r| !r.contains("error=")).count()
    );
    println!(
        "failed={}",
        results.iter().filter(|r| r.contains("error=")).count()
    );
    for result in &results {
        println!("{result}");
    }
    Ok(())
}

pub fn handle_resources_list(state: &CliState) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    for resource in
        state
            .metadata
            .list_resource_sources(&cli.account_id, &cli.user_id, 100, None)?
    {
        println!("resource_id={}", resource.resource_id);
        println!("logical_name={}", resource.logical_name);
        println!("root_uri={}", resource.canonical_root_uri);
        println!("source_kind={}", resource.source_kind);
        println!(
            "source_host={}",
            resource.source_host.as_deref().unwrap_or("none")
        );
        println!(
            "source_namespace={}",
            resource.source_namespace.as_deref().unwrap_or("none")
        );
        println!(
            "source_repo={}",
            resource.source_repo.as_deref().unwrap_or("none")
        );
        println!(
            "source_ref={}",
            resource.source_ref.as_deref().unwrap_or("none")
        );
        println!("status={}", resource.status);
    }
    Ok(())
}

pub fn handle_resource_refresh(
    state: &CliState,
    resource_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let resource = state
        .metadata
        .get_resource_source(resource_id)?
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "resource not found"))?;
    let task_key = crate::helpers::semantic_task_key("refresh", resource_id);
    crate::helpers::upsert_semantic_task(
        &state.metadata,
        cli,
        &task_key,
        "pending",
        Some(&format!("refresh {}", resource.canonical_root_uri)),
        None,
        0,
        2,
        "queued",
        None,
    )?;
    crate::helpers::update_resource_status(&state.metadata, resource_id, "processing")?;
    crate::helpers::spawn_resource_worker(
        cli,
        "__complete-resource-refresh",
        &task_key,
        resource_id,
    )?;
    println!("task_key={task_key}");
    println!("resource_id={resource_id}");
    println!("root_uri={}", resource.canonical_root_uri);
    println!("state=pending");
    Ok(())
}

pub fn handle_resource_rebuild(
    state: &CliState,
    resource_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let resource = state
        .metadata
        .get_resource_source(resource_id)?
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "resource not found"))?;
    let task_key = crate::helpers::semantic_task_key("rebuild", resource_id);
    crate::helpers::upsert_semantic_task(
        &state.metadata,
        cli,
        &task_key,
        "pending",
        Some(&format!("rebuild {}", resource.canonical_root_uri)),
        None,
        0,
        2,
        "queued",
        None,
    )?;
    crate::helpers::update_resource_status(&state.metadata, resource_id, "processing")?;
    crate::helpers::spawn_resource_worker(
        cli,
        "__complete-resource-rebuild",
        &task_key,
        resource_id,
    )?;
    println!("task_key={task_key}");
    println!("resource_id={resource_id}");
    println!("root_uri={}", resource.canonical_root_uri);
    println!("state=pending");
    Ok(())
}

pub async fn handle_resource_export(
    state: &CliState,
    resource_id: &str,
    output_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let manifest = export_resource_pack(
        &state.metadata,
        &cli.workspace_root,
        &identity,
        resource_id,
        output_path,
    )
    .await?;
    crate::helpers::append_cli_audit(
        &state.metadata,
        cli,
        "resource.export",
        &manifest.canonical_root_uri,
    )?;
    println!("output_path={}", output_path.display());
    println!("logical_name={}", manifest.logical_name);
    Ok(())
}

pub async fn handle_resource_import(
    state: &CliState,
    pack_path: &Path,
    name: &Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let prepared = import_resource_pack(
        &state.metadata,
        &cli.workspace_root,
        &identity,
        pack_path,
        name.as_deref(),
    )
    .await?;
    let task_key = crate::helpers::semantic_task_key("import", &prepared.resource_id);
    crate::helpers::upsert_semantic_task(
        &state.metadata,
        cli,
        &task_key,
        "pending",
        Some(&format!("import {}", prepared.root_uri)),
        None,
        0,
        2,
        "queued",
        None,
    )?;
    crate::helpers::spawn_resource_worker(
        cli,
        "__complete-resource-ingest",
        &task_key,
        &prepared.resource_id,
    )?;
    println!("task_key={task_key}");
    println!("resource_id={}", prepared.resource_id);
    println!("logical_name={}", prepared.logical_name);
    println!("root_uri={}", prepared.root_uri);
    println!("state=pending");
    Ok(())
}

pub fn handle_resource_watch(
    state: &CliState,
    resource_id: &str,
    interval_seconds: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let watch = register_resource_watch(&state.metadata, &identity, resource_id, interval_seconds)?;
    println!("resource_id={}", watch.resource_id);
    println!("interval_seconds={}", watch.interval_seconds);
    println!("enabled={}", watch.enabled);
    Ok(())
}

pub async fn handle_resource_watch_run(
    state: &CliState,
    resource_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let result =
        run_resource_watch(&state.metadata, &cli.workspace_root, &identity, resource_id).await?;
    crate::helpers::append_cli_audit(&state.metadata, cli, "resource.watch.run", &result.root_uri)?;
    println!("resource_id={}", result.resource_id);
    println!("refreshed={}", result.refreshed);
    println!("root_uri={}", result.root_uri);
    println!("snapshot_id={}", result.snapshot_id.unwrap_or_default());
    Ok(())
}

pub fn handle_resource_watch_disable(
    state: &CliState,
    resource_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let watch =
        disable_resource_watch(&state.metadata, &cli.account_id, &cli.user_id, resource_id)?;
    println!("resource_id={}", watch.resource_id);
    println!("enabled={}", watch.enabled);
    Ok(())
}

pub async fn handle_resource_watch_run_due(
    state: &CliState,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    for result in
        run_due_resource_watches(&state.metadata, &cli.workspace_root, &identity, 100).await?
    {
        crate::helpers::append_cli_audit(
            &state.metadata,
            cli,
            "resource.watch.run",
            &result.root_uri,
        )?;
        println!("resource_id={}", result.resource_id);
        println!("refreshed={}", result.refreshed);
        println!("root_uri={}", result.root_uri);
    }
    Ok(())
}

pub async fn handle_resource_watch_loop(
    state: &CliState,
    iterations: usize,
    sleep_ms: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let result = run_resource_watch_loop(
        &state.metadata,
        &cli.workspace_root,
        &identity,
        iterations,
        Duration::from_millis(sleep_ms),
        100,
    )
    .await?;
    println!("iterations={}", result.iterations);
    println!("total_runs={}", result.total_runs);
    for run in result.runs {
        crate::helpers::append_cli_audit(
            &state.metadata,
            cli,
            "resource.watch.run",
            &run.root_uri,
        )?;
        println!("resource_id={}", run.resource_id);
        println!("refreshed={}", run.refreshed);
        println!("root_uri={}", run.root_uri);
    }
    Ok(())
}

pub fn handle_watches_list(state: &CliState) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    for watch in list_resource_watch_statuses(&state.metadata, &cli.account_id, &cli.user_id, 100)?
    {
        println!("resource_id={}", watch.resource_id);
        println!("interval_seconds={}", watch.interval_seconds);
        println!("enabled={}", watch.enabled);
        println!("due={}", watch.due);
    }
    Ok(())
}

pub async fn handle_task_status(
    state: &CliState,
    task_key: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let session_engine = SessionEngine::open(&cli.workspace_root).await?;

    if let Some(task) = session_engine.task_status(task_key).await {
        print_session_task_record(&task);
        return Ok(());
    }

    if let Some(task) = state.metadata.get_task(task_key)? {
        print_metadata_task_record(&task);
    } else {
        println!("state=not_found");
    }
    Ok(())
}

pub async fn handle_wait_task(
    state: &CliState,
    task_key: &str,
    timeout_ms: u64,
    poll_ms: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let session_engine = SessionEngine::open(&cli.workspace_root).await?;
    match wait_for_task_completion(
        &state.metadata,
        &session_engine,
        task_key,
        Duration::from_millis(timeout_ms),
        Duration::from_millis(poll_ms),
    )
    .await?
    {
        WaitTaskOutcome::Session(task) => print_session_task_record(&task),
        WaitTaskOutcome::Metadata(task) => print_metadata_task_record(&task),
        WaitTaskOutcome::Timeout { task_id } => {
            println!("task_id={task_id}");
            println!("state=timeout");
        }
    }
    Ok(())
}

pub async fn handle_tasks_list(
    state: &CliState,
    limit: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let session_engine = SessionEngine::open(&cli.workspace_root).await?;
    for task in session_engine.list_tasks(limit).await? {
        print_task_list_session_record(&task);
    }
    for task in state
        .metadata
        .list_tasks(&cli.account_id, &cli.user_id, limit)?
    {
        print_task_list_metadata_record(&task);
    }
    Ok(())
}

pub async fn handle_watch_daemon_start(
    state: &CliState,
    poll_ms: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let pid = crate::daemon::start_watch_daemon(&state.cli, poll_ms).await?;
    println!("started=true");
    println!("pid={pid}");
    println!("poll_ms={poll_ms}");
    Ok(())
}

pub async fn handle_watch_daemon_status(
    state: &CliState,
) -> Result<(), Box<dyn std::error::Error>> {
    let status = crate::daemon::load_watch_daemon_status(&state.cli.workspace_root).await?;
    if let Some(status) = status {
        print_watch_daemon_status(&status);
    } else {
        println!("running=false");
    }
    Ok(())
}

pub async fn handle_watch_daemon_stop(state: &CliState) -> Result<(), Box<dyn std::error::Error>> {
    crate::daemon::stop_watch_daemon(&state.cli.workspace_root).await?;
    println!("stopped=true");
    Ok(())
}

pub async fn handle_complete_resource_ingest(
    state: &CliState,
    task_key: &str,
    resource_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let metadata = Arc::clone(&state.metadata);
    crate::helpers::execute_resource_task(
        &metadata,
        cli,
        &identity.clone(),
        task_key,
        resource_id,
        "resource.ingest",
        || {
            let metadata_ref = Arc::clone(&metadata);
            let identity = identity.clone();
            async move {
                let result = complete_registered_resource_ingest(
                    &metadata_ref,
                    &cli.workspace_root,
                    &identity,
                    resource_id,
                )
                .await?;
                Ok((result.root_uri, result.mode))
            }
        },
    )
    .await?;
    Ok(())
}

pub async fn handle_complete_resource_refresh(
    state: &CliState,
    task_key: &str,
    resource_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let metadata = Arc::clone(&state.metadata);
    crate::helpers::execute_resource_task(
        &metadata,
        cli,
        &identity.clone(),
        task_key,
        resource_id,
        "resource.refresh",
        || {
            let metadata_ref = Arc::clone(&metadata);
            let identity = identity.clone();
            async move {
                let result = refresh_registered_resource(
                    &metadata_ref,
                    &cli.workspace_root,
                    &identity,
                    resource_id,
                )
                .await?;
                Ok((result.root_uri, result.mode))
            }
        },
    )
    .await?;
    Ok(())
}

pub async fn handle_complete_resource_rebuild(
    state: &CliState,
    task_key: &str,
    resource_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    let metadata = Arc::clone(&state.metadata);
    crate::helpers::execute_resource_task(
        &metadata,
        cli,
        &identity.clone(),
        task_key,
        resource_id,
        "resource.rebuild",
        || {
            let metadata_ref = Arc::clone(&metadata);
            let identity = identity.clone();
            async move {
                let result = rebuild_registered_resource(
                    &metadata_ref,
                    &cli.workspace_root,
                    &identity,
                    resource_id,
                )
                .await?;
                Ok((result.root_uri, result.mode))
            }
        },
    )
    .await?;
    Ok(())
}

pub async fn handle_watch_daemon_run(
    state: &CliState,
    poll_ms: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    crate::daemon::run_watch_daemon_loop_with_metadata(
        &state.metadata,
        &cli.workspace_root,
        &identity,
        poll_ms,
    )
    .await?;
    Ok(())
}
