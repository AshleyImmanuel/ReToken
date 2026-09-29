use regex::Regex;
use std::sync::LazyLock;
use tracing::debug;

/// Compiled regex matching lines worth keeping: errors, failures, warnings,
/// stack-trace frames, file locations. Ported from Caveman's Go `importantLineRe`
/// and extended with Rust-specific patterns (.rs:line, thread 'main' panicked).
static IMPORTANT_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(\b(ERROR|FATAL|PANIC|EXCEPTION|TRACEBACK|FAIL|FAILED|FAILURE|WARN|WARNING)\b|^\s+at\s|^\s+File\s"|\.go:\d+|\.rs:\d+|\.py:\d+|\.js:\d+|\.ts:\d+|^\s+---\>|caused by|thread\s+'[^']+'\s+panicked)"#
    ).expect("IMPORTANT_LINE_RE must compile")
});

/// Pre-filter literal set for fast rejection. If the lowercased line contains
/// none of these, it cannot match the full regex -- skip the expensive match.
/// This is the same optimisation Caveman applies in `importantLogLine()`.
const FAST_LITERALS: &[&str] = &[
    "error", "fatal", "panic", "exception", "traceback",
    "fail", "warn", ".go:", ".rs:", ".py:", ".js:", ".ts:",
    "caused by", "thread",
];

/// Returns true when `line` carries signal: errors, stack frames, warnings.
fn is_important_line(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    // Whitespace-leading lines go straight to the regex (potential stack frames)
    if line.starts_with(' ') || line.starts_with('\t') {
        return IMPORTANT_LINE_RE.is_match(line);
    }
    // Fast literal pre-filter
    let lower = line.to_ascii_lowercase();
    let might_match = FAST_LITERALS.iter().any(|lit| lower.contains(lit));
    if !might_match {
        return false;
    }
    IMPORTANT_LINE_RE.is_match(line)
}

/// Marker previously emitted by ReToken, kept on re-compression for idempotency.
static LOG_MARKER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"lines elided \(retoken\)").expect("LOG_MARKER_RE must compile")
});

/// Compresses log/command output using Caveman-style heuristics:
///
/// 1. Keep the first `keep_head` and last `keep_tail` lines for context.
/// 2. Keep any line matching the critical-signal regex (errors, stack traces,
///    warnings, file locations).
/// 3. Keep any line that is an existing elision marker (idempotency).
/// 4. Collapse consecutive unmatched lines into `... [N] lines elided (retoken) ...`.
///
/// This is a **strict superset** of Caveman's `logCompressor` -- it additionally
/// detects Rust panics, .rs/.py/.js/.ts file locations, and handles mixed
/// important/unimportant runs more efficiently.
pub fn compress_logs_caveman(log: &str, keep_head: usize, keep_tail: usize) -> String {
    let lines: Vec<&str> = log.lines().collect();
    let n = lines.len();

    if n <= keep_head + keep_tail + 2 {
        // Too short to compress -- return as-is
        return log.to_string();
    }

    // Mark which lines to keep
    let mut keep = vec![false; n];

    // Head anchors
    for i in 0..keep_head.min(n) {
        keep[i] = true;
    }
    // Tail anchors
    for i in n.saturating_sub(keep_tail)..n {
        keep[i] = true;
    }
    // Signal lines
    for (i, line) in lines.iter().enumerate() {
        if is_important_line(line) || LOG_MARKER_RE.is_match(line) {
            keep[i] = true;
        }
    }

    // Build output, collapsing runs of dropped lines
    let mut output = Vec::with_capacity(n);
    let mut elided_run = 0usize;

    for (i, line) in lines.iter().enumerate() {
        if keep[i] {
            if elided_run > 0 {
                output.push(format!("... [{}] lines elided (retoken) ...", elided_run));
                elided_run = 0;
            }
            output.push(line.to_string());
        } else {
            elided_run += 1;
        }
    }
    if elided_run > 0 {
        output.push(format!("... [{}] lines elided (retoken) ...", elided_run));
    }

    let result = output.join("\n");
    let saved = log.len().saturating_sub(result.len());
    if saved > 0 {
        debug!(
            "Log compressor: {} -> {} bytes ({:.1}% reduction, {} lines elided)",
            log.len(),
            result.len(),
            (saved as f64 / log.len() as f64) * 100.0,
            n - keep.iter().filter(|&&k| k).count()
        );
    }
    result
}

/// Convenience wrapper with Caveman defaults (keep first 2, last 2).
pub fn compress_logs(log: &str) -> String {
    compress_logs_caveman(log, 2, 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_errors_and_head_tail() {
        let log = (0..20)
            .map(|i| {
                if i == 10 {
                    "ERROR: something broke".to_string()
                } else {
                    format!("info: compiling step {}", i)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let compressed = compress_logs(&log);
        assert!(compressed.contains("ERROR: something broke"));
        assert!(compressed.contains("step 0"));  // head
        assert!(compressed.contains("step 1"));  // head
        assert!(compressed.contains("step 19")); // tail
        assert!(compressed.contains("step 18")); // tail
        assert!(compressed.contains("lines elided (retoken)"));
    }

    #[test]
    fn short_logs_pass_through() {
        let log = "line 1\nline 2\nline 3\nline 4\nline 5";
        let compressed = compress_logs(log);
        assert_eq!(compressed, log);
    }

    #[test]
    fn idempotent() {
        let log = (0..30)
            .map(|i| format!("info: step {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let first = compress_logs(&log);
        let second = compress_logs(&first);
        assert_eq!(first, second);
    }
}
