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

    let persona = if config.terse_mode {
        &config.persona_prompt
    } else {
        ""
    };

    // Phase 1: Provider-aware heuristic compression pipeline (only run if compression_enabled)
    let mut pipeline_stats = None;
    if config.compression_enabled {
        // CCR handle elision requires both ccr_enabled and supported provider
        let recovery_supported = is_anthropic && !is_stream && config.ccr_enabled;
        let stats = pipeline::run_pipeline_for_provider(
            json_body,
            ccr_store,
            is_stream,
            persona,
            &config.elision_note,
            is_anthropic,
            recovery_supported,
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
        if let Some(stats) = pipeline_stats {
            info!(
                "Optimized request: {} -> {} bytes ({:.1}% reduction) | pipeline: logs={} json={} ccr={}",
                original_size,
                optimized_size,
                ratio * 100.0,
                stats.log_blocks_compressed,
                stats.json_arrays_collapsed,
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
