use compressors::ccr::CcrStore;
use tracing::{info, warn};

/// Represents a detected CCR retrieval tool invocation from the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CcrInvocation {
    pub tool_use_id: String,
    pub handle: String,
}

/// Detects if an Anthropic response contains any `retoken_retrieve` tool calls.
pub fn find_ccr_tool_calls(resp_json: &serde_json::Value) -> Vec<CcrInvocation> {
    let mut calls = Vec::new();
    if let Some(content) = resp_json.get("content").and_then(|c| c.as_array()) {
        for block in content {
            if block.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                && block.get("name").and_then(|n| n.as_str()) == Some("retoken_retrieve")
            {
                let id = block
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or_default()
                    .to_string();
                let handle = block
                    .get("input")
                    .and_then(|i| i.get("handle"))
                    .and_then(|h| h.as_str())
                    .unwrap_or_default()
                    .to_string();
                if !handle.is_empty() {
                    calls.push(CcrInvocation {
                        tool_use_id: id,
                        handle,
                    });
                }
            }
        }
    }
    calls
}

/// Builds the follow-up tool_result turn for Anthropic to allow the model to reason
/// over the recovered data in a true multi-turn recovery loop.
pub fn build_ccr_tool_result_messages(
    assistant_content: &serde_json::Value,
    calls: &[CcrInvocation],
    ccr_store: &CcrStore,
) -> (serde_json::Value, serde_json::Value) {
    let assistant_msg = serde_json::json!({
        "role": "assistant",
        "content": assistant_content
    });

    let tool_results: Vec<serde_json::Value> = calls
        .iter()
        .map(|call| match ccr_store.retrieve(&call.handle) {
            Ok(Some(data)) => {
                let content_str = String::from_utf8_lossy(&data).to_string();
                serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": call.tool_use_id,
                    "content": content_str
                })
            }
            Ok(None) => serde_json::json!({
                "type": "tool_result",
                "tool_use_id": call.tool_use_id,
                "is_error": true,
                "content": format!("CCR handle not found: {}", call.handle)
            }),
            Err(e) => serde_json::json!({
                "type": "tool_result",
                "tool_use_id": call.tool_use_id,
                "is_error": true,
                "content": format!("CCR retrieval error: {}", e)
            }),
        })
        .collect();

    let user_tool_result_msg = serde_json::json!({
        "role": "user",
        "content": tool_results
    });

    (assistant_msg, user_tool_result_msg)
}

/// Intercepts `retoken_retrieve` tool calls in Anthropic's response and resolves
/// them from the CCR store.
///
/// For Anthropic:
/// When the model calls `retoken_retrieve(handle)`, the proxy intercepts the `tool_use`
/// block, resolves the handle from the CCR store, replaces the tool_use block with a text
/// block containing the recovered data, and updates `stop_reason` to `end_turn` if no other
/// tool calls remain.
///
/// For OpenAI and other unsupported formats:
/// Transparent single-hop response rewriting without a client execution loop is not
/// protocol-compliant. Unsupported formats are explicitly left unchanged.
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
                            // Replace the tool_use block with a text block containing the recovered data
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

    // If we intercepted and resolved tool_use blocks in Anthropic format,
    // update stop_reason if no tool_use blocks remain
    if intercepted {
        if let Some(content) = resp_json.get("content").and_then(|c| c.as_array()) {
            let has_other_tool_use = content
                .iter()
                .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"));

            if !has_other_tool_use {
                if let Some(stop_reason) = resp_json.get_mut("stop_reason") {
                    if stop_reason.as_str() == Some("tool_use") {
                        *stop_reason = serde_json::json!("end_turn");
                    }
                }
            }
        }

        return serde_json::to_vec(&resp_json).unwrap_or_else(|_| resp_bytes.to_vec());
    }

    // For OpenAI or unhandled formats: pass response through unchanged
    resp_bytes.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_store_with_data(handle: &str, data: &[u8]) -> (CcrStore, String) {
        let store = CcrStore::open_in_memory().unwrap();
        let stored_handle = store.store(data, "test").unwrap();
        assert_eq!(stored_handle, handle);
        (store, stored_handle)
    }

    #[test]
    fn test_anthropic_tool_use_intercepted_and_resolved() {
        let handle = CcrStore::handle(b"original secret context");
        let (store, _) = test_store_with_data(&handle, b"original secret context");

        let response = json!({
            "id": "msg_01",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_01",
                    "name": "retoken_retrieve",
                    "input": { "handle": handle }
                }
            ],
            "stop_reason": "tool_use"
        });

        let resp_bytes = serde_json::to_vec(&response).unwrap();
        let intercepted = intercept_ccr_tool_calls(&resp_bytes, &store);

        let parsed: serde_json::Value = serde_json::from_slice(&intercepted).unwrap();
        let content = parsed["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        let text = content[0]["text"].as_str().unwrap();
        assert!(text.contains("[ReToken CCR Recovery]"));
        assert!(text.contains("original secret context"));

        // stop_reason should be updated to end_turn
        assert_eq!(parsed["stop_reason"], "end_turn");
    }

    #[test]
    fn test_anthropic_missing_handle_unchanged() {
        let store = CcrStore::open_in_memory().unwrap();

        let response = json!({
            "id": "msg_01",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_01",
                    "name": "retoken_retrieve",
                    "input": { "handle": "ccr:missing" }
                }
            ],
            "stop_reason": "tool_use"
        });

        let resp_bytes = serde_json::to_vec(&response).unwrap();
        let intercepted = intercept_ccr_tool_calls(&resp_bytes, &store);

        // Missing handle leaves bytes unchanged
        assert_eq!(intercepted, resp_bytes);
    }

    #[test]
    fn test_openai_format_left_unchanged() {
        let store = CcrStore::open_in_memory().unwrap();

        let response = json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "tool_calls": [
                            {
                                "id": "call_1",
                                "type": "function",
                                "function": {
                                    "name": "retoken_retrieve",
                                    "arguments": "{\"handle\":\"ccr:123\"}"
                                }
                            }
                        ]
                    }
                }
            ]
        });

        let resp_bytes = serde_json::to_vec(&response).unwrap();
        let intercepted = intercept_ccr_tool_calls(&resp_bytes, &store);

        // Unsupported OpenAI format is left unchanged
        assert_eq!(intercepted, resp_bytes);
    }

    #[test]
    fn test_find_ccr_tool_calls_and_build_multi_turn() {
        let store = CcrStore::open_in_memory().unwrap();
        let handle = store.store(b"recovered secret logs", "test").unwrap();

        let response = json!({
            "id": "msg_01",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "text",
                    "text": "Retrieving data"
                },
                {
                    "type": "tool_use",
                    "id": "toolu_01",
                    "name": "retoken_retrieve",
                    "input": { "handle": handle }
                }
            ],
            "stop_reason": "tool_use"
        });

        let calls = find_ccr_tool_calls(&response);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_use_id, "toolu_01");
        assert_eq!(calls[0].handle, handle);

        let (assistant_msg, tool_result_msg) =
            build_ccr_tool_result_messages(&response["content"], &calls, &store);

        assert_eq!(assistant_msg["role"], "assistant");
        assert_eq!(tool_result_msg["role"], "user");
        let results = tool_result_msg["content"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["type"], "tool_result");
        assert_eq!(results[0]["tool_use_id"], "toolu_01");
        assert_eq!(results[0]["content"], "recovered secret logs");
    }
}
