//! Code skeleton data model.
//!
//! A CodeSkeleton captures the structural outline of a source file —
//! imports, class definitions, and function signatures — without
//! the full implementation body. This is the core abstraction used
//! for code-aware indexing and summarization.

use serde::{Deserialize, Serialize};

use crate::language::Language;

/// Error type for skeleton extraction.
#[derive(Debug, thiserror::Error)]
pub enum SkeletonError {
    #[error("unsupported language for path: {path}")]
    UnsupportedLanguage { path: String },
    #[error("tree-sitter not available for language: {language}")]
    TreeSitterNotAvailable { language: String },
    #[error("extraction failed: {message}")]
    ExtractionFailed { message: String },
}

/// A function or method signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionSig {
    pub name: String,
    pub params: Vec<String>,
    pub return_type: Option<String>,
    pub docstring: Option<String>,
}

/// A class, struct, or interface definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassSkeleton {
    pub name: String,
    pub bases: Vec<String>,
    pub docstring: Option<String>,
    pub methods: Vec<FunctionSig>,
}

/// An import or include declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportDecl {
    pub module: String,
    pub items: Vec<String>,
}

/// The complete structural outline of a source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeSkeleton {
    pub file_name: String,
    pub language: Language,
    pub module_doc: Option<String>,
    pub imports: Vec<ImportDecl>,
    pub classes: Vec<ClassSkeleton>,
    pub functions: Vec<FunctionSig>,
}

/// Mode for generating skeleton text output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkeletonTextMode {
    /// Compact mode for embedding: only docstring first lines
    Compact,
    /// Verbose mode for LLM context: full docstrings
    Verbose,
}

impl CodeSkeleton {
    /// Generate text representation of the skeleton for indexing or LLM input.
    pub fn to_text(&self, mode: SkeletonTextMode) -> String {
        let mut parts = Vec::new();

        // File header
        parts.push(format!("```{}", self.language.name()));
        parts.push(format!("// File: {}", self.file_name));

        // Module docstring
        if let Some(doc) = &self.module_doc {
            if mode == SkeletonTextMode::Verbose {
                parts.push(doc.clone());
            } else {
                // Compact: just first line
                let first_line = doc.lines().next().unwrap_or(doc);
                parts.push(first_line.to_string());
            }
        }

        // Imports
        for imp in &self.imports {
            if imp.items.is_empty() {
                parts.push(format!("import {}", imp.module));
            } else {
                parts.push(format!("import {} [{}]", imp.module, imp.items.join(", ")));
            }
        }

        // Classes
        for cls in &self.classes {
            let bases = if cls.bases.is_empty() {
                String::new()
            } else {
                format!(" extends {}", cls.bases.join(", "))
            };
            parts.push(format!("class {}{} {{", cls.name, bases));

            if let Some(doc) = &cls.docstring {
                match mode {
                    SkeletonTextMode::Verbose => parts.push(format!("  /** {} */", doc)),
                    SkeletonTextMode::Compact => {
                        let first = doc.lines().next().unwrap_or(doc);
                        parts.push(format!("  // {}", first));
                    }
                }
            }

            for method in &cls.methods {
                parts.push(format!("  {}({})", method.name, method.params.join(", ")));
                if let Some(rt) = &method.return_type {
                    parts.push(format!("    -> {}", rt));
                }
                if let Some(doc) = &method.docstring {
                    match mode {
                        SkeletonTextMode::Verbose => {
                            parts.push(format!("    /** {} */", doc));
                        }
                        SkeletonTextMode::Compact => {
                            let first = doc.lines().next().unwrap_or(doc);
                            parts.push(format!("    // {}", first));
                        }
                    }
                }
            }
            parts.push("}".to_string());
        }

        // Free functions
        for func in &self.functions {
            let rt = func
                .return_type
                .as_ref()
                .map(|r| format!(" -> {}", r))
                .unwrap_or_default();
            parts.push(format!(
                "fn {}({}){}",
                func.name,
                func.params.join(", "),
                rt
            ));

            if let Some(doc) = &func.docstring {
                match mode {
                    SkeletonTextMode::Verbose => {
                        parts.push(format!("  /** {} */", doc));
                    }
                    SkeletonTextMode::Compact => {
                        let first = doc.lines().next().unwrap_or(doc);
                        parts.push(format!("  // {}", first));
                    }
                }
            }
        }

        parts.push("```".to_string());
        parts.join("\n")
    }

    /// Count of top-level definitions (classes + functions).
    pub fn definition_count(&self) -> usize {
        self.classes.len() + self.functions.len()
    }

    /// All symbol names (class names + method names + function names).
    pub fn all_symbol_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.classes.iter().map(|c| c.name.clone()).collect();
        for cls in &self.classes {
            for m in &cls.methods {
                names.push(format!("{}.{}", cls.name, m.name));
            }
        }
        for f in &self.functions {
            names.push(f.name.clone());
        }
        names
    }
}
