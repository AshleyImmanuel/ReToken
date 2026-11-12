use axum::{extract::State, response::Json};
use serde_json::{json, Value};
use std::sync::Arc;

use super::GatewayState;

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
    let uptime_secs = state.start_time.elapsed().as_secs();

    (
        axum::http::StatusCode::OK,
        Json(json!({
            "status": "ok",
            "total_hits": cache.total_hits,
            "total_misses": cache.total_misses,
            "cached_entries": cache.len(),
            "max_cache_entries": cache.max_entries(),
            "cache_enabled": state.config.cache_enabled,
            "port": state.config.port,
            "uptime_secs": uptime_secs
        })),
    )
}
