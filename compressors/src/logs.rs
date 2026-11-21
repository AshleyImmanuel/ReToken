use regex::Regex;
use std::sync::LazyLock;
use tracing::debug;

/// Compiled regex matching lines worth keeping: errors, failures, warnings,
/// stack-trace frames, file locations. Ported from legacy proxy `importantLineRe`
/// and extended with Rust-specific patterns (.rs:line, thread 'main' panicked).
static IMPORTANT_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(\b(ERROR|FATAL|PANIC|EXCEPTION|TRACEBACK|FAIL|FAILED|FAILURE|WARN|WARNING)\b|\btest result\b|\b\d+ (passed|failed|tests?)\b|^\s+at\s|^\s+File\s"|\.go:\d+|\.rs:\d+|\.py:\d+|\.js:\d+|\.ts:\d+|^\s+---\>|caused by|thread\s+'[^']+'\s+panicked|\bassert|^E\s|^\s*(left|right|expected|received|actual)\s*:)"#
    ).expect("IMPORTANT_LINE_RE must compile")
});

/// Lines that start a failure report. The lines right after them usually explain the failure
/// in formats no keyword list can anticipate, so they are kept as context.
static FAILURE_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(FAILED|FAIL\b|panicked|Traceback|Exception|Error:|^error(\[|:)|^\s*E\s{2,}|^\s*●)"#,
    )
    .expect("FAILURE_LINE_RE must compile")
});

/// Lines kept after each failure line (see [`FAILURE_LINE_RE`]).
const FAILURE_CONTEXT_AFTER: usize = 4;
/// Lines kept before each failure line.
const FAILURE_CONTEXT_BEFORE: usize = 1;

/// Pre-filter literal set for fast rejection. If the lowercased line contains
/// none of these, it cannot match the full regex -- skip the expensive match.
/// This is the same optimisation applied in `importantLogLine()`.
const FAST_LITERALS: &[&str] = &[
    "error",
    "fatal",
    "panic",
    "exception",
    "traceback",
    "fail",
    "warn",
    ".go:",
    ".rs:",
    ".py:",
    ".js:",
    ".ts:",
    "caused by",
    "thread",
    "test result",
    "passed",
    "failed",
    "test",
    "assert",
    "left",
    "right",
    "expected",
    "received",
    "actual",
];

/// Returns true when `line` carries signal: errors, stack frames, warnings.
fn is_important_line(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    // Whitespace-leading lines (potential stack frames) and pytest `E ` detail lines go
    // straight to the regex
    if line.starts_with(' ') || line.starts_with('\t') || line.starts_with("E ") {
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
static LOG_MARKER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"lines elided \(retoken\)").expect("LOG_MARKER_RE must compile"));

/// Compresses log/command output using advanced heuristics:
///
/// 1. Keep the first `keep_head` and last `keep_tail` lines for context.
/// 2. Keep any line matching the critical-signal regex (errors, stack traces,
///    warnings, file locations).
/// 3. Keep any line that is an existing elision marker (idempotency).
/// 4. Collapse consecutive unmatched lines into `... [N] lines elided (retoken) ...`.
///
/// This is a **strict superset** of the legacy `logCompressor` -- it additionally
/// detects Rust panics, .rs/.py/.js/.ts file locations, and handles mixed
/// important/unimportant runs more efficiently.
pub fn compress_logs_heuristic(log: &str, keep_head: usize, keep_tail: usize) -> String {
    let lines: Vec<&str> = log.lines().collect();
    let n = lines.len();

    if n <= keep_head + keep_tail + 2 {
        // Too short to compress -- return as-is
        return log.to_string();
    }

    // Mark which lines to keep
    let mut keep = vec![false; n];

    // Head anchors
    for item in keep.iter_mut().take(keep_head.min(n)) {
        *item = true;
    }
    // Tail anchors
    for item in keep.iter_mut().take(n).skip(n.saturating_sub(keep_tail)) {
        *item = true;
    }
    // Signal lines, plus context around each failure line
    for (i, line) in lines.iter().enumerate() {
        if is_important_line(line) || LOG_MARKER_RE.is_match(line) {
            keep[i] = true;
        }
        if FAILURE_LINE_RE.is_match(line) {
            let from = i.saturating_sub(FAILURE_CONTEXT_BEFORE);
            let to = (i + FAILURE_CONTEXT_AFTER).min(n - 1);
            keep[from..=to].iter_mut().for_each(|k| *k = true);
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

/// Default log compression: keep the first 8 and last 15 lines (commands usually print their
/// summary at the end) plus every signal line.
pub fn compress_logs(log: &str) -> String {
    compress_logs_heuristic(log, 8, 15)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_errors_and_head_tail() {
        let log = (0..60)
            .map(|i| {
                if i == 30 {
                    "ERROR: something broke".to_string()
                } else {
                    format!("info: compiling step {}", i)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let compressed = compress_logs(&log);
        assert!(compressed.contains("ERROR: something broke"));
        assert!(compressed.contains("step 0")); // head
        assert!(compressed.contains("step 1")); // head
        assert!(compressed.contains("step 59")); // tail
        assert!(compressed.contains("step 58")); // tail
        assert!(!compressed.contains(
            "step 20
"
        )); // middle elided
        assert!(compressed.contains("lines elided (retoken)"));
    }

    #[test]
    fn keeps_failure_details_rust_pytest_jest() {
        let mut log = String::new();
        for i in 0..40 {
            log.push_str(&format!("test tests::case_{i} ... ok\n"));
        }
        log.push_str("thread 'tests::parses_header' panicked at src/lib.rs:42:9:\n");
        log.push_str("assertion `left == right` failed\n  left: \"Content-Type\"\n right: \"content-type\"\n");
        for i in 0..30 {
            log.push_str(&format!("   filler line {i}\n"));
        }
        log.push_str(">       assert parse('a') == 2\nE       assert 1 == 2\nE        +  where 1 = parse('a')\n");
        for i in 0..30 {
            log.push_str(&format!("   more filler {i}\n"));
        }
        log.push_str("  ● sum › adds\n    expect(received).toBe(expected)\n    Expected: 3\n    Received: 4\n");
        for i in 0..30 {
            log.push_str(&format!("   tail filler {i}\n"));
        }
        log.push_str("test result: FAILED. 80 passed; 1 failed\n");

        let out = compress_logs(&log);
        for needle in [
            "panicked at src/lib.rs:42:9",
            "left: \"Content-Type\"",
            "right: \"content-type\"",
            "E       assert 1 == 2",
            "where 1 = parse('a')",
            "sum › adds",
            "Expected: 3",
            "Received: 4",
            "test result: FAILED",
        ] {
            assert!(out.contains(needle), "dropped: {needle}");
        }
        assert!(!out.contains("filler line 15\n"), "noise still elided");
        assert!(out.len() < log.len() / 2);
    }

    #[test]
    fn passing_runs_get_no_failure_context() {
        // "0 failed" in a passing summary must not pull in surrounding lines.
        let mut log: String = (0..60).map(|i| format!("test t{i} ... ok\n")).collect();
        log.push_str("test result: ok. 60 passed; 0 failed\n");
        log.push_str(
            &(0..60)
                .map(|i| format!("test u{i} ... ok\n"))
                .collect::<String>(),
        );
        let out = compress_logs(&log);
        assert!(!out.contains("test t58 ... ok"));
        assert!(!out.contains("test u1 ... ok"));
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
