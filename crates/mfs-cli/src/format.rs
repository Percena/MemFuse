use mfs_metadata::AuditRecord;
use mfs_metadata::StoredTask;
use mfs_retrieval::{MatchedContext, SearchResult};
use mfs_session::TaskRecord as SessionTaskRecord;
use mfs_session::{SessionArchiveView, SessionContextView, SessionSummary};
use mfs_workspace::TreeNode;

pub fn print_session_task_record(task: &SessionTaskRecord) {
    println!("task_id={}", task.task_id);
    println!("archive_uri={}", task.archive_uri);
    println!("status={:?}", task.status);
    println!(
        "retry_state={}",
        task.retry_state.clone().unwrap_or_default()
    );
    println!(
        "processing_mode={}",
        task.processing_mode.clone().unwrap_or_default()
    );
    println!("used_contexts={}", task.used_contexts);
    println!("used_skills={}", task.used_skills);
    println!("used_tools={}", task.used_tools);
    for (kind, count) in &task.memories_extracted {
        println!("memories_extracted.{kind}={count}");
    }
    for (kind, count) in &task.artifacts_written {
        println!("artifacts_written.{kind}={count}");
    }
    if let Some(error) = &task.error {
        println!("error={error}");
    }
}

pub fn print_metadata_task_record(task: &StoredTask) {
    println!("task_key={}", task.task_key);
    println!("state={}", task.state);
    println!("attempt_count={}", task.attempt_count);
    println!("max_attempts={}", task.max_attempts);
    println!("retry_state={}", task.retry_state);
    println!(
        "processing_mode={}",
        task.processing_mode.clone().unwrap_or_default()
    );
    if let Some(summary) = &task.summary {
        println!("summary={summary}");
    }
    if let Some(last_error) = &task.last_error {
        println!("last_error={last_error}");
    }
}

pub fn print_task_list_session_record(task: &SessionTaskRecord) {
    println!("kind=session");
    println!("task_id={}", task.task_id);
    println!("status={:?}", task.status);
    println!("archive_uri={}", task.archive_uri);
}

pub fn print_task_list_metadata_record(task: &StoredTask) {
    println!("kind=metadata");
    println!("task_id={}", task.task_key);
    println!("status={}", task.state);
    if let Some(summary) = &task.summary {
        println!("summary={summary}");
    }
}

pub fn print_audit_record(record: &AuditRecord) {
    println!("event_type={}", record.event_type);
    if let Some(subject_uri) = &record.subject_uri {
        println!("subject_uri={subject_uri}");
    }
}

pub fn print_tree(node: &TreeNode, depth: usize) {
    let indent = "  ".repeat(depth);
    println!("{indent}{}", node.name);
    for child in &node.children {
        print_tree(child, depth + 1);
    }
}

pub fn print_session_summary(session: &SessionSummary) {
    println!("session_id={}", session.session_id);
    println!("account_id={}", session.account_id);
    println!("user_id={}", session.user_id);
    println!("agent_id={}", session.agent_id);
    println!("message_count={}", session.message_count);
    println!("commit_count={}", session.commit_count);
    if let Some(last_commit_archive_id) = &session.last_commit_archive_id {
        println!("last_commit_archive_id={last_commit_archive_id}");
    }
}

pub fn print_session_context(context: &SessionContextView) {
    println!(
        "latest_archive_overview={}",
        context.latest_archive_overview.replace('\n', "\\n")
    );
    for abstract_view in &context.pre_archive_abstracts {
        println!(
            "pre_archive_abstract={} {}",
            abstract_view.archive_id,
            abstract_view.abstract_text.replace('\n', "\\n")
        );
    }
    for message in &context.messages {
        println!("message={} {}", message.role, message.content);
    }
}

pub fn print_session_archive(archive: &SessionArchiveView) {
    println!("archive_id={}", archive.archive_id);
    println!(
        "abstract_text={}",
        archive.abstract_text.replace('\n', "\\n")
    );
    println!(
        "overview_text={}",
        archive.overview_text.replace('\n', "\\n")
    );
    for message in &archive.messages {
        println!("message={} {}", message.role, message.content);
    }
}

pub fn print_retrieval_result(result: &SearchResult) {
    print_query_plan(result);
    print_trajectory(result);
    print_matches("resources", &result.resources);
    print_matches("memories", &result.memories);
    print_matches("skills", &result.skills);
}

pub fn print_query_plan(result: &SearchResult) {
    println!("[query_plan]");
    println!("mode={}", result.query_plan.mode.as_str());
    if let Some(skip_reason) = &result.query_plan.skip_reason {
        println!("skip_reason={skip_reason}");
    }
    for typed_query in &result.query_plan.typed_queries {
        println!(
            "query={} priority={} intent={} context_type={} source={}",
            typed_query.query,
            typed_query.priority,
            typed_query.intent,
            typed_query.context_type,
            typed_query.source
        );
    }

    if result.query_plan.typed_queries.is_empty() && result.typed_queries.is_empty() {
        return;
    }

    for typed_query in &result.typed_queries {
        println!(
            "legacy_query={}:{}",
            typed_query.context_type, typed_query.query
        );
    }
}

pub fn print_trajectory(result: &SearchResult) {
    if result.trajectory.steps.is_empty() {
        return;
    }

    println!("[trajectory]");
    for step in &result.trajectory.steps {
        println!("{}={}", step.stage, step.detail);
    }
}

pub fn print_matches(label: &str, matches: &[MatchedContext]) {
    if matches.is_empty() {
        return;
    }

    println!("[{label}]");
    for item in matches {
        println!("{}", item.uri);
        println!("  retrieval_plane={}", item.retrieval_plane);
        println!("  match_reason={}", item.match_reason);
        if let Some(provenance) = &item.provenance {
            println!("  projection_view_id={}", provenance.projection_view_id);
            if let Some(source_kind) = &provenance.source_kind {
                println!("  source_kind={source_kind}");
            }
            if let Some(source_identifier) = &provenance.source_identifier {
                println!("  source_identifier={source_identifier}");
            }
            if let Some(source_snapshot_id) = &provenance.source_snapshot_id {
                println!("  source_snapshot_id={source_snapshot_id}");
            }
            println!("  workspace_path={}", provenance.workspace_path);
            for audit_event in &provenance.audit_events {
                println!(
                    "  audit={}@{}",
                    audit_event.event_type, audit_event.recorded_at
                );
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct WatchDaemonStatus {
    pub pid: u32,
    pub running: bool,
    pub poll_ms: u64,
    pub started_at_ms: u128,
    pub stopped_at_ms: Option<u128>,
    pub last_tick_at_ms: Option<u128>,
    pub total_ticks: u64,
    pub total_runs: u64,
    pub last_run_count: u64,
}

impl WatchDaemonStatus {
    pub fn render(&self) -> String {
        [
            format!("pid={}", self.pid),
            format!("running={}", self.running),
            format!("poll_ms={}", self.poll_ms),
            format!("started_at_ms={}", self.started_at_ms),
            format!(
                "stopped_at_ms={}",
                self.stopped_at_ms
                    .map(|value| value.to_string())
                    .unwrap_or_default()
            ),
            format!(
                "last_tick_at_ms={}",
                self.last_tick_at_ms
                    .map(|value| value.to_string())
                    .unwrap_or_default()
            ),
            format!("total_ticks={}", self.total_ticks),
            format!("total_runs={}", self.total_runs),
            format!("last_run_count={}", self.last_run_count),
        ]
        .join("\n")
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let mut pid = None;
        let mut running = None;
        let mut poll_ms = None;
        let mut started_at_ms = None;
        let mut stopped_at_ms = None;
        let mut last_tick_at_ms = None;
        let mut total_ticks = None;
        let mut total_runs = None;
        let mut last_run_count = None;
        for line in raw.lines() {
            let (key, value) = line.split_once('=')?;
            match key {
                "pid" => pid = value.parse::<u32>().ok(),
                "running" => running = value.parse::<bool>().ok(),
                "poll_ms" => poll_ms = value.parse::<u64>().ok(),
                "started_at_ms" => started_at_ms = value.parse::<u128>().ok(),
                "stopped_at_ms" => {
                    stopped_at_ms = if value.is_empty() {
                        Some(None)
                    } else {
                        value.parse::<u128>().ok().map(Some)
                    }
                }
                "last_tick_at_ms" => {
                    last_tick_at_ms = if value.is_empty() {
                        Some(None)
                    } else {
                        value.parse::<u128>().ok().map(Some)
                    }
                }
                "total_ticks" => total_ticks = value.parse::<u64>().ok(),
                "total_runs" => total_runs = value.parse::<u64>().ok(),
                "last_run_count" => last_run_count = value.parse::<u64>().ok(),
                _ => {}
            }
        }
        Some(Self {
            pid: pid?,
            running: running?,
            poll_ms: poll_ms?,
            started_at_ms: started_at_ms?,
            stopped_at_ms: stopped_at_ms.unwrap_or(None),
            last_tick_at_ms: last_tick_at_ms.unwrap_or(None),
            total_ticks: total_ticks?,
            total_runs: total_runs?,
            last_run_count: last_run_count?,
        })
    }
}

pub fn print_watch_daemon_status(status: &WatchDaemonStatus) {
    println!("pid={}", status.pid);
    println!("running={}", status.running);
    println!("poll_ms={}", status.poll_ms);
    println!("total_ticks={}", status.total_ticks);
    println!("total_runs={}", status.total_runs);
    println!("last_run_count={}", status.last_run_count);
}
