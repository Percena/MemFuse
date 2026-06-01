//! Language detection based on file extensions and heuristics.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Supported programming languages for AST extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    Python,
    JavaScript,
    TypeScript,
    Rust,
    Go,
    Java,
    C,
    Cpp,
    CSharp,
    Php,
    Lua,
    Ruby,
    Swift,
    Kotlin,
}

impl Language {
    pub fn name(&self) -> &'static str {
        match self {
            Language::Python => "python",
            Language::JavaScript => "javascript",
            Language::TypeScript => "typescript",
            Language::Rust => "rust",
            Language::Go => "go",
            Language::Java => "java",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::CSharp => "csharp",
            Language::Php => "php",
            Language::Lua => "lua",
            Language::Ruby => "ruby",
            Language::Swift => "swift",
            Language::Kotlin => "kotlin",
        }
    }

    /// Whether this language typically uses braces for block delimiters.
    pub fn uses_braces(&self) -> bool {
        !matches!(self, Language::Python | Language::Lua | Language::Ruby)
    }

    /// Whether this language uses semicolons as statement terminators.
    pub fn uses_semicolons(&self) -> bool {
        !matches!(self, Language::Python | Language::Lua | Language::Ruby)
    }
}

/// Mapping of file extensions to languages.
pub static LANGUAGE_EXTENSIONS: &[(&str, Language)] = &[
    ("py", Language::Python),
    ("pyw", Language::Python),
    ("js", Language::JavaScript),
    ("mjs", Language::JavaScript),
    ("cjs", Language::JavaScript),
    ("jsx", Language::JavaScript),
    ("ts", Language::TypeScript),
    ("tsx", Language::TypeScript),
    ("mts", Language::TypeScript),
    ("cts", Language::TypeScript),
    ("rs", Language::Rust),
    ("go", Language::Go),
    ("java", Language::Java),
    ("c", Language::C),
    ("h", Language::C),
    ("cpp", Language::Cpp),
    ("cc", Language::Cpp),
    ("cxx", Language::Cpp),
    ("hpp", Language::Cpp),
    ("hh", Language::Cpp),
    ("cs", Language::CSharp),
    ("php", Language::Php),
    ("lua", Language::Lua),
    ("rb", Language::Ruby),
    ("swift", Language::Swift),
    ("kt", Language::Kotlin),
    ("kts", Language::Kotlin),
];

/// Detect programming language from file path extension.
/// Returns None for non-code files (markdown, json, yaml, etc.).
pub fn detect_language(path: &str) -> Option<Language> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())?;

    LANGUAGE_EXTENSIONS
        .iter()
        .find(|(ext_match, _)| *ext_match == ext)
        .map(|(_, lang)| *lang)
}

/// Check if a file path looks like a source code file.
pub fn is_code_file(path: &str) -> bool {
    detect_language(path).is_some()
}
