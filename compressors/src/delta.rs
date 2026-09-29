use regex::{Captures, Regex};
use similar::{ChangeTag, TextDiff};
use std::sync::LazyLock;
use tracing::debug;
use crate::ccr::CcrStore;

/// Matches Claude/Agy style XML file tags: `<file path="...">...</file>`
static XML_FILE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<file\s+path="([^"]+)">\n?(.*?)\n?</file>"#).unwrap()
});

/// Matches Markdown style code blocks with a file path: ` ```rust\n// path/to/file.rs\n... ``` `
static MD_FILE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Matches ```language \n // File: path/to/file.ext \n content \n ```
    // Or just ``` \n // path/to/file.ext \n
    Regex::new(r#"(?sm)^```[a-zA-Z0-9]*\s*\n(?:(?://|#)\s*(?:File:\s*|path:\s*)?([a-zA-Z0-9_\-\./\\]+\.[a-zA-Z0-9]+)\s*\n)(.*?)\n```"#).unwrap()
});

/// Contextual Delta Encoding.
/// Searches for file blocks in the prompt. If the file has been seen before
/// in this session (via CcrStore), it replaces the full content with a unified diff,
/// saving massive amounts of tokens.
pub fn compress_deltas(text: &str, store: &CcrStore) -> String {
    let mut compressed = text.to_string();

    compressed = XML_FILE_RE.replace_all(&compressed, |caps: &Captures| {
        let path = caps.get(1).map_or("", |m| m.as_str());
        let content = caps.get(2).map_or("", |m| m.as_str());

        // Ignore tiny files, the diff overhead isn't worth it
        if content.len() < 500 {
            return caps.get(0).unwrap().as_str().to_string();
        }

        // Try to get previous state
        if let Some(prev_content) = store.get_file_state(path) {
            // Compute unified diff
            let diff = TextDiff::from_lines(&prev_content, content);
            
            // If it's 100% identical, return a huge truncation marker
            if diff.ratio() == 1.0 {
                return format!("<file path=\"{}\">\n// [ReToken: File unchanged from previous turn]\n</file>", path);
            }

            // Otherwise, generate a unified diff
            let mut diff_output = String::new();
            for op in diff.ops() {
                for change in diff.iter_changes(op) {
                    match change.tag() {
                        ChangeTag::Delete => diff_output.push_str(&format!("-{}", change)),
                        ChangeTag::Insert => diff_output.push_str(&format!("+{}", change)),
                        ChangeTag::Equal => {
                            // Only print context for Equal, to keep it short.
                            // We don't want a full diff. We want a unified diff.
                            // `similar` has a built-in unified diff formatter!
                        }
                    }
                }
            }
            
            // Generate standard unified diff with 3 lines of context
            let unified = diff.unified_diff().context_radius(3).header(path, path).to_string();
            
            // Update the state for next time
            store.set_file_state(path, content);
            
            debug!("CDE: Compressed {} from {} bytes to {} bytes", path, content.len(), unified.len());
            
            return format!("<file_diff path=\"{}\">\n{}\n</file_diff>", path, unified);
        } else {
            // First time seeing this file, save its state but don't compress yet
            store.set_file_state(path, content);
            return caps.get(0).unwrap().as_str().to_string();
        }
    }).to_string();

    compressed = MD_FILE_RE.replace_all(&compressed, |caps: &Captures| {
        let path = caps.get(1).map_or("", |m| m.as_str());
        let content = caps.get(2).map_or("", |m| m.as_str());

        // Ignore tiny files, the diff overhead isn't worth it
        if content.len() < 500 {
            return caps.get(0).unwrap().as_str().to_string();
        }

        // Try to get previous state
        if let Some(prev_content) = store.get_file_state(path) {
            let diff = TextDiff::from_lines(&prev_content, content);
            
            if diff.ratio() == 1.0 {
                return format!("```\n// [ReToken: File {} unchanged from previous turn]\n```", path);
            }

            let unified = diff.unified_diff().context_radius(3).header(path, path).to_string();
            store.set_file_state(path, content);
            
            debug!("CDE: Compressed {} from {} bytes to {} bytes", path, content.len(), unified.len());
            
            return format!("```diff\n{}\n```", unified);
        } else {
            store.set_file_state(path, content);
            return caps.get(0).unwrap().as_str().to_string();
        }
    }).to_string();

    compressed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cde_xml_files() {
        let store = CcrStore::open(None).unwrap();
        let path = "src/main.rs";
        let content1 = format!("<file path=\"{}\">\n{}\n</file>", path, "A".repeat(1000));
        
        // First time, no compression
        let result1 = compress_deltas(&content1, &store);
        assert_eq!(result1, content1);

        // Second time, exactly same content
        let result2 = compress_deltas(&content1, &store);
        assert!(result2.contains("unchanged from previous"));

        // Third time, slightly modified
        let content2 = format!("<file path=\"{}\">\n{}\nB\n</file>", path, "A".repeat(1000));
        let result3 = compress_deltas(&content2, &store);
        assert!(result3.contains("<file_diff"));
        assert!(result3.contains("+B"));
    }

    #[test]
    fn test_cde_md_files() {
        let store = CcrStore::open(None).unwrap();
        let path = "src/lib.rs";
        let content1 = format!("```rust\n// path: {}\n{}\n```", path, "C".repeat(1000));
        
        let result1 = compress_deltas(&content1, &store);
        assert_eq!(result1, content1);

        let content2 = format!("```rust\n// path: {}\n{}\nD\n```", path, "C".repeat(1000));
        let result2 = compress_deltas(&content2, &store);
        assert!(result2.contains("```diff"));
        assert!(result2.contains("+D"));
    }
}
