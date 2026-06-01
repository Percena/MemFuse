use clap::Parser;
use mfs_memory::heuristics::{retrieve_heuristics, retrieve_l0_confirmed, validate_tags};
use mfs_metadata::MetadataStore;
use mfs_ops::{
    WaitTaskOutcome, ingest_skill, list_resource_watch_statuses, list_skills, observer_status,
    run_due_resource_watches, run_resource_watch_loop, system_status, wait_for_task_completion,
};
use mfs_retrieval::RetrievalEngine;
use mfs_session::SessionEngine;
use mfs_types::IdentityContext;
use mfs_workspace::WorkspaceFs;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "mfs-mcp")]
pub struct Cli {
    #[arg(long)]
    pub workspace_root: PathBuf,
    #[arg(long, default_value = "default")]
    pub account_id: String,
    #[arg(long, default_value = "default")]
    pub user_id: String,
    #[arg(long, default_value = "default")]
    pub agent_id: String,
}

/// Long-lived state shared across all MCP tool calls.
/// Eliminates per-tool `MetadataStore::open_at` and `SessionEngine::open` overhead.
struct McpState {
    cli: Cli,
    metadata: Arc<MetadataStore>,
    session_engine: Arc<SessionEngine>,
    /// Cached embedding provider for future MCP tool use.
    #[allow(dead_code)]
    embedding_provider: Box<dyn mfs_semantic::EmbeddingProvider>,
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut cli = Cli::parse();
    cli.workspace_root = mfs_types::expand_tilde_path(&cli.workspace_root);
    let metadata_path = cli.workspace_root.join("_system").join("metadata.sqlite");
    let metadata = Arc::new(MetadataStore::open_at(&metadata_path, false)?);

    let runtime = tokio::runtime::Runtime::new()?;
    let auto_commit_threshold = std::env::var("MEMFUSE_AUTO_COMMIT_THRESHOLD")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(mfs_session::AUTO_COMMIT_THRESHOLD_DEFAULT);
    let session_engine = Arc::new(runtime.block_on(SessionEngine::open_with_threshold(
        &cli.workspace_root,
        auto_commit_threshold,
    ))?);

    let state = McpState {
        cli,
        metadata,
        session_engine,
        embedding_provider: mfs_semantic::embedding_provider_from_env(256),
    };

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();

    while let Some(request) = read_message(&mut reader)? {
        if let Some(response) = runtime.block_on(handle_message(&state, request)) {
            write_message(&mut writer, &response)?;
        }
    }

    Ok(())
}

async fn handle_message(state: &McpState, request: Value) -> Option<Value> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method")?.as_str()?;

    match method {
        "initialize" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": { "listChanged": false }
                },
                "serverInfo": {
                    "name": "mfs-mcp",
                    "version": "0.1.0"
                }
            }
        })),
        "notifications/initialized" => None,
        "tools/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": tools()
            }
        })),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or(Value::Null);
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            Some(tool_call_response(id, call_tool(state, name, args).await))
        }
        _ => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("method not found: {method}") }
        })),
    }
}

fn tool_call_response(id: Value, result: Result<Value, String>) -> Value {
    match result {
        Ok(value) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{ "type": "text", "text": serde_json::to_string(&value).unwrap() }],
                "isError": false
            }
        }),
        Err(error) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{ "type": "text", "text": error }],
                "isError": true
            }
        }),
    }
}

async fn call_tool(state: &McpState, name: &str, args: Value) -> Result<Value, String> {
    let cli = &state.cli;
    let identity = IdentityContext::new(&cli.account_id, &cli.user_id, &cli.agent_id);
    match name {
        "find" => {
            let query = required_string(&args, "query")?;
            let target = optional_string(&args, "target");
            let fs = open_fs(cli, target.as_deref()).map_err(to_string)?;
            let retrieval = RetrievalEngine::from_workspace(
                &cli.workspace_root,
                &identity,
                fs.projection_root(),
                fs.projection_uri(),
            )
            .await
            .map_err(to_string)?;
            serde_json::to_value(
                retrieval
                    .find(&query, target.as_deref())
                    .await
                    .map_err(to_string)?,
            )
            .map_err(to_string)
        }
        "search" => {
            let query = required_string(&args, "query")?;
            let target = optional_string(&args, "target");
            let session_context = optional_string(&args, "session_context");
            let fs = open_fs(cli, target.as_deref()).map_err(to_string)?;
            let retrieval = RetrievalEngine::from_workspace(
                &cli.workspace_root,
                &identity,
                fs.projection_root(),
                fs.projection_uri(),
            )
            .await
            .map_err(to_string)?;
            serde_json::to_value(
                retrieval
                    .search(&query, target.as_deref(), session_context.as_deref())
                    .await
                    .map_err(to_string)?,
            )
            .map_err(to_string)
        }
        "read" => {
            let uri = required_string(&args, "uri")?;
            let fs = open_fs(cli, Some(&uri)).map_err(to_string)?;
            Ok(json!({ "uri": uri, "content": fs.read(&uri).await.map_err(to_string)? }))
        }
        "ls" => {
            let uri = required_string(&args, "uri")?;
            let fs = open_fs(cli, Some(&uri)).map_err(to_string)?;
            Ok(json!(
                fs.ls(&uri)
                    .await
                    .map_err(to_string)?
                    .into_iter()
                    .map(|entry| json!({
                        "name": entry.name,
                        "is_dir": entry.is_dir,
                    }))
                    .collect::<Vec<_>>()
            ))
        }
        "task_status" => {
            let task_id = required_string(&args, "task_id")?;
            if let Some(task) = state.session_engine.task_status(&task_id).await {
                return serde_json::to_value(task).map_err(to_string);
            }
            match state.metadata.get_task(&task_id).map_err(to_string)? {
                Some(task) => serde_json::to_value(task).map_err(to_string),
                None => Ok(json!({ "status": "not_found", "task_id": task_id })),
            }
        }
        "wait_task" => {
            let task_id = required_string(&args, "task_id")?;
            let timeout_ms = optional_u64(&args, "timeout_ms").unwrap_or(5_000);
            let poll_ms = optional_u64(&args, "poll_ms").unwrap_or(50);
            match wait_for_task_completion(
                &state.metadata,
                &state.session_engine,
                &task_id,
                Duration::from_millis(timeout_ms),
                Duration::from_millis(poll_ms),
            )
            .await
            .map_err(to_string)?
            {
                WaitTaskOutcome::Session(task) => serde_json::to_value(task).map_err(to_string),
                WaitTaskOutcome::Metadata(task) => serde_json::to_value(task).map_err(to_string),
                WaitTaskOutcome::Timeout { task_id } => {
                    Ok(json!({ "status": "timeout", "task_id": task_id }))
                }
            }
        }
        "session_list" => serde_json::to_value(
            state
                .session_engine
                .list_sessions(&cli.account_id, &cli.user_id, &cli.agent_id)
                .await
                .map_err(to_string)?,
        )
        .map_err(to_string),
        "session_get" => {
            let session_id = required_string(&args, "session_id")?;
            serde_json::to_value(
                state
                    .session_engine
                    .get_session(&session_id)
                    .await
                    .map_err(to_string)?,
            )
            .map_err(to_string)
        }
        "session_context" => {
            let session_id = required_string(&args, "session_id")?;
            let token_budget = optional_u64(&args, "token_budget").unwrap_or(128_000) as usize;
            serde_json::to_value(
                state
                    .session_engine
                    .get_session_context(&session_id, token_budget)
                    .await
                    .map_err(to_string)?,
            )
            .map_err(to_string)
        }
        "session_archive" => {
            let session_id = required_string(&args, "session_id")?;
            let archive_id = required_string(&args, "archive_id")?;
            serde_json::to_value(
                state
                    .session_engine
                    .get_session_archive(&session_id, &archive_id)
                    .await
                    .map_err(to_string)?,
            )
            .map_err(to_string)
        }
        "system_status" => serde_json::to_value(
            system_status(&state.metadata, &cli.workspace_root, &identity)
                .await
                .map_err(to_string)?,
        )
        .map_err(to_string),
        "observer_status" => {
            serde_json::to_value(observer_status(&cli.workspace_root).map_err(to_string)?)
                .map_err(to_string)
        }
        "watches_list" => serde_json::to_value(
            list_resource_watch_statuses(&state.metadata, &cli.account_id, &cli.user_id, 100)
                .map_err(to_string)?,
        )
        .map_err(to_string),
        "watch_run_due" => serde_json::to_value(
            run_due_resource_watches(&state.metadata, &cli.workspace_root, &identity, 100)
                .await
                .map_err(to_string)?,
        )
        .map_err(to_string),
        "watch_run_loop" => serde_json::to_value(
            run_resource_watch_loop(
                &state.metadata,
                &cli.workspace_root,
                &identity,
                optional_u64(&args, "iterations").unwrap_or(1) as usize,
                Duration::from_millis(optional_u64(&args, "sleep_ms").unwrap_or(100)),
                100,
            )
            .await
            .map_err(to_string)?,
        )
        .map_err(to_string),
        "add_skill" => {
            let path = required_string(&args, "path")?;
            serde_json::to_value(
                ingest_skill(
                    &state.metadata,
                    &cli.workspace_root,
                    &identity,
                    std::path::Path::new(&path),
                )
                .await
                .map_err(to_string)?,
            )
            .map_err(to_string)
        }
        "skills_list" => serde_json::to_value(
            list_skills(&cli.workspace_root, &identity)
                .await
                .map_err(to_string)?,
        )
        .map_err(to_string),
        "relation_link" => {
            let from_uri = required_string(&args, "from_uri")?;
            let to_uri = required_string(&args, "to_uri")?;
            let relation_type =
                optional_string(&args, "relation_type").unwrap_or_else(|| "references".to_owned());
            state
                .metadata
                .upsert_relation(&mfs_metadata::RelationRecord {
                    account_id: &cli.account_id,
                    user_id: &cli.user_id,
                    agent_id: Some(&cli.agent_id),
                    from_uri: &from_uri,
                    to_uri: &to_uri,
                    relation_type: &relation_type,
                })
                .map_err(to_string)?;
            Ok(json!({ "ok": true }))
        }
        "relations_list" => {
            let uri = required_string(&args, "uri")?;
            let rows = state
                .metadata
                .list_relations(&cli.account_id, &cli.user_id, &uri, 20)
                .map_err(to_string)?
                .into_iter()
                .map(|relation| {
                    let (direction, peer_uri) = if relation.from_uri == uri {
                        ("outbound", relation.to_uri)
                    } else {
                        ("inbound", relation.from_uri)
                    };
                    json!({
                        "relation_type": relation.relation_type,
                        "direction": direction,
                        "peer_uri": peer_uri,
                    })
                })
                .collect::<Vec<_>>();
            Ok(Value::Array(rows))
        }
        "relation_unlink" => {
            let from_uri = required_string(&args, "from_uri")?;
            let to_uri = required_string(&args, "to_uri")?;
            let relation_type =
                optional_string(&args, "relation_type").unwrap_or_else(|| "references".to_owned());
            state
                .metadata
                .remove_relation(
                    &cli.account_id,
                    &cli.user_id,
                    &from_uri,
                    &to_uri,
                    &relation_type,
                )
                .map_err(to_string)?;
            Ok(json!({ "ok": true }))
        }
        "session_delete" => {
            let session_id = required_string(&args, "session_id")?;
            state
                .session_engine
                .delete_session(&session_id)
                .await
                .map_err(to_string)?;
            Ok(json!({ "deleted": true, "session_id": session_id }))
        }
        // Heuristic tools (T2H Phase 1)
        "heuristics_create_rule" => {
            let rule_text = required_string(&args, "rule_text")?;
            let tags: Vec<String> = args
                .get("tags")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let counter_examples: Vec<String> = args
                .get("counter_examples")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let lifecycle_stage =
                optional_string(&args, "lifecycle_stage").unwrap_or_else(|| "draft".to_owned());
            let valid_tags = validate_tags(&tags);
            let rule_id = mfs_uri::short_hash_hex(
                format!("{}:{}:{}", &cli.account_id, &cli.user_id, rule_text).as_bytes(),
                12,
            );
            let tags_json = serde_json::to_string(&valid_tags).unwrap_or_else(|_| "[]".to_owned());
            let ce_json =
                serde_json::to_string(&counter_examples).unwrap_or_else(|_| "[]".to_owned());
            state
                .metadata
                .insert_heuristic_rule(&mfs_metadata::HeuristicRuleRecord {
                    rule_id: &rule_id,
                    account_id: &cli.account_id,
                    user_id: &cli.user_id,
                    agent_id: Some(&cli.agent_id),
                    tags_json: &tags_json,
                    rule_text: &rule_text,
                    counter_examples_json: &ce_json,
                    lifecycle_stage: &lifecycle_stage,
                    evidence_count: 0,
                    aggregate_weight: 0.0,
                    last_evidence_at: None,
                    source_instance_ids_json: None,
                    promoted_at: None,
                    user_confirmed: false,
                })
                .map_err(to_string)?;
            Ok(
                json!({ "rule_id": rule_id, "lifecycle_stage": lifecycle_stage, "tags": valid_tags }),
            )
        }
        "heuristics_list_rules" => {
            let stored = state
                .metadata
                .list_heuristic_rules(&cli.account_id, &cli.user_id)
                .map_err(to_string)?;
            let entries: Vec<Value> = stored.into_iter().map(|r| json!({
                "rule_id": r.rule_id,
                "rule_text": r.rule_text,
                "tags": serde_json::from_str::<Vec<String>>(&r.tags_json).unwrap_or_default(),
                "counter_examples": serde_json::from_str::<Vec<String>>(&r.counter_examples_json).unwrap_or_default(),
                "lifecycle_stage": r.lifecycle_stage,
                "evidence_count": r.evidence_count,
                "aggregate_weight": r.aggregate_weight,
            })).collect();
            Ok(Value::Array(entries))
        }
        "heuristics_promote_rule" => {
            let rule_id = required_string(&args, "rule_id")?;
            let new_stage = required_string(&args, "new_stage")?;
            state
                .metadata
                .update_rule_lifecycle(&rule_id, &new_stage)
                .map_err(to_string)?;
            Ok(json!({ "rule_id": rule_id, "new_stage": new_stage, "status": "updated" }))
        }
        "heuristics_confirm_rule" => {
            // Mark a rule as user-confirmed (roadmap §5.4).
            // User-confirmed rules are exempt from automatic decay,
            // distinct from lifecycle_stage 'confirmed' (auto-promotion).
            let rule_id = required_string(&args, "rule_id")?;
            let updated = state.metadata.confirm_heuristic_rule(
                &rule_id,
                &state.cli.account_id,
                &state.cli.user_id,
            );
            if !updated {
                return Err(format!(
                    "heuristic_rule:{rule_id} not found or not owned by this account"
                ));
            }
            Ok(json!({ "rule_id": rule_id, "user_confirmed": true, "status": "confirmed" }))
        }
        "heuristics_create_instance" => {
            let context_summary = required_string(&args, "context_summary")?;
            let user_reaction = required_string(&args, "user_reaction")?;
            let signal_type = required_string(&args, "signal_type")?;
            let tags: Vec<String> = args
                .get("tags")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let agent_proposal = optional_string(&args, "agent_proposal");
            let outcome = optional_string(&args, "outcome");
            let session_id = optional_string(&args, "session_id");
            let valid_tags = validate_tags(&tags);
            let instance_id = mfs_uri::short_hash_hex(
                format!(
                    "{}:{}:{}:{}",
                    &cli.account_id, &cli.user_id, signal_type, user_reaction
                )
                .as_bytes(),
                12,
            );
            let tags_json = serde_json::to_string(&valid_tags).unwrap_or_else(|_| "[]".to_owned());
            state
                .metadata
                .insert_heuristic_instance(&mfs_metadata::HeuristicInstanceRecord {
                    instance_id: &instance_id,
                    account_id: &cli.account_id,
                    user_id: &cli.user_id,
                    agent_id: Some(&cli.agent_id),
                    context_summary: &context_summary,
                    agent_proposal: agent_proposal.as_deref(),
                    user_reaction: &user_reaction,
                    outcome: outcome.as_deref(),
                    signal_type: &signal_type,
                    tags_json: &tags_json,
                    session_id: session_id.as_deref(),
                    source_turn_ids_json: None,
                    derived_rule_id: None,
                    instance_status: "open",
                    resolved_at: None,
                })
                .map_err(to_string)?;
            Ok(
                json!({ "instance_id": instance_id, "signal_type": signal_type, "tags": valid_tags }),
            )
        }
        "heuristics_retrieve" => {
            let query = required_string(&args, "query")?;
            let tags: Vec<String> = args
                .get("tags")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let top_k = optional_u64(&args, "top_k").unwrap_or(10) as usize;
            let valid_tags = validate_tags(&tags);
            let entries = retrieve_heuristics(
                &state.metadata,
                &cli.account_id,
                &cli.user_id,
                &valid_tags,
                &query,
                top_k,
            );
            serde_json::to_value(entries).map_err(to_string)
        }
        "heuristics_l0_confirmed" => {
            let max_rules = optional_u64(&args, "max_rules").unwrap_or(5) as usize;
            let entries =
                retrieve_l0_confirmed(&state.metadata, &cli.account_id, &cli.user_id, max_rules);
            serde_json::to_value(entries).map_err(to_string)
        }
        "simulate_reaction" => {
            let scenario = required_string(&args, "scenario")?;
            let tags: Vec<String> = args
                .get("tags")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let valid_tags = validate_tags(&tags);
            let entries = retrieve_heuristics(
                &state.metadata,
                &cli.account_id,
                &cli.user_id,
                &valid_tags,
                &scenario,
                5,
            );
            // Build a prediction prompt based on relevant heuristic rules
            let rules_text = entries
                .iter()
                .map(|e| {
                    format!(
                        "{} {}: {}",
                        marker_for_stage(&e.lifecycle_stage),
                        e.lifecycle_stage,
                        e.rule_text
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(json!({
                "scenario": scenario,
                "relevant_rules": entries,
                "rules_summary": rules_text,
                "prediction": if entries.is_empty() {
                    "No relevant heuristic rules found for this scenario.".to_owned()
                } else {
                    format!("Based on {} learned preference(s), the user is likely to prefer approaches aligned with the confirmed/candidate rules listed above, and may reject approaches that match the counter-examples.", entries.len())
                }
            }))
        }
        "get_repo_manifest" => {
            let repo_id =
                optional_string(&args, "repo_id").unwrap_or_else(|| cli.account_id.clone());
            let manifest = state
                .metadata
                .get_manifest_identity(&repo_id)
                .map_err(to_string)?;
            match manifest {
                Some(m) => {
                    // SaaS mode: manifest_yaml_path may be None (cloud manifest without local file)
                    let Some(ref yaml_path) = m.manifest_yaml_path else {
                        // Return SQLite metadata directly without reading local YAML
                        let mut manifest = json!({});
                        merge_mcp_manifest_metadata(&mut manifest, &m, &state.metadata)?;
                        return Ok(json!({
                            "status": "ok",
                            "data": manifest,
                            "version_hash": m.last_verified_at,
                            "hint": null,
                        }));
                    };
                    let mut manifest = read_mcp_manifest_json(Path::new(yaml_path))?;
                    merge_mcp_manifest_metadata(&mut manifest, &m, &state.metadata)?;
                    Ok(json!({
                        "status": "ok",
                        "data": manifest,
                        "version_hash": m.last_verified_at,
                        "hint": null,
                    }))
                }
                None => Ok(json!({
                    "status": "unavailable",
                    "error": format!("no manifest found for repo_id '{}'", repo_id),
                    "hint": "Register the repo first via /manifest/update",
                    "version_hash": null,
                })),
            }
        }
        "query_canvas" => {
            let repo_id = required_string(&args, "repo_id")?;
            let component = optional_string(&args, "component");
            let canvas_type = optional_string(&args, "type").unwrap_or_else(|| "structural".into());
            if !["structural", "contracts", "status"].contains(&canvas_type.as_str()) {
                return Err(
                    "invalid type; expected one of structural, contracts, status".to_owned(),
                );
            }
            let node_type = optional_string(&args, "node_type");
            let status = optional_string(&args, "status");
            let mut nodes = state
                .metadata
                .list_canvas_nodes(&repo_id, node_type.as_deref(), component.as_deref())
                .map_err(to_string)?;
            let node_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
            let has_node_filter = node_type.is_some() || component.is_some();
            let mut edges = if !node_ids.is_empty() {
                state
                    .metadata
                    .list_canvas_edges_by_nodes(&repo_id, &node_ids)
                    .map_err(to_string)?
            } else if has_node_filter {
                Vec::new()
            } else {
                state
                    .metadata
                    .list_canvas_edges_by_repo(&repo_id)
                    .map_err(to_string)?
            };
            if component.is_some() {
                let mut known_ids: BTreeSet<String> =
                    nodes.iter().map(|node| node.id.clone()).collect();
                for edge in &edges {
                    for id in [&edge.source_node_id, &edge.target_node_id] {
                        if known_ids.contains(id) {
                            continue;
                        }
                        if let Some(node) = state.metadata.get_canvas_node(id).map_err(to_string)? {
                            known_ids.insert(node.id.clone());
                            nodes.push(node);
                        }
                    }
                }
            }
            match canvas_type.as_str() {
                "structural" => {
                    edges.retain(|edge| edge.edge_type != "contract");
                }
                "contracts" => {
                    edges.retain(|edge| edge.edge_type == "contract");
                    let connected_ids: BTreeSet<String> = edges
                        .iter()
                        .flat_map(|edge| [edge.source_node_id.clone(), edge.target_node_id.clone()])
                        .collect();
                    nodes.retain(|node| connected_ids.contains(&node.id));
                }
                "status" => {
                    edges.clear();
                }
                _ => {}
            }
            let overlays = state
                .metadata
                .list_active_overlays(&repo_id, status.as_deref())
                .map_err(to_string)?;
            let conflicts = detect_mcp_overlay_conflicts(&overlays);
            let version_hash = nodes
                .iter()
                .map(|n| n.version_hash.clone())
                .max()
                .or_else(|| edges.iter().map(|e| e.version_hash.clone()).max())
                .unwrap_or_default();
            Ok(json!({
                "status": "ok",
                "data": {
                    "nodes": nodes,
                    "edges": edges,
                    "overlays": overlays,
                    "conflicts": conflicts,
                },
                "version_hash": version_hash,
                "hint": null,
            }))
        }
        "propose_active_overlay" => {
            let repo_id = required_string(&args, "repo_id")?;
            let overlay_type = required_string(&args, "overlay_type")?;
            let content_json = normalize_json_argument(&args, "content_json")?;
            // Validate overlay_type against allowed values
            const VALID_OVERLAY_TYPES: &[&str] = &[
                "planned_change",
                "planned_contract",
                "conflict_declaration",
                "planned_test",
                "planned_config",
            ];
            if !VALID_OVERLAY_TYPES.contains(&overlay_type.as_str()) {
                return Err(format!(
                    "invalid overlay_type '{}'; valid values: {}",
                    overlay_type,
                    VALID_OVERLAY_TYPES.join(", ")
                ));
            }
            // Verify manifest exists for this repo
            let manifest = state
                .metadata
                .get_manifest_identity(&repo_id)
                .map_err(to_string)?;
            if manifest.is_none() {
                return Err(format!(
                    "no manifest found for repo_id '{}'; register the repo first via /manifest/update",
                    repo_id
                ));
            }
            let affected_nodes = optional_string_array(&args, "affected_nodes")?;
            let affected_edges = optional_string_array(&args, "affected_edges")?;
            validate_mcp_affected_refs(
                &state.metadata,
                &repo_id,
                &affected_nodes,
                &affected_edges,
            )?;
            let affected_nodes_json =
                serde_json::to_string(&affected_nodes).unwrap_or_else(|_| "[]".to_owned());
            let affected_edges_json =
                serde_json::to_string(&affected_edges).unwrap_or_else(|_| "[]".to_owned());
            let branch = optional_string(&args, "branch");
            let tracker =
                optional_string(&args, "tracker").unwrap_or_else(|| "github_projects".to_owned());
            let tracker_content_id = required_string(&args, "tracker_content_id")?;
            let tracker_project_item_id = optional_string(&args, "tracker_project_item_id");
            let tracker_identifier = required_string(&args, "tracker_identifier")?;
            let author = optional_string(&args, "author").unwrap_or_else(|| cli.agent_id.clone());
            let now = chrono::Utc::now().to_rfc3339();
            let overlay_id = mfs_uri::short_hash_hex(
                format!(
                    "{}:{}:{}:{}",
                    &cli.account_id, &repo_id, &overlay_type, &now
                )
                .as_bytes(),
                12,
            );
            let record = mfs_metadata::OverlayRecord {
                id: &overlay_id,
                repo_id: &repo_id,
                overlay_type: &overlay_type,
                tracker: &tracker,
                tracker_content_id: &tracker_content_id,
                tracker_project_item_id: tracker_project_item_id.as_deref(),
                tracker_identifier: &tracker_identifier,
                issue_number: None,
                branch: branch.as_deref(),
                pr_url: None,
                agent_session_id: Some(&cli.agent_id),
                author: &author,
                status: "proposed",
                content_json: &content_json,
                affected_nodes: Some(&affected_nodes_json),
                affected_edges: Some(&affected_edges_json),
                affected_node_refs: Some("[]"),
                affected_edge_refs: Some("[]"),
                created_at: &now,
                updated_at: &now,
                superseded_by: None,
                manifest_id: Some(&repo_id),
                accepted_at: None,
                implemented_at: None,
                merged_at: None,
                stale_at: None,
                abandoned_at: None,
            };
            state.metadata.insert_overlay(&record).map_err(to_string)?;
            // Insert initial transition record
            let transition_id =
                mfs_uri::short_hash_hex(format!("{}:{}:proposed", &overlay_id, &now).as_bytes(), 8);
            let transition = mfs_metadata::OverlayTransitionRecord {
                id: &transition_id,
                overlay_id: &overlay_id,
                from_status: "(none)",
                to_status: "proposed",
                triggered_by: "agent",
                reason: Some("Initial proposal"),
                created_at: &now,
            };
            state
                .metadata
                .insert_overlay_transition(&transition)
                .map_err(to_string)?;
            Ok(json!({
                "status": "ok",
                "data": {
                    "overlay_id": overlay_id,
                    "status": "proposed",
                    "overlay_type": overlay_type,
                },
                "hint": null,
                "version_hash": now,
            }))
        }
        "report_conflict" => {
            let repo_id = required_string(&args, "repo_id")?;
            let overlay_id_1 = required_string(&args, "overlay_id_1")?;
            let overlay_id_2 = required_string(&args, "overlay_id_2")?;
            let overlay_a = state
                .metadata
                .get_overlay(&overlay_id_1)
                .map_err(to_string)?;
            let overlay_b = state
                .metadata
                .get_overlay(&overlay_id_2)
                .map_err(to_string)?;
            match (overlay_a, overlay_b) {
                (Some(a), Some(b)) => {
                    if a.repo_id != repo_id || b.repo_id != repo_id {
                        return Err("both overlays must belong to repo_id".to_owned());
                    }
                    // Parse affected_nodes/edges as JSON arrays (consistent with HTTP handler format)
                    let a_nodes: Vec<String> = serde_json::from_str(
                        &a.affected_nodes.clone().unwrap_or_else(|| "[]".to_owned()),
                    )
                    .unwrap_or_default();
                    let b_nodes: Vec<String> = serde_json::from_str(
                        &b.affected_nodes.clone().unwrap_or_else(|| "[]".to_owned()),
                    )
                    .unwrap_or_default();
                    let a_edges: Vec<String> = serde_json::from_str(
                        &a.affected_edges.clone().unwrap_or_else(|| "[]".to_owned()),
                    )
                    .unwrap_or_default();
                    let b_edges: Vec<String> = serde_json::from_str(
                        &b.affected_edges.clone().unwrap_or_else(|| "[]".to_owned()),
                    )
                    .unwrap_or_default();
                    let overlap_nodes: Vec<&String> =
                        a_nodes.iter().filter(|n| b_nodes.contains(n)).collect();
                    let overlap_edges: Vec<&String> =
                        a_edges.iter().filter(|e| b_edges.contains(e)).collect();
                    let active_statuses = ["accepted", "implemented"];
                    let requires_human_review = (!overlap_nodes.is_empty()
                        || !overlap_edges.is_empty())
                        && (active_statuses.contains(&a.status.as_str())
                            || active_statuses.contains(&b.status.as_str()));
                    let now = chrono::Utc::now().to_rfc3339();
                    let conflict_id = format!(
                        "conflict_{}",
                        mfs_uri::short_hash_hex(
                            format!("{repo_id}:{overlay_id_1}:{overlay_id_2}:{now}").as_bytes(),
                            12,
                        )
                    );
                    Ok(json!({
                        "status": "ok",
                        "data": {
                            "conflict_id": conflict_id,
                            "overlay_id_1": overlay_id_1,
                            "overlay_id_2": overlay_id_2,
                            "has_conflict": !overlap_nodes.is_empty() || !overlap_edges.is_empty(),
                            "requires_human_review": requires_human_review,
                            "overlap_nodes": overlap_nodes,
                            "overlap_edges": overlap_edges,
                            "description": optional_string(&args, "conflict_description"),
                        },
                        "hint": null,
                        "version_hash": now,
                    }))
                }
                _ => Err("one or both overlays not found".to_owned()),
            }
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

fn tools() -> Value {
    json!([
        tool(
            "find",
            "Find MemFuse contexts with retrieval planning",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "target": { "type": "string" }
                },
                "required": ["query"]
            })
        ),
        tool(
            "search",
            "Search MemFuse contexts with optional session context",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "target": { "type": "string" },
                    "session_context": { "type": "string" }
                },
                "required": ["query"]
            })
        ),
        tool(
            "read",
            "Read a MemFuse URI",
            json!({
                "type": "object",
                "properties": { "uri": { "type": "string" } },
                "required": ["uri"]
            })
        ),
        tool(
            "ls",
            "List a MemFuse directory URI",
            json!({
                "type": "object",
                "properties": { "uri": { "type": "string" } },
                "required": ["uri"]
            })
        ),
        tool(
            "task_status",
            "Read session or metadata task state",
            json!({
                "type": "object",
                "properties": { "task_id": { "type": "string" } },
                "required": ["task_id"]
            })
        ),
        tool(
            "wait_task",
            "Wait for a task to reach terminal state",
            json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "timeout_ms": { "type": "integer" },
                    "poll_ms": { "type": "integer" }
                },
                "required": ["task_id"]
            })
        ),
        tool(
            "session_list",
            "List MemFuse sessions",
            json!({
                "type": "object",
                "properties": {}
            })
        ),
        tool(
            "session_get",
            "Get a MemFuse session summary",
            json!({
                "type": "object",
                "properties": { "session_id": { "type": "string" } },
                "required": ["session_id"]
            })
        ),
        tool(
            "session_context",
            "Get assembled MemFuse session context",
            json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "token_budget": { "type": "integer" }
                },
                "required": ["session_id"]
            })
        ),
        tool(
            "session_archive",
            "Get a MemFuse session archive",
            json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "archive_id": { "type": "string" }
                },
                "required": ["session_id", "archive_id"]
            })
        ),
        tool(
            "system_status",
            "Summarize MemFuse system status",
            json!({
                "type": "object",
                "properties": {}
            })
        ),
        tool(
            "observer_status",
            "Read MemFuse observer/runtime status",
            json!({
                "type": "object",
                "properties": {}
            })
        ),
        tool(
            "watches_list",
            "List resource watches and due state",
            json!({
                "type": "object",
                "properties": {}
            })
        ),
        tool(
            "watch_run_due",
            "Run due resource watches",
            json!({
                "type": "object",
                "properties": {}
            })
        ),
        tool(
            "add_skill",
            "Ingest a local skill into MemFuse",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"]
            })
        ),
        tool(
            "skills_list",
            "List MemFuse skills",
            json!({
                "type": "object",
                "properties": {}
            })
        ),
        tool(
            "relation_link",
            "Create a relation between MemFuse URIs",
            json!({
                "type": "object",
                "properties": {
                    "from_uri": { "type": "string" },
                    "to_uri": { "type": "string" },
                    "relation_type": { "type": "string" }
                },
                "required": ["from_uri", "to_uri"]
            })
        ),
        tool(
            "relations_list",
            "List relations for a MemFuse URI",
            json!({
                "type": "object",
                "properties": {
                    "uri": { "type": "string" }
                },
                "required": ["uri"]
            })
        ),
        tool(
            "relation_unlink",
            "Remove a relation between MemFuse URIs",
            json!({
                "type": "object",
                "properties": {
                    "from_uri": { "type": "string" },
                    "to_uri": { "type": "string" },
                    "relation_type": { "type": "string" }
                },
                "required": ["from_uri", "to_uri"]
            })
        ),
        tool(
            "watch_run_loop",
            "Run watch due checks over multiple iterations",
            json!({
                "type": "object",
                "properties": {
                    "iterations": { "type": "integer" },
                    "sleep_ms": { "type": "integer" }
                }
            })
        ),
        tool(
            "session_delete",
            "Delete a MemFuse session and its read-side state",
            json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" }
                },
                "required": ["session_id"]
            })
        ),
        // Heuristic tools (T2H Phase 1)
        tool(
            "heuristics_create_rule",
            "Create a behavioral heuristic rule (T2H). Rules capture user preferences and behavioral patterns.",
            json!({
                "type": "object",
                "properties": {
                    "rule_text": { "type": "string", "description": "Natural language rule describing the preference" },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Structured tags (domain:X, phase:X, language:X, topic:X, pressure:X)" },
                    "counter_examples": { "type": "array", "items": { "type": "string" }, "description": "Exceptions to this rule" },
                    "lifecycle_stage": { "type": "string", "description": "draft, candidate, confirmed, or archived (default: draft)" }
                },
                "required": ["rule_text"]
            })
        ),
        tool(
            "heuristics_list_rules",
            "List all heuristic rules for the current user",
            json!({
                "type": "object",
                "properties": {}
            })
        ),
        tool(
            "heuristics_promote_rule",
            "Promote a heuristic rule to a higher lifecycle stage (draft → candidate → confirmed)",
            json!({
                "type": "object",
                "properties": {
                    "rule_id": { "type": "string" },
                    "new_stage": { "type": "string", "description": "draft, candidate, confirmed, or archived" }
                },
                "required": ["rule_id", "new_stage"]
            })
        ),
        tool(
            "heuristics_confirm_rule",
            "Mark a heuristic rule as user-confirmed (roadmap §5.4). User-confirmed rules are exempt from automatic decay, distinct from lifecycle_stage 'confirmed' which is reached via auto-promotion.",
            json!({
                "type": "object",
                "properties": {
                    "rule_id": { "type": "string" }
                },
                "required": ["rule_id"]
            })
        ),
        tool(
            "heuristics_create_instance",
            "Record a feedback signal instance (T2H). Captures user reaction to agent behavior.",
            json!({
                "type": "object",
                "properties": {
                    "context_summary": { "type": "string", "description": "What was happening when the signal occurred" },
                    "user_reaction": { "type": "string", "description": "What the user said or did" },
                    "signal_type": { "type": "string", "description": "explicit_negation, implicit_negation, preference_declaration, or tradeoff_decision" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "agent_proposal": { "type": "string" },
                    "outcome": { "type": "string" },
                    "session_id": { "type": "string" }
                },
                "required": ["context_summary", "user_reaction", "signal_type"]
            })
        ),
        tool(
            "heuristics_retrieve",
            "Retrieve heuristic rules matching a query and tags (three-phase retrieval)",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "top_k": { "type": "integer" }
                },
                "required": ["query"]
            })
        ),
        tool(
            "heuristics_l0_confirmed",
            "Get top confirmed heuristic rules for L0 session-start injection",
            json!({
                "type": "object",
                "properties": {
                    "max_rules": { "type": "integer", "description": "Maximum rules to return (default: 5)" }
                }
            })
        ),
        tool(
            "simulate_reaction",
            "Simulate the user's likely reaction to a proposed action based on learned heuristic rules (L2 injection). Returns relevant rules and a prediction summary.",
            json!({
                "type": "object",
                "properties": {
                    "scenario": { "type": "string", "description": "Description of the proposed action or scenario to evaluate" },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Optional tags to narrow rule search (domain:X, phase:X, etc.)" }
                },
                "required": ["scenario"]
            })
        ),
        tool(
            "get_repo_manifest",
            "Get the manifest identity for a repository (repo_id, resource_uri, default_branch, primary_languages, etc.)",
            json!({
                "type": "object",
                "properties": {
                    "repo_id": { "type": "string", "description": "Unique identifier for the repository (e.g. 'github:owner/repo'). Defaults to account_id if omitted." }
                }
            })
        ),
        tool(
            "query_canvas",
            "Query the canvas for a repository: returns nodes, edges, active overlays, and conflicts",
            json!({
                "type": "object",
                    "properties": {
                        "repo_id": { "type": "string", "description": "Unique identifier for the repository" },
                        "component": { "type": "string", "description": "Optional component/module/function name filter" },
                        "type": { "type": "string", "enum": ["structural", "contracts", "status"], "description": "Optional canvas subset" },
                        "node_type": { "type": "string", "description": "Optional filter by node type (module, interface, data_model, api_endpoint, utility, config, entry_point)" },
                        "status": { "type": "string", "description": "Optional filter overlays by status (proposed, accepted, implemented, merged, abandoned, stale, rejected)" }
                    },
                "required": ["repo_id"]
            })
        ),
        tool(
            "propose_active_overlay",
            "Propose a new active overlay (planned change, contract, test, config, or conflict declaration). Agent-authored overlays start in 'proposed' status.",
            json!({
                "type": "object",
                "properties": {
                        "repo_id": { "type": "string", "description": "Repository identifier (must have a registered manifest)" },
                        "overlay_type": { "type": "string", "description": "One of: planned_change, planned_contract, conflict_declaration, planned_test, planned_config" },
                        "content_json": { "description": "JSON value describing the overlay content" },
                        "affected_nodes": { "type": "array", "items": { "type": "string" }, "description": "Canvas node IDs this overlay touches" },
                        "affected_edges": { "type": "array", "items": { "type": "string" }, "description": "Canvas edge IDs this overlay touches" },
                        "branch": { "type": "string", "description": "Optional branch name" },
                        "tracker": { "type": "string", "description": "Tracker type, e.g. 'github_projects' (default: 'github_projects')" },
                        "tracker_content_id": { "type": "string", "description": "Tracker content ID (GitHub Issue/PR node id) — required" },
                        "tracker_project_item_id": { "type": "string", "description": "Optional tracker project item ID" },
                        "tracker_identifier": { "type": "string", "description": "Tracker identifier (e.g. 'owner/repo#42') — required" },
                        "author": { "type": "string", "description": "Optional author identity; defaults to agent_id" }
                    },
                "required": ["repo_id", "overlay_type", "content_json", "tracker_content_id", "tracker_identifier"]
            })
        ),
        tool(
            "report_conflict",
            "Report a conflict between two overlays by checking node/edge overlap",
            json!({
                "type": "object",
                "properties": {
                    "repo_id": { "type": "string", "description": "Repository identifier" },
                    "overlay_id_1": { "type": "string", "description": "First overlay ID" },
                    "overlay_id_2": { "type": "string", "description": "Second overlay ID" },
                    "conflict_description": { "type": "string", "description": "Optional human-readable description of the conflict" }
                },
                "required": ["repo_id", "overlay_id_1", "overlay_id_2"]
            })
        )
    ])
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema
    })
}

fn open_fs(cli: &Cli, uri: Option<&str>) -> Result<WorkspaceFs, Box<dyn std::error::Error>> {
    let scope = uri.unwrap_or("mfs://resources");
    Ok(WorkspaceFs::open_existing_for_uri(
        &cli.workspace_root,
        &cli.account_id,
        &cli.user_id,
        &cli.agent_id,
        Some(scope),
    )?)
}

fn required_string(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("missing required string argument: {key}"))
}

fn normalize_json_argument(args: &Value, key: &str) -> Result<String, String> {
    let value = args
        .get(key)
        .ok_or_else(|| format!("missing required argument: {key}"))?;
    if let Some(raw) = value.as_str() {
        if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
            return serde_json::to_string(&parsed).map_err(to_string);
        }
    }
    serde_json::to_string(value).map_err(to_string)
}

fn optional_string_array(args: &Value, key: &str) -> Result<Vec<String>, String> {
    let Some(value) = args.get(key) else {
        return Ok(Vec::new());
    };
    if let Some(items) = value.as_array() {
        return items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{key} must contain only strings"))
            })
            .collect();
    }
    if let Some(raw) = value.as_str() {
        if raw.trim().is_empty() {
            return Ok(Vec::new());
        }
        if let Ok(items) = serde_json::from_str::<Vec<String>>(raw) {
            return Ok(items);
        }
        return Ok(raw
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect());
    }
    Err(format!("{key} must be an array of strings"))
}

fn read_mcp_manifest_json(path: &Path) -> Result<Value, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read Manifest YAML '{}': {}", path.display(), error))?;
    serde_yaml::from_str::<Value>(&raw)
        .map_err(|error| format!("invalid Manifest YAML '{}': {}", path.display(), error))
}

fn merge_mcp_manifest_metadata(
    manifest: &mut Value,
    identity: &mfs_metadata::StoredManifestIdentity,
    metadata: &MetadataStore,
) -> Result<(), String> {
    let object = manifest
        .as_object_mut()
        .ok_or_else(|| "Manifest YAML must be a mapping".to_owned())?;
    let repo_identity = object
        .entry("repo_identity")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| "repo_identity must be an object".to_owned())?;
    repo_identity.insert("repo_id".into(), json!(identity.repo_id));
    repo_identity.insert("resource_uri".into(), json!(identity.resource_uri));
    repo_identity.insert("default_branch".into(), json!(identity.default_branch));
    let languages =
        serde_json::from_str::<Value>(&identity.primary_languages).unwrap_or_else(|_| json!([]));
    repo_identity.insert("primary_languages".into(), languages);
    repo_identity.insert("created_at".into(), json!(identity.created_at));
    repo_identity.insert("last_verified_at".into(), json!(identity.last_verified_at));
    object.insert(
        "manifest_yaml_path".into(),
        json!(identity.manifest_yaml_path),
    );

    let overlays = metadata
        .list_active_overlays(&identity.repo_id, None)
        .map_err(to_string)?;
    object.insert(
        "active_overlays".into(),
        serde_json::to_value(overlays).unwrap_or_else(|_| json!([])),
    );
    Ok(())
}

fn parse_overlay_refs(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
}

fn detect_mcp_overlay_conflicts(overlays: &[mfs_metadata::StoredOverlay]) -> Vec<Value> {
    let active_statuses = ["accepted", "implemented"];
    let active_overlays: Vec<&mfs_metadata::StoredOverlay> = overlays
        .iter()
        .filter(|overlay| active_statuses.contains(&overlay.status.as_str()))
        .collect();
    let mut conflicts = Vec::new();
    for i in 0..active_overlays.len() {
        for j in (i + 1)..active_overlays.len() {
            let a = active_overlays[i];
            let b = active_overlays[j];
            let a_nodes = parse_overlay_refs(a.affected_nodes.as_deref());
            let b_nodes = parse_overlay_refs(b.affected_nodes.as_deref());
            let a_edges = parse_overlay_refs(a.affected_edges.as_deref());
            let b_edges = parse_overlay_refs(b.affected_edges.as_deref());
            let overlap_nodes: Vec<String> = a_nodes
                .iter()
                .filter(|node| b_nodes.contains(*node))
                .cloned()
                .collect();
            let overlap_edges: Vec<String> = a_edges
                .iter()
                .filter(|edge| b_edges.contains(*edge))
                .cloned()
                .collect();
            if !overlap_nodes.is_empty() || !overlap_edges.is_empty() {
                conflicts.push(json!({
                    "overlay_a": a.id,
                    "overlay_b": b.id,
                    "overlap_nodes": overlap_nodes,
                    "overlap_edges": overlap_edges,
                }));
            }
        }
    }
    conflicts
}

fn validate_mcp_affected_refs(
    metadata: &MetadataStore,
    repo_id: &str,
    node_ids: &[String],
    edge_ids: &[String],
) -> Result<(), String> {
    for node_id in node_ids {
        match metadata.get_canvas_node(node_id).map_err(to_string)? {
            Some(node) if node.repo_id == repo_id => {}
            Some(_) => return Err(format!("node '{node_id}' belongs to a different repo")),
            None => return Err(format!("node '{node_id}' does not exist")),
        }
    }
    for edge_id in edge_ids {
        match metadata.get_canvas_edge(edge_id).map_err(to_string)? {
            Some(edge) if edge.repo_id == repo_id => {}
            Some(_) => return Err(format!("edge '{edge_id}' belongs to a different repo")),
            None => return Err(format!("edge '{edge_id}' does not exist")),
        }
    }
    Ok(())
}

fn optional_string(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn optional_u64(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(Value::as_u64)
}

fn to_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn marker_for_stage(stage: &str) -> &'static str {
    match stage {
        "confirmed" => "★",
        "candidate" => "◆",
        "draft" => "○",
        _ => "?",
    }
}

fn read_message(reader: &mut impl BufRead) -> Result<Option<Value>, Box<dyn std::error::Error>> {
    let mut header = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            if header.is_empty() {
                return Ok(None);
            }
            return Err("unexpected EOF while reading MCP headers".into());
        }
        header.push_str(&line);
        if header.ends_with("\r\n\r\n") {
            break;
        }
    }
    let content_length = header
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .ok_or("missing Content-Length header")?
        .trim()
        .parse::<usize>()?;
    let mut body = vec![0_u8; content_length];
    reader.read_exact(&mut body)?;
    Ok(Some(serde_json::from_slice(&body)?))
}

fn write_message(
    writer: &mut impl Write,
    payload: &Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let body = payload.to_string();
    write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfs_metadata::{CanvasEdgeRecord, CanvasNodeRecord, ManifestIdentityRecord, OverlayRecord};

    async fn test_state(
        workspace_root: PathBuf,
        metadata: MetadataStore,
    ) -> Result<McpState, Box<dyn std::error::Error>> {
        let session_engine = SessionEngine::open_with_threshold(
            &workspace_root,
            mfs_session::AUTO_COMMIT_THRESHOLD_DEFAULT,
        )
        .await?;
        Ok(McpState {
            cli: Cli {
                workspace_root,
                account_id: "acct".into(),
                user_id: "user".into(),
                agent_id: "agent".into(),
            },
            metadata: Arc::new(metadata),
            session_engine: Arc::new(session_engine),
            embedding_provider: mfs_semantic::embedding_provider_from_env(256),
        })
    }

    #[tokio::test]
    async fn repo_intelligence_tools_return_prd_shaped_data() {
        let workspace = tempfile::tempdir().unwrap();
        let manifest_path = workspace.path().join("MANIFEST.yaml");
        std::fs::write(
            &manifest_path,
            r#"repo_identity:
  repo_id: symphony-gh
  default_branch: main
  primary_languages:
    - elixir
memory_assets: []
canvas_indexes:
  - type: structural
    location: mfs://resources/localfs/symphony-gh/canvas/structural
    generator: regex-deterministic
    generator_version: 0.1.0
    generated_at: "2026-05-11T00:00:00Z"
    version_hash: v1
    confidence: deterministic
    freshness: on_change
source_roots: []
active_overlays: []
quality_gates: []
conflicts: []
"#,
        )
        .unwrap();

        let metadata = MetadataStore::open_in_memory(false).unwrap();
        metadata
            .upsert_manifest_identity(&ManifestIdentityRecord {
                repo_id: "symphony-gh",
                resource_uri: "mfs://resources/localfs/symphony-gh/MANIFEST.yaml",
                default_branch: "main",
                primary_languages: r#"["elixir"]"#,
                created_at: "2026-05-11T00:00:00Z",
                last_verified_at: "2026-05-11T01:00:00Z",
                manifest_yaml_path: Some(manifest_path.to_str().unwrap()),
                repo_name: None,
                repo_path: None,
                last_commit_hash: None,
                last_commit_date: None,
                manifest_version: "1",
                yaml_hash: None,
                source_roots_json: "[]",
                quality_gates_json: "{}",
                updated_at: "2026-05-11T01:00:00Z",
            })
            .unwrap();
        metadata
            .upsert_canvas_node(&CanvasNodeRecord {
                id: "node-runner",
                repo_id: "symphony-gh",
                node_type: "module",
                name: "SymphonyGh.Runner",
                path: Some("lib/runner.ex"),
                language: Some("elixir"),
                purpose: Some("module"),
                confidence: "deterministic",
                generator: "regex-deterministic",
                generated_at: "2026-05-11T00:00:00Z",
                version_hash: "v1",
                source: None,
                manifest_id: Some("symphony-gh"),
                created_at: "2026-05-11T00:00:00Z",
                updated_at: "2026-05-11T00:00:00Z",
            })
            .unwrap();
        metadata
            .upsert_canvas_node(&CanvasNodeRecord {
                id: "node-start-link",
                repo_id: "symphony-gh",
                node_type: "function",
                name: "start_link",
                path: Some("lib/runner.ex"),
                language: Some("elixir"),
                purpose: Some("function"),
                confidence: "deterministic",
                generator: "regex-deterministic",
                generated_at: "2026-05-11T00:00:00Z",
                version_hash: "v1",
                source: None,
                manifest_id: Some("symphony-gh"),
                created_at: "2026-05-11T00:00:00Z",
                updated_at: "2026-05-11T00:00:00Z",
            })
            .unwrap();
        metadata
            .upsert_canvas_edge(&CanvasEdgeRecord {
                id: "edge-start-link",
                repo_id: "symphony-gh",
                edge_type: "implements",
                source_node_id: "node-runner",
                target_node_id: "node-start-link",
                contract_spec: None,
                confidence: "deterministic",
                generator: "regex-deterministic",
                generated_at: "2026-05-11T00:00:00Z",
                version_hash: "v1",
                manifest_id: Some("symphony-gh"),
                created_at: "2026-05-11T00:00:00Z",
                updated_at: "2026-05-11T00:00:00Z",
            })
            .unwrap();
        for overlay_id in ["overlay-a", "overlay-b"] {
            metadata
                .insert_overlay(&OverlayRecord {
                    id: overlay_id,
                    repo_id: "symphony-gh",
                    overlay_type: "planned_change",
                    tracker: "github_projects",
                    tracker_content_id: overlay_id,
                    tracker_project_item_id: None,
                    tracker_identifier: "owner/repo#1",
                    issue_number: None,
                    branch: None,
                    pr_url: None,
                    agent_session_id: Some("agent"),
                    author: "agent",
                    status: "accepted",
                    content_json: r#"{"summary":"change"}"#,
                    affected_nodes: Some(r#"["node-runner"]"#),
                    affected_edges: None,
                    affected_node_refs: Some("[]"),
                    affected_edge_refs: Some("[]"),
                    created_at: "2026-05-11T00:00:00Z",
                    updated_at: "2026-05-11T00:00:00Z",
                    superseded_by: None,
                    manifest_id: Some("symphony-gh"),
                    accepted_at: None,
                    implemented_at: None,
                    merged_at: None,
                    stale_at: None,
                    abandoned_at: None,
                })
                .unwrap();
        }

        let state = test_state(workspace.path().to_path_buf(), metadata)
            .await
            .unwrap();

        let manifest = call_tool(
            &state,
            "get_repo_manifest",
            json!({ "repo_id": "symphony-gh" }),
        )
        .await
        .unwrap();
        assert_eq!(manifest["data"]["repo_identity"]["repo_id"], "symphony-gh");
        assert_eq!(
            manifest["data"]["repo_identity"]["resource_uri"],
            "mfs://resources/localfs/symphony-gh/MANIFEST.yaml"
        );
        assert_eq!(
            manifest["data"]["active_overlays"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        let no_match = call_tool(
            &state,
            "query_canvas",
            json!({ "repo_id": "symphony-gh", "component": "Missing", "type": "structural" }),
        )
        .await
        .unwrap();
        assert_eq!(no_match["data"]["nodes"].as_array().unwrap().len(), 0);
        assert_eq!(no_match["data"]["edges"].as_array().unwrap().len(), 0);

        let contracts = call_tool(
            &state,
            "query_canvas",
            json!({ "repo_id": "symphony-gh", "type": "contracts" }),
        )
        .await
        .unwrap();
        assert_eq!(contracts["data"]["nodes"].as_array().unwrap().len(), 0);
        assert_eq!(contracts["data"]["edges"].as_array().unwrap().len(), 0);

        let conflict = call_tool(
            &state,
            "report_conflict",
            json!({
                "repo_id": "symphony-gh",
                "overlay_id_1": "overlay-a",
                "overlay_id_2": "overlay-b",
                "conflict_description": "same module"
            }),
        )
        .await
        .unwrap();
        assert!(
            conflict["data"]["conflict_id"]
                .as_str()
                .unwrap()
                .starts_with("conflict_")
        );
        assert_eq!(conflict["data"]["has_conflict"], true);
        assert_eq!(conflict["data"]["requires_human_review"], true);
        assert_eq!(conflict["data"]["description"], "same module");

        let proposed = call_tool(
            &state,
            "propose_active_overlay",
            json!({
                "repo_id": "symphony-gh",
                "overlay_type": "planned_test",
                "content_json": { "summary": "cover runner" },
                "affected_nodes": ["node-runner"],
                "affected_edges": ["edge-start-link"],
                "tracker_content_id": "content-1",
                "tracker_identifier": "owner/repo#2",
                "author": "codex"
            }),
        )
        .await
        .unwrap();
        assert_eq!(proposed["data"]["status"], "proposed");
        let overlay_id = proposed["data"]["overlay_id"].as_str().unwrap();
        let stored = state
            .metadata
            .get_overlay(overlay_id)
            .unwrap()
            .expect("overlay should persist");
        assert_eq!(stored.tracker_identifier, "owner/repo#2");
        assert_eq!(stored.author, "codex");
        assert_eq!(stored.content_json, r#"{"summary":"cover runner"}"#);
    }
}
