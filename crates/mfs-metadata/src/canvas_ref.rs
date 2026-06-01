//! Canonical Canvas Ref specification (SaaS §5.3, Appendix A).
//!
//! This module defines the `canvas://` URI scheme for referencing Canvas objects
//! (nodes and edges) across domains, and provides bidirectional conversion
//! between local Canvas IDs (Elixir regex-generator format) and canonical refs.

// ─── Types ──────────────────────────────────────────────────────────────────

/// Kind of Canvas object referenced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanvasRefKind {
    Node,
    Edge,
}

/// Result of resolving a canonical ref against local Canvas data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveResult {
    /// Successfully mapped to a local canvas_nodes/edges ID.
    Resolved { local_id: String },
    /// Valid ref but currently cannot map (e.g., component renamed or Canvas stale).
    Unresolved { reason: String },
    /// Invalid ref format.
    InvalidRef { reason: String },
}

/// Components extracted from a parsed canonical ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalRefComponents {
    pub repo_id: String,
    pub kind: CanvasRefKind,
    /// For nodes: "module", "function", "type", "config".
    /// For edges: the edge_type (e.g., "implements", "call").
    pub sub_type: String,
    /// For nodes: qualified name (e.g., "Symphony.Orchestrator").
    /// For edges: "source_tail->target_tail".
    pub qualified_name: String,
}

// ─── Canonical ref generators ───────────────────────────────────────────────

/// Generate a canonical node ref: `canvas://{repo_id}/node/{node_type}/{qualified_name}`
pub fn node_ref(repo_id: &str, node_type: &str, qualified_name: &str) -> String {
    format!("canvas://{repo_id}/node/{node_type}/{qualified_name}")
}

/// Generate a canonical function ref (with arity):
/// `canvas://{repo_id}/node/function/{qualified_module}.{function}/{arity}`
pub fn function_ref(repo_id: &str, qualified_module: &str, function: &str, arity: usize) -> String {
    format!("canvas://{repo_id}/node/function/{qualified_module}.{function}/{arity}")
}

/// Generate a canonical edge ref:
/// `canvas://{repo_id}/edge/{edge_type}/{source_tail}->{target_tail}`
pub fn edge_ref(repo_id: &str, edge_type: &str, source_tail: &str, target_tail: &str) -> String {
    format!("canvas://{repo_id}/edge/{edge_type}/{source_tail}->{target_tail}")
}

/// Generate a canonical config ref:
/// `canvas://{repo_id}/node/config/{path}#{key}`
pub fn config_ref(repo_id: &str, path: &str, key: &str) -> String {
    format!("canvas://{repo_id}/node/config/{path}#{key}")
}

// ─── Local ID → canonical ref ───────────────────────────────────────────────

/// Convert a local Canvas ID to its canonical ref equivalent.
///
/// Supported local ID formats (Elixir regex generator):
/// - Module:  `{repo_id}:module:{module_name}`
/// - Function: `{repo_id}:function:{module_name}.{function_name}/{arity}`
/// - Type:    `{repo_id}:type:{type_name}`
/// - Config:  `{repo_id}:config:{path}#{key}`
/// - Edge:    `{repo_id}:module:{module_name}:{edge_type}:{repo_id}:function:{module_name}.{function_name}/{arity}`
///
/// The edge format embeds both source and target local IDs separated by the edge type.
/// We also handle edge IDs where the source/target use other node types (type, config, etc.).
pub fn local_id_to_canonical_ref(local_id: &str) -> ResolveResult {
    // ── Edge IDs ────────────────────────────────────────────────────────
    // Edge IDs contain two embedded local IDs separated by an edge_type colon segment.
    // Pattern: {repo_id}:{source_node_type}:{source_name}:{edge_type}:{repo_id}:{target_node_type}:{target_name}
    //
    // We detect edges by checking whether the ID contains an edge_type keyword
    // between two colon-separated local-ID-like segments.
    let edge_types = [
        "import",
        "call",
        "contract",
        "depends_on",
        "implements",
        "tests",
    ];

    for et in &edge_types {
        // Look for `:{et}:` separator inside the ID.
        // The format is:  left_part :edge_type: right_part
        // where left_part and right_part each look like a local node ID.
        if let Some(idx) = local_id.find(&format!(":{et}:")) {
            let source_local = &local_id[..idx];
            let target_local = &local_id[idx + et.len() + 2..]; // skip `:et:`

            // Extract repo_id from source (first colon-delimited segment).
            let source_repo = match source_local.split(':').next() {
                Some(r) => r,
                None => {
                    return ResolveResult::InvalidRef {
                        reason: "edge ID: cannot extract source repo_id".into(),
                    };
                }
            };

            let source_tail = local_node_id_to_ref_tail(source_local);
            let target_tail = local_node_id_to_ref_tail(target_local);

            match (source_tail, target_tail) {
                (Ok(s), Ok(t)) => {
                    return ResolveResult::Resolved {
                        local_id: edge_ref(source_repo, et, &s, &t),
                    };
                }
                _ => {
                    return ResolveResult::InvalidRef {
                        reason: format!(
                            "edge ID: cannot parse source or target local ID in '{}'",
                            local_id
                        ),
                    };
                }
            }
        }
    }

    // ── Node IDs ────────────────────────────────────────────────────────
    // Pattern: {repo_id}:{node_type}:{qualified_name}
    let parts: Vec<&str> = local_id.splitn(3, ':').collect();
    if parts.len() != 3 {
        return ResolveResult::InvalidRef {
            reason: format!(
                "local ID must have at least 3 colon-separated parts, got '{}'",
                local_id
            ),
        };
    }

    let repo_id = parts[0];
    let node_type = parts[1];
    let name_part = parts[2];

    // Validate node_type against known Canvas node types.
    let valid_node_types = [
        "module",
        "function",
        "type",
        "config",
        "component",
        "entry_point",
        "test_suite",
    ];
    if !valid_node_types.contains(&node_type) {
        return ResolveResult::InvalidRef {
            reason: format!(
                "unknown node_type '{}' in local ID '{}'",
                node_type, local_id
            ),
        };
    }

    match node_type {
        "module" => ResolveResult::Resolved {
            local_id: node_ref(repo_id, "module", name_part),
        },
        "function" => {
            // name_part is `{module}.{function}/{arity}` — already matches canonical format.
            ResolveResult::Resolved {
                local_id: node_ref(repo_id, "function", name_part),
            }
        }
        "type" => ResolveResult::Resolved {
            local_id: node_ref(repo_id, "type", name_part),
        },
        "config" => {
            // name_part is `{path}#{key}` — already matches canonical format.
            ResolveResult::Resolved {
                local_id: node_ref(repo_id, "config", name_part),
            }
        }
        other => ResolveResult::Resolved {
            local_id: node_ref(repo_id, other, name_part),
        },
    }
}

/// Extract the "ref tail" from a local node ID — the part after `node/{node_type}/`
/// in a canonical ref, derived from the local ID's name segment.
///
/// For a local ID like `symphony-gh:module:Symphony.Orchestrator`,
/// the tail is `Symphony.Orchestrator`.
///
/// For a local ID like `symphony-gh:function:Symphony.Orchestrator.run/1`,
/// the tail is `Symphony.Orchestrator.run/1`.
fn local_node_id_to_ref_tail(local_node_id: &str) -> Result<String, String> {
    let parts: Vec<&str> = local_node_id.splitn(3, ':').collect();
    if parts.len() != 3 {
        return Err(format!("cannot parse local node ID '{}'", local_node_id));
    }
    // The name part is already in the canonical format for the tail.
    Ok(parts[2].to_string())
}

// ─── Canonical ref parser ───────────────────────────────────────────────────

/// Parse a canonical ref string and return its components.
///
/// Expected formats:
/// - Node: `canvas://{repo_id}/node/{sub_type}/{qualified_name}`
/// - Edge: `canvas://{repo_id}/edge/{sub_type}/{source_tail}->{target_tail}`
pub fn parse_canonical_ref(ref_str: &str) -> Result<CanonicalRefComponents, String> {
    if !ref_str.starts_with("canvas://") {
        return Err(format!(
            "canonical ref must start with 'canvas://', got '{}'",
            ref_str
        ));
    }

    let without_scheme = &ref_str["canvas://".len()..];

    // Split into repo_id and the rest.
    let slash_idx = without_scheme
        .find('/')
        .ok_or_else(|| format!("canonical ref missing '/' after repo_id in '{}'", ref_str))?;

    let repo_id = &without_scheme[..slash_idx];
    let rest = &without_scheme[slash_idx + 1..];

    if repo_id.is_empty() {
        return Err(format!("repo_id is empty in canonical ref '{}'", ref_str));
    }

    // rest is either "node/{sub_type}/{qualified_name}" or "edge/{sub_type}/{source}->{target}"
    let (kind_str, after_kind) = rest
        .split_once('/')
        .ok_or_else(|| format!("canonical ref missing '/' after kind in '{}'", ref_str))?;

    let kind = match kind_str {
        "node" => CanvasRefKind::Node,
        "edge" => CanvasRefKind::Edge,
        other => {
            return Err(format!(
                "canonical ref kind must be 'node' or 'edge', got '{}' in '{}'",
                other, ref_str
            ));
        }
    };

    // After kind: "{sub_type}/{qualified_name}" or "{sub_type}/{source}->{target}"
    let (sub_type, name_part) = after_kind
        .split_once('/')
        .ok_or_else(|| format!("canonical ref missing '/' after sub_type in '{}'", ref_str))?;

    if sub_type.is_empty() {
        return Err(format!("sub_type is empty in canonical ref '{}'", ref_str));
    }

    // Validate sub_type based on kind.
    match &kind {
        CanvasRefKind::Node => {
            let valid_node_sub_types = [
                "module",
                "function",
                "type",
                "config",
                "component",
                "entry_point",
                "test_suite",
            ];
            if !valid_node_sub_types.contains(&sub_type) {
                return Err(format!(
                    "invalid node sub_type '{}' in canonical ref '{}'",
                    sub_type, ref_str
                ));
            }
        }
        CanvasRefKind::Edge => {
            let valid_edge_types = [
                "import",
                "call",
                "contract",
                "depends_on",
                "implements",
                "tests",
            ];
            if !valid_edge_types.contains(&sub_type) {
                return Err(format!(
                    "invalid edge_type '{}' in canonical ref '{}'",
                    sub_type, ref_str
                ));
            }
        }
    }

    if name_part.is_empty() {
        return Err(format!(
            "qualified_name is empty in canonical ref '{}'",
            ref_str
        ));
    }

    // For edges, validate that name_part contains "->".
    if kind == CanvasRefKind::Edge && !name_part.contains("->") {
        return Err(format!(
            "edge canonical ref qualified_name must contain '->', got '{}' in '{}'",
            name_part, ref_str
        ));
    }

    Ok(CanonicalRefComponents {
        repo_id: repo_id.to_string(),
        kind,
        sub_type: sub_type.to_string(),
        qualified_name: name_part.to_string(),
    })
}

/// Validate that a string is a well-formed canonical ref.
pub fn validate_canonical_ref(ref_str: &str) -> Result<(), String> {
    parse_canonical_ref(ref_str)?;
    Ok(())
}

// ─── Unit tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── node_ref tests ──────────────────────────────────────────────────

    #[test]
    fn node_ref_module() {
        assert_eq!(
            node_ref("symphony-gh", "module", "Symphony.Orchestrator"),
            "canvas://symphony-gh/node/module/Symphony.Orchestrator"
        );
    }

    #[test]
    fn node_ref_function() {
        assert_eq!(
            node_ref("symphony-gh", "function", "Symphony.Orchestrator.run/1"),
            "canvas://symphony-gh/node/function/Symphony.Orchestrator.run/1"
        );
    }

    #[test]
    fn node_ref_type() {
        assert_eq!(
            node_ref("symphony-gh", "type", "Symphony.Workflow"),
            "canvas://symphony-gh/node/type/Symphony.Workflow"
        );
    }

    #[test]
    fn node_ref_config() {
        assert_eq!(
            node_ref("symphony-gh", "config", "config/runtime.exs#database_url"),
            "canvas://symphony-gh/node/config/config/runtime.exs#database_url"
        );
    }

    // ── function_ref tests ──────────────────────────────────────────────

    #[test]
    fn function_ref_basic() {
        assert_eq!(
            function_ref("symphony-gh", "Symphony.Orchestrator", "run", 1),
            "canvas://symphony-gh/node/function/Symphony.Orchestrator.run/1"
        );
    }

    #[test]
    fn function_ref_arity_zero() {
        assert_eq!(
            function_ref("my-repo", "MyApp.Handler", "start", 0),
            "canvas://my-repo/node/function/MyApp.Handler.start/0"
        );
    }

    // ── edge_ref tests ──────────────────────────────────────────────────

    #[test]
    fn edge_ref_implements() {
        assert_eq!(
            edge_ref(
                "symphony-gh",
                "implements",
                "Symphony.Orchestrator",
                "Symphony.Orchestrator.run/1"
            ),
            "canvas://symphony-gh/edge/implements/Symphony.Orchestrator->Symphony.Orchestrator.run/1"
        );
    }

    #[test]
    fn edge_ref_call() {
        assert_eq!(
            edge_ref(
                "symphony-gh",
                "call",
                "Symphony.Engine",
                "Symphony.Engine.process/2"
            ),
            "canvas://symphony-gh/edge/call/Symphony.Engine->Symphony.Engine.process/2"
        );
    }

    #[test]
    fn edge_ref_depends_on() {
        assert_eq!(
            edge_ref("my-repo", "depends_on", "App.Router", "App.Config"),
            "canvas://my-repo/edge/depends_on/App.Router->App.Config"
        );
    }

    #[test]
    fn edge_ref_import() {
        assert_eq!(
            edge_ref("my-repo", "import", "App.Server", "App.Logger"),
            "canvas://my-repo/edge/import/App.Server->App.Logger"
        );
    }

    // ── config_ref tests ────────────────────────────────────────────────

    #[test]
    fn config_ref_basic() {
        assert_eq!(
            config_ref("symphony-gh", "config/runtime.exs", "database_url"),
            "canvas://symphony-gh/node/config/config/runtime.exs#database_url"
        );
    }

    // ── local_id_to_canonical_ref tests ─────────────────────────────────

    #[test]
    fn local_id_module() {
        let result = local_id_to_canonical_ref("symphony-gh:module:Symphony.Orchestrator");
        match result {
            ResolveResult::Resolved { local_id } => {
                assert_eq!(
                    local_id,
                    "canvas://symphony-gh/node/module/Symphony.Orchestrator"
                );
            }
            _ => panic!("expected Resolved, got {:?}", result),
        }
    }

    #[test]
    fn local_id_function() {
        let result = local_id_to_canonical_ref("symphony-gh:function:Symphony.Orchestrator.run/1");
        match result {
            ResolveResult::Resolved { local_id } => {
                assert_eq!(
                    local_id,
                    "canvas://symphony-gh/node/function/Symphony.Orchestrator.run/1"
                );
            }
            _ => panic!("expected Resolved, got {:?}", result),
        }
    }

    #[test]
    fn local_id_type() {
        let result = local_id_to_canonical_ref("symphony-gh:type:Symphony.Workflow");
        match result {
            ResolveResult::Resolved { local_id } => {
                assert_eq!(local_id, "canvas://symphony-gh/node/type/Symphony.Workflow");
            }
            _ => panic!("expected Resolved, got {:?}", result),
        }
    }

    #[test]
    fn local_id_config() {
        let result =
            local_id_to_canonical_ref("symphony-gh:config:config/runtime.exs#database_url");
        match result {
            ResolveResult::Resolved { local_id } => {
                assert_eq!(
                    local_id,
                    "canvas://symphony-gh/node/config/config/runtime.exs#database_url"
                );
            }
            _ => panic!("expected Resolved, got {:?}", result),
        }
    }

    #[test]
    fn local_id_edge_implements() {
        // Format: {repo_id}:module:{module_name}:implements:{repo_id}:function:{module_name}.{function}/{arity}
        let result = local_id_to_canonical_ref(
            "symphony-gh:module:Symphony.Orchestrator:implements:symphony-gh:function:Symphony.Orchestrator.run/1",
        );
        match result {
            ResolveResult::Resolved { local_id } => {
                assert_eq!(
                    local_id,
                    "canvas://symphony-gh/edge/implements/Symphony.Orchestrator->Symphony.Orchestrator.run/1"
                );
            }
            _ => panic!("expected Resolved, got {:?}", result),
        }
    }

    #[test]
    fn local_id_edge_call() {
        let result = local_id_to_canonical_ref(
            "my-repo:module:App.Server:call:my-repo:function:App.Server.handle/2",
        );
        match result {
            ResolveResult::Resolved { local_id } => {
                assert_eq!(
                    local_id,
                    "canvas://my-repo/edge/call/App.Server->App.Server.handle/2"
                );
            }
            _ => panic!("expected Resolved, got {:?}", result),
        }
    }

    #[test]
    fn local_id_edge_type_to_function() {
        // Edge where source is a type node, target is a function node.
        let result = local_id_to_canonical_ref(
            "symphony-gh:type:Symphony.Workflow:tests:symphony-gh:function:Symphony.Workflow.validate/1",
        );
        match result {
            ResolveResult::Resolved { local_id } => {
                assert_eq!(
                    local_id,
                    "canvas://symphony-gh/edge/tests/Symphony.Workflow->Symphony.Workflow.validate/1"
                );
            }
            _ => panic!("expected Resolved, got {:?}", result),
        }
    }

    #[test]
    fn local_id_invalid_format() {
        let result = local_id_to_canonical_ref("no-colons-here");
        match result {
            ResolveResult::InvalidRef { reason } => {
                assert!(reason.contains("3 colon-separated parts"));
            }
            _ => panic!("expected InvalidRef, got {:?}", result),
        }
    }

    #[test]
    fn local_id_unknown_node_type() {
        let result = local_id_to_canonical_ref("repo:unknown_type:SomeName");
        match result {
            ResolveResult::InvalidRef { reason } => {
                assert!(reason.contains("unknown node_type"));
            }
            _ => panic!("expected InvalidRef, got {:?}", result),
        }
    }

    // ── Same-named function in different module must NOT collide ─────────

    #[test]
    fn same_function_name_different_module_no_collision() {
        let ref_a = function_ref("symphony-gh", "Symphony.Orchestrator", "run", 1);
        let ref_b = function_ref("symphony-gh", "Symphony.Engine", "run", 1);
        assert_ne!(ref_a, ref_b);
        assert_eq!(
            ref_a,
            "canvas://symphony-gh/node/function/Symphony.Orchestrator.run/1"
        );
        assert_eq!(
            ref_b,
            "canvas://symphony-gh/node/function/Symphony.Engine.run/1"
        );
    }

    // ── Same-named module in different repo must NOT collide ────────────

    #[test]
    fn same_module_name_different_repo_no_collision() {
        let ref_a = node_ref("symphony-gh", "module", "Symphony.Orchestrator");
        let ref_b = node_ref("other-repo", "module", "Symphony.Orchestrator");
        assert_ne!(ref_a, ref_b);
        assert_eq!(
            ref_a,
            "canvas://symphony-gh/node/module/Symphony.Orchestrator"
        );
        assert_eq!(
            ref_b,
            "canvas://other-repo/node/module/Symphony.Orchestrator"
        );
    }

    // ── parse_canonical_ref roundtrip tests ──────────────────────────────

    #[test]
    fn parse_node_module_roundtrip() {
        let ref_str = node_ref("symphony-gh", "module", "Symphony.Orchestrator");
        let components = parse_canonical_ref(&ref_str).unwrap();
        assert_eq!(components.repo_id, "symphony-gh");
        assert_eq!(components.kind, CanvasRefKind::Node);
        assert_eq!(components.sub_type, "module");
        assert_eq!(components.qualified_name, "Symphony.Orchestrator");
    }

    #[test]
    fn parse_function_roundtrip() {
        let ref_str = function_ref("symphony-gh", "Symphony.Orchestrator", "run", 1);
        let components = parse_canonical_ref(&ref_str).unwrap();
        assert_eq!(components.repo_id, "symphony-gh");
        assert_eq!(components.kind, CanvasRefKind::Node);
        assert_eq!(components.sub_type, "function");
        assert_eq!(components.qualified_name, "Symphony.Orchestrator.run/1");
    }

    #[test]
    fn parse_type_roundtrip() {
        let ref_str = node_ref("symphony-gh", "type", "Symphony.Workflow");
        let components = parse_canonical_ref(&ref_str).unwrap();
        assert_eq!(components.repo_id, "symphony-gh");
        assert_eq!(components.kind, CanvasRefKind::Node);
        assert_eq!(components.sub_type, "type");
        assert_eq!(components.qualified_name, "Symphony.Workflow");
    }

    #[test]
    fn parse_config_roundtrip() {
        let ref_str = config_ref("symphony-gh", "config/runtime.exs", "database_url");
        let components = parse_canonical_ref(&ref_str).unwrap();
        assert_eq!(components.repo_id, "symphony-gh");
        assert_eq!(components.kind, CanvasRefKind::Node);
        assert_eq!(components.sub_type, "config");
        assert_eq!(components.qualified_name, "config/runtime.exs#database_url");
    }

    #[test]
    fn parse_edge_roundtrip() {
        let ref_str = edge_ref(
            "symphony-gh",
            "implements",
            "Symphony.Orchestrator",
            "Symphony.Orchestrator.run/1",
        );
        let components = parse_canonical_ref(&ref_str).unwrap();
        assert_eq!(components.repo_id, "symphony-gh");
        assert_eq!(components.kind, CanvasRefKind::Edge);
        assert_eq!(components.sub_type, "implements");
        assert_eq!(
            components.qualified_name,
            "Symphony.Orchestrator->Symphony.Orchestrator.run/1"
        );
    }

    #[test]
    fn parse_edge_call_roundtrip() {
        let ref_str = edge_ref("my-repo", "call", "App.Router", "App.Handler.process/2");
        let components = parse_canonical_ref(&ref_str).unwrap();
        assert_eq!(components.repo_id, "my-repo");
        assert_eq!(components.kind, CanvasRefKind::Edge);
        assert_eq!(components.sub_type, "call");
        assert_eq!(
            components.qualified_name,
            "App.Router->App.Handler.process/2"
        );
    }

    // ── validate_canonical_ref tests ─────────────────────────────────────

    #[test]
    fn validate_valid_node_ref() {
        assert!(
            validate_canonical_ref("canvas://symphony-gh/node/module/Symphony.Orchestrator")
                .is_ok()
        );
        assert!(
            validate_canonical_ref(
                "canvas://symphony-gh/node/function/Symphony.Orchestrator.run/1"
            )
            .is_ok()
        );
        assert!(validate_canonical_ref("canvas://symphony-gh/node/type/Symphony.Workflow").is_ok());
        assert!(
            validate_canonical_ref(
                "canvas://symphony-gh/node/config/config/runtime.exs#database_url"
            )
            .is_ok()
        );
    }

    #[test]
    fn validate_valid_edge_ref() {
        assert!(validate_canonical_ref(
            "canvas://symphony-gh/edge/implements/Symphony.Orchestrator->Symphony.Orchestrator.run/1"
        ).is_ok());
        assert!(
            validate_canonical_ref("canvas://my-repo/edge/call/App.Router->App.Handler.process/2")
                .is_ok()
        );
    }

    #[test]
    fn validate_invalid_missing_scheme() {
        assert!(
            validate_canonical_ref("http://symphony-gh/node/module/Symphony.Orchestrator").is_err()
        );
    }

    #[test]
    fn validate_invalid_empty_repo_id() {
        assert!(validate_canonical_ref("canvas:///node/module/Symphony.Orchestrator").is_err());
    }

    #[test]
    fn validate_invalid_kind() {
        assert!(
            validate_canonical_ref("canvas://symphony-gh/other/module/Symphony.Orchestrator")
                .is_err()
        );
    }

    #[test]
    fn validate_invalid_node_sub_type() {
        assert!(
            validate_canonical_ref("canvas://symphony-gh/node/unknown_type/Symphony.Orchestrator")
                .is_err()
        );
    }

    #[test]
    fn validate_invalid_edge_type() {
        assert!(validate_canonical_ref("canvas://symphony-gh/edge/unknown_type/A->B").is_err());
    }

    #[test]
    fn validate_invalid_edge_missing_arrow() {
        assert!(
            validate_canonical_ref("canvas://symphony-gh/edge/implements/NoArrowHere").is_err()
        );
    }

    #[test]
    fn validate_invalid_empty_qualified_name() {
        assert!(validate_canonical_ref("canvas://symphony-gh/node/module/").is_err());
    }

    #[test]
    fn validate_invalid_garbage_string() {
        assert!(validate_canonical_ref("not-a-ref-at-all").is_err());
    }

    // ── Full roundtrip: generate → parse → regenerate ───────────────────

    #[test]
    fn full_roundtrip_module() {
        let original = node_ref("symphony-gh", "module", "Symphony.Orchestrator");
        let parsed = parse_canonical_ref(&original).unwrap();
        let regenerated = node_ref(&parsed.repo_id, &parsed.sub_type, &parsed.qualified_name);
        assert_eq!(original, regenerated);
    }

    #[test]
    fn full_roundtrip_function() {
        let original = function_ref("symphony-gh", "Symphony.Orchestrator", "run", 1);
        let parsed = parse_canonical_ref(&original).unwrap();
        // For function refs, qualified_name includes module.function/arity
        let regenerated = node_ref(&parsed.repo_id, &parsed.sub_type, &parsed.qualified_name);
        assert_eq!(original, regenerated);
    }

    #[test]
    fn full_roundtrip_edge() {
        let original = edge_ref(
            "symphony-gh",
            "implements",
            "Symphony.Orchestrator",
            "Symphony.Orchestrator.run/1",
        );
        let parsed = parse_canonical_ref(&original).unwrap();
        // qualified_name for edges is "source->target"
        let (source, target) = parsed
            .qualified_name
            .split_once("->")
            .expect("edge qualified_name must contain ->");
        let regenerated = edge_ref(&parsed.repo_id, &parsed.sub_type, source, target);
        assert_eq!(original, regenerated);
    }
}
