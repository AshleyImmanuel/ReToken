//! Index of tool invocations, used to attribute each tool result to the call that produced it.

use serde_json::Value;
use std::collections::HashMap;

/// The tool invocation that produced a tool result.
pub struct ToolCall {
    pub name: String,
    pub input: Value,
}

impl ToolCall {
    /// Short human-readable form, e.g. `Read(gateway/src/provider.rs)` or `Bash(cargo test)`:
    /// the tool name plus its main argument, truncated to stay compact.
    pub fn describe(&self) -> String {
        const MAX_ARG_CHARS: usize = 80;
        let arg = [
            "file_path",
            "path",
            "command",
            "cmd",
            "pattern",
            "query",
            "url",
        ]
        .iter()
        .find_map(|key| match self.input.get(*key)? {
            Value::String(s) => Some(s.clone()),
            Value::Array(parts) => Some(
                parts
                    .iter()
                    .filter_map(|p| p.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => None,
        });
        match arg {
            Some(arg) => {
                let mut arg = arg.replace('\n', " ");
                if arg.chars().count() > MAX_ARG_CHARS {
                    arg = arg.chars().take(MAX_ARG_CHARS).collect::<String>() + "...";
                }
                format!("{}({})", self.name, arg)
            }
            None => self.name.clone(),
        }
    }
}

/// Tool calls seen so far in a conversation, keyed by call id.
#[derive(Default)]
pub struct ToolCallIndex {
    calls: HashMap<String, ToolCall>,
}

impl ToolCallIndex {
    /// Records the tool invocations of an assistant message: Anthropic `tool_use` content
    /// blocks or OpenAI `tool_calls`.
    pub fn record(&mut self, assistant_msg: &Value) {
        if let Some(blocks) = assistant_msg.get("content").and_then(|c| c.as_array()) {
            for block in blocks {
                if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                    continue;
                }
                if let (Some(id), Some(name)) = (
                    block.get("id").and_then(|v| v.as_str()),
                    block.get("name").and_then(|v| v.as_str()),
                ) {
                    let input = block.get("input").cloned().unwrap_or(Value::Null);
                    self.insert(id, name, input);
                }
            }
        }
        if let Some(tool_calls) = assistant_msg.get("tool_calls").and_then(|c| c.as_array()) {
            for tc in tool_calls {
                let function = tc.get("function");
                if let (Some(id), Some(name)) = (
                    tc.get("id").and_then(|v| v.as_str()),
                    function
                        .and_then(|f| f.get("name"))
                        .and_then(|v| v.as_str()),
                ) {
                    // OpenAI encodes arguments as a JSON string.
                    let input = function
                        .and_then(|f| f.get("arguments"))
                        .and_then(|a| a.as_str())
                        .and_then(|a| serde_json::from_str(a).ok())
                        .unwrap_or(Value::Null);
                    self.insert(id, name, input);
                }
            }
        }
    }

    /// The call with this id, if it was recorded.
    pub fn get(&self, id: Option<&str>) -> Option<&ToolCall> {
        self.calls.get(id?)
    }

    fn insert(&mut self, id: &str, name: &str, input: Value) {
        self.calls.insert(
            id.to_string(),
            ToolCall {
                name: name.to_string(),
                input,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn records_anthropic_and_openai_calls() {
        let mut index = ToolCallIndex::default();
        index.record(&json!({"role": "assistant", "content": [
            {"type": "text", "text": "running"},
            {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "make"}}
        ]}));
        index.record(&json!({"role": "assistant", "tool_calls": [
            {"id": "c1", "type": "function",
             "function": {"name": "shell", "arguments": "{\"command\":\"ls\"}"}}
        ]}));

        let t1 = index.get(Some("t1")).unwrap();
        assert_eq!(t1.name, "Bash");
        assert_eq!(t1.input["command"], "make");
        assert_eq!(index.get(Some("c1")).unwrap().input["command"], "ls");
        assert!(index.get(Some("missing")).is_none());
        assert!(index.get(None).is_none());

        assert_eq!(t1.describe(), "Bash(make)");
        let long = ToolCall {
            name: "Bash".into(),
            input: json!({"command": "x".repeat(200)}),
        };
        assert!(long.describe().ends_with("...)"));
        let bare = ToolCall {
            name: "TodoWrite".into(),
            input: json!({"todos": []}),
        };
        assert_eq!(bare.describe(), "TodoWrite");
    }
}
