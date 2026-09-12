use ignore::WalkBuilder;
use std::path::PathBuf;

pub struct Scanner {
    root: PathBuf,
}

impl Scanner {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Walk the repository, respecting .gitignore, and return source files.
    pub fn scan(&self) -> Vec<PathBuf> {
        let mut builder = WalkBuilder::new(&self.root);
        // We only care about standard source code files for MVP (rs, ts, js, py)
        builder.hidden(true)
               .git_ignore(true)
               .git_exclude(true);

        let mut files = Vec::new();

        for entry in builder.build().flatten() {
            if entry.file_type().is_some_and(|ft| ft.is_file()) {
                let path = entry.path().to_path_buf();
                if let Some("rs" | "ts" | "tsx" | "js" | "jsx" | "py") = path.extension().and_then(|e| e.to_str()) {
                    files.push(path);
                }
            }
        }
        
        files
    }
}
