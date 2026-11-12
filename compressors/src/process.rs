use serde_json::Value;
use tracing::debug;

use crate::ccr::CcrStore;
use crate::json::compress_json_heuristic_tracked;
use crate::logs::compress_logs;
use crate::pipeline::CompressionStats;

/// Threshold in bytes for content to be eligible for CCR (store + elide).
pub fn ccr_size_threshold() -> usize {
    std::env::var("RETOKEN_CCR_SIZE_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2048)
}

/// Threshold for log-like content detection (line count).
pub fn log_line_threshold() -> usize {
    std::env::var("RETOKEN_LOG_LINE_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15)
}

/// Walk the messages array and compress ONLY clearly identified tool output blocks.
/// User prompts, system messages, and assistant reasoning history are strictly preserved intact.
pub fn compress_messages(
    body: &mut Value,
    ccr_store: &CcrStore,
    stats: &mut CompressionStats,
    elision_note: &str,
) {
    let messages = match body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        Some(m) => m,
        None => return,
    };

    for msg in messages.iter_mut() {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");

        // 1. OpenAI tool result message: role == "tool"
        if role == "tool" {
            if let Some(content) = msg.get_mut("content") {
                compress_tool_content(content, ccr_store, stats, elision_note);
            }
        } else if let Some(content_array) = msg.get_mut("content").and_then(|c| c.as_array_mut()) {
            // 2. Anthropic tool result blocks inside messages: block type == "tool_result"
            for block in content_array.iter_mut() {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                    if let Some(inner) = block.get_mut("content") {
                        compress_tool_content(inner, ccr_store, stats, elision_note);
                    }
                }
            }
        }
        // User messages (role == "user"), assistant messages (role == "assistant"),
        // and system messages (role == "system") are NEVER touched.
    }
}

/// Compress tool content (string or array of text blocks).
pub fn compress_tool_content(
    content: &mut Value,
    ccr_store: &CcrStore,
    stats: &mut CompressionStats,
    elision_note: &str,
) {
    match content {
        Value::String(text) => {
            if text.len() < ccr_size_threshold() {
                return;
            }
            let compressed = compress_text(text, ccr_store, stats, elision_note);
            *content = Value::String(compressed);
        }
        Value::Array(blocks) => {
            for block in blocks.iter_mut() {
                if let Some(text_val) = block.get_mut("text") {
                    if let Some(s) = text_val.as_str() {
                        if s.len() >= ccr_size_threshold() {
                            let compressed = compress_text(s, ccr_store, stats, elision_note);
                            *text_val = Value::String(compressed);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Core text compression logic: detects content type and applies appropriate compressor.
pub fn compress_text(
    text: &str,
    ccr_store: &CcrStore,
    stats: &mut CompressionStats,
    elision_note: &str,
) -> String {
    let text = crate::minify::minify_code_blocks(text);

    let text = crate::delta::compress_deltas(&text, ccr_store);

    if let Ok(json_val) = serde_json::from_str::<Value>(&text) {
        if json_val.is_array() || json_val.is_object() {
            return compress_json_text(&json_val, &text, ccr_store, stats, elision_note);
        }
    }

    let line_count = text.lines().count();
    if line_count >= log_line_threshold() {
        return compress_log_text(&text, ccr_store, stats);
    }

    text.to_string()
}

/// Compress JSON content: apply array collapse, store original in CCR.
pub fn compress_json_text(
    json_val: &Value,
    original_text: &str,
    ccr_store: &CcrStore,
    stats: &mut CompressionStats,
    elision_note: &str,
) -> String {
    let (compressed, did_compress) = compress_json_heuristic_tracked(json_val, elision_note);

    if !did_compress {
        return original_text.to_string();
    }

    match ccr_store.store(original_text.as_bytes(), "json") {
        Ok(handle) => {
            stats.ccr_entries_stored += 1;
            stats.json_arrays_collapsed += 1;
            debug!("CCR stored JSON original: {}", handle);

            let mut result =
                serde_json::to_string(&compressed).unwrap_or_else(|_| original_text.to_string());
            result.push_str(&format!("\n<<{}>>", handle));
            result
        }
        Err(e) => {
            tracing::warn!(
                "CCR store failed, passing original content through unchanged: {}",
                e
            );
            original_text.to_string()
        }
    }
}

/// Compress log content: apply line elision, store original in CCR.
pub fn compress_log_text(text: &str, ccr_store: &CcrStore, stats: &mut CompressionStats) -> String {
    let compressed = compress_logs(text);

    if compressed.len() >= text.len() {
        return text.to_string(); // No savings
    }

    match ccr_store.store(text.as_bytes(), "log") {
        Ok(handle) => {
            stats.ccr_entries_stored += 1;
            stats.log_blocks_compressed += 1;
            debug!("CCR stored log original: {}", handle);

            format!("{}\n<<{}>>", compressed, handle)
        }
        Err(e) => {
            tracing::warn!(
                "CCR store failed, passing original log through unchanged: {}",
                e
            );
            text.to_string()
        }
    }
}
