use crate::symbols::{SymbolExtractor, SymbolType};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

            graph.files.insert(
                path.clone(),
                FileNode {
                    path,
                    functions,
                    classes,
                    imports,
                    exports,
                },
            );
        }

        graph
    }

    /// Returns all file paths that the given file depends on (imported by target).
    pub fn dependencies_of(&self, target_path: &PathBuf) -> Vec<PathBuf> {
        let mut deps = Vec::new();
        if let Some(target_node) = self.files.get(target_path) {
            for (path, node) in &self.files {
                if path == target_path {
                    continue;
                }
                for import in &target_node.imports {
                    if matches_import(import, target_path, path, node) {
                        deps.push(path.clone());
                        break;
                    }
                }
            }
        }
        deps
    }

    /// Returns all file paths that depend on the given file (import target).
    pub fn dependents_of(&self, target_path: &PathBuf) -> Vec<PathBuf> {
        let mut dependents = Vec::new();
        if let Some(target_node) = self.files.get(target_path) {
            for (path, node) in &self.files {
                if path == target_path {
                    continue;
                }
                for import in &node.imports {
                    if matches_import(import, path, target_path, target_node) {
                        dependents.push(path.clone());
                        break;
                    }
                }
            }
        }
        dependents
    }
}

/// Evaluates whether an imported module or symbol matches a candidate file.
fn matches_import(
    import: &str,
    importer_path: &Path,
    candidate_path: &Path,
    candidate_node: &FileNode,
) -> bool {
    // 1. Exact symbol match: does candidate export or define the imported symbol?
    if candidate_node.exports.iter().any(|e| e == import)
        || candidate_node.functions.iter().any(|f| f == import)
        || candidate_node.classes.iter().any(|c| c == import)
    {
        return true;
    }

    let clean_import = import.trim_start_matches("./").trim_start_matches("../");
    let leaf = clean_import.rsplit("::").next().unwrap_or(clean_import);
    let leaf = leaf.rsplit('.').next().unwrap_or(leaf);

    // 2. Relative import matching (e.g. `./utils`, `../helper`)
    if import.starts_with("./") || import.starts_with("../") {
        if let Some(parent) = importer_path.parent() {
            let candidate_norm = candidate_path.to_string_lossy().replace('\\', "/");
            let resolved = parent.join(import);
            let resolved_norm = resolved.to_string_lossy().replace('\\', "/");
            let clean_resolved = resolved_norm.trim_start_matches("./");

            if candidate_norm.contains(clean_resolved) {
                return true;
            }
        }
    }

    // 3. File stem exact match (e.g. `import helper` matching `helper.rs` or `helper.py`)
    if let Some(stem) = candidate_path.file_stem() {
        let stem_str = stem.to_string_lossy();
        if stem_str == leaf || stem_str == clean_import {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dependency_graph_dependencies_and_dependents() {
        let mut graph = DependencyGraph::new();
        let main_path = PathBuf::from("src/main.rs");
        let helper_path = PathBuf::from("src/helper.rs");

        graph.files.insert(
            main_path.clone(),
            FileNode {
                path: main_path.clone(),
                functions: vec!["main".into()],
                classes: vec![],
                imports: vec!["helper_func".into()],
                exports: vec![],
            },
        );

        graph.files.insert(
            helper_path.clone(),
            FileNode {
                path: helper_path.clone(),
                functions: vec!["helper_func".into()],
                classes: vec![],
                imports: vec![],
                exports: vec!["helper_func".into()],
            },
        );

        let deps = graph.dependencies_of(&main_path);
        assert_eq!(deps, vec![helper_path.clone()]);

        let dependents = graph.dependents_of(&helper_path);
        assert_eq!(dependents, vec![main_path.clone()]);
    }
}
