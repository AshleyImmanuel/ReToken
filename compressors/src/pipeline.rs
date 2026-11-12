use serde_json::Value;
use tracing::info;

use crate::ccr::CcrStore;
use crate::inject::{inject_ccr_tool_for_provider, inject_output_persona_for_provider};
use crate::process::compress_messages;

/// The full heuristic compression pipeline applied to an LLM API request body.
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

/// Runs the full compression pipeline on an API request body for a specific provider.
pub fn run_pipeline_for_provider(
    body: &mut Value,
    ccr_store: &CcrStore,
    is_stream: bool,
    persona_prompt: &str,
    elision_note: &str,
    is_anthropic: bool,
    recovery_supported: bool,
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

    // If the request is streaming, or if transparent recovery is not supported for this provider,
    // we MUST NOT perform lossy CCR elisions or inject fictitious retrieval tools.
    if !is_stream && recovery_supported {
        compress_messages(body, ccr_store, &mut stats, elision_note);

        if stats.ccr_entries_stored > 0 {
            inject_ccr_tool_for_provider(body, is_anthropic);
        }
    }

    inject_output_persona_for_provider(body, persona_prompt, is_anthropic);

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

/// Runs the default compression pipeline (assuming Anthropic format and recovery supported).
pub fn run_pipeline(
    body: &mut Value,
    ccr_store: &CcrStore,
    is_stream: bool,
    persona_prompt: &str,
    elision_note: &str,
) -> CompressionStats {
    run_pipeline_for_provider(
        body,
        ccr_store,
        is_stream,
        persona_prompt,
        elision_note,
        true,
        true,
    )
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

        let stats = run_pipeline(&mut body, &store, false, "test persona", "test elision");
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

        let stats = run_pipeline(&mut body, &store, false, "test persona", "test elision");
        assert!(stats.log_blocks_compressed > 0);
    }

    #[test]
    fn pipeline_skips_ccr_when_recovery_unsupported() {
        let store = test_store();
        let big_array: Vec<Value> = (0..200)
            .map(|i| json!({"file": format!("src/file_{}.rs", i), "size": 1024, "status": "ok"}))
            .collect();

        let raw_tool_content = serde_json::to_string(&big_array).unwrap();
        let mut body = json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "user", "content": "list files"},
                {"role": "tool", "content": raw_tool_content.clone()}
            ]
        });

        // Provider is OpenAI (is_anthropic: false, recovery_supported: false)
        let stats = run_pipeline_for_provider(
            &mut body,
            &store,
            false,
            "test persona",
            "test elision",
            false,
            false,
        );

        assert_eq!(stats.ccr_entries_stored, 0);
        // Tools array must NOT have been created or injected with fake retrieval tool
        assert!(body.get("tools").is_none());
        // Tool content should remain intact
        let msgs = body.get("messages").unwrap().as_array().unwrap();
        let tool_msg = msgs
            .iter()
            .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("tool"))
            .unwrap();
        assert_eq!(tool_msg["content"].as_str().unwrap(), raw_tool_content);
    }

    #[test]
    fn pipeline_injects_ccr_tool_when_needed() {
        let store = test_store();
        let big_array: Vec<Value> = (0..20).map(|i| json!({"id": i, "val": 42})).collect();

        let mut body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [
                {"role": "tool", "content": serde_json::to_string(&big_array).unwrap()}
            ]
        });

        let stats = run_pipeline(&mut body, &store, false, "test persona", "test elision");

        if stats.ccr_entries_stored > 0 {
            let tools = body.get("tools").and_then(|t| t.as_array());
            assert!(tools.is_some());
            let has_retrieve = tools
                .unwrap()
                .iter()
                .any(|t| t.get("name").and_then(|n| n.as_str()) == Some("retoken_retrieve"));
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

        run_pipeline_for_provider(
            &mut body,
            &store,
            false,
            "test persona",
            "test elision",
            false,
            false,
        );

        let messages = body.get("messages").unwrap().as_array().unwrap();
        let has_persona = messages.iter().any(|m| {
            m.get("content")
                .and_then(|c| c.as_str())
                .map(|s| s.contains("test persona"))
                .unwrap_or(false)
        });
        assert!(has_persona);
    }
}
