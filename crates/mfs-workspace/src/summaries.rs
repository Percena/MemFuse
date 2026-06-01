use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const ABSTRACT_FILE_NAME: &str = ".abstract.md";
const OVERVIEW_FILE_NAME: &str = ".overview.md";
const EXCERPT_TOKEN_LIMIT: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdaptiveBudget {
    pub total_tokens: usize,
    pub kind: String,
    pub l0_target: usize,
    pub l1_target: usize,
}

impl AdaptiveBudget {
    pub fn for_tokens(total_tokens: usize, kind: &str) -> Self {
        let kind = normalize_kind(kind);
        let l0_base = match kind.as_str() {
            "pdf" => 120,
            "markdown" => 96,
            _ => 88,
        };
        let l1_cap = match kind.as_str() {
            "pdf" => 2_200,
            "markdown" => 1_400,
            _ => 1_800,
        };
        let l0_target = (l0_base + (total_tokens / 2_000) * 8).clamp(72, 150);
        let l1_target = (240 + (total_tokens / 12)).clamp(240, l1_cap);

        Self {
            total_tokens,
            kind,
            l0_target,
            l1_target,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayeredSummaries {
    pub abstract_markdown: String,
    pub overview_markdown: String,
    pub budget: AdaptiveBudget,
}

#[derive(Debug)]
pub enum SummaryError {
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl SummaryError {
    fn io(action: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.into(),
            source,
        }
    }
}

impl Display for SummaryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                action,
                path,
                source,
            } => write!(f, "failed to {action} '{}': {source}", path.display()),
        }
    }
}

impl Error for SummaryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceExcerpt {
    relative_path: PathBuf,
    title: String,
    excerpt: String,
    estimated_tokens: usize,
    kind: String,
}

pub fn write_layered_summaries(
    root: &Path,
    canonical_uri: &str,
) -> Result<LayeredSummaries, SummaryError> {
    let summaries = generate_layered_summaries(root, canonical_uri)?;
    let abstract_path = root.join(ABSTRACT_FILE_NAME);
    let overview_path = root.join(OVERVIEW_FILE_NAME);

    fs::write(&abstract_path, &summaries.abstract_markdown)
        .map_err(|source| SummaryError::io("write abstract summary", &abstract_path, source))?;
    fs::write(&overview_path, &summaries.overview_markdown)
        .map_err(|source| SummaryError::io("write overview summary", &overview_path, source))?;

    for file_path in direct_files(root)? {
        let file_name = file_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        let file_uri = format!("{}/{}", canonical_uri.trim_end_matches('/'), file_name);
        write_file_layered_summaries(&file_path, &file_uri)?;
    }

    Ok(summaries)
}

fn write_file_layered_summaries(
    path: &Path,
    canonical_uri: &str,
) -> Result<LayeredSummaries, SummaryError> {
    let summaries = generate_layered_summaries(path, canonical_uri)?;
    let abstract_path = file_summary_path(path, ABSTRACT_FILE_NAME);
    let overview_path = file_summary_path(path, OVERVIEW_FILE_NAME);

    fs::write(&abstract_path, &summaries.abstract_markdown).map_err(|source| {
        SummaryError::io("write file abstract summary", &abstract_path, source)
    })?;
    fs::write(&overview_path, &summaries.overview_markdown).map_err(|source| {
        SummaryError::io("write file overview summary", &overview_path, source)
    })?;

    Ok(summaries)
}

fn generate_layered_summaries(
    root: &Path,
    canonical_uri: &str,
) -> Result<LayeredSummaries, SummaryError> {
    let files = collect_source_excerpts(root)?;
    let kind = infer_resource_kind(&files);
    let total_tokens = files
        .iter()
        .map(|file| file.estimated_tokens)
        .sum::<usize>()
        .max(1);
    let budget = AdaptiveBudget::for_tokens(total_tokens, &kind);

    Ok(LayeredSummaries {
        abstract_markdown: build_abstract(canonical_uri, &files, budget.l0_target),
        overview_markdown: build_overview(canonical_uri, &files, &budget),
        budget,
    })
}

fn collect_source_excerpts(root: &Path) -> Result<Vec<SourceExcerpt>, SummaryError> {
    if root.is_file() {
        if summary_file_name(root) {
            return Ok(Vec::new());
        }

        return Ok(vec![build_source_excerpt(
            root.parent().unwrap_or_else(|| Path::new("")),
            root,
        )?]);
    }

    let mut files = Vec::new();
    collect_source_excerpts_recursive(root, root, &mut files)?;
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}

fn collect_source_excerpts_recursive(
    root: &Path,
    current: &Path,
    files: &mut Vec<SourceExcerpt>,
) -> Result<(), SummaryError> {
    for entry in fs::read_dir(current)
        .map_err(|source| SummaryError::io("read materialized directory", current, source))?
    {
        let entry = entry.map_err(|source| {
            SummaryError::io("read materialized directory entry", current, source)
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| SummaryError::io("inspect materialized entry", &path, source))?;

        if file_type.is_dir() {
            collect_source_excerpts_recursive(root, &path, files)?;
            continue;
        }

        if summary_file_name(&path) {
            continue;
        }

        files.push(build_source_excerpt(root, &path)?);
    }

    Ok(())
}

fn build_source_excerpt(root: &Path, path: &Path) -> Result<SourceExcerpt, SummaryError> {
    let relative_path = path.strip_prefix(root).unwrap_or(path).to_path_buf();
    let normalized = read_text_excerpt(path)?;
    let kind = kind_for_path(path);
    let title = infer_title(path, &normalized);
    let excerpt = if normalized.is_empty() {
        format!(
            "{} contains binary or non-text content.",
            path_fragment(&relative_path)
        )
    } else {
        trim_to_tokens(&normalized, EXCERPT_TOKEN_LIMIT)
    };
    let estimated_tokens = estimate_tokens(&normalized).max(1);

    Ok(SourceExcerpt {
        relative_path,
        title,
        excerpt,
        estimated_tokens,
        kind,
    })
}

fn direct_files(root: &Path) -> Result<Vec<PathBuf>, SummaryError> {
    let mut files = Vec::new();

    for entry in fs::read_dir(root)
        .map_err(|source| SummaryError::io("read materialized directory", root, source))?
    {
        let entry = entry.map_err(|source| {
            SummaryError::io("read materialized directory entry", root, source)
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| SummaryError::io("inspect materialized entry", &path, source))?;
        if file_type.is_file() && !summary_file_name(&path) {
            files.push(path);
        }
    }

    files.sort();
    Ok(files)
}

fn file_summary_path(path: &Path, suffix: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    path.with_file_name(format!("{file_name}{suffix}"))
}

fn read_text_excerpt(path: &Path) -> Result<String, SummaryError> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(normalize_whitespace(&content)),
        Err(source) if source.kind() == io::ErrorKind::InvalidData => Ok(String::new()),
        Err(source) => Err(SummaryError::io("read materialized file", path, source)),
    }
}

fn build_abstract(canonical_uri: &str, files: &[SourceExcerpt], target_tokens: usize) -> String {
    if files.is_empty() {
        return trim_to_tokens(
            &format!(
                "Deterministic summary for {canonical_uri}: no readable files were materialized."
            ),
            target_tokens,
        );
    }

    let file_names = files
        .iter()
        .map(|file| path_fragment(&file.relative_path))
        .collect::<Vec<_>>()
        .join(", ");
    let highlights = files
        .iter()
        .take(2)
        .map(|file| format!("{} from {}", file.title, path_fragment(&file.relative_path)))
        .collect::<Vec<_>>()
        .join("; ");
    let summary = format!(
        "Deterministic summary for {canonical_uri}: {} materialized files ({file_names}). Highlights: {highlights}.",
        files.len()
    );

    trim_to_tokens(&summary, target_tokens)
}

fn build_overview(canonical_uri: &str, files: &[SourceExcerpt], budget: &AdaptiveBudget) -> String {
    let mut lines = vec![
        "# Overview".to_owned(),
        String::new(),
        format!("Resource: `{canonical_uri}`"),
        format!("Detected kind: `{}`", budget.kind),
        format!("Estimated source tokens: {}", budget.total_tokens),
        format!(
            "Adaptive budget: L0 <= {} tokens, L1 <= {} tokens.",
            budget.l0_target, budget.l1_target
        ),
        String::new(),
        "## Files".to_owned(),
    ];

    if files.is_empty() {
        lines.push("- No readable materialized files were found.".to_owned());
    } else {
        for file in files {
            lines.push(format!(
                "- `{}` ({}, ~{} tokens): {}",
                path_fragment(&file.relative_path),
                file.kind,
                file.estimated_tokens,
                file.excerpt
            ));
        }
    }

    lines.push(String::new());
    lines.push("## Access".to_owned());

    if let Some(first_file) = files.first() {
        let first_uri = format!(
            "{}/{}",
            canonical_uri.trim_end_matches('/'),
            path_fragment(&first_file.relative_path)
        );
        lines.push(format!(
            "Read the original materialized files under `{canonical_uri}` for full detail. Start with `{first_uri}`."
        ));
    } else {
        lines.push(format!(
            "Read the materialized resource root `{canonical_uri}` for any newly added files."
        ));
    }

    trim_to_tokens(&lines.join("\n"), budget.l1_target)
}

fn infer_resource_kind(files: &[SourceExcerpt]) -> String {
    if files.iter().any(|file| file.kind == "pdf") {
        return "pdf".to_owned();
    }

    if !files.is_empty() && files.iter().all(|file| file.kind == "markdown") {
        return "markdown".to_owned();
    }

    "text".to_owned()
}

fn infer_title(path: &Path, normalized_content: &str) -> String {
    if let Some(title_line) = normalized_content
        .split('.')
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        return title_line.trim_start_matches('#').trim().to_owned();
    }

    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("untitled")
        .replace(['_', '-'], " ")
}

fn kind_for_path(path: &Path) -> String {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    normalize_kind(extension)
}

fn normalize_kind(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "md" | "markdown" => "markdown".to_owned(),
        "pdf" => "pdf".to_owned(),
        "html" | "htm" => "html".to_owned(),
        "txt" | "text" | "" => "text".to_owned(),
        _ => "text".to_owned(),
    }
}

fn normalize_whitespace(content: &str) -> String {
    content.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn estimate_tokens(content: &str) -> usize {
    content.split_whitespace().count()
}

fn trim_to_tokens(content: &str, limit: usize) -> String {
    let tokens = content.split_whitespace().collect::<Vec<_>>();

    if tokens.len() <= limit {
        return content.to_owned();
    }

    let mut truncated = tokens.into_iter().take(limit).collect::<Vec<_>>().join(" ");
    truncated.push_str(" ...");
    truncated
}

fn path_fragment(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn summary_file_name(path: &Path) -> bool {
    matches!(path.file_name().and_then(|name| name.to_str()), Some(name) if
        name == ABSTRACT_FILE_NAME
            || name == OVERVIEW_FILE_NAME
            || name.ends_with(ABSTRACT_FILE_NAME)
            || name.ends_with(OVERVIEW_FILE_NAME))
}
