use crate::symbols::{SymbolExtractor, SymbolType};
use std::collections::HashMap;
use std::path::PathBuf;

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
                    let path_str = path.to_string_lossy();
                    if path_str.contains(import.as_str())
                        || node.exports.contains(import)
                        || node.functions.contains(import)
                        || node.classes.contains(import)
                    {
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
            let target_str = target_path.to_string_lossy();
            for (path, node) in &self.files {
                if path == target_path {
                    continue;
                }
                for import in &node.imports {
                    if target_str.contains(import.as_str())
                        || target_node.exports.contains(import)
                        || target_node.functions.contains(import)
                        || target_node.classes.contains(import)
                    {
                        dependents.push(path.clone());
                        break;
                    }
                }
            }
        }
        dependents
    }
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
