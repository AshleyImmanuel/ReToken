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
    import_regex: Regex,
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
            // Very naive regexes for MVP, intended for JS/TS/Rust
            func_regex: Regex::new(r#"^(?:pub\s+|export\s+|async\s+)*(?:fn|function)\s+([a-zA-Z0-9_]+)"#).unwrap(),
            class_regex: Regex::new(r#"^(?:pub\s+|export\s+)*class\s+([a-zA-Z0-9_]+)"#).unwrap(),
            import_regex: Regex::new(r#"^(?:import|use)\s+.*(?:from\s+)?['"]([^'"]+)['"]"#).unwrap(),
            export_regex: Regex::new(r#"^export\s+.*from\s+['"]([^'"]+)['"]"#).unwrap(),
        }
    }

    pub fn extract(&self, path: &PathBuf) -> Vec<Symbol> {
        let mut symbols = Vec::new();
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return symbols,
        };

        for (i, line) in content.lines().enumerate() {
            let line_num = i + 1;
            let trimmed = line.trim_start();
            
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
            } else if let Some(caps) = self.import_regex.captures(trimmed) {
                symbols.push(Symbol {
                    name: caps[1].to_string(),
                    symbol_type: SymbolType::Import,
                    line: line_num,
                });
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
