/// The tool schema injected into outbound requests so the LLM can retrieve
/// elided data. This is a standard OpenAI/Anthropic tool definition.
pub fn ccr_tool_schema() -> serde_json::Value {
    serde_json::json!({
        "name": "retoken_retrieve",
        "description": "Fetch original data for a given ccr:... handle.",
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

/// Returns the recovery marker string for embedding in compressed payloads.
pub fn ccr_marker(handle: &str) -> String {
    format!("<<ccr:{}>>", handle.strip_prefix("ccr:").unwrap_or(handle))
}
