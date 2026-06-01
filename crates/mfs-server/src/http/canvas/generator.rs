//! Elixir canvas generation logic.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub(super) struct GeneratedCanvas {
    pub nodes: Vec<GeneratedNode>,
    pub edges: Vec<GeneratedEdge>,
    pub version_hash: String,
}

pub(super) struct GeneratedNode {
    pub id: String,
    pub node_type: String,
    pub name: String,
    pub path: Option<String>,
    pub purpose: String,
    pub source: Option<String>,
}

pub(super) struct GeneratedEdge {
    pub id: String,
    pub edge_type: String,
    pub source_node_id: String,
    pub target_node_id: String,
}

pub(super) fn generate_elixir_canvas(
    source_root: &Path,
    repo_id: &str,
    _generator: &str,
) -> std::io::Result<GeneratedCanvas> {
    let mut files = Vec::new();
    collect_elixir_files(source_root, &mut files)?;
    files.sort();

    let mut hasher = Sha256::new();
    hasher.update(repo_id.as_bytes());
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    for file in files {
        let content = std::fs::read_to_string(&file)?;
        let rel_path = file
            .strip_prefix(source_root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        hasher.update(rel_path.as_bytes());
        hasher.update(content.as_bytes());

        let mut current_module_id: Option<String> = None;
        let mut current_module_name: Option<String> = None;
        for (index, raw_line) in content.lines().enumerate() {
            let line = raw_line.trim_start();
            if let Some(module_name) = parse_elixir_module(line) {
                let module_id = format!("{repo_id}:module:{module_name}");
                current_module_id = Some(module_id.clone());
                current_module_name = Some(module_name.clone());
                nodes.push(GeneratedNode {
                    id: module_id,
                    node_type: "module".into(),
                    name: module_name,
                    path: Some(rel_path.clone()),
                    purpose: "Elixir module skeleton".into(),
                    source: Some(format!("{}:{}", rel_path, index + 1)),
                });
                continue;
            }

            let Some(function_name) = parse_elixir_function(line) else {
                continue;
            };
            let Some(module_id) = current_module_id.clone() else {
                continue;
            };
            let module_name = current_module_name
                .clone()
                .unwrap_or_else(|| "script".into());
            let arity = parse_elixir_arity(line);
            let function_id = format!("{repo_id}:function:{module_name}.{function_name}/{arity}");
            nodes.push(GeneratedNode {
                id: function_id.clone(),
                node_type: "function".into(),
                name: function_name,
                path: Some(rel_path.clone()),
                purpose: format!("Elixir function skeleton in {module_name}"),
                source: Some(format!("{}:{}", rel_path, index + 1)),
            });
            edges.push(GeneratedEdge {
                id: format!("{}:implements:{}", module_id, function_id),
                edge_type: "implements".into(),
                source_node_id: module_id,
                target_node_id: function_id,
            });
        }
    }

    let digest = hasher.finalize();
    Ok(GeneratedCanvas {
        nodes,
        edges,
        version_hash: format!("v1-content:{}", hex::encode(&digest[..8])),
    })
}

fn collect_elixir_files(root: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_elixir_files(&path, files)?;
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension == "ex" || extension == "exs")
        {
            files.push(path);
        }
    }
    Ok(())
}

fn parse_elixir_module(line: &str) -> Option<String> {
    let rest = line.strip_prefix("defmodule ")?;
    let module = rest
        .split_whitespace()
        .next()?
        .trim_end_matches("do")
        .trim();
    if module.is_empty() {
        None
    } else {
        Some(module.to_owned())
    }
}

fn parse_elixir_function(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("defp ")
        .or_else(|| line.strip_prefix("def "))?
        .trim_start();
    let name: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '!' | '?'))
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

fn parse_elixir_arity(line: &str) -> usize {
    let Some(open) = line.find('(') else {
        return 0;
    };
    let Some(close) = line[open + 1..].find(')') else {
        return 0;
    };
    let args = &line[open + 1..open + 1 + close];
    if args.trim().is_empty() {
        0
    } else {
        args.split(',').filter(|arg| !arg.trim().is_empty()).count()
    }
}
