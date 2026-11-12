pub mod admin;
pub mod cache;
pub mod response;
pub mod state;

use axum::{
    body::Body,
    extract::{Request, State},
    response::Response,
};
use compressors::ccr::CcrStore;
use reqwest::Client;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::{info, warn};

use retoken_core::cache::ResponseCache;
use retoken_core::config::AppConfig;
use retoken_core::recorder::FlightRecord;
use retoken_core::state::StateEngine;

use crate::bypass::forward_raw;
use crate::optimizer;
use crate::provider::{
    build_target_url, detect_provider, is_hop_by_hop_header, is_internal_header,
    is_sensitive_header,
};

use self::cache::build_cache_hit_response;
use self::state::insert_into_state_engine;

pub struct GatewayState {
    pub client: Client,
    pub state_engine: Arc<Mutex<StateEngine>>,
    pub response_cache: Arc<Mutex<ResponseCache>>,
    pub config: AppConfig,
    pub ccr_store: Arc<CcrStore>,
    pub start_time: std::time::Instant,
    pub telemetry: Arc<Mutex<retoken_core::FlightTelemetry>>,
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
    let req_headers: Vec<(String, String)> = parts
        .headers
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_str().unwrap_or("").to_string()))
        .collect();

    let detected_provider = match detect_provider(
        path_and_query,
        &req_headers,
        state.config.allow_private_upstreams,
        &state.config.allowed_upstream_hosts,
    ) {
        Ok(p) => p,
        Err(err) => {
            warn!("Provider detection error: {}", err);
            return Err(axum::http::StatusCode::BAD_REQUEST);
        }
    };

    let target_url = match build_target_url(
        detected_provider.base_url(&state.config),
        path_and_query,
        &detected_provider,
    ) {
        Ok(url) => url,
        Err(err) => {
            warn!("Failed to build target URL: {}", err);
            return Err(axum::http::StatusCode::BAD_REQUEST);
        }
    };

    info!("Provider: {} -> {}", detected_provider.name(), target_url);

    let mut record = FlightRecord {
        provider: detected_provider.name().to_string(),
        ..Default::default()
    };
    let start_time = Instant::now();

    // Build outbound request
    let mut req_builder = state.client.request(method.clone(), &target_url);
    for (header_name, header_value) in parts.headers.iter() {
        let name_str = header_name.as_str();
        if header_name != axum::http::header::HOST
            && header_name != axum::http::header::CONTENT_LENGTH
            && !is_hop_by_hop_header(name_str)
            && !is_internal_header(name_str)
        {
            req_builder = req_builder.header(header_name, header_value);
        }
        if !is_sensitive_header(name_str) {
            info!(
                "  Header: {}: {}",
                header_name,
                header_value.to_str().unwrap_or("[binary]")
            );
        }
    }

    let max_bytes = state.config.max_request_body_bytes;
    let bytes = match axum::body::to_bytes(body, max_bytes).await {
        Ok(b) => b,
        Err(e) => {
            warn!(
                "Request body exceeded max limit of {} bytes: {}",
                max_bytes, e
            );
            return Err(axum::http::StatusCode::PAYLOAD_TOO_LARGE);
        }
    };

    record.request_size = bytes.len();
    let original_size = bytes.len();

    let mut json_body = serde_json::from_slice::<serde_json::Value>(&bytes).ok();

    let mut is_stream = false;
    let mut temperature: Option<f64> = None;
    let mut multiple_completions = false;
    let mut has_seed = false;

    if let Some(ref jb) = json_body {
        if let Some(model) = jb.get("model").and_then(|m| m.as_str()) {
            record.model = model.to_string();
        }
        if let Some(temp) = jb.get("temperature").and_then(|t| t.as_f64()) {
            temperature = Some(temp);
        }
        if let Some(stream_val) = jb.get("stream").and_then(|s| s.as_bool()) {
            is_stream = stream_val;
        }
        if let Some(n) = jb.get("n").and_then(|n| n.as_u64()) {
            if n > 1 {
                multiple_completions = true;
            }
        }
        if jb.get("seed").is_some() {
            has_seed = true;
        }
    }

    // Apply Optimizations
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

    let repo_hash = req_headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("x-retoken-repo-hash"))
        .map(|(_, v)| v.as_str());

    // Extract auth and parameter fingerprint for strict credential and context isolation
    let auth_fingerprint = ResponseCache::compute_context_fingerprint(&req_headers);

    // Check for cache opt-out / no-cache directives
    let header_no_cache = req_headers.iter().any(|(n, v)| {
        (n.eq_ignore_ascii_case("cache-control")
            && (v.contains("no-cache") || v.contains("no-store")))
            || (n.eq_ignore_ascii_case("pragma") && v.contains("no-cache"))
            || (n.eq_ignore_ascii_case("x-retoken-cache")
                && (v == "false" || v == "0" || v == "off"))
            || (n.eq_ignore_ascii_case("x-retoken-no-cache") && (v == "true" || v == "1"))
    });

    // LLM generation endpoints (/messages, /chat/completions) require explicit opt-in to cache
    // to prevent returning stale model generations when temperature is omitted or implicit sampling occurs
    let is_generation = is_generation_endpoint(path_and_query);
    let allow_generation_cache = req_headers.iter().any(|(n, v)| {
        (n.eq_ignore_ascii_case("x-retoken-cache") && (v == "true" || v == "1"))
            || (n.eq_ignore_ascii_case("x-retoken-cache-exact") && (v == "true" || v == "1"))
            || (n.eq_ignore_ascii_case("x-retoken-cache-stochastic") && (v == "true" || v == "1"))
    });
    let generation_bypassed = is_generation && !allow_generation_cache;

    // A generation request is non-deterministic (stochastic) if:
    // 1. temperature is omitted (LLMs default to stochastic sampling, e.g. 1.0) or > 0.0, AND seed is not provided.
    // 2. n > 1 (multiple random completions).
    let is_stochastic = if is_generation {
        if multiple_completions {
            true
        } else {
            !(has_seed || temperature == Some(0.0))
        }
    } else {
        temperature.is_some_and(|t| t > 0.0)
    };

    let allow_stochastic_cache = req_headers.iter().any(|(n, v)| {
        n.eq_ignore_ascii_case("x-retoken-cache-stochastic") && (v == "true" || v == "1")
    });
    let stochastic_bypassed = (is_stochastic && !allow_stochastic_cache) || multiple_completions;
    let no_cache = header_no_cache || generation_bypassed || stochastic_bypassed;

    let upstream_base = detected_provider.base_url(&state.config);

    // Exact Cache Lookup (NEVER check or return cache for streaming, generation-bypassed, stochastic, or no-cache requests)
    if state.config.cache_enabled && !is_stream && !no_cache {
        let cache_key = ResponseCache::cache_key_with_context(
            method.as_str(),
            path_and_query,
            detected_provider.name(),
            Some(upstream_base),
            &final_bytes,
            repo_hash,
            auth_fingerprint.as_deref(),
        );
        let mut cache = state.response_cache.lock().await;

        if let Some(cached) = cache.get(&cache_key) {
            record.cache_hits = 1;
            state.telemetry.lock().await.record(&record);
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
    let reqwest_req = req_builder
        .body(reqwest_body)
        .build()
        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;

    // Pre-flight SSRF check on outgoing request URL destination
    if !state.config.allow_private_upstreams {
        if let Some(host) = reqwest_req.url().host_str() {
            if crate::provider::is_private_or_loopback_host(host) {
                warn!(
                    "Outbound request destination '{}' blocked by SSRF policy",
                    host
                );
                record.errors = 1;
                state.telemetry.lock().await.record(&record);
                return Err(axum::http::StatusCode::FORBIDDEN);
            }
        }
    }

    let resp = state.client.execute(reqwest_req).await.map_err(|e| {
        warn!("Provider request failed: {}", e);
        record.errors = 1;
        let tel = state.telemetry.clone();
        let rec = record.clone();
        tokio::spawn(async move {
            tel.lock().await.record(&rec);
        });
        axum::http::StatusCode::BAD_GATEWAY
    })?;

    record.network_duration_ms = start_time.elapsed().as_millis() as u64;

    let status = resp.status();
    let resp_headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.to_string(), v.to_string()))
        })
        .collect();

    response::process_response(
        method.as_str(),
        path_and_query,
        status,
        resp_headers,
        resp,
        is_stream,
        &detected_provider,
        Some(upstream_base),
        &mut record,
        &state.ccr_store,
        state.config.cache_enabled,
        no_cache,
        &state.response_cache,
        &final_bytes,
        repo_hash,
        auth_fingerprint.as_deref(),
        &state.telemetry,
    )
    .await
}

/// Returns true if the request path targets an LLM completion/generation endpoint.
pub fn is_generation_endpoint(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("/messages")
        || lower.contains("/chat/completions")
        || lower.contains("/completions")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_generation_endpoint() {
        assert!(is_generation_endpoint("/v1/messages"));
        assert!(is_generation_endpoint("/messages"));
        assert!(is_generation_endpoint("/v1/chat/completions"));
        assert!(is_generation_endpoint("/chat/completions"));
        assert!(is_generation_endpoint("/v1/completions"));
        assert!(!is_generation_endpoint("/v1/models"));
        assert!(!is_generation_endpoint("/v1/embeddings"));
        assert!(!is_generation_endpoint("/_retoken/stats"));
    }
}
