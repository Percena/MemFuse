use mfs_ast::{Language, SkeletonTextMode, detect_language, extract_skeleton, is_code_file};

#[test]
fn test_detect_language_rust() {
    assert_eq!(detect_language("main.rs"), Some(Language::Rust));
    assert_eq!(detect_language("lib.rs"), Some(Language::Rust));
}

#[test]
fn test_detect_language_python() {
    assert_eq!(detect_language("app.py"), Some(Language::Python));
    assert_eq!(detect_language("app.pyw"), Some(Language::Python));
}

#[test]
fn test_detect_language_javascript() {
    assert_eq!(detect_language("index.js"), Some(Language::JavaScript));
    assert_eq!(detect_language("index.mjs"), Some(Language::JavaScript));
    assert_eq!(detect_language("index.cjs"), Some(Language::JavaScript));
}

#[test]
fn test_detect_language_typescript() {
    assert_eq!(detect_language("app.ts"), Some(Language::TypeScript));
    assert_eq!(detect_language("app.tsx"), Some(Language::TypeScript));
}

#[test]
fn test_detect_language_go() {
    assert_eq!(detect_language("main.go"), Some(Language::Go));
}

#[test]
fn test_detect_language_java() {
    assert_eq!(detect_language("Main.java"), Some(Language::Java));
}

#[test]
fn test_detect_language_c_cpp() {
    assert_eq!(detect_language("main.c"), Some(Language::C));
    assert_eq!(detect_language("main.cpp"), Some(Language::Cpp));
    assert_eq!(detect_language("util.h"), Some(Language::C));
    assert_eq!(detect_language("util.hpp"), Some(Language::Cpp));
}

#[test]
fn test_detect_language_non_code() {
    assert_eq!(detect_language("README.md"), None);
    assert_eq!(detect_language("config.json"), None);
    assert_eq!(detect_language("style.css"), None);
    assert_eq!(detect_language("data.yaml"), None);
}

#[test]
fn test_is_code_file() {
    assert!(is_code_file("main.rs"));
    assert!(!is_code_file("README.md"));
}

#[test]
fn test_extract_skeleton_rust() {
    let code = r#"
//! A module for processing data.
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

pub struct DataProcessor {
    source: String,
}

impl DataProcessor {
    pub fn new(source: &str) -> Self {
        Self { source: source.to_string() }
    }
    pub fn process(&self, input: &str) -> String {
        input.to_uppercase()
    }
}

fn helper(x: i32) -> i32 {
    x + 1
}
"#;

    let skeleton = extract_skeleton("processor.rs", code).unwrap();
    assert_eq!(skeleton.language, Language::Rust);
    assert_eq!(skeleton.file_name, "processor.rs");

    // Module doc
    assert!(skeleton.module_doc.is_some());
    assert!(
        skeleton
            .module_doc
            .as_ref()
            .unwrap()
            .contains("A module for processing data")
    );

    // Imports
    assert!(skeleton.imports.len() >= 2);

    // Classes (struct)
    assert!(skeleton.classes.iter().any(|c| c.name == "DataProcessor"));

    // Functions (impl methods and free functions appear as top-level functions)
    assert!(skeleton.functions.iter().any(|f| f.name == "helper"));
    // impl methods also captured as functions (pub fn new, pub fn process)
    assert!(
        skeleton
            .functions
            .iter()
            .any(|f| f.name == "new" || f.name == "process")
    );

    // Text output
    let text = skeleton.to_text(SkeletonTextMode::Compact);
    assert!(text.contains("processor.rs"));
    assert!(text.contains("DataProcessor"));
}

#[test]
fn test_extract_skeleton_python() {
    let code = r#"
"""A utility module for data handling."""

from collections import defaultdict
from typing import List, Optional

class DataHandler:
    """Handles data processing."""

    def __init__(self, source: str):
        self.source = source

    def process(self, items: List[str]) -> List[str]:
        return [x.upper() for x in items]

def merge(a: dict, b: dict) -> dict:
    """Merge two dictionaries."""
    return {**a, **b}
"#;

    let skeleton = extract_skeleton("handler.py", code).unwrap();
    assert_eq!(skeleton.language, Language::Python);
    assert!(skeleton.module_doc.is_some());
    assert!(skeleton.classes.iter().any(|c| c.name == "DataHandler"));
    assert!(skeleton.functions.iter().any(|f| f.name == "merge"));
}

#[test]
fn test_extract_skeleton_go() {
    let code = r#"
// Package processor handles data transformation.
package processor

import (
    "fmt"
    "strings"
)

type Processor struct {
    Name string
}

func (p *Processor) Transform(input string) string {
    return strings.ToUpper(input)
}

func NewProcessor(name string) *Processor {
    return &Processor{Name: name}
}
"#;

    let skeleton = extract_skeleton("processor.go", code).unwrap();
    assert_eq!(skeleton.language, Language::Go);
    assert!(skeleton.module_doc.is_some());
    assert!(skeleton.classes.iter().any(|c| c.name == "Processor"));
    assert!(
        skeleton
            .functions
            .iter()
            .any(|f| f.name == "Transform" || f.name == "NewProcessor")
    );
}

#[test]
fn test_extract_skeleton_javascript() {
    let code = r#"
import React from 'react';
import { useState, useEffect } from 'react';

class App extends React.Component {
  render() {
    return null;
  }
}

function handleClick(event) {
  console.log(event);
}

export default App;
"#;

    let skeleton = extract_skeleton("App.js", code).unwrap();
    assert_eq!(skeleton.language, Language::JavaScript);
    assert!(skeleton.classes.iter().any(|c| c.name == "App"));
    assert!(skeleton.functions.iter().any(|f| f.name == "handleClick"));
}

#[test]
fn test_extract_skeleton_unsupported() {
    let result = extract_skeleton("README.md", "# Hello\nWorld");
    assert!(result.is_err());
}

#[test]
fn test_skeleton_text_modes() {
    let code = "def foo(x, y) -> int:\n    pass\n";
    let skeleton = extract_skeleton("test.py", code).unwrap();

    let compact = skeleton.to_text(SkeletonTextMode::Compact);
    let verbose = skeleton.to_text(SkeletonTextMode::Verbose);

    // Both should contain function name
    assert!(compact.contains("foo"));
    assert!(verbose.contains("foo"));
}

#[test]
fn test_definition_count() {
    let code = "def a(): pass\ndef b(): pass\n";
    let skeleton = extract_skeleton("test.py", code).unwrap();
    assert_eq!(skeleton.definition_count(), 2);
}

#[test]
fn test_all_symbol_names() {
    let code = "class Foo:\n    def bar(self): pass\ndef baz(): pass\n";
    let skeleton = extract_skeleton("test.py", code).unwrap();
    let names = skeleton.all_symbol_names();
    // Foo class appears in classes, bar appears as free function (not method)
    assert!(names.contains(&"Foo".to_string()));
    // Methods in impl/class blocks are extracted as separate functions in deterministic mode
    assert!(names.contains(&"baz".to_string()));
}
