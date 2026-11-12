use serde_json::Value;
use tracing::debug;

/// Inject the `retoken_retrieve` tool into the request's tools array using provider-specific schema.
/// Only adds it if not already present.
pub fn inject_ccr_tool_for_provider(body: &mut Value, is_anthropic: bool) {
    let tools = body.as_object_mut().and_then(|obj| {
        if !obj.contains_key("tools") {
            obj.insert("tools".to_string(), Value::Array(Vec::new()));
        }
        obj.get_mut("tools").and_then(|t| t.as_array_mut())
    });

    let Some(tools) = tools else {
        return;
    };

    if is_anthropic {
        let already_present = tools
            .iter()
            .any(|t| t.get("name").and_then(|n| n.as_str()) == Some("retoken_retrieve"));
        if !already_present {
            tools.push(crate::ccr::ccr_tool_schema());
            debug!("Injected Anthropic retoken_retrieve tool schema");
        }
    } else {
        let already_present = tools.iter().any(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                == Some("retoken_retrieve")
        });
        if !already_present {
            let openai_schema = serde_json::json!({
                "type": "function",
                "function": {
                    "name": "retoken_retrieve",
                    "description": "Fetch original data for a given ccr:... handle.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "handle": {
                                "type": "string",
                                "description": "CCR handle"
                            }
                        },
                        "required": ["handle"]
                    }
                }
            });
            tools.push(openai_schema);
            debug!("Injected OpenAI retoken_retrieve tool schema");
        }
    }
}

/// Backwards-compatible CCR tool injection (defaults to Anthropic schema).
pub fn inject_ccr_tool(body: &mut Value) {
    inject_ccr_tool_for_provider(body, true);
}

/// Inject the output persona prompt to force terse LLM output, respecting provider protocols.
pub fn inject_output_persona_for_provider(body: &mut Value, persona: &str, is_anthropic: bool) {
    if persona.trim().is_empty() {
        return;
    }

    if is_anthropic {
        // Anthropic: system prompt MUST be placed in top-level "system" field
        if let Some(obj) = body.as_object_mut() {
            if let Some(system) = obj.get_mut("system") {
                match system {
                    Value::String(s) => {
                        if !s.contains(persona) {
                            s.push_str("\n\n");
                            s.push_str(persona);
                        }
                    }
                    Value::Array(arr) => {
                        let already = arr.iter().any(|block| {
                            block
                                .get("text")
                                .and_then(|t| t.as_str())
                                .map(|s| s.contains(persona))
                                .unwrap_or(false)
                        });
                        if !already {
                            arr.push(serde_json::json!({
                                "type": "text",
                                "text": persona
                            }));
                        }
                    }
                    _ => {
                        *system = Value::String(persona.to_string());
                    }
                }
            } else {
                obj.insert("system".to_string(), Value::String(persona.to_string()));
            }
        }
    } else {
        // OpenAI / Generic: system prompt belongs in messages array
        if let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
            let mut found_system = false;
            for msg in messages.iter_mut() {
                if msg.get("role").and_then(|r| r.as_str()) == Some("system") {
                    found_system = true;
                    if let Some(content) = msg.get_mut("content") {
                        if let Some(s) = content.as_str() {
                            if !s.contains(persona) {
                                *content = Value::String(format!("{}\n\n{}", s, persona));
                            }
                        }
                    }
                    break;
                }
            }

            if !found_system {
                let msg = serde_json::json!({
                    "role": "system",
                    "content": persona
                });
                messages.insert(0, msg);
            }
        }
    }
}

/// Backwards-compatible output persona injection.
pub fn inject_output_persona(body: &mut Value, persona: &str) {
    // If top-level system is present or there are no messages, treat as Anthropic
    let is_anthropic = body.get("system").is_some() || body.get("messages").is_none();
    inject_output_persona_for_provider(body, persona, is_anthropic);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_anthropic_persona_injection_top_level_string() {
        let mut body = json!({
            "system": "Base instruction",
            "messages": [{"role": "user", "content": "hello"}]
        });
        inject_output_persona_for_provider(&mut body, "Be terse", true);

        // System prompt appended to top-level system
        let sys = body.get("system").and_then(|s| s.as_str()).unwrap();
        assert!(sys.contains("Base instruction"));
        assert!(sys.contains("Be terse"));

        // Messages array MUST NOT have role: system inserted!
        let msgs = body.get("messages").and_then(|m| m.as_array()).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
    }

    #[test]
    fn test_anthropic_persona_injection_missing_system() {
        let mut body = json!({
            "messages": [{"role": "user", "content": "hello"}]
        });
        inject_output_persona_for_provider(&mut body, "Be terse", true);

        let sys = body.get("system").and_then(|s| s.as_str()).unwrap();
        assert_eq!(sys, "Be terse");

        let msgs = body.get("messages").and_then(|m| m.as_array()).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
    }

    #[test]
    fn test_openai_persona_injection() {
        let mut body = json!({
            "messages": [{"role": "user", "content": "hello"}]
        });
        inject_output_persona_for_provider(&mut body, "Be terse", false);

        let msgs = body.get("messages").and_then(|m| m.as_array()).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "Be terse");
        assert_eq!(msgs[1]["role"], "user");
    }

    #[test]
    fn test_openai_ccr_tool_schema_format() {
        let mut body = json!({
            "messages": [{"role": "user", "content": "hello"}]
        });
        inject_ccr_tool_for_provider(&mut body, false);

        let tools = body.get("tools").and_then(|t| t.as_array()).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "retoken_retrieve");
        assert!(tools[0]["function"]["parameters"].is_object());
    }

    #[test]
    fn test_anthropic_ccr_tool_schema_format() {
        let mut body = json!({
            "messages": [{"role": "user", "content": "hello"}]
        });
        inject_ccr_tool_for_provider(&mut body, true);

        let tools = body.get("tools").and_then(|t| t.as_array()).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "retoken_retrieve");
        assert!(tools[0]["input_schema"].is_object());
    }
}
