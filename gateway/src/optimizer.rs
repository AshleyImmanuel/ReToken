use serde_json::Value;
use tracing::info;
use retrieval::packer;
use crate::provider::Provider;
use retoken_core::recorder::FlightRecord;
use retoken_core::config::AppConfig;

/// Applies all structural optimizations to the request payload before forwarding to the provider.
pub fn optimize_payload(
    json_body: &mut Value,
    original_size: usize,
    detected_provider: &Provider,
    config: &AppConfig,
    record: &mut FlightRecord,
) -> Vec<u8> {
    if config.terse_mode {
        packer::inject_terse_mode(json_body);
    }

    // FIX #8 + #1: Reorder messages for provider cache optimization
    // Only apply for Anthropic since their API supports cache_control
    if *detected_provider == Provider::Anthropic {
        packer::reorder_for_cache(json_body);
        packer::inject_cache_breakpoint(json_body);
    }

    // FIX #10: Compress tool schemas in the request
    if let Some(tools) = json_body.get_mut("tools").and_then(|t| t.as_array_mut()) {
        for tool in tools.iter_mut() {
            strip_tool_descriptions(tool);
        }
    }

    // Return the optimized bytes
    let optimized = serde_json::to_vec(json_body).unwrap_or_default();
    let optimized_size = optimized.len();
    
    if optimized_size > 0 && optimized_size < original_size {
        let ratio = 1.0 - (optimized_size as f64 / original_size as f64);
        record.compression_ratio = Some(ratio);
        record.context_reduction = Some(ratio);
        info!("Optimized request: {} -> {} bytes ({:.1}% reduction)", 
            original_size, optimized_size, ratio * 100.0);
    }
    
    optimized
}

/// Recursively strip "description" fields from JSON schema definitions
pub fn strip_tool_descriptions(tool: &mut Value) {
    if let Some(obj) = tool.as_object_mut() {
        obj.remove("description");
        if let Some(input_schema) = obj.get_mut("input_schema") {
            strip_descriptions_recursive(input_schema);
        }
        // OpenAI uses "parameters" instead of "input_schema"
        if let Some(params) = obj.get_mut("parameters") {
            strip_descriptions_recursive(params);
        }
    }
}

fn strip_descriptions_recursive(value: &mut Value) {
    if let Some(obj) = value.as_object_mut() {
        obj.remove("description");
        obj.remove("examples");
        for (_, v) in obj.iter_mut() {
            strip_descriptions_recursive(v);
        }
    }
    if let Some(arr) = value.as_array_mut() {
        for v in arr.iter_mut() {
            strip_descriptions_recursive(v);
        }
    }
}
