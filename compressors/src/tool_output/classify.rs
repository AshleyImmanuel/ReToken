//! Classifies tool calls by what their output is: content the agent asked to read, or
//! command output. Only command output is ever elided.

use serde_json::Value;

use super::calls::ToolCall;

/// True if the tool reads files or searches content: the agent asked for exactly this text,
/// so it must reach the model verbatim.
pub fn is_file_read(call: &ToolCall) -> bool {
    const READ_TOOLS: &[&str] = &[
        "read", "view", "open", "cat", "grep", "search", "glob", "fetch",
    ];
    let name = base_tool_name(&call.name);
    if READ_TOOLS.iter().any(|t| name.contains(t)) || name.contains("file") {
        return true;
    }
    command_text(&call.input).is_some_and(|cmd| is_read_command(&cmd))
}

/// True if the call prints a file's contents (as opposed to search results or git output).
/// Only these are outlined once stale.
pub fn is_file_content_read(call: &ToolCall) -> bool {
    const FILE_TOOLS: &[&str] = &["read", "view", "open", "cat"];
    const NOT_FILE_TOOLS: &[&str] = &["grep", "search", "glob", "fetch", "web", "dir", "list"];
    let name = base_tool_name(&call.name);
    if NOT_FILE_TOOLS.iter().any(|t| name.contains(t)) {
        return false;
    }
    if FILE_TOOLS.iter().any(|t| name.contains(t)) {
        return true;
    }
    command_text(&call.input).is_some_and(|cmd| prints_file(&cmd))
}

/// True if the read asks for a specific line range (`offset`/`limit`, `start_line`, `sed -n`):
/// the agent already knows what it needs, so the result is never outlined on arrival.
pub fn is_ranged_read(call: &ToolCall) -> bool {
    const RANGE_KEYS: &[&str] = &[
        "offset",
        "limit",
        "start_line",
        "end_line",
        "line_start",
        "line_end",
        "view_range",
    ];
    RANGE_KEYS.iter().any(|k| call.input.get(*k).is_some())
        || command_text(&call.input).is_some_and(|cmd| cmd.contains("sed -n"))
}

/// True if a pipeline segment's program prints file contents (`cat`, `sed -n`, `Get-Content`...).
fn prints_file(cmd: &str) -> bool {
    const FILE_PRINTERS: &[&str] = &[
        "cat",
        "type",
        "head",
        "tail",
        "less",
        "more",
        "bat",
        "nl",
        "sed",
        "get-content",
        "gc",
    ];
    cmd.split(['|', ';', '&', '\n'])
        .filter_map(|segment| segment.split_whitespace().next())
        .map(program_name)
        .any(|p| FILE_PRINTERS.contains(&p.as_str()))
}

/// Lowercase program name without directory or `.exe`.
fn program_name(word: &str) -> String {
    word.rsplit(['/', '\\'])
        .next()
        .unwrap_or(word)
        .trim_end_matches(".exe")
        .to_ascii_lowercase()
}

/// True if the tool runs shell commands, so its output is command/log output.
pub fn is_command(call: &ToolCall) -> bool {
    const SHELL_TOOLS: &[&str] = &[
        "bash",
        "shell",
        "terminal",
        "exec",
        "command",
        "powershell",
        "run",
    ];
    let name = base_tool_name(&call.name);
    SHELL_TOOLS.iter().any(|t| name.contains(t)) || command_text(&call.input).is_some()
}

/// Tool name with MCP-style prefixes removed (`mcp__server__read_file` -> `read_file`).
fn base_tool_name(name: &str) -> String {
    name.rsplit("__")
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase()
}

/// The shell command of a tool call, if any (`{"command": "..."}` or `{"command": [...]}`).
fn command_text(input: &Value) -> Option<String> {
    match input.get("command").or_else(|| input.get("cmd"))? {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => None,
    }
}

/// True if any pipeline segment of the command prints file contents or search results.
fn is_read_command(cmd: &str) -> bool {
    const READ_COMMANDS: &[&str] = &[
        "cat",
        "type",
        "head",
        "tail",
        "less",
        "more",
        "bat",
        "nl",
        "sed",
        "awk",
        "grep",
        "rg",
        "ag",
        "findstr",
        "get-content",
        "gc",
        "select-string",
        "sls",
        "jq",
        "diff",
    ];
    cmd.split(['|', ';', '&', '\n']).any(|segment| {
        let Some(first) = segment.split_whitespace().next() else {
            return false;
        };
        let program = first
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(first)
            .trim_end_matches(".exe")
            .to_ascii_lowercase();
        // git output (status, log, diff, show, blame) is structured information the agent
        // asked for, not build noise.
        program == "git" || READ_COMMANDS.contains(&program.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str, input: Value) -> ToolCall {
        ToolCall {
            name: name.into(),
            input,
        }
    }

    #[test]
    fn read_commands_are_detected_across_pipelines() {
        assert!(is_read_command("cat src/main.rs"));
        assert!(is_read_command("cd /repo && sed -n 1,80p src/lib.rs"));
        assert!(is_read_command("grep -rn foo . | head -20"));
        assert!(is_read_command("Get-Content file.txt"));
        assert!(is_read_command("git --no-pager diff HEAD~1"));
        assert!(!is_read_command("cargo build --release"));
        assert!(!is_read_command("npm test"));
        assert!(is_read_command("git status"));
        assert!(is_read_command("git log --stat -30"));
        assert!(!is_read_command("cargo test --workspace"));
    }

    #[test]
    fn tool_name_classification() {
        assert!(is_file_read(&call("Read", json!({"file_path": "a.rs"}))));
        assert!(is_file_read(&call("mcp__fs__read_text_file", json!({}))));
        assert!(is_file_read(&call("Bash", json!({"command": "cat a.rs"}))));
        assert!(!is_file_read(&call(
            "Bash",
            json!({"command": "cargo test"})
        )));
        assert!(is_command(&call("Bash", json!({"command": "cargo test"}))));
        assert!(is_command(&call(
            "shell",
            json!({"command": ["bash", "-lc", "make"]})
        )));
        assert!(!is_command(&call("WebSearch", json!({"query": "x"}))));
        assert!(is_file_content_read(&call(
            "Read",
            json!({"file_path": "a.rs"})
        )));
        assert!(is_file_content_read(&call(
            "Bash",
            json!({"command": "sed -n 1,80p a.rs"})
        )));
        assert!(!is_file_content_read(&call(
            "Grep",
            json!({"pattern": "fn"})
        )));
        assert!(!is_file_content_read(&call(
            "Bash",
            json!({"command": "rg fn src"})
        )));
        assert!(!is_file_content_read(&call(
            "Bash",
            json!({"command": "git show HEAD"})
        )));
    }
}
