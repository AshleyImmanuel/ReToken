//! The `retoken_retrieve` tool: returns originals that the gateway elided, from the shared
//! CCR store, optionally limited to a line range.

use compressors::ccr::CcrStore;
use serde_json::{json, Value};

pub const TOOL_NAME: &str = "retoken_retrieve";

/// Retrievals larger than this are truncated with a hint to request a line range, so a huge
/// original does not flood the context window (and stays under clients' MCP output limits).
const MAX_OUTPUT_CHARS: usize = 80_000;

/// MCP tool definition for `tools/list`.
pub fn definition() -> Value {
    json!({
        "name": TOOL_NAME,
        "description": "Fetch the full original of tool output that ReToken elided. Use the handle \
            from a '[ReToken: ... handle \"ccr:...\" ...]' marker. For large outputs, pass \
            pattern to get only matching lines (with line numbers), or start_line/end_line to \
            read only the part you need.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "handle": { "type": "string", "description": "The ccr:... handle from the marker" },
                "start_line": { "type": "integer", "minimum": 1, "description": "First line to return (1-based, optional)" },
                "end_line": { "type": "integer", "minimum": 1, "description": "Last line to return (inclusive, optional)" },
                "pattern": { "type": "string", "description": "Return only lines containing this text, case-insensitive (optional)" }
            },
            "required": ["handle"]
        }
    })
}

/// Executes the tool with `tools/call` arguments; returns an MCP tool result.
pub fn call(store: &CcrStore, args: &Value) -> Value {
    let Some(handle) = args.get("handle").and_then(|h| h.as_str()) else {
        return error("Missing required argument: handle");
    };
    let line_arg = |key: &str| {
        args.get(key)
            .and_then(|v| v.as_u64())
            .map(|n| n.max(1) as usize)
    };

    match store.retrieve(handle) {
        Ok(Some(bytes)) => {
            let text = String::from_utf8_lossy(&bytes);
            let rendered = match args.get("pattern").and_then(|p| p.as_str()) {
                Some(pattern) if !pattern.is_empty() => search(&text, pattern),
                _ => render(&text, line_arg("start_line"), line_arg("end_line")),
            };
            json!({ "content": [{ "type": "text", "text": rendered }] })
        }
        Ok(None) => error(&format!(
            "No original stored for handle {handle}. It may have expired; re-run the command if needed."
        )),
        Err(e) => error(&format!("Retrieval failed: {e}")),
    }
}

/// An MCP tool result flagged as an error (shown to the model, not a protocol error).
pub fn error(message: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

/// Lines containing `pattern` (case-insensitive), prefixed with their 1-based line numbers so
/// the model can follow up with a start_line/end_line range.
fn search(text: &str, pattern: &str) -> String {
    let needle = pattern.to_lowercase();
    let total = text.lines().count();
    let hits: Vec<String> = text
        .lines()
        .enumerate()
        .filter(|(_, line)| line.to_lowercase().contains(&needle))
        .map(|(i, line)| format!("{}: {}", i + 1, line))
        .collect();
    let header = format!("[{} of {} lines match \"{}\"]", hits.len(), total, pattern);
    truncate_with_hint(format!("{}\n{}", header, hits.join("\n")), total)
}

/// Renders the requested line range, truncating oversized output with a range hint.
fn render(text: &str, start_line: Option<usize>, end_line: Option<usize>) -> String {
    let total = text.lines().count();
    let out = if start_line.is_none() && end_line.is_none() {
        text.to_string()
    } else {
        let start = start_line.unwrap_or(1);
        let end = end_line.unwrap_or(total).min(total);
        if start > end {
            return format!(
                "[Requested lines {start}-{end} are outside the output ({total} lines)]"
            );
        }
        let body: Vec<&str> = text.lines().skip(start - 1).take(end + 1 - start).collect();
        format!("[Lines {start}-{end} of {total}]\n{}", body.join("\n"))
    };
    truncate_with_hint(out, total)
}

/// Caps output at [`MAX_OUTPUT_CHARS`] with a hint to narrow the request.
fn truncate_with_hint(mut out: String, total_lines: usize) -> String {
    if out.len() > MAX_OUTPUT_CHARS {
        let mut cut = MAX_OUTPUT_CHARS;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push_str(&format!(
            "\n[Truncated at {MAX_OUTPUT_CHARS} characters; the full output has {total_lines} lines. \
             Call {TOOL_NAME} again with pattern or start_line/end_line to narrow it.]"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_range_and_missing_retrievals() {
        let store = CcrStore::open_in_memory().unwrap();
        let handle = store.store(b"line1\nline2\nline3\nline4", "log").unwrap();

        let full = call(&store, &json!({"handle": handle}));
        assert_eq!(full["content"][0]["text"], "line1\nline2\nline3\nline4");

        let hits = call(&store, &json!({"handle": handle, "pattern": "LINE3"}));
        assert_eq!(
            hits["content"][0]["text"],
            "[1 of 4 lines match \"LINE3\"]\n3: line3"
        );

        let range = call(
            &store,
            &json!({"handle": handle, "start_line": 2, "end_line": 3}),
        );
        assert_eq!(
            range["content"][0]["text"],
            "[Lines 2-3 of 4]\nline2\nline3"
        );

        let missing = call(
            &store,
            &json!({"handle": format!("ccr:{}", "0".repeat(64))}),
        );
        assert_eq!(missing["isError"], true);
        assert_eq!(call(&store, &json!({}))["isError"], true);
    }

    #[test]
    fn oversized_output_is_truncated_with_hint() {
        let text = "x".repeat(MAX_OUTPUT_CHARS + 100);
        let out = render(&text, None, None);
        assert!(out.len() < MAX_OUTPUT_CHARS + 300);
        assert!(out.contains("start_line/end_line"));
    }
}
