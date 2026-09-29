use serde_json::Value;
use tracing::debug;

/// Inject the `retoken_retrieve` tool into the request's tools array.
/// Only adds it if not already present.
pub fn inject_ccr_tool(body: &mut Value) {
    let tool_schema = crate::ccr::ccr_tool_schema();

    // Get or create the tools array
    let tools = body
        .as_object_mut()
        .and_then(|obj| {
            if !obj.contains_key("tools") {
                obj.insert("tools".to_string(), Value::Array(Vec::new()));
            }
            obj.get_mut("tools").and_then(|t| t.as_array_mut())
        });

    if let Some(tools) = tools {
        // Check if already present
        let already_present = tools.iter().any(|t| {
            t.get("name").and_then(|n| n.as_str()) == Some("retoken_retrieve")
        });
        if !already_present {
            tools.push(tool_schema);
            debug!("Injected retoken_retrieve tool schema");
        }
    }
}

/// Inject the output persona prompt to force terse LLM output.
/// This is stronger than ReToken's existing terse_mode and specifically
/// targets credit optimization.
pub fn inject_output_persona(body: &mut Value) {
    let persona = "[ReToken proxy active. Terse mode. No pleasantries. Output only actionable code/findings. Call retoken_retrieve if you need full data for a ccr:... handle.]";

    // Try OpenAI format first (messages array with system role)
    if let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        // Check if we already injected
        let already_injected = messages.iter().any(|m| {
            m.get("content")
                .and_then(|c| c.as_str())
                .map(|s| s.contains("ReToken proxy active"))
                .unwrap_or(false)
        });
        if !already_injected {
            let msg = serde_json::json!({
                "role": "system",
                "content": persona
            });
            messages.insert(0, msg);
        }
        return;
    }

    // Try Anthropic format (system field at top level)
    if let Some(obj) = body.as_object_mut() {
        if let Some(system) = obj.get_mut("system") {
            match system {
                Value::String(s) => {
                    if !s.contains("ReToken proxy active") {
                        s.push_str("\n\n");
                        s.push_str(persona);
                    }
                }
                Value::Array(arr) => {
                    let already = arr.iter().any(|block| {
                        block
                            .get("text")
                            .and_then(|t| t.as_str())
                            .map(|s| s.contains("ReToken proxy active"))
                            .unwrap_or(false)
                    });
                    if !already {
                        arr.push(serde_json::json!({
                            "type": "text",
                            "text": persona
                        }));
                    }
                }
                _ => {}
            }
        }
    }
}
