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
        return output.to_string(); // Fallback just in case
    }

    let head = lines[..head_lines].join("\n");
    let tail = lines[total - tail_lines..].join("\n");
    let omitted = total - head_lines - tail_lines;

    format!(
        "{}\n\n... < {} lines omitted by ReToken Virtualizer > ...\n\n{}",
        head, omitted, tail
    )
}
