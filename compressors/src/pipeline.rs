use serde_json::Value;
use tracing::info;

use crate::ccr::CcrStore;
use crate::inject::{inject_ccr_tool_for_provider, inject_output_persona_for_provider};
use crate::tool_output::{compress_messages, CompressOptions};

/// Statistics for one run of the compression pipeline.
#[derive(Debug, Default, Clone)]
pub struct CompressionStats {
    pub log_blocks_compressed: usize,
    pub json_arrays_collapsed: usize,
    pub duplicates_removed: usize,
    pub code_outlines: usize,
    /// True if a fresh tool result will be replaced next turn (a full read that becomes an
    /// outline): caching it would pay the cache-write premium for bytes that are discarded.
    pub volatile_tail: bool,
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
        self.bytes_saved as f64 / self.original_size as f64
    }
}

/// Name of the retrieval tool the proxy injects when it resolves retrievals itself.
pub const PROXY_RETRIEVE_TOOL: &str = "retoken_retrieve";

/// How the model can recover content that ReToken elides. Lossy elision is only performed
/// when a route exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovery {
    /// No route: only lossless transforms are applied.
    None,
    /// The proxy injects `retoken_retrieve` and resolves calls itself (non-streaming only).
    Proxy,
    /// The client exposes a retrieval tool (the `retoken mcp` server) under this name.
    /// Works for streaming requests because the agent executes the tool itself.
    ClientTool(String),
}

impl Recovery {
    fn tool_name(&self) -> Option<&str> {
        match self {
            Recovery::None => None,
            Recovery::Proxy => Some(PROXY_RETRIEVE_TOOL),
            Recovery::ClientTool(name) => Some(name),
        }
    }
}

/// Runs the compression pipeline on a request body.
pub fn run_pipeline_for_provider(
    body: &mut Value,
    ccr_store: &CcrStore,
    persona_prompt: &str,
    elision_note: &str,
    is_anthropic: bool,
    recovery: &Recovery,
) -> CompressionStats {
    let original_size = serde_json::to_vec(body).map(|b| b.len()).unwrap_or(0);
    let mut stats = CompressionStats {
        original_size,
        ..Default::default()
    };

    let opts = CompressOptions {
        retrieve_tool: recovery.tool_name(),
        elision_note,
    };
    compress_messages(body, ccr_store, &mut stats, &opts);

    if *recovery == Recovery::Proxy && stats.ccr_entries_stored > 0 {
        inject_ccr_tool_for_provider(body, is_anthropic);
    }

    inject_output_persona_for_provider(body, persona_prompt, is_anthropic);

    stats.compressed_size = serde_json::to_vec(body).map(|b| b.len()).unwrap_or(0);
    stats.bytes_saved = original_size.saturating_sub(stats.compressed_size);

    if stats.bytes_saved > 0 {
        info!(
            "Pipeline: {} -> {} bytes ({:.1}% reduction) | logs={} json={} dups={} outlines={} ccr={}",
            stats.original_size,
            stats.compressed_size,
            stats.ratio() * 100.0,
            stats.log_blocks_compressed,
            stats.json_arrays_collapsed,
            stats.duplicates_removed,
            stats.code_outlines,
            stats.ccr_entries_stored
        );
    }

    stats
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

        let stats = run_pipeline_for_provider(
            &mut body,
            &store,
            "test persona",
            "test elision",
            true,
            &Recovery::Proxy,
        );
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
                {"role": "assistant", "tool_calls": [{"id": "c1", "type": "function",
                    "function": {"name": "shell", "arguments": "{\"command\":\"make\"}"}}]},
                {"role": "tool", "tool_call_id": "c1", "content": log_text}
            ]
        });

        let stats = run_pipeline_for_provider(
            &mut body,
            &store,
            "test persona",
            "test elision",
            true,
            &Recovery::Proxy,
        );
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
            "test persona",
            "test elision",
            false,
            &Recovery::None,
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

        let stats = run_pipeline_for_provider(
            &mut body,
            &store,
            "test persona",
            "test elision",
            true,
            &Recovery::Proxy,
        );

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
            "test persona",
            "test elision",
            false,
            &Recovery::None,
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
