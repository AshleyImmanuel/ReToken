use serde_json::Value;
use tracing::debug;

use crate::ccr::CcrStore;
use crate::json::compress_json_heuristic_tracked;
use crate::logs::compress_logs;
use crate::pipeline::CompressionStats;

/// Threshold in bytes for content to be eligible for CCR (store + elide).
pub const CCR_SIZE_THRESHOLD: usize = 2048;

/// Threshold for log-like content detection (line count).
pub const LOG_LINE_THRESHOLD: usize = 15;

/// Walk the messages array and compress individual message content blocks.
pub fn compress_messages(body: &mut Value, ccr_store: &CcrStore, stats: &mut CompressionStats) {
    let messages = match body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        Some(m) => m,
        None => return,
    };

    for msg in messages.iter_mut() {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("").to_string();
        let is_tool_result = role == "tool"
            || msg.get("type").and_then(|t| t.as_str()) == Some("tool_result");

        // Process content field
        if let Some(content) = msg.get_mut("content") {
            compress_content_field(content, &role, ccr_store, stats);

            // Anthropic format: content can be an array of content blocks
            // Handle tool_result content blocks specifically
            if is_tool_result {
                if let Some(arr) = content.as_array_mut() {
                    for block in arr.iter_mut() {
                        if let Some(text) = block.get_mut("text") {
                            compress_text_value(text, ccr_store, stats);
                        }
                    }
                }
            }
        }
    }
}

/// Compress a content field (string or array of content blocks).
pub fn compress_content_field(
    content: &mut Value,
    role: &str,
    ccr_store: &CcrStore,
    stats: &mut CompressionStats,
) {
    match content {
        Value::String(text) => {
            // Don't compress system prompts or short messages
            if role == "system" || text.len() < CCR_SIZE_THRESHOLD {
                return;
            }
            let compressed = compress_text(text, ccr_store, stats);
            *content = Value::String(compressed);
        }
        Value::Array(blocks) => {
            for block in blocks.iter_mut() {
                if let Some(text) = block.get_mut("text") {
                    compress_text_value(text, ccr_store, stats);
                }
            }
        }
        _ => {}
    }
}

/// Compress a text Value in-place.
pub fn compress_text_value(text: &mut Value, ccr_store: &CcrStore, stats: &mut CompressionStats) {
    if let Some(s) = text.as_str() {
        if s.len() < CCR_SIZE_THRESHOLD {
            return;
        }
        let compressed = compress_text(s, ccr_store, stats);
        *text = Value::String(compressed);
    }
}

/// Core text compression logic: detects content type and applies appropriate compressor.
pub fn compress_text(text: &str, ccr_store: &CcrStore, stats: &mut CompressionStats) -> String {
    // 1. AST-Aware Minification: strip comments/blanks from code blocks
    let text = crate::minify::minify_code_blocks(text);

    // 2. Try Contextual Delta Encoding for source code files
    let text = crate::delta::compress_deltas(&text, ccr_store);

    // Try to parse as JSON first
    if let Ok(json_val) = serde_json::from_str::<Value>(&text) {
        if json_val.is_array() || json_val.is_object() {
            return compress_json_text(&json_val, &text, ccr_store, stats);
        }
    }

    // Check if it looks like log output
    let line_count = text.lines().count();
    if line_count >= LOG_LINE_THRESHOLD {
        return compress_log_text(&text, ccr_store, stats);
    }

    // Not compressible
    text.to_string()
}

/// Compress JSON content: apply array collapse, store original in CCR.
pub fn compress_json_text(
    json_val: &Value,
    original_text: &str,
    ccr_store: &CcrStore,
    stats: &mut CompressionStats,
) -> String {
    let (compressed, did_compress) = compress_json_heuristic_tracked(json_val);

    if !did_compress {
        return original_text.to_string();
    }

    // Store original in CCR
    match ccr_store.store(original_text.as_bytes(), "json") {
        Ok(handle) => {
            stats.ccr_entries_stored += 1;
            stats.json_arrays_collapsed += 1;
            debug!("CCR stored JSON original: {}", handle);

            // Append CCR marker to compressed output
            let mut result = serde_json::to_string(&compressed).unwrap_or_else(|_| original_text.to_string());
            result.push_str(&format!("\n<<{}>>", handle));
            result
        }
        Err(e) => {
            debug!("CCR store failed, using compressed without CCR: {}", e);
            stats.json_arrays_collapsed += 1;
            serde_json::to_string(&compressed).unwrap_or_else(|_| original_text.to_string())
        }
    }
}

/// Compress log content: apply line elision, store original in CCR.
pub fn compress_log_text(text: &str, ccr_store: &CcrStore, stats: &mut CompressionStats) -> String {
    let compressed = compress_logs(text);

    if compressed.len() >= text.len() {
        return text.to_string(); // No savings
    }

    // Store original in CCR
    match ccr_store.store(text.as_bytes(), "log") {
        Ok(handle) => {
            stats.ccr_entries_stored += 1;
            stats.log_blocks_compressed += 1;
            debug!("CCR stored log original: {}", handle);

            // Append CCR marker
            format!("{}\n<<{}>>", compressed, handle)
        }
        Err(e) => {
            debug!("CCR store failed, using compressed without CCR: {}", e);
            stats.log_blocks_compressed += 1;
            compressed
        }
    }
}
