use compressors::ccr::CcrStore;
use compressors::pipeline;
use retoken_core::config::AppConfig;
use retoken_core::recorder::FlightRecord;
use retrieval::packer;
use serde_json::Value;
use tracing::info;

use crate::provider::Provider;

/// Applies all structural optimizations to the request payload before forwarding to the provider.
///
/// Optimization pipeline:
/// 1. Heuristic compression pipeline (provider-aware, lossless or CCR when supported)
/// 2. Anthropic cache reordering + breakpoint injection (Anthropic only)
pub fn optimize_payload(
    json_body: &mut Value,
    original_size: usize,
    detected_provider: &Provider,
    config: &AppConfig,
    record: &mut FlightRecord,
    ccr_store: &CcrStore,
    is_stream: bool,
) -> Vec<u8> {
    let is_anthropic = *detected_provider == Provider::Anthropic;
    // CCR recovery is currently only implemented and safe for Anthropic non-streaming requests
    let recovery_supported = is_anthropic && !is_stream;

    let persona = if config.terse_mode {
        &config.persona_prompt
    } else {
        ""
    };

    // Phase 1: Provider-aware heuristic compression pipeline
    let pipeline_stats = pipeline::run_pipeline_for_provider(
        json_body,
        ccr_store,
        is_stream,
        persona,
        &config.elision_note,
        is_anthropic,
        recovery_supported,
    );

    // Phase 2: Provider-specific cache optimization (Anthropic prompt caching)
    if is_anthropic {
        packer::reorder_for_cache(json_body);
        packer::inject_cache_breakpoint(json_body);
    }

    // Return the optimized bytes
    let optimized = serde_json::to_vec(json_body).unwrap_or_default();
    let optimized_size = optimized.len();

    if optimized_size > 0 && optimized_size < original_size {
        let ratio = 1.0 - (optimized_size as f64 / original_size as f64);
        record.compression_ratio = Some(ratio);
        record.context_reduction = Some(ratio);
        info!(
            "Optimized request: {} -> {} bytes ({:.1}% reduction) | pipeline: logs={} json={} ccr={}",
            original_size,
            optimized_size,
            ratio * 100.0,
            pipeline_stats.log_blocks_compressed,
            pipeline_stats.json_arrays_collapsed,
            pipeline_stats.ccr_entries_stored
        );
    }

    optimized
}
