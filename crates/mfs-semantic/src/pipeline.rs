use std::path::{Path, PathBuf};
use std::sync::Arc;

use mfs_ast::{SkeletonTextMode, detect_language, extract_skeleton};
use mfs_index::{SemanticDocument, SqliteSemanticIndex};
use tokio::sync::Semaphore;

use crate::config::{SemanticPipelineConfig, summary_concurrency_from_env};
use crate::providers::{
    EmbeddingProvider, NamedSummary, ProcessingMode, SummaryPair, SummaryProvider,
};

pub struct SemanticPipeline {
    config: SemanticPipelineConfig,
}

impl SemanticPipeline {
    pub fn new(config: SemanticPipelineConfig) -> Self {
        Self { config }
    }

    pub fn mode(&self) -> ProcessingMode {
        if self.config.summary_provider.mode() == ProcessingMode::Full
            && self.config.embedding_provider.mode() == ProcessingMode::Full
        {
            ProcessingMode::Full
        } else {
            ProcessingMode::Degraded
        }
    }

    pub async fn process_resource_root(
        &self,
        root: &Path,
        projection_view_id: &str,
        canonical_root_uri: &str,
        resource_id: Option<&str>,
        index: &SqliteSemanticIndex,
    ) -> Result<SemanticProcessingReport, SemanticError> {
        self.process_root(
            root,
            projection_view_id,
            canonical_root_uri,
            "resource",
            resource_id,
            index,
        )
        .await
    }

    pub async fn process_root(
        &self,
        root: &Path,
        projection_view_id: &str,
        canonical_root_uri: &str,
        context_type: &str,
        resource_id: Option<&str>,
        index: &SqliteSemanticIndex,
    ) -> Result<SemanticProcessingReport, SemanticError> {
        let mut indexed_documents = 0;
        self.process_directory(
            root,
            projection_view_id,
            canonical_root_uri,
            context_type,
            resource_id,
            index,
            &mut indexed_documents,
        )
        .await?;
        Ok(SemanticProcessingReport {
            mode: self.mode(),
            indexed_documents,
        })
    }

    async fn process_directory(
        &self,
        dir: &Path,
        projection_view_id: &str,
        dir_uri: &str,
        context_type: &str,
        resource_id: Option<&str>,
        index: &SqliteSemanticIndex,
        indexed_documents: &mut usize,
    ) -> Result<NamedSummary, SemanticError> {
        let mut entries = std::fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());

        let mut files = Vec::new();
        let mut children = Vec::new();
        let mut pending_files = Vec::new();

        for entry in entries {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                let child_uri = format!("{}/{}", dir_uri.trim_end_matches('/'), file_name);
                children.push(
                    Box::pin(self.process_directory(
                        &path,
                        projection_view_id,
                        &child_uri,
                        context_type,
                        resource_id,
                        index,
                        indexed_documents,
                    ))
                    .await?,
                );
                continue;
            }
            if is_summary_sidecar(&path) {
                continue;
            }

            let content = read_text_file(&path)?;

            // For code files, augment content with AST skeleton
            let augmented_content = augment_code_content(&file_name, &content);

            pending_files.push(PendingFile {
                path,
                file_name,
                content: augmented_content,
            });
        }

        for summarized in summarize_files(
            Arc::clone(&self.config.summary_provider),
            pending_files,
            summary_concurrency_from_env(),
        )
        .await
        {
            let summary = summarized.summary;
            let path = summarized.path;
            let file_name = summarized.file_name;
            let (content_kind, language) = classify_semantic_file(&file_name);
            write_file_summary_sidecars(&path, &summary).await?;
            index.upsert_document(&SemanticDocument {
                projection_view_id: projection_view_id.to_owned(),
                uri: format!("{}/{}", dir_uri.trim_end_matches('/'), file_name),
                context_type: context_type.to_owned(),
                resource_id: resource_id.map(str::to_owned),
                content_kind: Some(content_kind),
                language,
                level: 2,
                title: file_name.clone(),
                body: format!("{}\n\n{}", summary.abstract_text, summary.overview_text),
                embedding: embed_text(&*self.config.embedding_provider, &summary.overview_text)
                    .await,
            })?;
            *indexed_documents += 1;
            files.push(NamedSummary {
                name: file_name,
                abstract_text: summary.abstract_text,
                overview_text: summary.overview_text,
            });
        }

        let summary =
            summarize_directory(&*self.config.summary_provider, dir_uri, &files, &children).await;
        tokio::fs::write(dir.join(".abstract.md"), &summary.abstract_text).await?;
        tokio::fs::write(dir.join(".overview.md"), &summary.overview_text).await?;

        index.upsert_document(&SemanticDocument {
            projection_view_id: projection_view_id.to_owned(),
            uri: dir_uri.to_owned(),
            context_type: context_type.to_owned(),
            resource_id: resource_id.map(str::to_owned),
            content_kind: Some("directory".to_owned()),
            language: None,
            level: 0,
            title: ".abstract.md".to_owned(),
            body: summary.abstract_text.clone(),
            embedding: embed_text(&*self.config.embedding_provider, &summary.abstract_text).await,
        })?;
        *indexed_documents += 1;

        index.upsert_document(&SemanticDocument {
            projection_view_id: projection_view_id.to_owned(),
            uri: dir_uri.to_owned(),
            context_type: context_type.to_owned(),
            resource_id: resource_id.map(str::to_owned),
            content_kind: Some("directory".to_owned()),
            language: None,
            level: 1,
            title: ".overview.md".to_owned(),
            body: summary.overview_text.clone(),
            embedding: embed_text(&*self.config.embedding_provider, &summary.overview_text).await,
        })?;
        *indexed_documents += 1;

        Ok(NamedSummary {
            name: dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("root")
                .to_owned(),
            abstract_text: summary.abstract_text,
            overview_text: summary.overview_text,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticProcessingReport {
    pub mode: ProcessingMode,
    pub indexed_documents: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum SemanticError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("index error: {0}")]
    Index(#[from] mfs_index::IndexError),
}

#[derive(Debug)]
struct PendingFile {
    path: PathBuf,
    file_name: String,
    content: String,
}

#[derive(Debug)]
struct SummarizedFile {
    path: PathBuf,
    file_name: String,
    summary: SummaryPair,
}

fn read_text_file(path: &Path) -> Result<String, std::io::Error> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(content),
        Err(source) if source.kind() == std::io::ErrorKind::InvalidData => Ok(String::new()),
        Err(source) => Err(source),
    }
}

async fn write_file_summary_sidecars(
    path: &Path,
    summary: &SummaryPair,
) -> Result<(), std::io::Error> {
    let abstract_path = file_summary_path(path, ".abstract.md");
    let overview_path = file_summary_path(path, ".overview.md");
    tokio::fs::write(abstract_path, &summary.abstract_text).await?;
    tokio::fs::write(overview_path, &summary.overview_text).await?;
    Ok(())
}

fn file_summary_path(path: &Path, suffix: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    path.with_file_name(format!("{file_name}{suffix}"))
}

fn is_summary_sidecar(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(name)
            if name == ".abstract.md"
                || name == ".overview.md"
                || name.ends_with(".abstract.md")
                || name.ends_with(".overview.md")
                || name.ends_with(".skeleton.md")
    )
}

fn classify_semantic_file(file_name: &str) -> (String, Option<String>) {
    let lower = file_name.to_ascii_lowercase();
    let extension = Path::new(file_name)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let language = detect_language(file_name).map(|language| language.name().to_owned());
    let content_kind = if language.is_some() {
        "code"
    } else if lower.starts_with("readme") || matches!(extension.as_str(), "md" | "rst" | "txt") {
        "repo_doc"
    } else if lower == "dockerfile"
        || lower == "makefile"
        || matches!(
            extension.as_str(),
            "json" | "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "lock"
        )
    {
        "config"
    } else {
        "binary"
    };
    (content_kind.to_owned(), language)
}

/// Augment code file content with AST skeleton for better semantic indexing.
/// For non-code files, returns content unchanged.
fn augment_code_content(file_name: &str, content: &str) -> String {
    if content.is_empty() || detect_language(file_name).is_none() {
        return content.to_string();
    }

    match extract_skeleton(file_name, content) {
        Ok(skeleton) => {
            if skeleton.definition_count() == 0 {
                return content.to_string();
            }
            let skeleton_text = skeleton.to_text(SkeletonTextMode::Compact);
            // Prepend skeleton text so LLM summarizer sees structure first
            format!(
                "## Code Structure\n{}\n\n## Full Content\n{}",
                skeleton_text, content
            )
        }
        Err(_) => content.to_string(), // Graceful degradation
    }
}

async fn summarize_files(
    provider: Arc<dyn SummaryProvider>,
    files: Vec<PendingFile>,
    concurrency: usize,
) -> Vec<SummarizedFile> {
    let semaphore = Arc::new(Semaphore::new(concurrency.max(1)));
    let mut results: Vec<Option<SummarizedFile>> = (0..files.len()).map(|_| None).collect();
    let mut join_set = tokio::task::JoinSet::new();

    for (i, file) in files.into_iter().enumerate() {
        let provider = Arc::clone(&provider);
        let permit = Arc::clone(&semaphore);
        join_set.spawn(async move {
            let _permit = permit.acquire().await.expect("semaphore closed");
            let summary = provider.summarize_file(&file.path, &file.content).await;
            (
                i,
                SummarizedFile {
                    path: file.path,
                    file_name: file.file_name,
                    summary,
                },
            )
        });
    }

    while let Some(res) = join_set.join_next().await {
        if let Ok((i, sf)) = res {
            results[i] = Some(sf);
        }
    }

    results.into_iter().flatten().collect()
}

async fn summarize_directory(
    provider: &dyn SummaryProvider,
    uri: &str,
    files: &[NamedSummary],
    children: &[NamedSummary],
) -> SummaryPair {
    provider.summarize_directory(uri, files, children).await
}

async fn embed_text(provider: &dyn EmbeddingProvider, text: &str) -> Vec<f32> {
    provider.embed_text(text).await
}
