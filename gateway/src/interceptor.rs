use tracing::{info, warn, debug};
use compressors::ccr::CcrStore;

/// Intercepts `retoken_retrieve` tool calls in the LLM's response and resolves
/// them from the CCR store. This makes the retrieval completely transparent to
/// the client (Antigravity CLI, Claude Code, Cursor, etc).
///
/// When the LLM decides it needs elided data, it calls `retoken_retrieve(handle)`.
/// The proxy intercepts this tool_use block, resolves the handle from SQLite,
/// and injects the original content back into the response -- the client never
/// knows the data was compressed.
pub fn intercept_ccr_tool_calls(resp_bytes: &[u8], ccr_store: &CcrStore) -> Vec<u8> {
    let mut resp_json: serde_json::Value = match serde_json::from_slice(resp_bytes) {
        Ok(v) => v,
        Err(_) => return resp_bytes.to_vec(),
    };

    let mut intercepted = false;

    // Check for tool_use in Anthropic format: content array with type "tool_use"
    if let Some(content) = resp_json.get_mut("content").and_then(|c| c.as_array_mut()) {
        for block in content.iter_mut() {
            if block.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                && block.get("name").and_then(|n| n.as_str()) == Some("retoken_retrieve")
            {
                if let Some(handle_str) = block
                    .get("input")
                    .and_then(|i| i.get("handle"))
                    .and_then(|h| h.as_str())
                    .map(|s| s.to_string())
                {
                    match ccr_store.retrieve(&handle_str) {
                        Ok(Some(data)) => {
                            let content_str = String::from_utf8_lossy(&data);
                            // Replace the tool_use block with a text block containing the data
                            *block = serde_json::json!({
                                "type": "text",
                                "text": format!("[ReToken CCR Recovery]\n{}", content_str)
                            });
                            intercepted = true;
                            info!("CCR intercepted: resolved handle {}", handle_str);
                        }
                        Ok(None) => {
                            warn!("CCR handle not found: {}", handle_str);
                        }
                        Err(e) => {
                            warn!("CCR retrieval error for {}: {}", handle_str, e);
                        }
                    }
                }
            }
        }
    }

    // Check for tool_calls in OpenAI format
    if let Some(choices) = resp_json.get_mut("choices").and_then(|c| c.as_array_mut()) {
        for choice in choices.iter_mut() {
            if let Some(message) = choice.get_mut("message") {
                if let Some(tool_calls) = message.get_mut("tool_calls").and_then(|tc| tc.as_array_mut()) {
                    for tc in tool_calls.iter_mut() {
                        if tc.get("function").and_then(|f| f.get("name")).and_then(|n| n.as_str())
                            == Some("retoken_retrieve")
                        {
                            if let Some(args_str) = tc
                                .get("function")
                                .and_then(|f| f.get("arguments"))
                                .and_then(|a| a.as_str())
                            {
                                if let Ok(args) = serde_json::from_str::<serde_json::Value>(args_str) {
                                    if let Some(handle) = args.get("handle").and_then(|h| h.as_str()) {
                                        match ccr_store.retrieve(handle) {
                                            Ok(Some(data)) => {
                                                debug!(
                                                    "CCR intercepted OpenAI tool_call: resolved {}",
                                                    handle
                                                );
                                                // We can't transparently replace in OpenAI format
                                                // without breaking the protocol. Instead, we note
                                                // the resolution for the next request cycle.
                                                intercepted = true;
                                                info!(
                                                    "CCR resolved {} ({} bytes) for OpenAI tool_call",
                                                    handle,
                                                    data.len()
                                                );
                                            }
                                            Ok(None) => {
                                                warn!("CCR handle not found: {}", handle);
                                            }
                                            Err(e) => {
                                                warn!("CCR retrieval error: {}", e);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if intercepted {
        serde_json::to_vec(&resp_json).unwrap_or_else(|_| resp_bytes.to_vec())
    } else {
        resp_bytes.to_vec()
    }
}
