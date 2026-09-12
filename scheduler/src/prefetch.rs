use std::collections::HashSet;

/// Predicts likely next files the agent will request based on the dependency graph.
///
/// Strategy (PRD Section 22):
///   When the model reads file A, use the dependency graph to predict
///   that it will need A's imports and A's test file next.
///   Pre-load these into cache while the model is still thinking.
pub struct Prefetcher {
    /// Maximum depth to traverse the dependency graph.
    pub max_depth: usize,
    /// Maximum number of files to prefetch per trigger.
    pub max_prefetch: usize,
}

impl Prefetcher {
    pub fn new(max_depth: usize, max_prefetch: usize) -> Self {
        Self {
            max_depth,
            max_prefetch,
        }
    }

    /// Given a file that was just accessed, predict which files will be needed next.
    /// Uses a simple heuristic:
    ///   1. Direct imports of the accessed file
    ///   2. Test files matching the accessed file's name
    ///   3. Files that import the accessed file (reverse dependencies)
    pub fn predict_next_files(
        &self,
        accessed_file: &str,
        imports: &[(String, Vec<String>)], // (file_path, [imported_paths])
    ) -> Vec<String> {
        let mut predictions: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        seen.insert(accessed_file.to_string());

        // 1. Direct imports of the accessed file
        for (file, deps) in imports {
            if file == accessed_file {
                for dep in deps {
                    if seen.insert(dep.clone()) {
                        predictions.push(dep.clone());
                    }
                }
            }
        }

        // 2. Test file heuristic
        let test_patterns = generate_test_paths(accessed_file);
        for pattern in &test_patterns {
            for (file, _) in imports {
                if file.contains(pattern) && seen.insert(file.clone()) {
                    predictions.push(file.clone());
                }
            }
        }

        // 3. Reverse dependencies (files that import the accessed file)
        for (file, deps) in imports {
            if deps.iter().any(|d| d == accessed_file) && seen.insert(file.clone()) {
                predictions.push(file.clone());
            }
        }

        // Clamp to max_prefetch
        predictions.truncate(self.max_prefetch);
        predictions
    }
}

/// Generate possible test file path patterns from a source file.
/// e.g., "src/auth/login.ts" -> ["login.test", "login_test", "test_login"]
fn generate_test_paths(file_path: &str) -> Vec<String> {
    let stem = std::path::Path::new(file_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    if stem.is_empty() {
        return vec![];
    }

    vec![
        format!("{}.test", stem),
        format!("{}_test", stem),
        format!("test_{}", stem),
        format!("{}.spec", stem),
    ]
}
