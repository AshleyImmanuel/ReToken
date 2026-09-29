pub mod cache;
pub mod state;
pub mod response;

use axum::{
    body::Body,
    extract::{Request, State},
    response::Response,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};
use std::time::Instant;

use retoken_core::recorder::FlightRecord;
use retoken_core::state::StateEngine;
use retoken_core::cache::ResponseCache;
use retoken_core::config::AppConfig;
use compressors::ccr::CcrStore;
use reqwest::Client;

use crate::provider::{detect_provider, is_sensitive_header};
use crate::bypass::forward_raw;
use crate::optimizer;


use self::cache::build_cache_hit_response;
use self::state::insert_into_state_engine;

pub struct GatewayState {
    pub client: Client,
    pub state_engine: Arc<Mutex<StateEngine>>,
    pub response_cache: Arc<Mutex<ResponseCache>>,
    pub config: AppConfig,
    pub ccr_store: Arc<CcrStore>,
}

pub async fn handler(
    State(state): State<Arc<GatewayState>>,
    req: Request<Body>,
) -> Result<Response<Body>, axum::http::StatusCode> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");

    // Bypass mode
    if req.headers().get("x-retoken-bypass").is_some() {
        info!("Bypass mode: forwarding raw request");
        return forward_raw(&state.client, method, path_and_query, req, &state.config).await;
    }

    info!("Intercepted: {} {}", method, path_and_query);

    let (parts, body) = req.into_parts();
    
    // Collect headers
    let req_headers: Vec<(String, String)> = parts.headers.iter()
        .map(|(name, value)| {
            (name.to_string(), value.to_str().unwrap_or("").to_string())
        })
        .collect();

    let detected_provider = detect_provider(path_and_query, &req_headers);
    let target_url = format!("{}{}", detected_provider.base_url(&state.config), path_and_query);
    info!("Provider: {} -> {}", detected_provider.name(), target_url);

    let mut record = FlightRecord {
        provider: detected_provider.name().to_string(),
        ..Default::default()
    };
    let start_time = Instant::now();

    // Build outbound request
    let mut req_builder = state.client.request(method.clone(), &target_url);
    for (header_name, header_value) in parts.headers.iter() {
        if header_name != axum::http::header::HOST {
            req_builder = req_builder.header(header_name, header_value);
        }
        if !is_sensitive_header(header_name.as_str()) {
            info!("  Header: {}: {}", header_name, header_value.to_str().unwrap_or("[binary]"));
        }
    }

    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;

    record.request_size = bytes.len();
    let original_size = bytes.len();

    let mut json_body = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
    
    let mut is_stream = false;
    if let Some(ref jb) = json_body {
        if let Some(model) = jb.get("model").and_then(|m| m.as_str()) {
            record.model = model.to_string();
        }
        if let Some(stream_val) = jb.get("stream").and_then(|s| s.as_bool()) {
            is_stream = stream_val;
        }
    }

    // Apply Optimizations (now with heuristic pipeline)
    let final_bytes = if let Some(ref mut body_json) = json_body {
        optimizer::optimize_payload(
            body_json,
            original_size,
            &detected_provider,
            &state.config,
            &mut record,
            &state.ccr_store,
            is_stream,
        )
    } else {
        bytes.to_vec()
    };

    // Insert into Context State Engine
    insert_into_state_engine(&state.state_engine, &final_bytes, target_url.clone()).await;

    let repo_hash = req_headers.iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("x-retoken-repo-hash"))
        .map(|(_, v)| v.as_str());

    // Exact Cache Lookup
    if state.config.cache_enabled {
        let cache_key = ResponseCache::cache_key(&final_bytes, repo_hash);
        let mut cache = state.response_cache.lock().await;
        
        if let Some(cached) = cache.get(&cache_key) {
            return build_cache_hit_response(
                cached.response_status,
                &cached.response_headers,
                cached.response_bytes.clone(),
                &detected_provider,
                &mut record,
            );
        }
        record.cache_misses = 1;
    }

    // Forward to Provider
    let reqwest_body = reqwest::Body::from(final_bytes.clone());
    let reqwest_req = req_builder.body(reqwest_body).build()
        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;

    let resp = state.client.execute(reqwest_req).await.map_err(|e| {
        warn!("Provider request failed: {}", e);
        record.errors = 1;
        axum::http::StatusCode::BAD_GATEWAY
    })?;

    record.network_duration_ms = start_time.elapsed().as_millis() as u64;

    let status = resp.status();
    let resp_headers: Vec<(String, String)> = resp.headers().iter()
        .filter_map(|(name, value)| {
            value.to_str().ok().map(|v| (name.to_string(), v.to_string()))
        })
        .collect();

    response::process_response(
        status,
        resp_headers,
        resp,
        is_stream,
        &detected_provider,
        &mut record,
        &state.ccr_store,
        state.config.cache_enabled,
        &state.response_cache,
        &final_bytes,
        repo_hash,
    ).await
}
