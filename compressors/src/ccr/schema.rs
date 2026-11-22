/// The tool schema injected into outbound requests so the LLM can retrieve
/// elided data. This is a standard OpenAI/Anthropic tool definition.
pub fn ccr_tool_schema() -> serde_json::Value {
    serde_json::json!({
        "name": "retoken_retrieve",
        "description": "Fetch the full original of tool output that ReToken elided, by its ccr:... handle.",
        "input_schema": {
            "type": "object",
            "properties": {
                "handle": {
                    "type": "string",
                    "description": "CCR handle"
                }
            },
            "required": ["handle"]
        }
    })
}

/// Recovery marker embedded where content was elided. Names the exact tool the model can
/// call, so recovery works whether the tool is proxy-injected or provided by an MCP client.
pub fn ccr_marker(handle: &str, tool: &str) -> String {
    format!(
        "[ReToken: full original elided; call `{tool}` with handle \"{handle}\" to retrieve it \
         (if the tool is deferred, load it with ToolSearch first)]"
    )
}
