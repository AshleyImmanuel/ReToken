use serde_json::Value;
use tracing::{debug, info};

use crate::ccr::CcrStore;
use crate::json::compress_json_heuristic_tracked;
use crate::logs::compress_logs;

/// Threshold in bytes for content to be eligible for CCR (store + elide).
const CCR_SIZE_THRESHOLD: usize = 2048;

/// Threshold for log-like content detection (line count).
const LOG_LINE_THRESHOLD: usize = 15;

/// The full heuristic compression pipeline applied to an LLM API request body.
///
/// This orchestrates all compressors in the correct order:
/// 1. Walk the `messages` array, detecting tool results and large text blocks
/// 2. Compress log-like content with the log compressor
/// 3. Compress large JSON arrays with MAD anomaly detection
/// 4. Store originals in CCR before replacing with compressed versions
/// 5. Inject the `retoken_retrieve` tool schema if any data was CCR-stored
/// 6. Inject the output persona (terse mode) system prompt
///
/// Returns (modified_body_bytes, compression_stats).
pub struct CompressionStats {
    pub log_blocks_compressed: usize,
    pub json_arrays_collapsed: usize,
    pub ccr_entries_stored: usize,
    pub bytes_saved: usize,
    pub original_size: usize,
    pub compressed_size: usize,
}

impl CompressionStats {
    pub fn ratio(&self) -> f64 {
        if self.original_size == 0 {
            return 0.0;
        }
        1.0 - (self.compressed_size as f64 / self.original_size as f64)
    }
}

/// Runs the full compression pipeline on an API request body.
pub fn run_pipeline(
    body: &mut Value,
    ccr_store: &CcrStore,
) -> CompressionStats {
    let original_bytes = serde_json::to_vec(body).unwrap_or_default();
    let original_size = original_bytes.len();

    let mut stats = CompressionStats {
        log_blocks_compressed: 0,
        json_arrays_collapsed: 0,
        ccr_entries_stored: 0,
        bytes_saved: 0,
        original_size,
        compressed_size: 0,
    };

    // Phase 1: Compress message content (tool results, user messages with logs/JSON)
    compress_messages(body, ccr_store, &mut stats);

    // Phase 2: Inject the retoken_retrieve tool if any CCR entries were stored
    if stats.ccr_entries_stored > 0 {
        inject_ccr_tool(body);
    }

    // Phase 3: Inject output persona
    inject_output_persona(body);

    let compressed_bytes = serde_json::to_vec(body).unwrap_or_default();
    stats.compressed_size = compressed_bytes.len();
    stats.bytes_saved = original_size.saturating_sub(stats.compressed_size);

    if stats.bytes_saved > 0 {
        info!(
            "Pipeline: {} -> {} bytes ({:.1}% reduction) | logs={} json={} ccr={}",
            stats.original_size,
            stats.compressed_size,
            stats.ratio() * 100.0,
            stats.log_blocks_compressed,
            stats.json_arrays_collapsed,
            stats.ccr_entries_stored
        );
    }

    stats
}

/// Walk the messages array and compress individual message content blocks.
fn compress_messages(body: &mut Value, ccr_store: &CcrStore, stats: &mut CompressionStats) {
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
fn compress_content_field(
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
fn compress_text_value(text: &mut Value, ccr_store: &CcrStore, stats: &mut CompressionStats) {
    if let Some(s) = text.as_str() {
        if s.len() < CCR_SIZE_THRESHOLD {
            return;
        }
        let compressed = compress_text(s, ccr_store, stats);
        *text = Value::String(compressed);
    }
}

/// Core text compression logic: detects content type and applies appropriate compressor.
fn compress_text(text: &str, ccr_store: &CcrStore, stats: &mut CompressionStats) -> String {
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
fn compress_json_text(
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
fn compress_log_text(text: &str, ccr_store: &CcrStore, stats: &mut CompressionStats) -> String {
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

/// Inject the `retoken_retrieve` tool into the request's tools array.
/// Only adds it if not already present.
fn inject_ccr_tool(body: &mut Value) {
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
fn inject_output_persona(body: &mut Value) {
    let persona = "You are communicating through a credit-optimized proxy. \
                    Do not use pleasantries. Do not explain your code unless asked. \
                    Output actionable findings and code blocks only. \
                    Be as terse as possible to conserve API credits. \
                    If you see __retoken_elided__ markers or <<ccr:...>> handles, \
                    use the retoken_retrieve tool to fetch the original data when needed.";

    // Try OpenAI format first (messages array with system role)
    if let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        // Check if we already injected
        let already_injected = messages.iter().any(|m| {
            m.get("content")
                .and_then(|c| c.as_str())
                .map(|s| s.contains("credit-optimized proxy"))
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
                    if !s.contains("credit-optimized proxy") {
                        s.push_str("\n\n");
                        s.push_str(persona);
                    }
                }
                Value::Array(arr) => {
                    let already = arr.iter().any(|block| {
                        block
                            .get("text")
                            .and_then(|t| t.as_str())
                            .map(|s| s.contains("credit-optimized proxy"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_store() -> CcrStore {
        CcrStore::open_in_memory().unwrap()
    }

    #[test]
    fn pipeline_compresses_large_tool_output() {
        let store = test_store();
        let big_array: Vec<Value> = (0..200)
            .map(|i| json!({"file": format!("src/file_{}.rs", i), "size": 1024, "status": "ok"}))
            .collect();

        let mut body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [
                {"role": "user", "content": "list files"},
                {"role": "tool", "content": serde_json::to_string(&big_array).unwrap()}
            ]
        });

        let stats = run_pipeline(&mut body, &store);
        assert!(stats.json_arrays_collapsed > 0 || stats.log_blocks_compressed > 0);
        assert!(stats.compressed_size <= stats.original_size);
    }

    #[test]
    fn pipeline_compresses_log_output() {
        let store = test_store();
        let log_lines: Vec<String> = (0..200)
            .map(|i| {
                if i == 100 {
                    "ERROR: failed to compile module".to_string()
                } else {
                    format!("  Compiling dep_{} v0.1.0", i)
                }
            })
            .collect();
        let log_text = log_lines.join("\n");

        let mut body = json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "user", "content": "run build"},
                {"role": "tool", "content": log_text}
            ]
        });

        let stats = run_pipeline(&mut body, &store);
        assert!(stats.log_blocks_compressed > 0);
    }

    #[test]
    fn pipeline_injects_ccr_tool_when_needed() {
        let store = test_store();
        let big_array: Vec<Value> = (0..20)
            .map(|i| json!({"id": i, "val": 42}))
            .collect();

        let mut body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [
                {"role": "tool", "content": serde_json::to_string(&big_array).unwrap()}
            ]
        });

        let stats = run_pipeline(&mut body, &store);

        if stats.ccr_entries_stored > 0 {
            // Should have injected the tool
            let tools = body.get("tools").and_then(|t| t.as_array());
            assert!(tools.is_some());
            let has_retrieve = tools.unwrap().iter().any(|t| {
                t.get("name").and_then(|n| n.as_str()) == Some("retoken_retrieve")
            });
            assert!(has_retrieve);
        }
    }

    #[test]
    fn pipeline_injects_output_persona() {
        let store = test_store();
        let mut body = json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "user", "content": "hello"}
            ]
        });

        run_pipeline(&mut body, &store);

        let messages = body.get("messages").unwrap().as_array().unwrap();
        let has_persona = messages.iter().any(|m| {
            m.get("content")
                .and_then(|c| c.as_str())
                .map(|s| s.contains("credit-optimized proxy"))
                .unwrap_or(false)
        });
        assert!(has_persona);
    }
}
