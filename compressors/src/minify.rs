use regex::{Captures, Regex};
use std::sync::LazyLock;
use tracing::debug;

// Match XML <file> or Markdown code blocks
static CODE_BLOCK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?sm)(<file(?:_diff)?\s+path=[^>]*>\n?)(.*?)(\n?</file(?:_diff)?>)|(^```[a-zA-Z0-9_\-]*\n)(.*?)(\n?```)").unwrap()
});

// Safely match line comments that are the ONLY thing on the line (ignoring whitespace).
// This prevents accidental stripping of "http://" inside strings.
// Ignores doc comments (///, //!) and our own markers (// [ReToken).
static LINE_COMMENT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*//(.*)$").unwrap()
});

// Collapse multiple blank lines into a single blank line
static MULTI_BLANK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*\n(\s*\n)+").unwrap()
});

/// AST-Aware Code Minification (Heuristic-based for safety and zero-latency).
/// Strips non-essential comments and excessive whitespace from code blocks.
pub fn minify_code_blocks(text: &str) -> String {
    CODE_BLOCK_RE.replace_all(text, |caps: &Captures| {
        let (prefix, mut content, suffix) = if let Some(m) = caps.get(1) {
            (m.as_str(), caps.get(2).unwrap().as_str().to_string(), caps.get(3).unwrap().as_str())
        } else {
            (caps.get(4).unwrap().as_str(), caps.get(5).unwrap().as_str().to_string(), caps.get(6).unwrap().as_str())
        };

        // Don't minify if it's too small
        if content.len() < 200 {
            return caps.get(0).unwrap().as_str().to_string();
        }

        let orig_len = content.len();
        
        // 1. Strip full-line comments (that are not doc comments)
        content = LINE_COMMENT_RE.replace_all(&content, |caps: &Captures| {
            let inner = caps.get(1).unwrap().as_str();
            if inner.starts_with('/') || inner.starts_with('!') || inner.starts_with(" [ReToken") {
                caps.get(0).unwrap().as_str().to_string()
            } else {
                "".to_string()
            }
        }).to_string();

        // 2. Collapse multiple blank lines into a single blank line
        content = MULTI_BLANK_RE.replace_all(&content, "\n").to_string();

        if content.len() < orig_len {
            debug!("Heuristic Minifier: Compressed block from {} to {} bytes", orig_len, content.len());
        }

        format!("{}{}{}", prefix, content, suffix)
    }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_minify_removes_comments_and_blanks() {
        let code = r#"```rust
fn main() {
    // This is a useless comment
    let x = 1;


    // Another one
    let y = 2;
    // [ReToken: Keep this]
    let url = "http://google.com"; // Keep this too

    let padding = "This padding string makes the block larger than 200 bytes so that the minifier doesn't skip it!";
    println!("{}", padding);
}
```"#;
        let minified = minify_code_blocks(code);
        assert!(!minified.contains("useless comment"));
        assert!(!minified.contains("Another one"));
        assert!(minified.contains("[ReToken: Keep this]"));
        assert!(minified.contains("http://google.com"));
        assert!(minified.contains("Keep this too"));
        // Check that double blank lines were collapsed
        assert!(!minified.contains("let x = 1;\n\n\n"));
    }
}
