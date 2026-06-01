//! MFS AST extraction module.
//!
//! Provides code-aware indexing for AI agent context engines. Extracts
//! structured skeletons (function signatures, class definitions, imports)
//! from source code files for more precise semantic search and summarization.
//!
//! Two extraction strategies:
//! - **Deterministic**: regex-based extraction, always available, no external deps
//! - **TreeSitter**: precise AST-based extraction (when tree-sitter crates available)
//!
//! The module gracefully degrades: if tree-sitter is not compiled in or fails
//! for a specific file, it falls back to deterministic extraction.

mod deterministic;
mod language;
mod skeleton;

pub use language::{LANGUAGE_EXTENSIONS, Language, detect_language, is_code_file};
pub use skeleton::{
    ClassSkeleton, CodeSkeleton, FunctionSig, ImportDecl, SkeletonError, SkeletonTextMode,
};

/// Main extraction entry point. Detects language, then extracts skeleton.
/// Falls back to deterministic extraction on any failure.
pub fn extract_skeleton(path: &str, content: &str) -> Result<CodeSkeleton, SkeletonError> {
    let lang = detect_language(path);
    match lang {
        Some(language) => {
            // Try tree-sitter first (if compiled in), then fallback to deterministic
            let result = try_treesitter_extract(content, language);
            match result {
                Ok(skeleton) => Ok(skeleton),
                Err(_) => deterministic::extract(content, language, path),
            }
        }
        None => Err(SkeletonError::UnsupportedLanguage {
            path: path.to_string(),
        }),
    }
}

/// Try tree-sitter extraction. Returns None if tree-sitter is not compiled in.
fn try_treesitter_extract(
    _content: &str,
    language: Language,
) -> Result<CodeSkeleton, SkeletonError> {
    // tree-sitter is not yet compiled into the binary.
    // This function always returns an error, triggering deterministic fallback.
    // When tree-sitter crates are added as optional dependencies, this
    // function will be conditionally compiled.
    Err(SkeletonError::TreeSitterNotAvailable {
        language: language.name().to_string(),
    })
}
