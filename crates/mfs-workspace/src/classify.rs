use std::fs;
use std::path::{Path, PathBuf};

use mfs_ast::detect_language;
use mfs_uri::short_hash_hex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedPath {
    pub content_kind: String,
    pub language: Option<String>,
    pub is_text: bool,
    pub is_generated: bool,
}

pub fn should_skip_path(relative_path: &Path, is_dir: bool) -> bool {
    let lower_segments = relative_path
        .iter()
        .map(|segment| segment.to_string_lossy().to_ascii_lowercase())
        .collect::<Vec<_>>();
    if lower_segments.is_empty() {
        return false;
    }

    let generated_dirs = [
        ".git",
        "node_modules",
        "__pycache__",
        ".next",
        "target",
        "dist",
        "build",
        ".idea",
        ".vscode",
        "_system",
        "tenants",
    ];
    if lower_segments
        .iter()
        .any(|segment| generated_dirs.contains(&segment.as_str()))
    {
        return true;
    }

    if !is_dir {
        let file_name = lower_segments.last().cloned().unwrap_or_default();
        if file_name.ends_with(".pyc")
            || file_name.ends_with(".pyo")
            || file_name.ends_with(".so")
            || file_name.ends_with(".dll")
            || file_name.ends_with(".dylib")
            || file_name.ends_with(".exe")
            || file_name.ends_with(".bin")
            || file_name.ends_with(".class")
        {
            return true;
        }
    }

    false
}

pub fn classify_path(relative_path: &Path, is_dir: bool) -> ClassifiedPath {
    if is_dir {
        return ClassifiedPath {
            content_kind: "directory".to_owned(),
            language: None,
            is_text: false,
            is_generated: should_skip_path(relative_path, true),
        };
    }

    let relative = normalize_relative(relative_path);
    let file_name = relative_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let file_name_lower = file_name.to_ascii_lowercase();
    let language = detect_language(file_name).map(|language| language.name().to_owned());
    let extension = relative_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let is_generated = should_skip_path(relative_path, false);
    let content_kind = if language.is_some() {
        "code"
    } else if file_name_lower == "dockerfile"
        || file_name_lower == "makefile"
        || matches!(
            extension.as_str(),
            "json" | "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "lock"
        )
    {
        "config"
    } else if file_name_lower.starts_with("readme")
        || relative.starts_with("docs/")
        || matches!(extension.as_str(), "md" | "rst" | "txt" | "adoc")
    {
        "repo_doc"
    } else if matches!(
        extension.as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "svg"
            | "webp"
            | "mp4"
            | "mov"
            | "avi"
            | "mp3"
            | "wav"
            | "zip"
            | "gz"
            | "tar"
            | "pdf"
    ) {
        "asset"
    } else if is_generated {
        "generated"
    } else {
        "binary"
    };

    let is_text = !matches!(content_kind, "asset" | "binary");

    ClassifiedPath {
        content_kind: content_kind.to_owned(),
        language,
        is_text,
        is_generated,
    }
}

pub fn infer_resource_kind_from_path(root: &Path) -> std::io::Result<String> {
    let mut code_count = 0usize;
    let mut doc_count = 0usize;
    let mut stack = vec![root.to_path_buf()];

    while let Some(path) = stack.pop() {
        let metadata = fs::metadata(&path)?;
        if metadata.is_dir() {
            for entry in fs::read_dir(&path)? {
                let entry = entry?;
                let child = entry.path();
                let relative = child.strip_prefix(root).unwrap_or(&child);
                if should_skip_path(relative, entry.file_type()?.is_dir()) {
                    continue;
                }
                stack.push(child);
            }
            continue;
        }

        let relative = path.strip_prefix(root).unwrap_or(&path);
        let classified = classify_path(relative, false);
        match classified.content_kind.as_str() {
            "code" => code_count += 1,
            "repo_doc" => doc_count += 1,
            _ => {}
        }
    }

    Ok(if code_count > 0 && doc_count > 0 {
        "mixed_repo".to_owned()
    } else if code_count > 0 {
        "code_repo".to_owned()
    } else {
        "generic_docs".to_owned()
    })
}

pub fn content_digest_for_path(path: &Path) -> std::io::Result<String> {
    let bytes = fs::read(path)?;
    Ok(content_digest_for_bytes(&bytes))
}

pub fn content_digest_for_bytes(bytes: &[u8]) -> String {
    format!("fnv64-{}", short_hash_hex(bytes, 16))
}

pub fn directory_metadata_digest(path: &Path) -> std::io::Result<String> {
    let mut entries = fs::read_dir(path)?
        .filter_map(|entry| entry.ok())
        .map(|entry| {
            let file_type = entry.file_type().ok();
            let name = entry.file_name().to_string_lossy().into_owned();
            let marker = match file_type {
                Some(kind) if kind.is_dir() => "d",
                Some(kind) if kind.is_file() => "f",
                _ => "?",
            };
            format!("{marker}:{name}")
        })
        .collect::<Vec<_>>();
    entries.sort();
    Ok(content_digest_for_bytes(entries.join("\n").as_bytes()))
}

pub fn is_summary_sidecar(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(name)
            if name == ".abstract.md"
                || name == ".overview.md"
                || name.ends_with(".abstract.md")
                || name.ends_with(".overview.md")
    )
}

fn normalize_relative(relative_path: &Path) -> String {
    relative_path.to_string_lossy().replace('\\', "/")
}

#[allow(dead_code)]
fn _path_buf(relative: &str) -> PathBuf {
    PathBuf::from(relative)
}
