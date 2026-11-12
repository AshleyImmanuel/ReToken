/// Virtualizes huge text logs to save token budget.
/// If `output` exceeds `max_lines`, it will keep `head_lines` and `tail_lines`
/// and replace the middle with a summary marker.
pub fn slice_log(output: &str, max_lines: usize, head_lines: usize, tail_lines: usize) -> String {
    let lines: Vec<&str> = output.lines().collect();
    let total = lines.len();

    if total <= max_lines {
        return output.to_string();
    }

    if head_lines + tail_lines >= total {
        return output.to_string();
    }

    let head = lines[..head_lines].join("\n");
    let tail = lines[total - tail_lines..].join("\n");
    let omitted = total - head_lines - tail_lines;

    format!(
        "{}\n\n... < {} lines omitted by ReToken Virtualizer > ...\n\n{}",
        head, omitted, tail
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slice_log_within_limit() {
        let log = "line 1\nline 2\nline 3";
        assert_eq!(slice_log(log, 10, 3, 3), log);
    }

    #[test]
    fn test_slice_log_exceeding_limit() {
        let lines: Vec<String> = (1..=20).map(|i| format!("line {}", i)).collect();
        let log = lines.join("\n");

        let sliced = slice_log(&log, 10, 3, 3);
        assert!(sliced.starts_with("line 1\nline 2\nline 3"));
        assert!(sliced.ends_with("line 18\nline 19\nline 20"));
        assert!(sliced.contains("... < 14 lines omitted by ReToken Virtualizer > ..."));
    }
}
