//! Structural outline of source code: keeps declarations (imports, types, signatures, doc
//! comments, closing braces) and replaces function bodies with a short line-range note.
//!
//! Handles brace languages (Rust, C-family, JS/TS, Go, Java, C#) and indentation languages
//! (Python). Line-number prefixes from file-read tools (`   12\t`, `   12→`) are preserved, so
//! the model can re-read an exact range of what was elided.

use regex::Regex;
use std::sync::LazyLock;

/// Bodies shorter than this stay verbatim: eliding them saves too little.
const MIN_BODY_LINES: usize = 4;

/// `cat -n` style prefix used by agent file-read tools: spaces, line number, tab or arrow.
static LINE_PREFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(\d+)(\t|→)").expect("LINE_PREFIX_RE must compile"));

/// A line that opens a function or method body in a brace language.
static BRACE_FN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(\b(fn|function|func)\b|=>\s*\{\s*$|^\s*(public|private|protected|internal|static|async|override|virtual|export)?[\w\s<>\[\],.*&?:]*\b\w+\s*\([^;]*\)\s*(const\s*)?(->\s*[^{]+|:\s*[^{]+|throws\s+[\w.,\s]+)?\{\s*$)",
    )
    .expect("BRACE_FN_RE must compile")
});

/// Control-flow openers that look like calls but are not declarations.
static CONTROL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*(\}\s*)?(if|else|for|while|match|switch|catch|loop|do|try|return|let|const|var)\b",
    )
    .expect("CONTROL_RE must compile")
});

/// Python function definition.
static PY_DEF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(async\s+)?def\s+\w+").expect("PY_DEF_RE must compile"));

/// One source line, split into its tool prefix (if any) and code.
struct Line<'a> {
    raw: &'a str,
    code: &'a str,
    number: Option<usize>,
}

fn split_lines(text: &str) -> Vec<Line<'_>> {
    text.lines()
        .enumerate()
        .map(|(i, raw)| match LINE_PREFIX_RE.captures(raw) {
            Some(c) => Line {
                raw,
                code: &raw[c.get(0).unwrap().end()..],
                number: c[1].parse().ok(),
            },
            None => Line {
                raw,
                code: raw,
                number: Some(i + 1),
            },
        })
        .collect()
}

/// Returns an outline of `text`, or `None` if it does not look like code or nothing large
/// enough to elide was found.
pub fn outline(text: &str) -> Option<String> {
    let lines = split_lines(text);
    let is_python = lines.iter().filter(|l| PY_DEF_RE.is_match(l.code)).count() >= 2
        && !lines.iter().any(|l| l.code.trim_end().ends_with('{'));
    let elided = if is_python {
        python_bodies(&lines)
    } else {
        brace_bodies(&lines)
    };
    if elided.is_empty() {
        return None;
    }

    let mut out = Vec::new();
    let mut ranges = elided.iter().peekable();
    let mut i = 0;
    while i < lines.len() {
        if let Some(&&(start, end)) = ranges.peek() {
            if i == start {
                let first = lines[start].number.unwrap_or(start + 1);
                let last = lines[end].number.unwrap_or(end + 1);
                let indent: String = lines[start]
                    .code
                    .chars()
                    .take_while(|c| c.is_whitespace())
                    .collect();
                out.push(format!("{indent}// … body elided (lines {first}-{last})"));
                i = end + 1;
                ranges.next();
                continue;
            }
        }
        out.push(lines[i].raw.to_string());
        i += 1;
    }
    Some(out.join("\n"))
}

/// Longest multi-line signature (lines from the `fn` keyword to the opening brace).
const MAX_SIGNATURE_LINES: usize = 12;

/// If a function signature starts at `i`, returns the index of the line holding its opening
/// brace (the same line for one-line signatures). Declarations without a body (`;`) and
/// control-flow lines return `None`.
fn signature_end(lines: &[Line], i: usize) -> Option<usize> {
    let code = strip_comment(lines[i].code);
    if !BRACE_FN_RE.is_match(code) || CONTROL_RE.is_match(code) {
        return None;
    }
    for (j, line) in lines.iter().enumerate().skip(i).take(MAX_SIGNATURE_LINES) {
        let code = strip_comment(line.code).trim_end();
        if code.ends_with('{') {
            return Some(j);
        }
        if code.ends_with(';') || (j > i && code.ends_with('}')) {
            return None;
        }
    }
    None
}

/// Inclusive line-index ranges of function bodies in a brace language.
fn brace_bodies(lines: &[Line]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(open) = signature_end(lines, i) else {
            i += 1;
            continue;
        };
        i = open;
        let code = strip_comment(lines[i].code);
        // Body runs until the brace opened on this line closes.
        let mut depth = brace_delta(code);
        let mut j = i + 1;
        while j < lines.len() && depth > 0 {
            depth += brace_delta(strip_comment(lines[j].code));
            if depth <= 0 {
                break;
            }
            j += 1;
        }
        if j >= lines.len() {
            break; // unbalanced (truncated file): keep the rest verbatim
        }
        // lines[i] is the signature, lines[j] the closing brace; elide what is between.
        if j - i > MIN_BODY_LINES {
            ranges.push((i + 1, j - 1));
        }
        i = j + 1;
    }
    ranges
}

/// Inclusive line-index ranges of Python function bodies.
fn python_bodies(lines: &[Line]) -> Vec<(usize, usize)> {
    let indent = |s: &str| s.len() - s.trim_start().len();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !PY_DEF_RE.is_match(lines[i].code) {
            i += 1;
            continue;
        }
        let def_indent = indent(lines[i].code);
        // Skip a multi-line signature up to the line ending with ':'.
        let mut start = i;
        while start < lines.len() && !strip_comment(lines[start].code).trim_end().ends_with(':') {
            start += 1;
        }
        let mut end = start;
        let mut j = start + 1;
        while j < lines.len() {
            let code = lines[j].code;
            if !code.trim().is_empty() && indent(code) <= def_indent {
                break;
            }
            if !code.trim().is_empty() {
                end = j;
            }
            j += 1;
        }
        if end > start && end - start >= MIN_BODY_LINES {
            ranges.push((start + 1, end));
        }
        i = end.max(i) + 1;
    }
    ranges
}

/// Removes a trailing `//` or `#` comment (outside string literals, roughly).
fn strip_comment(code: &str) -> &str {
    let mut in_str: Option<char> = None;
    let bytes: Vec<char> = code.chars().collect();
    let mut byte_idx = 0;
    for (k, &c) in bytes.iter().enumerate() {
        match in_str {
            Some(q) if c == q && (k == 0 || bytes[k - 1] != '\\') => in_str = None,
            Some(_) => {}
            None if c == '"' || c == '\'' || c == '`' => in_str = Some(c),
            None if c == '/' && bytes.get(k + 1) == Some(&'/') => return &code[..byte_idx],
            None => {}
        }
        byte_idx += c.len_utf8();
    }
    code
}

/// Net `{` minus `}` on a line, ignoring braces inside string and char literals.
fn brace_delta(code: &str) -> i32 {
    let mut depth = 0;
    let mut in_str: Option<char> = None;
    let mut prev = '\0';
    for c in code.chars() {
        match in_str {
            Some(q) if c == q && prev != '\\' => in_str = None,
            Some(_) => {}
            None if c == '"' || c == '`' => in_str = Some(c),
            None if c == '{' => depth += 1,
            None if c == '}' => depth -= 1,
            None => {}
        }
        prev = c;
    }
    depth
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_bodies_elided_signatures_and_types_kept() {
        let src = r#"use std::fmt;

/// Doc comment kept.
pub struct Config {
    pub port: u16,
}

impl Config {
    /// Builds it.
    pub fn new(port: u16) -> Self {
        let a = 1;
        let b = 2;
        let c = a + b;
        if c > 2 {
            println!("{}", c);
        }
        Self { port }
    }

    fn tiny(&self) -> u16 {
        self.port
    }
}
"#;
        let out = outline(src).unwrap();
        assert!(out.contains("pub struct Config {"));
        assert!(out.contains("pub port: u16,"));
        assert!(out.contains("/// Builds it."));
        assert!(out.contains("pub fn new(port: u16) -> Self {"));
        assert!(!out.contains("let c = a + b;"));
        assert!(out.contains("// … body elided (lines 11-17)"));
        assert!(out.contains("self.port"), "short bodies kept verbatim");
    }

    #[test]
    fn read_tool_line_numbers_are_preserved() {
        let body: String = (3..=9)
            .map(|n| format!("{n:>6}\t    step({n});\n"))
            .collect();
        let src = format!("     1\tfn main() {{\n     2\t    init();\n{body}    10\t}}\n");
        let out = outline(&src).unwrap();
        assert!(out.starts_with("     1\tfn main() {"));
        assert!(out.contains("body elided (lines 2-9)"));
        assert!(out.ends_with("    10\t}"));
    }

    #[test]
    fn python_bodies_elided() {
        let src = "import os\n\nclass A:\n    def run(self, x):\n        a = 1\n        b = 2\n        c = 3\n        d = 4\n        return a\n\n    def other(self):\n        return 1\n\ndef top():\n    x = 1\n    y = 2\n    z = 3\n    w = 4\n    return x\n";
        let out = outline(src).unwrap();
        assert!(out.contains("class A:"));
        assert!(out.contains("    def run(self, x):"));
        assert!(!out.contains("c = 3"));
        assert!(out.contains("def top():"));
        assert!(!out.contains("z = 3"));
        assert!(out.contains("return 1"), "short bodies kept");
    }

    #[test]
    fn non_code_returns_none() {
        assert!(outline("just some prose\nwith lines\n").is_none());
        assert!(outline("{\"a\": 1}").is_none());
    }

    #[test]
    fn multi_line_signatures_and_bodyless_declarations() {
        let src = "pub async fn handler(\n    a: u8,\n    b: u8,\n) -> Result<u8, ()> {\n    one();\n    two();\n    three();\n    four();\n    Ok(a)\n}\n\ntrait T {\n    fn decl(\n        x: u8,\n    );\n}\n";
        let out = outline(src).unwrap();
        assert!(out.contains(") -> Result<u8, ()> {"));
        assert!(!out.contains("three();"));
        assert!(out.contains("body elided (lines 5-9)"));
        assert!(out.contains("fn decl("), "trait declarations untouched");
    }

    #[test]
    fn braces_in_strings_do_not_confuse_depth() {
        let src = "fn f() {\n    let s = \"}{}}\";\n    a();\n    b();\n    c();\n    d();\n}\nfn g() {}\n";
        let out = outline(src).unwrap();
        assert!(out.contains("fn g() {}"));
        assert!(!out.contains("c();"));
    }
}
