use std::collections::HashMap;
use std::path::PathBuf;
use crate::symbols::{SymbolExtractor, SymbolType};

#[derive(Debug, Default)]
pub struct DependencyGraph {
    pub files: HashMap<PathBuf, FileNode>,
}

#[derive(Debug)]
pub struct FileNode {
    pub path: PathBuf,
    pub functions: Vec<String>,
    pub classes: Vec<String>,
    pub imports: Vec<String>,
    pub exports: Vec<String>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn build(paths: Vec<PathBuf>, extractor: &SymbolExtractor) -> Self {
        let mut graph = Self::new();
        
        for path in paths {
            let symbols = extractor.extract(&path);
            
            let mut functions = Vec::new();
            let mut classes = Vec::new();
            let mut imports = Vec::new();
            let mut exports = Vec::new();

            for sym in symbols {
                match sym.symbol_type {
                    SymbolType::Function => functions.push(sym.name),
                    SymbolType::Class => classes.push(sym.name),
                    SymbolType::Import => imports.push(sym.name),
                    SymbolType::Export => exports.push(sym.name),
                }
            }

            graph.files.insert(path.clone(), FileNode {
                path,
                functions,
                classes,
                imports,
                exports,
            });
        }

        graph
    }
}
