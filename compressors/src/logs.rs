/// Compresses log output by deduplicating consecutive identical lines.
///
/// Example:
///   PASS test_a
///   PASS test_b
///   PASS test_c
///   ... (x497 more)
///   FAIL test_z
///
/// Becomes:
///   PASS test_a
///   PASS test_b
///   PASS test_c
///   ... (497 similar lines collapsed)
///   FAIL test_z
pub fn compress_logs(log: &str, max_consecutive_dupes: usize) -> String {
    let lines: Vec<&str> = log.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    let mut output = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let current = lines[i];
        
        // Count consecutive lines that match a pattern (same prefix up to first dynamic token)
        let prefix = extract_prefix(current);
        let mut run_count = 1;
        
        while i + run_count < lines.len() {
            let next_prefix = extract_prefix(lines[i + run_count]);
            if next_prefix == prefix && !prefix.is_empty() {
                run_count += 1;
            } else {
                break;
            }
        }

        if run_count > max_consecutive_dupes {
            // Keep first few, collapse the rest
            for j in 0..max_consecutive_dupes {
                output.push(lines[i + j].to_string());
            }
            let collapsed = run_count - max_consecutive_dupes;
            output.push(format!("... ({} similar lines collapsed by ReToken)", collapsed));
        } else {
            for j in 0..run_count {
                output.push(lines[i + j].to_string());
            }
        }

        i += run_count;
    }

    output.join("\n")
}

/// Extracts a "prefix" from a line for dedup grouping.
/// For lines like "PASS src/test_foo.rs", the prefix is "PASS".
/// For lines like "  Compiling serde v1.0.229", the prefix is "Compiling".
fn extract_prefix(line: &str) -> &str {
    let trimmed = line.trim();
    trimmed.split_whitespace().next().unwrap_or("")
}
