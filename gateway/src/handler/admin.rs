use axum::{
    extract::{Query, State},
    response::Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use super::GatewayState;

#[derive(Debug, Deserialize)]
pub struct InvalidateQuery {
    pub key: Option<String>,
}

/// Administrative handler: returns active cached entries and metadata.
pub async fn admin_cache_list(
    State(state): State<Arc<GatewayState>>,
) -> (axum::http::StatusCode, Json<Value>) {
    let cache = state.response_cache.lock().await;
    let entries = cache.list_entries();

    (
        axum::http::StatusCode::OK,
        Json(json!({
            "status": "ok",
            "count": entries.len(),
            "max_capacity": cache.max_entries(),
            "max_bytes": cache.max_bytes(),
            "current_bytes": cache.current_bytes(),
            "entries": entries,
        })),
    )
}

/// Administrative handler: invalidates a single entry from the cache.
pub async fn admin_cache_invalidate(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<InvalidateQuery>,
) -> (axum::http::StatusCode, Json<Value>) {
    if let Some(key) = query.key {
        let mut cache = state.response_cache.lock().await;
        cache.invalidate(&key);
        (
            axum::http::StatusCode::OK,
            Json(json!({
                "status": "ok",
                "message": format!("Entry '{}' invalidated", key),
                "key": key
            })),
        )
    } else {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({
                "status": "error",
                "message": "Missing 'key' query parameter"
            })),
        )
    }
}

/// Administrative handler: clears all in-memory response cache entries.
pub async fn admin_cache_clear(
    State(state): State<Arc<GatewayState>>,
) -> (axum::http::StatusCode, Json<Value>) {
    let mut cache = state.response_cache.lock().await;
    let cleared_count = cache.len();
    cache.clear();

    (
        axum::http::StatusCode::OK,
        Json(json!({
            "status": "ok",
            "message": "Response cache cleared successfully",
            "cleared_entries": cleared_count
        })),
    )
}

/// Administrative handler: returns live runtime metrics and cache status.
pub async fn admin_stats(
    State(state): State<Arc<GatewayState>>,
) -> (axum::http::StatusCode, Json<Value>) {
    let cache = state.response_cache.lock().await;
    let tel = state.telemetry.lock().await;
    let uptime_secs = state.start_time.elapsed().as_secs();

    (
        axum::http::StatusCode::OK,
        Json(json!({
            "status": "ok",
            "uptime_secs": uptime_secs,
            "port": state.config.port,
            "cache_enabled": state.config.cache_enabled,
            "compression_enabled": state.config.compression_enabled,
            "ccr_enabled": state.config.ccr_enabled,
            "prompt_caching_enabled": state.config.prompt_caching_enabled,
            "terse_mode": state.config.terse_mode,
            "cached_entries": cache.len(),
            "max_cache_entries": cache.max_entries(),
            "max_cache_bytes": cache.max_bytes(),
            "current_cache_bytes": cache.current_bytes(),
            "total_hits": cache.total_hits,
            "total_misses": cache.total_misses,
            "total_requests": tel.total_requests,
            "total_errors": tel.total_errors,
            "total_bytes_transferred": tel.total_bytes_transferred,
            "total_input_tokens": tel.total_input_tokens,
            "total_output_tokens": tel.total_output_tokens,
            "total_cached_tokens": tel.total_cached_tokens,
            "total_reasoning_tokens": tel.total_reasoning_tokens,
        })),
    )
}
