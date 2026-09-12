use analyzers::graph::DependencyGraph;
use std::collections::HashMap;
use std::path::PathBuf;
use crate::package::ContextPackage;

pub struct ContextPlanner {
    default_budget: usize,
}

impl ContextPlanner {
    pub fn new(default_budget: usize) -> Self {
        Self { default_budget }
    }

    /// Ranks files in the DependencyGraph and returns a ContextPackage that respects the token budget.
    pub fn plan(&self, graph: &DependencyGraph, active_files: &[PathBuf]) -> ContextPackage {
        let mut package = ContextPackage::new(self.default_budget);
        
        // 1. Calculate scores
        let mut scores: HashMap<PathBuf, i32> = HashMap::new();
        
        for (path, node) in &graph.files {
            let mut score = 0;
            
            // Base heuristics
            if active_files.contains(path) {
                score += 50; // High priority for actively edited/requested files
            }
            
            // For MVP, we give points if the file is imported by an active file
            for active in active_files {
                if let Some(active_node) = graph.files.get(active) {
                    for import in &active_node.imports {
                        // If this node exports something the active file imports
                        if node.exports.contains(import) || node.classes.contains(import) || node.functions.contains(import) {
                            score += 20;
                        }
                    }
                }
            }
            
            scores.insert(path.clone(), score);
        }
        
        // 2. Sort files by score descending
        let mut sorted_files: Vec<_> = scores.into_iter().collect();
        sorted_files.sort_by_key(|a| std::cmp::Reverse(a.1));
        
        // 3. Pack into the package
        for (path, score) in sorted_files {
            if score <= 0 {
                continue;
            }
            
            // In a real system, we'd look up the exact StateObject token size.
            // For MVP, we estimate based on file size: 1 token ~ 4 bytes.
            let token_estimate = if let Ok(metadata) = std::fs::metadata(&path) {
                (metadata.len() as usize) / 4
            } else {
                500 // fallback if file missing
            };
            
            let file_id = path.to_string_lossy().into_owned(); // Normally we'd use the SHA hash object_id
            
            if !package.add(file_id, token_estimate) {
                break; // Budget full
            }
        }
        
        package
    }
}
