use serde_json::Value;
use tracing::info;
use retrieval::packer;
use crate::provider::Provider;
use retoken_core::recorder::FlightRecord;
use retoken_core::config::AppConfig;
use compressors::ccr::CcrStore;
use compressors::pipeline;

/// Applies all structural optimizations to the request payload before forwarding to the provider.
///
/// Optimization pipeline (in order):
/// 1. Heuristic compression pipeline (log compressor, JSON array collapse with MAD, CCR storage,
///    retoken_retrieve tool injection, output persona injection)
/// 2. Terse mode system prompt injection (if enabled and not already injected by pipeline)
/// 3. Anthropic cache reordering + breakpoint injection
/// 4. Tool schema description stripping
pub fn optimize_payload(
    json_body: &mut Value,
    original_size: usize,
    detected_provider: &Provider,
    config: &AppConfig,
    record: &mut FlightRecord,
    ccr_store: &CcrStore,
    is_stream: bool,
) -> Vec<u8> {
    // Phase 1: Heuristic compression pipeline
    // This handles: log compression, JSON array collapse, CCR storage,
    // retoken_retrieve tool injection, and output persona injection
    let pipeline_stats = pipeline::run_pipeline(
        json_body,
        ccr_store,
        is_stream,
        &config.persona_prompt,
        &config.elision_note,
    );

    // Phase 2: Terse mode (if enabled and the pipeline's persona wasn't enough)
    if config.terse_mode {
        packer::inject_terse_mode(json_body);
    }

    // Phase 3: Reorder messages for provider cache optimization
    // Only apply for Anthropic since their API supports cache_control
    if *detected_provider == Provider::Anthropic {
        packer::reorder_for_cache(json_body);
        packer::inject_cache_breakpoint(json_body);
    }

    // Phase 4 removed: We cannot strip tool descriptions, as this breaks LLM function calling by removing the context required for the LLM to understand how to use tools.

    // Return the optimized bytes
    let optimized = serde_json::to_vec(json_body).unwrap_or_default();
    let optimized_size = optimized.len();
    
    if optimized_size > 0 && optimized_size < original_size {
        let ratio = 1.0 - (optimized_size as f64 / original_size as f64);
        record.compression_ratio = Some(ratio);
        record.context_reduction = Some(ratio);
        info!(
            "Optimized request: {} -> {} bytes ({:.1}% reduction) | pipeline: logs={} json={} ccr={}",
            original_size, optimized_size, ratio * 100.0,
            pipeline_stats.log_blocks_compressed,
            pipeline_stats.json_arrays_collapsed,
            pipeline_stats.ccr_entries_stored
        );
    }
    
    optimized
}

