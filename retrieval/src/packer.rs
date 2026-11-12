use serde_json::Value;

/// Reorganizes request messages for provider cache optimization and schema validity.
///
/// For Anthropic: extracts any misplaced "system" role messages from the `messages` array
/// and merges them into the top-level "system" parameter.
/// CRITICAL: Preserves the strict chronological ordering of user, assistant, and tool messages
/// so multi-turn tool use conversations are not corrupted or invalidated.
pub fn reorder_for_cache(request_body: &mut Value) {
    let messages = match request_body
        .get_mut("messages")
        .and_then(|m| m.as_array_mut())
    {
        Some(m) => m,
        None => return,
    };

    let mut system_contents = Vec::new();
    let mut clean_messages = Vec::with_capacity(messages.len());

    for msg in messages.drain(..) {
        if msg.get("role").and_then(|r| r.as_str()) == Some("system") {
            if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                system_contents.push(content.to_string());
            }
        } else {
            clean_messages.push(msg);
        }
    }

    *messages = clean_messages;

    if !system_contents.is_empty() {
        if let Some(obj) = request_body.as_object_mut() {
            let combined = system_contents.join("\n\n");
            if let Some(existing_sys) = obj.get_mut("system") {
                if let Some(s) = existing_sys.as_str() {
                    *existing_sys = serde_json::json!(format!("{}\n\n{}", s, combined));
                }
            } else {
                obj.insert("system".to_string(), serde_json::json!(combined));
            }
        }
    }
}

/// Injects an Anthropic-compliant prompt caching breakpoint (`cache_control: {"type": "ephemeral"}`).
///
/// In Anthropic's API, `cache_control` MUST be placed on a content block (inside `system` or inside
/// a `messages` content array), NEVER directly as a root attribute of a message object.
pub fn inject_cache_breakpoint(request_body: &mut Value) {
    let ephemeral_marker = serde_json::json!({"type": "ephemeral"});

    // 1. Prefer placing on the top-level system block if available
    if let Some(system) = request_body.get_mut("system") {
        match system {
            Value::String(s) => {
                let text = s.clone();
                *system = serde_json::json!([
                    {
                        "type": "text",
                        "text": text,
                        "cache_control": ephemeral_marker
                    }
                ]);
                return;
            }
            Value::Array(blocks) => {
                if let Some(last_block) = blocks.last_mut().and_then(|b| b.as_object_mut()) {
                    if !last_block.contains_key("cache_control") {
                        last_block.insert("cache_control".to_string(), ephemeral_marker);
                    }
                    return;
                }
            }
            _ => {}
        }
    }

    // 2. Otherwise place on the last eligible content block in messages
    if let Some(messages) = request_body
        .get_mut("messages")
        .and_then(|m| m.as_array_mut())
    {
        if let Some(last_msg) = messages.last_mut() {
            if let Some(content) = last_msg.get_mut("content") {
                match content {
                    Value::String(text) => {
                        let text_val = text.clone();
                        *content = serde_json::json!([
                            {
                                "type": "text",
                                "text": text_val,
                                "cache_control": ephemeral_marker
                            }
                        ]);
                    }
                    Value::Array(blocks) => {
                        if let Some(last_block) = blocks.last_mut().and_then(|b| b.as_object_mut())
                        {
                            if !last_block.contains_key("cache_control") {
                                last_block.insert("cache_control".to_string(), ephemeral_marker);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Injects a terse mode instruction if no terse instruction is already present.
pub fn inject_terse_mode(request_body: &mut Value) {
    let terse_text = "Respond with extreme brevity. Omit all pleasantries, filler words, and conversational framing. Output only the direct technical answer, code, or tool call.";

    // If top-level system exists (Anthropic format), append to it
    if let Some(system) = request_body.get_mut("system") {
        match system {
            Value::String(s) => {
                if !s.contains(terse_text) {
                    s.push_str("\n\n");
                    s.push_str(terse_text);
                }
                return;
            }
            Value::Array(arr) => {
                let already = arr.iter().any(|b| {
                    b.get("text")
                        .and_then(|t| t.as_str())
                        .map(|t| t.contains(terse_text))
                        .unwrap_or(false)
                });
                if !already {
                    arr.push(serde_json::json!({
                        "type": "text",
                        "text": terse_text
                    }));
                }
                return;
            }
            _ => {}
        }
    }

    // Otherwise handle OpenAI format in messages
    if let Some(messages) = request_body
        .get_mut("messages")
        .and_then(|m| m.as_array_mut())
    {
        let already = messages.iter().any(|m| {
            m.get("content")
                .and_then(|c| c.as_str())
                .map(|s| s.contains(terse_text))
                .unwrap_or(false)
        });
        if !already {
            let terse_instruction = serde_json::json!({
                "role": "system",
                "content": terse_text
            });
            messages.insert(0, terse_instruction);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_reorder_for_cache_preserves_conversation_and_tool_order() {
        let mut body = json!({
            "messages": [
                {"role": "user", "content": "turn 1"},
                {"role": "assistant", "content": "calling tool"},
                {"role": "tool", "content": "tool result 1"},
                {"role": "user", "content": "turn 2"}
            ]
        });

        reorder_for_cache(&mut body);

        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[2]["role"], "tool");
        assert_eq!(msgs[3]["role"], "user");
    }

    #[test]
    fn test_reorder_for_cache_extracts_system_messages_to_top_level() {
        let mut body = json!({
            "system": "Initial system",
            "messages": [
                {"role": "system", "content": "Extra system"},
                {"role": "user", "content": "hello"}
            ]
        });

        reorder_for_cache(&mut body);

        // System role message removed from messages
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");

        // Merged into top-level system
        let sys = body["system"].as_str().unwrap();
        assert!(sys.contains("Initial system"));
        assert!(sys.contains("Extra system"));
    }

    #[test]
    fn test_inject_cache_breakpoint_anthropic_format() {
        let mut body = json!({
            "system": "You are a helpful assistant",
            "messages": [
                {"role": "user", "content": "hello"}
            ]
        });

        inject_cache_breakpoint(&mut body);

        // System should now be an array of blocks with cache_control
        let sys = body["system"].as_array().unwrap();
        assert_eq!(sys.len(), 1);
        assert_eq!(sys[0]["type"], "text");
        assert_eq!(sys[0]["cache_control"]["type"], "ephemeral");

        // Messages should NOT have cache_control on the message object
        let msgs = body["messages"].as_array().unwrap();
        assert!(msgs[0].get("cache_control").is_none());
    }
}
