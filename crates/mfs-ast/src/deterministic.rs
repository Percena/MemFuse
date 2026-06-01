//! Deterministic (regex-based) skeleton extraction.
//!
//! This module provides a reliable fallback when tree-sitter is not available.
//! It uses line-pattern heuristics to extract function signatures, class
//! declarations, and imports from common programming languages.

use crate::language::Language;
use crate::skeleton::{ClassSkeleton, CodeSkeleton, FunctionSig, ImportDecl, SkeletonError};

/// Extract a skeleton using deterministic line-pattern heuristics.
#[allow(clippy::unnecessary_wraps)]
pub fn extract(
    content: &str,
    language: Language,
    file_name: &str,
) -> Result<CodeSkeleton, SkeletonError> {
    let module_doc = extract_module_doc(content, language);
    let imports = extract_imports(content, language);
    let classes = extract_classes(content, language);
    let functions = extract_functions(content, language);

    Ok(CodeSkeleton {
        file_name: file_name.to_string(),
        language,
        module_doc,
        imports,
        classes,
        functions,
    })
}

fn extract_module_doc(content: &str, language: Language) -> Option<String> {
    let lines = content.lines().take(10);
    match language {
        Language::Python => {
            // Python: triple-quoted docstrings at file top
            let mut in_doc = false;
            let mut doc_lines = Vec::new();
            for line in lines {
                if line.starts_with("\"\"\"") || line.starts_with("'''") {
                    if in_doc {
                        break;
                    }
                    in_doc = true;
                    let after = &line[3..];
                    if after.ends_with("\"\"\"") || after.ends_with("'''") {
                        let inner = &after[..after.len() - 3];
                        if !inner.is_empty() {
                            return Some(inner.to_string());
                        }
                        break;
                    }
                    if !after.is_empty() {
                        doc_lines.push(after.to_string());
                    }
                } else if in_doc {
                    doc_lines.push(line.to_string());
                }
            }
            if doc_lines.is_empty() {
                None
            } else {
                Some(doc_lines.join("\n"))
            }
        }
        Language::Rust => {
            // Rust: //! doc comments at file top
            let doc_lines: Vec<String> = lines
                .filter(|line| line.starts_with("//!"))
                .map(|line| line.trim_start_matches("//!").trim().to_string())
                .collect();
            if doc_lines.is_empty() {
                None
            } else {
                Some(doc_lines.join("\n"))
            }
        }
        Language::Go => {
            // Go: // comments at file top (before package)
            let doc_lines: Vec<String> = lines
                .filter(|line| line.starts_with("//") && !line.starts_with("//go:"))
                .map(|line| line.trim_start_matches("//").trim().to_string())
                .take_while(|line| !line.is_empty())
                .collect();
            if doc_lines.is_empty() {
                None
            } else {
                Some(doc_lines.join("\n"))
            }
        }
        _ => {
            // JS/TS/Java/C/C++: /** ... */ block at file top
            let mut in_doc = false;
            let mut doc_lines = Vec::new();
            for line in lines {
                if line.contains("/**") {
                    in_doc = true;
                    let cleaned = line
                        .trim_start_matches("/**")
                        .trim_start_matches('*')
                        .trim();
                    if let Some(stripped) = cleaned.strip_suffix("*/") {
                        let inner = stripped.trim();
                        if !inner.is_empty() {
                            doc_lines.push(inner.to_string());
                        }
                        break;
                    }
                    if !cleaned.is_empty() {
                        doc_lines.push(cleaned.to_string());
                    }
                } else if in_doc {
                    if line.contains("*/") {
                        let before = line[..line.find("*/").unwrap_or(line.len())]
                            .trim_start_matches('*')
                            .trim();
                        if !before.is_empty() {
                            doc_lines.push(before.to_string());
                        }
                        break;
                    }
                    let cleaned = line.trim_start_matches('*').trim();
                    if !cleaned.is_empty() {
                        doc_lines.push(cleaned.to_string());
                    }
                }
            }
            if doc_lines.is_empty() {
                None
            } else {
                Some(doc_lines.join("\n"))
            }
        }
    }
}

fn extract_imports(content: &str, language: Language) -> Vec<ImportDecl> {
    let mut imports = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        match language {
            Language::Python => {
                // import X / from X import Y, Z
                if let Some(rest) = trimmed.strip_prefix("import ") {
                    let module = rest
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_string();
                    imports.push(ImportDecl {
                        module,
                        items: vec![],
                    });
                } else if let Some(rest) = trimmed.strip_prefix("from ") {
                    if let Some(pos) = rest.find(" import ") {
                        let module = rest[..pos].to_string();
                        let items_str = rest[pos + 8..].trim();
                        let items: Vec<String> = items_str
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        imports.push(ImportDecl { module, items });
                    }
                }
            }
            Language::Rust
                // use X::Y; / use X::{Y, Z};
                if trimmed.starts_with("use ") && trimmed.ends_with(';') => {
                    let path = &trimmed[4..trimmed.len() - 1];
                    if let Some(pos) = path.find("::{") {
                        let module = path[..pos].to_string();
                        let items_str = &path[pos + 3..path.len() - 1];
                        let items: Vec<String> = items_str
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        imports.push(ImportDecl { module, items });
                    } else {
                        imports.push(ImportDecl {
                            module: path.to_string(),
                            items: vec![],
                        });
                    }
                }
            Language::Go
                // import "X" / import ( "Y" ... )
                if trimmed.starts_with("import ")
                    && trimmed.contains('"') => {
                        let start = trimmed.find('"').unwrap_or(0) + 1;
                        let end = trimmed.rfind('"').unwrap_or(trimmed.len());
                        imports.push(ImportDecl {
                            module: trimmed[start..end].to_string(),
                            items: vec![],
                        });
                    }
            Language::JavaScript | Language::TypeScript => {
                // import X from 'Y' / import { A, B } from 'Y'
                if trimmed.starts_with("import ") {
                    if let Some(from_pos) = trimmed.find(" from ") {
                        let before_from = &trimmed[7..from_pos];
                        let module_str = &trimmed[from_pos + 7..];
                        let module = module_str
                            .trim_start_matches('\'')
                            .trim_start_matches('"')
                            .trim_end_matches('\'')
                            .trim_end_matches('"')
                            .to_string();
                        if before_from.starts_with('{') && before_from.ends_with('}') {
                            let items: Vec<String> = before_from[1..before_from.len() - 1]
                                .split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect();
                            imports.push(ImportDecl { module, items });
                        } else {
                            imports.push(ImportDecl {
                                module,
                                items: vec![before_from.trim().to_string()],
                            });
                        }
                    } else if trimmed.starts_with("import \"") || trimmed.starts_with("import '") {
                        let module = trimmed[8..trimmed.len() - 2].to_string();
                        imports.push(ImportDecl {
                            module,
                            items: vec![],
                        });
                    }
                }
                // require('X')
                if trimmed.contains("require(") {
                    let start = trimmed.find("require(").unwrap_or(0) + 8;
                    let module = trimmed[start..]
                        .split(')')
                        .next()
                        .unwrap_or("")
                        .trim_start_matches('\'')
                        .trim_start_matches('"')
                        .trim_end_matches('\'')
                        .trim_end_matches('"')
                        .to_string();
                    if !module.is_empty() {
                        imports.push(ImportDecl {
                            module,
                            items: vec![],
                        });
                    }
                }
            }
            Language::Java | Language::Kotlin
                // import X.Y;
                if trimmed.starts_with("import ") && trimmed.ends_with(';') => {
                    imports.push(ImportDecl {
                        module: trimmed[7..trimmed.len() - 1].to_string(),
                        items: vec![],
                    });
                }
            Language::C | Language::Cpp
                // #include <X> / #include "Y"
                if trimmed.starts_with("#include ") => {
                    let inc = &trimmed[10..];
                    let module = inc
                        .trim_start_matches('<')
                        .trim_start_matches('"')
                        .trim_end_matches('>')
                        .trim_end_matches('"')
                        .to_string();
                    imports.push(ImportDecl {
                        module,
                        items: vec![],
                    });
                }
            _ => {} // PHP, CSharp, Lua, Ruby, Swift: skip import extraction in deterministic mode
        }
    }
    // Cap at 30 imports
    imports.truncate(30);
    imports
}

fn extract_functions(content: &str, language: Language) -> Vec<FunctionSig> {
    let mut functions = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        match language {
            Language::Python => {
                // def foo(x, y) -> Z:
                if let Some(after) = trimmed.strip_prefix("def ") {
                    let name_end = after.find('(').unwrap_or(after.len());
                    let name = after[..name_end].to_string();
                    let params_str = if name_end < after.len() {
                        let paren_start = name_end + 1;
                        let paren_end = after.find(')').unwrap_or(after.len());
                        after[paren_start..paren_end].to_string()
                    } else {
                        String::new()
                    };
                    let params = parse_params(&params_str);
                    let return_type = after.find("->").map(|pos| after[pos + 2..].trim_end_matches(':').trim().to_string());
                    functions.push(FunctionSig {
                        name,
                        params,
                        return_type,
                        docstring: None,
                    });
                }
            }
            Language::Rust
                // fn foo(x: T, y: U) -> Z { / pub fn foo(...) -> Z {
                if (trimmed.starts_with("fn ")
                    || trimmed.starts_with("pub fn ")
                    || trimmed.starts_with("async fn ")
                    || trimmed.starts_with("pub async fn "))
                => {
                    let after = if let Some(s) = trimmed.strip_prefix("pub async fn ") {
                        s
                    } else if let Some(s) = trimmed.strip_prefix("async fn ") {
                        s
                    } else if let Some(s) = trimmed.strip_prefix("pub fn ") {
                        s
                    } else {
                        trimmed.strip_prefix("fn ").unwrap_or(trimmed)
                    };
                    let name_end = after.find('(').unwrap_or(after.len());
                    let name = after[..name_end].to_string();
                    let params_str = if name_end < after.len() {
                        let paren_start = name_end + 1;
                        let paren_end = after.find(')').unwrap_or(after.len());
                        after[paren_start..paren_end].to_string()
                    } else {
                        String::new()
                    };
                    let params = parse_params(&params_str);
                    let return_type = after.find("->").map(|pos| after[pos + 2..]
                                .split_whitespace()
                                .next()
                                .unwrap_or("")
                                .to_string());
                    functions.push(FunctionSig {
                        name,
                        params,
                        return_type,
                        docstring: None,
                    });
                }
            Language::Go => {
                // func Foo(x T, y U) Z { / func (s *Type) Method(x T) Z {
                if let Some(after) = trimmed.strip_prefix("func ") {
                    // Check for method receiver: (s *Type)
                    let name_start = if after.starts_with('(') {
                        // Skip receiver
                        if let Some(end) = after.find(") ") {
                            end + 2
                        } else {
                            0
                        }
                    } else {
                        0
                    };
                    let name_section = &after[name_start..];
                    let name_end = name_section.find('(').unwrap_or(name_section.len());
                    let name = name_section[..name_end].to_string();
                    let params_str = if name_end < name_section.len() {
                        let paren_start = name_end + 1;
                        let paren_end = name_section.find(')').unwrap_or(name_section.len());
                        name_section[paren_start..paren_end].to_string()
                    } else {
                        String::new()
                    };
                    let params = parse_go_params(&params_str);
                    // Return type: after second )
                    let after_params = if let Some(pos) = name_section.find(')') {
                        &name_section[pos + 1..]
                    } else {
                        ""
                    };
                    let return_type = if after_params.trim_start().starts_with('{')
                        || after_params.trim().is_empty()
                    {
                        None
                    } else {
                        let rt = after_params
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .trim_end_matches('{')
                            .to_string();
                        if rt.is_empty() { None } else { Some(rt) }
                    };
                    functions.push(FunctionSig {
                        name,
                        params,
                        return_type,
                        docstring: None,
                    });
                }
            }
            Language::JavaScript | Language::TypeScript
                // function foo(x: T): Z / const foo = (x: T) => Z
                if (trimmed.starts_with("function ")
                    || trimmed.starts_with("export function ")
                    || trimmed.starts_with("async function ")
                    || trimmed.starts_with("export async function "))
                => {
                    let after = trimmed
                        .trim_start_matches("export")
                        .trim()
                        .trim_start_matches("async")
                        .trim()
                        .trim_start_matches("function")
                        .trim();
                    let name_end = after.find('(').unwrap_or(after.len());
                    let name = after[..name_end].to_string();
                    let params_str = if name_end < after.len() {
                        let paren_start = name_end + 1;
                        let paren_end = after.find(')').unwrap_or(after.len());
                        after[paren_start..paren_end].to_string()
                    } else {
                        String::new()
                    };
                    let params = parse_params(&params_str);
                    let return_type = if let Some(pos) = after.find(':') {
                        if pos > after.find(')').unwrap_or(0) {
                            Some(
                                after[pos + 1..]
                                    .split_whitespace()
                                    .next()
                                    .unwrap_or("")
                                    .to_string(),
                            )
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if !name.is_empty() {
                        functions.push(FunctionSig {
                            name,
                            params,
                            return_type,
                            docstring: None,
                        });
                    }
                }
            Language::Java | Language::Kotlin
                // public void foo(T x, U y) / static int bar(T x)
                if trimmed.contains('(')
                    && trimmed.contains(')')
                    && (trimmed.starts_with("public ")
                        || trimmed.starts_with("private ")
                        || trimmed.starts_with("protected ")
                        || trimmed.starts_with("static ")
                        || trimmed.starts_with("internal ")
                        || trimmed.starts_with("override ")
                        || trimmed.starts_with("fun "))
                => {
                    // Extract method name: word before (
                    let paren_pos = trimmed.find('(').unwrap_or(trimmed.len());
                    let before_paren = &trimmed[..paren_pos];
                    let words: Vec<&str> = before_paren.split_whitespace().collect();
                    if words.len() >= 2 {
                        let name = words[words.len() - 1].to_string();
                        let paren_end = trimmed.find(')').unwrap_or(trimmed.len());
                        let params_str = trimmed[paren_pos + 1..paren_end].to_string();
                        let params = parse_params(&params_str);
                        let return_type = if words.len() >= 3 {
                            Some(words[words.len() - 2].to_string())
                        } else {
                            None
                        };
                        functions.push(FunctionSig {
                            name,
                            params,
                            return_type,
                            docstring: None,
                        });
                    }
                }
            _ => {} // C/C++/PHP/Lua/Ruby/Swift/CSharp: limited deterministic extraction
        }
    }
    // Cap at 50 functions
    functions.truncate(50);
    functions
}

fn extract_classes(content: &str, language: Language) -> Vec<ClassSkeleton> {
    let mut classes = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        match language {
            Language::Python
                // class Foo(Base):
                if trimmed.starts_with("class ") && trimmed.ends_with(':') => {
                    let after = &trimmed[6..trimmed.len() - 1];
                    let (name, bases) = if let Some(pos) = after.find('(') {
                        let name = after[..pos].to_string();
                        let bases_str = after[pos + 1..after.len() - 1].to_string();
                        let bases: Vec<String> = bases_str
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        (name, bases)
                    } else {
                        (after.to_string(), vec![])
                    };
                    classes.push(ClassSkeleton {
                        name,
                        bases,
                        docstring: None,
                        methods: vec![],
                    });
                }
            Language::Rust => {
                // struct Foo { / enum Foo { / trait Foo { / pub struct Foo { / pub enum Foo {
                let after_vis = trimmed.strip_prefix("pub ").unwrap_or(trimmed);
                if after_vis.starts_with("struct ")
                    || after_vis.starts_with("enum ")
                    || after_vis.starts_with("trait ")
                {
                    let keyword_len = if after_vis.starts_with("struct ") {
                        7
                    } else if after_vis.starts_with("enum ") {
                        5
                    } else {
                        6
                    };
                    let after = &after_vis[keyword_len..];
                    let name = after
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .trim_end_matches('{')
                        .trim_end_matches(':')
                        .trim_end_matches('<')
                        .to_string();
                    if !name.is_empty() {
                        classes.push(ClassSkeleton {
                            name,
                            bases: vec![],
                            docstring: None,
                            methods: vec![],
                        });
                    }
                }
            }
            Language::Go
                // type Foo struct { / type Foo interface {
                if trimmed.starts_with("type ")
                    && (trimmed.contains("struct") || trimmed.contains("interface"))
                => {
                    let after = &trimmed[5..];
                    let name = after.split_whitespace().next().unwrap_or("").to_string();
                    if !name.is_empty() {
                        classes.push(ClassSkeleton {
                            name,
                            bases: vec![],
                            docstring: None,
                            methods: vec![],
                        });
                    }
                }
            Language::JavaScript | Language::TypeScript
                // class Foo extends Bar / class Foo implements Bar
                if (trimmed.starts_with("class ") || trimmed.starts_with("export class ")) => {
                    let after = trimmed
                        .trim_start_matches("export")
                        .trim()
                        .trim_start_matches("class")
                        .trim();
                    let name_end = after.find_whitespace().unwrap_or(after.len());
                    let name = after[..name_end].to_string();
                    let bases = if after.contains("extends ") {
                        let ext_pos = after.find("extends ").unwrap_or(0) + 8;
                        let impl_pos = after.find("implements ").unwrap_or(after.len());
                        after[ext_pos..impl_pos.min(after.len())]
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .collect()
                    } else {
                        vec![]
                    };
                    classes.push(ClassSkeleton {
                        name,
                        bases,
                        docstring: None,
                        methods: vec![],
                    });
                }
            Language::Java | Language::Kotlin
                // class Foo extends Bar / interface Foo
                if (trimmed.starts_with("public class ")
                    || trimmed.starts_with("private class ")
                    || trimmed.starts_with("class ")
                    || trimmed.starts_with("interface ")
                    || trimmed.starts_with("data class "))
                    && trimmed.contains('{')
                => {
                    let after = trimmed
                        .trim_start_matches("public")
                        .trim()
                        .trim_start_matches("private")
                        .trim()
                        .trim_start_matches("protected")
                        .trim()
                        .trim_start_matches("data")
                        .trim();
                    let keyword_len = if after.starts_with("class ") { 6 } else { 10 }; // "interface "
                    let rest = &after[keyword_len..];
                    let name_end = rest.find_whitespace().unwrap_or(rest.len());
                    let name = rest[..name_end].to_string();
                    let bases = if rest.contains("extends ") {
                        let pos = rest.find("extends ").unwrap_or(0) + 8;
                        let mut bases_str = rest[pos..].to_string();
                        if let Some(brace) = bases_str.find('{') {
                            bases_str = bases_str[..brace].to_string();
                        }
                        bases_str
                            .trim()
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .collect()
                    } else {
                        vec![]
                    };
                    classes.push(ClassSkeleton {
                        name,
                        bases,
                        docstring: None,
                        methods: vec![],
                    });
                }
            Language::C | Language::Cpp
                // struct Foo { / class Foo {
                if (trimmed.starts_with("struct ") || trimmed.starts_with("class "))
                    && trimmed.ends_with('{')
                => {
                    let keyword_len = if trimmed.starts_with("struct ") { 7 } else { 6 };
                    let after = &trimmed[keyword_len..];
                    let name = after.trim_end_matches('{').trim().to_string();
                    if !name.is_empty() {
                        classes.push(ClassSkeleton {
                            name,
                            bases: vec![],
                            docstring: None,
                            methods: vec![],
                        });
                    }
                }
            _ => {} // PHP, CSharp, Lua, Ruby, Swift: limited deterministic extraction
        }
    }
    // Cap at 20 classes
    classes.truncate(20);
    classes
}

/// Parse comma-separated params, stripping type annotations where possible.
fn parse_params(params_str: &str) -> Vec<String> {
    if params_str.is_empty() {
        return vec![];
    }
    params_str
        .split(',')
        .map(|p| {
            let p = p.trim();
            // Strip type annotations: "x: Type" -> "x"
            if let Some(pos) = p.find(':') {
                p[..pos].trim().to_string()
            } else {
                // Strip default values: "x = val" -> "x"
                if let Some(pos) = p.find('=') {
                    p[..pos].trim().to_string()
                } else {
                    p.to_string()
                }
            }
        })
        .filter(|s| {
            !s.is_empty() && s != "self" && s != "&self" && s != "mut self" && s != "&mut self"
        })
        .collect()
}

/// Parse Go params with type annotations: `x T, y U` -> `["x", "y"]`
fn parse_go_params(params_str: &str) -> Vec<String> {
    if params_str.is_empty() {
        return vec![];
    }
    params_str
        .split(',')
        .map(|p| {
            let p = p.trim();
            // Go params: "name Type" or just "Type" (unnamed)
            let words: Vec<&str> = p.split_whitespace().collect();
            if words.is_empty() {
                String::new()
            } else {
                words[0].to_string()
            }
        })
        .filter(|s| !s.is_empty())
        .collect()
}

trait FindWhitespace {
    fn find_whitespace(&self) -> Option<usize>;
}

impl FindWhitespace for str {
    fn find_whitespace(&self) -> Option<usize> {
        self.char_indices()
            .find(|(_, c)| c.is_whitespace())
            .map(|(i, _)| i)
    }
}
