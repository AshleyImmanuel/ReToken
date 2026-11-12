use regex::Regex;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum SymbolType {
    Function,
    Class,
    Import,
    Export,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub symbol_type: SymbolType,
    pub line: usize,
}

pub struct SymbolExtractor {
    func_regex: Regex,
    class_regex: Regex,
    import_quoted_regex: Regex,
    import_rust_regex: Regex,
    import_python_regex: Regex,
    export_regex: Regex,
}

impl Default for SymbolExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolExtractor {
    pub fn new() -> Self {
        Self {
            // Functions: Rust (fn), JS/TS (function), Python (def)
            func_regex: Regex::new(
                r#"^(?:pub\s+|export\s+|async\s+)*(?:fn|function|def)\s+([a-zA-Z0-9_]+)"#,
            )
            .unwrap(),
            // Types / Classes: class, struct, enum, trait
            class_regex: Regex::new(
                r#"^(?:pub\s+|export\s+)*(?:class|struct|enum|trait)\s+([a-zA-Z0-9_]+)"#,
            )
            .unwrap(),
            // JS/TS imports with quotes
            import_quoted_regex: Regex::new(r#"^(?:import|from)\s+.*(?:from\s+)?['"]([^'"]+)['"]"#)
                .unwrap(),
            // Rust use statements (e.g. `use crate::foo::Bar;`)
            import_rust_regex: Regex::new(
                r#"^(?:pub\s+)?use\s+([a-zA-Z0-9_:]+)(?:::([a-zA-Z0-9_]+|\{[^}]+\}))?;"#,
            )
            .unwrap(),
            // Python import statements (e.g. `import os` or `from os import path`)
            import_python_regex: Regex::new(
                r#"^(?:from\s+([a-zA-Z0-9_.]+)\s+import|import\s+([a-zA-Z0-9_.]+))"#,
            )
            .unwrap(),
            // Exports
            export_regex: Regex::new(r#"^export\s+.*from\s+['"]([^'"]+)['"]"#).unwrap(),
        }
    }

    pub fn extract(&self, path: &PathBuf) -> Vec<Symbol> {
        let mut symbols = Vec::new();
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return symbols,
        };

        let mut in_block_comment = false;

        for (i, line) in content.lines().enumerate() {
            let line_num = i + 1;
            let trimmed = line.trim();

            // Handle multi-line block comments (/* ... */)
            if in_block_comment {
                if trimmed.contains("*/") {
                    in_block_comment = false;
                }
                continue;
            }
            if trimmed.starts_with("/*") {
                if !trimmed.contains("*/") {
                    in_block_comment = true;
                }
                continue;
            }

            // Skip single-line comments and empty lines
            if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
                continue;
            }

            if let Some(caps) = self.func_regex.captures(trimmed) {
                symbols.push(Symbol {
                    name: caps[1].to_string(),
                    symbol_type: SymbolType::Function,
                    line: line_num,
                });
            } else if let Some(caps) = self.class_regex.captures(trimmed) {
                symbols.push(Symbol {
                    name: caps[1].to_string(),
                    symbol_type: SymbolType::Class,
                    line: line_num,
                });
            } else if let Some(caps) = self.import_quoted_regex.captures(trimmed) {
                let mod_path = caps[1].to_string();
                symbols.push(Symbol {
                    name: mod_path.clone(),
                    symbol_type: SymbolType::Import,
                    line: line_num,
                });
                // Extract destructured symbols if present (e.g. `import { A, B } from './foo'`)
                if let Some(brace_start) = trimmed.find('{') {
                    if let Some(brace_end) = trimmed.find('}') {
                        if brace_start < brace_end {
                            for item in trimmed[brace_start + 1..brace_end].split(',') {
                                let sym_name = item.split_whitespace().next_back().unwrap_or("");
                                if !sym_name.is_empty() {
                                    symbols.push(Symbol {
                                        name: sym_name.to_string(),
                                        symbol_type: SymbolType::Import,
                                        line: line_num,
                                    });
                                }
                            }
                        }
                    }
                }
            } else if let Some(caps) = self.import_rust_regex.captures(trimmed) {
                let base = caps[1].to_string();
                symbols.push(Symbol {
                    name: base.clone(),
                    symbol_type: SymbolType::Import,
                    line: line_num,
                });
                // Check if specific items are imported (e.g. `use crate::foo::{A, B};` or `use foo::Bar;`)
                if let Some(tail) = caps.get(2) {
                    let tail_str = tail.as_str().trim_matches(|c| c == '{' || c == '}');
                    for item in tail_str.split(',') {
                        let clean_item = item.trim();
                        if !clean_item.is_empty() {
                            symbols.push(Symbol {
                                name: clean_item.to_string(),
                                symbol_type: SymbolType::Import,
                                line: line_num,
                            });
                        }
                    }
                }
            } else if let Some(caps) = self.import_python_regex.captures(trimmed) {
                let name = caps
                    .get(1)
                    .or_else(|| caps.get(2))
                    .map(|m| m.as_str())
                    .unwrap_or("");
                symbols.push(Symbol {
                    name: name.to_string(),
                    symbol_type: SymbolType::Import,
                    line: line_num,
                });
                // If `from foo import a, b`, also extract individual imported symbols
                if trimmed.starts_with("from ") {
                    if let Some(import_idx) = trimmed.find(" import ") {
                        let imported_part = &trimmed[import_idx + 8..];
                        for item in imported_part.split(',') {
                            let clean_item = item.split_whitespace().next_back().unwrap_or("");
                            if !clean_item.is_empty() && clean_item != name {
                                symbols.push(Symbol {
                                    name: clean_item.to_string(),
                                    symbol_type: SymbolType::Import,
                                    line: line_num,
                                });
                            }
                        }
                    }
                }
            } else if let Some(caps) = self.export_regex.captures(trimmed) {
                symbols.push(Symbol {
                    name: caps[1].to_string(),
                    symbol_type: SymbolType::Export,
                    line: line_num,
                });
            }
        }

        symbols
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_symbols_from_source_string() {
        let extractor = SymbolExtractor::new();
        assert!(extractor.func_regex.is_match("pub fn calculate_total()"));
        assert!(extractor.func_regex.is_match("def process_request(data):"));
        assert!(extractor
            .func_regex
            .is_match("export async function handle()"));

        assert!(extractor.class_regex.is_match("pub struct CacheEngine {"));
        assert!(extractor.class_regex.is_match("pub enum ProviderType {"));
        assert!(extractor.class_regex.is_match("class UserSession:"));

        assert!(extractor.import_rust_regex.is_match("use std::sync::Arc;"));
        assert!(extractor
            .import_rust_regex
            .is_match("pub use crate::cache::ResponseCache;"));
        assert!(extractor
            .import_python_regex
            .is_match("from os import path"));
        assert!(extractor
            .import_quoted_regex
            .is_match("import React from 'react';"));
    }

    #[test]
    fn test_symbol_extractor_skips_comments() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_comments_extract.rs");
        let content = r#"
        // fn commented_func() {}
        # def commented_py(): pass
        /*
        fn block_commented_func() {}
        */
        pub fn real_func() {}
        "#;
        std::fs::write(&test_file, content).unwrap();

        let extractor = SymbolExtractor::new();
        let syms = extractor.extract(&test_file);
        let _ = std::fs::remove_file(&test_file);

        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "real_func");
    }
}
