use regex::{Captures, Regex};
use std::sync::LazyLock;
use tracing::debug;

// Match XML <file> or Markdown code blocks
static CODE_BLOCK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?sm)(<file(?:_diff)?\s+path=[^>]*>\n?)(.*?)(\n?</file(?:_diff)?>)|(^```[a-zA-Z0-9_\-]*\n)(.*?)(\n?```)").unwrap()
});

// Collapse multiple blank lines into a single blank line
static MULTI_BLANK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*\n(\s*\n)+").unwrap());

/// Whitespace minification for code blocks.
/// Collapses excessive blank lines while strictly preserving all developer comments,
/// docstrings, and syntax explaining behavior or security constraints.
pub fn minify_code_blocks(text: &str) -> String {
    CODE_BLOCK_RE
        .replace_all(text, |caps: &Captures| {
            let (prefix, mut content, suffix) = if let Some(m) = caps.get(1) {
                (
                    m.as_str(),
                    caps.get(2).unwrap().as_str().to_string(),
                    caps.get(3).unwrap().as_str(),
                )
            } else {
                (
                    caps.get(4).unwrap().as_str(),
                    caps.get(5).unwrap().as_str().to_string(),
                    caps.get(6).unwrap().as_str(),
                )
            };

            // Don't minify if it's too small
            if content.len() < 200 {
                return caps.get(0).unwrap().as_str().to_string();
            }

            let orig_len = content.len();

            // Collapse multiple blank lines into a single blank line (preserve comments intact)
            content = MULTI_BLANK_RE.replace_all(&content, "\n").to_string();

            if content.len() < orig_len {
                debug!(
                    "Whitespace Minifier: Collapsed blank lines from {} to {} bytes",
                    orig_len,
                    content.len()
                );
            }

            format!("{}{}{}", prefix, content, suffix)
        })
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_minify_preserves_comments_and_collapses_blanks() {
        let code = r#"```rust
fn main() {
    // This comment explains critical behavior
    let x = 1;


    // Security constraint: must validate bounds
    let y = 2;
    // [ReToken: Keep this]
    let url = "http://google.com"; // Keep this too

    let padding = "This padding string makes the block larger than 200 bytes so that the minifier doesn't skip it!";
    println!("{}", padding);
}
```"#;
        let minified = minify_code_blocks(code);
        // All developer comments must be strictly preserved
        assert!(minified.contains("This comment explains critical behavior"));
        assert!(minified.contains("Security constraint: must validate bounds"));
        assert!(minified.contains("[ReToken: Keep this]"));
        assert!(minified.contains("http://google.com"));
        assert!(minified.contains("Keep this too"));
        // Multiple blank lines must be collapsed
        assert!(!minified.contains("let x = 1;\n\n\n"));
    }
}
