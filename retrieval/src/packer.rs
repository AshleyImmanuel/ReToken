use serde_json::Value;

/// Reorders the messages array in an outbound API request to maximize
/// provider prompt cache hits.
///
/// Strategy (PRD Section 25):
///   STATIC PREFIX  (stays identical across turns)
///     - system instructions
///     - tool definitions
///     - repository map
///     - stable project metadata
///
///   VARIABLE SUFFIX (changes per turn)
///     - current task context
///     - changed files
///     - latest tool results
///     - user's latest message
///
/// By keeping the prefix stable, Anthropic's prompt caching gives us a
/// 90% discount on the cached portion.
pub fn reorder_for_cache(request_body: &mut Value) {
    // Ensure the messages array exists
    let messages = match request_body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        Some(m) => m,
        None => return, // No messages to reorder
    };

    if messages.len() < 2 {
        return; // Nothing to reorder
    }

    // Partition messages into stable (system-like, tool definitions) and variable (user, assistant)
    let mut system_msgs: Vec<Value> = Vec::new();
    let mut tool_result_msgs: Vec<Value> = Vec::new();
    let mut conversation_msgs: Vec<Value> = Vec::new();

    for msg in messages.drain(..) {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        match role {
            "system" => system_msgs.push(msg),
            "tool" => tool_result_msgs.push(msg),
            _ => conversation_msgs.push(msg), // user, assistant
        }
    }

    // Reconstruct: system first (stable prefix), then conversation, then tool results (variable suffix)
    messages.extend(system_msgs);
    messages.extend(conversation_msgs);
    messages.extend(tool_result_msgs);
}

/// Injects a cache_control breakpoint marker on the last system message
/// to hint to Anthropic where the cached prefix ends.
pub fn inject_cache_breakpoint(request_body: &mut Value) {
    let messages = match request_body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        Some(m) => m,
        None => return,
    };

    // Find the last system message and add cache_control
    let mut last_system_idx: Option<usize> = None;
    for (i, msg) in messages.iter().enumerate() {
        if msg.get("role").and_then(|r| r.as_str()) == Some("system") {
            last_system_idx = Some(i);
        }
    }

    if let Some(idx) = last_system_idx {
        if let Some(msg) = messages.get_mut(idx) {
            if let Some(obj) = msg.as_object_mut() {
                obj.insert(
                    "cache_control".to_string(),
                    serde_json::json!({"type": "ephemeral"}),
                );
            }
        }
    }
}

/// Dynamically injects an ultra-compact brevity instruction to slash output tokens 
/// and reduce model generation latency, acting as a lightweight "Caveman mode".
pub fn inject_terse_mode(request_body: &mut Value) {
    let messages = match request_body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        Some(m) => m,
        None => return,
    };

    let terse_instruction = serde_json::json!({
        "role": "system",
        "content": "Respond with extreme brevity. Omit all pleasantries, filler words, and conversational framing. Output only the direct technical answer, code, or tool call."
    });

    // We can just push it as the first message. `reorder_for_cache` will run after this
    // and naturally group it with other system messages at the top.
    messages.insert(0, terse_instruction);
}
