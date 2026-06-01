mod daemon;
mod doctor;
mod format;
mod helpers;
mod maintenance;
mod resource;
mod retrieval;
mod session;
mod skill;
mod system;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

use helpers::CliState;

#[derive(Parser)]
#[command(name = "mfs-cli")]
struct Cli {
    #[arg(long)]
    pub workspace_root: PathBuf,
    #[arg(long, default_value = "localfs")]
    pub source_kind: String,
    #[arg(long)]
    pub source_path: Option<PathBuf>,
    #[arg(long, default_value = "mfs://resources/localfs/docs")]
    target_uri: Option<String>,
    #[arg(long, default_value = "default")]
    account_id: String,
    #[arg(long, default_value = "default")]
    user_id: String,
    #[arg(long, default_value = "default")]
    agent_id: String,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Skill {
        #[command(subcommand)]
        command: skill::SkillCommands,
    },
    Ls {
        uri: String,
    },
    Tree {
        uri: String,
        #[arg(long, default_value_t = 3)]
        depth: usize,
    },
    Stat {
        uri: String,
    },
    Abstract {
        uri: String,
    },
    Overview {
        uri: String,
    },
    Read {
        uri: String,
    },
    Glob {
        uri: String,
        pattern: String,
    },
    Mkdir {
        uri: String,
    },
    Write {
        uri: String,
        #[arg(long)]
        content: String,
    },
    Mv {
        from_uri: String,
        to_uri: String,
    },
    Rm {
        uri: String,
    },
    Find {
        query: String,
        #[arg(long)]
        target: Option<String>,
    },
    Grep {
        query: String,
        #[arg(long)]
        target: Option<String>,
    },
    Search {
        query: String,
        #[arg(long)]
        target: Option<String>,
        #[arg(long)]
        session_context: Option<String>,
    },
    SessionsList,
    SessionGet {
        session_id: String,
    },
    SessionContext {
        session_id: String,
        #[arg(long, default_value_t = 128_000)]
        token_budget: usize,
    },
    SessionArchive {
        session_id: String,
        archive_id: String,
    },
    SessionDelete {
        session_id: String,
    },
    SkillsList,
    AddSkill {
        path: PathBuf,
    },
    AddResource {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        file_name: Option<String>,
        #[arg(long)]
        content: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        revision: Option<String>,
        #[arg(long, value_delimiter = ',')]
        paths: Option<Vec<String>>,
    },
    ResourcesList,
    ResourceRefresh {
        resource_id: String,
    },
    ResourceRebuild {
        resource_id: String,
    },
    ResourceExport {
        resource_id: String,
        output_path: PathBuf,
    },
    ResourceImport {
        pack_path: PathBuf,
        #[arg(long)]
        name: Option<String>,
    },
    ResourceWatch {
        resource_id: String,
        #[arg(long, default_value_t = 60)]
        interval_seconds: u32,
    },
    ResourceWatchRun {
        resource_id: String,
    },
    ResourceWatchDisable {
        resource_id: String,
    },
    ResourceWatchRunDue,
    ResourceWatchLoop {
        #[arg(long, default_value_t = 3)]
        iterations: usize,
        #[arg(long, default_value_t = 1_000)]
        sleep_ms: u64,
    },
    WatchDaemonStart {
        #[arg(long, default_value_t = 1_000)]
        poll_ms: u64,
    },
    WatchDaemonStatus,
    WatchDaemonStop,
    WatchesList,
    TaskStatus {
        task_key: String,
    },
    WaitTask {
        task_key: String,
        #[arg(long, default_value_t = 5_000)]
        timeout_ms: u64,
        #[arg(long, default_value_t = 50)]
        poll_ms: u64,
    },
    TasksList {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    Link {
        from_uri: String,
        to_uri: String,
        #[arg(long, default_value = "references")]
        relation_type: String,
    },
    Unlink {
        from_uri: String,
        to_uri: String,
        #[arg(long, default_value = "references")]
        relation_type: String,
    },
    Relations {
        uri: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    ObserverStatus,
    SystemStatus,
    Doctor,
    Rebuild,
    Refresh,
    SnapshotList,
    Audit {
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    HeuristicDecay,
    HeuristicConsolidate,
    /// Run periodic heuristic maintenance (decay + consolidation). With --schedule, loops forever.
    HeuristicScheduled {
        #[arg(long)]
        schedule: Option<u64>,
    },
    #[command(name = "__complete-resource-ingest", hide = true)]
    CompleteResourceIngest {
        #[arg(long)]
        task_key: String,
        #[arg(long)]
        resource_id: String,
    },
    #[command(name = "__complete-resource-refresh", hide = true)]
    CompleteResourceRefresh {
        #[arg(long)]
        task_key: String,
        #[arg(long)]
        resource_id: String,
    },
    #[command(name = "__complete-resource-rebuild", hide = true)]
    CompleteResourceRebuild {
        #[arg(long)]
        task_key: String,
        #[arg(long)]
        resource_id: String,
    },
    #[command(name = "__watch-daemon-run", hide = true)]
    WatchDaemonRun {
        #[arg(long)]
        poll_ms: u64,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let mut cli = Cli::parse();
    cli.workspace_root = mfs_types::expand_tilde_path(&cli.workspace_root);
    cli.source_path = cli.source_path.map(|p| mfs_types::expand_tilde_path(&p));
    let state = CliState::new(cli)?;
    let cli = &state.cli;

    match &cli.command {
        Commands::Skill { command } => match command {
            skill::SkillCommands::Export {
                platform,
                output_dir,
            } => {
                let bundle = skill::render_skill_bundle(*platform);
                if let Some(output_dir) = output_dir {
                    bundle.write_to(output_dir, *platform)?;
                } else {
                    print!("{}", bundle.skill_markdown());
                }
            }
        },
        Commands::Ls { uri } => resource::handle_ls(&state, uri).await?,
        Commands::Tree { uri, depth } => resource::handle_tree(&state, uri, *depth).await?,
        Commands::Stat { uri } => resource::handle_stat(&state, uri).await?,
        Commands::Abstract { uri } => resource::handle_abstract(&state, uri).await?,
        Commands::Overview { uri } => resource::handle_overview(&state, uri).await?,
        Commands::Read { uri } => resource::handle_read(&state, uri).await?,
        Commands::Glob { uri, pattern } => resource::handle_glob(&state, uri, pattern).await?,
        Commands::Mkdir { uri } => resource::handle_mkdir(&state, uri).await?,
        Commands::Write { uri, content } => resource::handle_write(&state, uri, content).await?,
        Commands::Mv { from_uri, to_uri } => resource::handle_mv(&state, from_uri, to_uri).await?,
        Commands::Rm { uri } => resource::handle_rm(&state, uri).await?,
        Commands::Find { query, target } => retrieval::handle_find(&state, query, target).await?,
        Commands::Grep { query, target } => retrieval::handle_grep(&state, query, target).await?,
        Commands::Search {
            query,
            target,
            session_context,
        } => retrieval::handle_search(&state, query, target, session_context).await?,
        Commands::SessionsList => session::handle_sessions_list(&state).await?,
        Commands::SessionGet { session_id } => {
            session::handle_session_get(&state, session_id).await?;
        }
        Commands::SessionContext {
            session_id,
            token_budget,
        } => session::handle_session_context(&state, session_id, *token_budget).await?,
        Commands::SessionArchive {
            session_id,
            archive_id,
        } => session::handle_session_archive(&state, session_id, archive_id).await?,
        Commands::SessionDelete { session_id } => {
            session::handle_session_delete(&state, session_id).await?;
        }
        Commands::SkillsList => resource::handle_skills_list(&state).await?,
        Commands::AddSkill { path } => resource::handle_add_skill(&state, path).await?,
        Commands::AddResource {
            name,
            file_name,
            content,
            branch,
            revision,
            paths,
        } => {
            if let Some(paths) = paths {
                resource::handle_add_resources_batch(
                    &state,
                    paths,
                    name,
                    branch.as_deref(),
                    revision.as_deref(),
                )
                .await?;
            } else {
                resource::handle_add_resource(&state, name, file_name, content).await?;
            }
        }
        Commands::ResourcesList => resource::handle_resources_list(&state)?,
        Commands::ResourceRefresh { resource_id } => {
            resource::handle_resource_refresh(&state, resource_id)?;
        }
        Commands::ResourceRebuild { resource_id } => {
            resource::handle_resource_rebuild(&state, resource_id)?;
        }
        Commands::ResourceExport {
            resource_id,
            output_path,
        } => resource::handle_resource_export(&state, resource_id, output_path).await?,
        Commands::ResourceImport { pack_path, name } => {
            resource::handle_resource_import(&state, pack_path, name).await?;
        }
        Commands::ResourceWatch {
            resource_id,
            interval_seconds,
        } => resource::handle_resource_watch(&state, resource_id, *interval_seconds)?,
        Commands::ResourceWatchRun { resource_id } => {
            resource::handle_resource_watch_run(&state, resource_id).await?;
        }
        Commands::ResourceWatchDisable { resource_id } => {
            resource::handle_resource_watch_disable(&state, resource_id)?;
        }
        Commands::ResourceWatchRunDue => resource::handle_resource_watch_run_due(&state).await?,
        Commands::ResourceWatchLoop {
            iterations,
            sleep_ms,
        } => resource::handle_resource_watch_loop(&state, *iterations, *sleep_ms).await?,
        Commands::WatchDaemonStart { poll_ms } => {
            resource::handle_watch_daemon_start(&state, *poll_ms).await?;
        }
        Commands::WatchDaemonStatus => resource::handle_watch_daemon_status(&state).await?,
        Commands::WatchDaemonStop => resource::handle_watch_daemon_stop(&state).await?,
        Commands::WatchesList => resource::handle_watches_list(&state)?,
        Commands::TaskStatus { task_key } => resource::handle_task_status(&state, task_key).await?,
        Commands::WaitTask {
            task_key,
            timeout_ms,
            poll_ms,
        } => resource::handle_wait_task(&state, task_key, *timeout_ms, *poll_ms).await?,
        Commands::TasksList { limit } => resource::handle_tasks_list(&state, *limit).await?,
        Commands::Link {
            from_uri,
            to_uri,
            relation_type,
        } => system::handle_link(&state, from_uri, to_uri, relation_type)?,
        Commands::Unlink {
            from_uri,
            to_uri,
            relation_type,
        } => system::handle_unlink(&state, from_uri, to_uri, relation_type)?,
        Commands::Relations { uri, limit } => system::handle_relations(&state, uri, *limit)?,
        Commands::ObserverStatus => system::handle_observer_status(&state).await?,
        Commands::SystemStatus => system::handle_system_status(&state).await?,
        Commands::Doctor => doctor::run_doctor(&cli.workspace_root),
        Commands::Rebuild => system::handle_rebuild(&state).await?,
        Commands::Refresh => system::handle_refresh(&state).await?,
        Commands::SnapshotList => system::handle_snapshot_list(&state)?,
        Commands::Audit { limit } => system::handle_audit(&state, *limit)?,
        Commands::HeuristicDecay => maintenance::handle_heuristic_decay(&state)?,
        Commands::HeuristicConsolidate => maintenance::handle_heuristic_consolidate(&state).await?,
        Commands::HeuristicScheduled { schedule } => {
            maintenance::handle_heuristic_scheduled(&state, *schedule).await?;
        }
        Commands::CompleteResourceIngest {
            task_key,
            resource_id,
        } => resource::handle_complete_resource_ingest(&state, task_key, resource_id).await?,
        Commands::CompleteResourceRefresh {
            task_key,
            resource_id,
        } => resource::handle_complete_resource_refresh(&state, task_key, resource_id).await?,
        Commands::CompleteResourceRebuild {
            task_key,
            resource_id,
        } => resource::handle_complete_resource_rebuild(&state, task_key, resource_id).await?,
        Commands::WatchDaemonRun { poll_ms } => {
            resource::handle_watch_daemon_run(&state, *poll_ms).await?;
        }
    }

    Ok(())
}
