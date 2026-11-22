use compressors::ccr::CcrStore;
use compressors::pipeline::{self, Recovery};
use retoken_core::config::AppConfig;
use retoken_core::recorder::FlightRecord;
use retrieval::packer;
use serde_json::Value;
use tracing::info;

use crate::provider::WireFormat;

/// Applies all structural optimizations to the request payload before forwarding to the provider.
///
/// Optimization pipeline:
/// 1. Tool-output compression: lossless dedup always; lossy elision only when the model can
///    recover originals (client MCP tool, or proxy-side recovery for non-streaming Anthropic)
/// 2. Prefix cache optimization: Anthropic breakpoints, or OpenAI prefix canonicalization
///
/// Behaviour follows the request's wire format, not the provider: a generic upstream serving
/// the Anthropic Messages API gets Anthropic handling.
pub fn optimize_payload(
    json_body: &mut Value,
    original_size: usize,
    format: WireFormat,
    config: &AppConfig,
    record: &mut FlightRecord,
    ccr_store: &CcrStore,
    is_stream: bool,
) -> Vec<u8> {
    let is_anthropic = format == WireFormat::Anthropic;

    let persona = if config.terse_mode {
        &config.persona_prompt
    } else {
        ""
    };

    // Phase 1: Provider-aware heuristic compression pipeline (only run if compression_enabled)
    let mut pipeline_stats = None;
    if config.compression_enabled {
        // A client-provided retrieval tool (`retoken mcp`) works for streaming requests too,
        // since the agent executes it. Proxy-side recovery needs a buffered Anthropic response.
        let recovery = match compressors::inject::find_client_retrieve_tool(json_body) {
            Some(tool) => Recovery::ClientTool(tool),
            None if is_anthropic && !is_stream && config.ccr_enabled => Recovery::Proxy,
            None => Recovery::None,
        };
        let stats = pipeline::run_pipeline_for_provider(
            json_body,
            ccr_store,
            persona,
            &config.elision_note,
            is_anthropic,
            &recovery,
        );
        if stats.bytes_saved > 0 {
            info!(
                "Compression pipeline saved {} bytes ({:.1}%)",
                stats.bytes_saved,
                stats.ratio() * 100.0
            );
        }
        pipeline_stats = Some(stats);
    } else if config.terse_mode {
        compressors::inject::inject_output_persona_for_provider(json_body, persona, is_anthropic);
    }

    // Phase 2: Format-specific prefix cache optimization
    if config.prompt_caching_enabled {
        match format {
            WireFormat::Anthropic => {
                packer::inject_cache_breakpoint(
                    json_body,
                    pipeline_stats.as_ref().is_some_and(|s| s.volatile_tail),
                );
            }
            WireFormat::OpenAI => {
                packer::optimize_openai_prefix(json_body);
            }
        }
    }

    // Return the optimized bytes
    let optimized = serde_json::to_vec(json_body).unwrap_or_default();
    let optimized_size = optimized.len();

    if optimized_size > 0 && optimized_size < original_size {
        record.bytes_saved = original_size - optimized_size;
        let ratio = 1.0 - (optimized_size as f64 / original_size as f64);
        record.compression_ratio = Some(ratio);
        record.context_reduction = Some(ratio);
        if let Some(stats) = pipeline_stats {
            info!(
                "Optimized request: {} -> {} bytes ({:.1}% reduction) | pipeline: logs={} json={} dups={} ccr={}",
                original_size,
                optimized_size,
                ratio * 100.0,
                stats.log_blocks_compressed,
                stats.json_arrays_collapsed,
                stats.duplicates_removed,
                stats.ccr_entries_stored
            );
        } else {
            info!(
                "Optimized request: {} -> {} bytes ({:.1}% reduction)",
                original_size,
                optimized_size,
                ratio * 100.0
            );
        }
    }

    optimized
}
