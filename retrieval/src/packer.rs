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

/// Injects Anthropic prompt caching breakpoints (`cache_control: {"type": "ephemeral"}`).
///
/// 1. A rolling breakpoint on the last content block of the last message. Anthropic caches the
///    whole prefix up to a breakpoint (tools, system, and the full conversation history), and
///    on the next turn finds that prefix again via its lookback, so agent loops pay the cached
///    rate for everything but the newest turn.
/// 2. A breakpoint on the system prompt, which stays valid even when the history changes.
///
/// Existing client breakpoints are respected: nothing is added beyond Anthropic's limit of 4,
/// and a message that already carries a breakpoint is left alone.
///
/// In Anthropic's API, `cache_control` MUST be placed on a content block (inside `system` or inside
/// a `messages` content array), NEVER directly as a root attribute of a message object.
pub fn inject_cache_breakpoint(request_body: &mut Value) {
    let mut budget =
        MAX_ANTHROPIC_CACHE_BREAKPOINTS.saturating_sub(count_cache_breakpoints(request_body));

    if budget > 0 {
        let last_content = request_body
            .get_mut("messages")
            .and_then(|m| m.as_array_mut())
            .and_then(|msgs| msgs.last_mut())
            .and_then(|msg| msg.get_mut("content"));
        if let Some(content) = last_content {
            if mark_last_block(content) {
                budget -= 1;
            }
        }
    }

    if budget > 0 {
        if let Some(system) = request_body.get_mut("system") {
            mark_last_block(system);
        }
    }
}

/// Puts a breakpoint on the last cacheable block of `content` (a string is converted to a text
/// block). Returns true if a breakpoint was added. Content that already has one is left alone.
fn mark_last_block(content: &mut Value) -> bool {
    let marker = serde_json::json!({"type": "ephemeral"});
    match content {
        Value::String(text) if !text.is_empty() => {
            *content = serde_json::json!([
                {"type": "text", "text": std::mem::take(text), "cache_control": marker}
            ]);
            true
        }
        Value::Array(blocks) => {
            if blocks.iter().any(|b| b.get("cache_control").is_some()) {
                return false;
            }
            // Thinking blocks and empty text blocks cannot carry cache_control.
            let target = blocks.iter_mut().rev().find(|b| {
                let kind = b.get("type").and_then(|t| t.as_str()).unwrap_or("");
                let empty_text = kind == "text"
                    && b.get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .is_empty();
                !matches!(kind, "thinking" | "redacted_thinking") && !empty_text
            });
            match target.and_then(|b| b.as_object_mut()) {
                Some(block) => {
                    block.insert("cache_control".to_string(), marker);
                    true
                }
                None => false,
            }
        }
        _ => false,
    }
}

/// Maximum number of `cache_control` breakpoints Anthropic accepts per request.
const MAX_ANTHROPIC_CACHE_BREAKPOINTS: usize = 4;

/// Counts `cache_control` markers already present on tools, system blocks, and message content blocks.
fn count_cache_breakpoints(request_body: &Value) -> usize {
    let blocks = |v: Option<&Value>| -> usize {
        v.and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|b| b.get("cache_control").is_some())
                    .count()
            })
            .unwrap_or(0)
    };
    let messages = request_body
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|msgs| msgs.iter().map(|m| blocks(m.get("content"))).sum())
        .unwrap_or(0);
    blocks(request_body.get("tools")) + blocks(request_body.get("system")) + messages
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

/// Canonicalizes an OpenAI / Codex request payload for automatic server-side prefix caching without altering semantics.
///
/// OpenAI's prompt caching automatically caches prefixes >= 1,024 tokens.
/// To maximize cache hits across consecutive turns:
/// 1. Canonicalizes the `tools` array by sorting functions deterministically by name.
/// 2. Ensures the `developer` or `system` message is placed at index 0 of `messages`.
///
/// This does not drop or alter a single token of user instructions, code, comments, or tool parameters.
pub fn optimize_openai_prefix(request_body: &mut Value) {
    // 1. Deterministically sort tools by function name
    if let Some(tools) = request_body.get_mut("tools").and_then(|t| t.as_array_mut()) {
        tools.sort_by(|a, b| {
            let name_a = a
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or_default();
            let name_b = b
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or_default();
            name_a.cmp(name_b)
        });
    }

    // 2. Ensure system/developer message is at index 0 of messages
    if let Some(messages) = request_body
        .get_mut("messages")
        .and_then(|m| m.as_array_mut())
    {
        let mut system_idx = None;
        for (i, msg) in messages.iter().enumerate() {
            if let Some(role) = msg.get("role").and_then(|r| r.as_str()) {
                if role == "system" || role == "developer" {
                    system_idx = Some(i);
                    break;
                }
            }
        }

        if let Some(idx) = system_idx {
            if idx > 0 {
                let sys_msg = messages.remove(idx);
                messages.insert(0, sys_msg);
            }
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
        // ...but the rolling breakpoint sits on the last message's content block
        assert_eq!(msgs[0]["content"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(msgs[0]["content"][0]["text"], "hello");
    }

    #[test]
    fn test_rolling_breakpoint_caches_history_and_skips_thinking() {
        let mut body = json!({
            "system": [{"type": "text", "text": "sys"}],
            "messages": [
                {"role": "user", "content": "q"},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "..."},
                    {"type": "tool_use", "id": "t1", "name": "Bash", "input": {}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "out"},
                    {"type": "text", "text": ""}
                ]}
            ]
        });
        inject_cache_breakpoint(&mut body);
        let last = &body["messages"][2]["content"];
        assert_eq!(
            last[0]["cache_control"]["type"], "ephemeral",
            "skips empty text"
        );
        assert!(last[1].get("cache_control").is_none());
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert!(
            body["messages"][0].get("content").unwrap().is_string(),
            "older turns untouched"
        );
    }

    #[test]
    fn test_client_breakpoints_are_respected() {
        // Client (e.g. Claude Code) already caches the tail; only the system slot is added.
        let mut body = json!({
            "system": "sys",
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "q", "cache_control": {"type": "ephemeral"}}
            ]}]
        });
        inject_cache_breakpoint(&mut body);
        assert_eq!(count_cache_breakpoints(&body), 2);
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn test_inject_cache_breakpoint_respects_anthropic_limit() {
        let marked = json!({"type": "text", "text": "x", "cache_control": {"type": "ephemeral"}});
        let mut body = json!({
            "tools": [{"name": "a", "input_schema": {}, "cache_control": {"type": "ephemeral"}}],
            "system": [{"type": "text", "text": "sys"}],
            "messages": [
                {"role": "user", "content": [marked.clone(), marked.clone(), marked]}
            ]
        });
        inject_cache_breakpoint(&mut body);
        assert!(
            body["system"][0].get("cache_control").is_none(),
            "a 5th breakpoint must not be injected"
        );
    }

    #[test]
    fn test_optimize_openai_prefix_canonicalization() {
        let mut body = json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "user", "content": "Analyze code."},
                {"role": "system", "content": "You are a code assistant."}
            ],
            "tools": [
                {
                    "type": "function",
                    "function": {"name": "write_file", "description": "Write a file"}
                },
                {
                    "type": "function",
                    "function": {"name": "diff_file", "description": "Diff a file"}
                },
                {
                    "type": "function",
                    "function": {"name": "read_file", "description": "Read a file"}
                }
            ]
        });

        optimize_openai_prefix(&mut body);

        // System message must be moved to index 0
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[1]["role"], "user");

        // Tools must be sorted alphabetically by function name
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0]["function"]["name"], "diff_file");
        assert_eq!(tools[1]["function"]["name"], "read_file");
        assert_eq!(tools[2]["function"]["name"], "write_file");
    }
}
