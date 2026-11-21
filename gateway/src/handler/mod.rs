//! The proxy request handler. This module only orchestrates; each step lives in its own module:
//! [`request`] reads the client request, [`cache_policy`] decides cacheability,
//! [`upstream`] sends and reads bounded responses, [`ccr_followup`] performs proxy-side
//! retrieval, and [`response`] builds the client response.

pub mod admin;
pub mod cache;
mod cache_policy;
mod ccr_followup;
mod request;
pub mod response;
mod upstream;

use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
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

use crate::bypass::forward_raw;
use crate::optimizer;
use crate::provider::{build_target_url, detect_provider, forwardable_request_headers, WireFormat};

use self::cache::build_cache_hit_response;
use self::request::RequestParams;
use self::response::{CacheStore, UpstreamBody};

pub struct GatewayState {
    pub client: Client,
    pub response_cache: Arc<Mutex<ResponseCache>>,
    pub config: AppConfig,
    pub ccr_store: Arc<CcrStore>,
    pub start_time: std::time::Instant,
    pub telemetry: Arc<Mutex<retoken_core::FlightTelemetry>>,
}

pub async fn handler(
    State(state): State<Arc<GatewayState>>,
    req: Request<Body>,
) -> Result<Response<Body>, StatusCode> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");

    if is_bypass(req.headers()) {
        info!("Bypass mode: forwarding raw request");
        return forward_raw(&state.client, method, path_and_query, req, &state.config).await;
    }
    info!("Intercepted: {} {}", method, path_and_query);

    let (parts, body) = req.into_parts();
    let req_headers = request::header_pairs(&parts.headers);

    // Route
    let provider = detect_provider(
        path_and_query,
        &req_headers,
        state.config.allow_private_upstreams,
        &state.config.allowed_upstream_hosts,
    )
    .map_err(|err| {
        warn!("Provider detection error: {}", err);
        StatusCode::BAD_REQUEST
    })?;
    let format = WireFormat::detect(&provider, path_and_query);
    let upstream_base = provider.base_url(&state.config);
    let target_url = build_target_url(upstream_base, path_and_query, &provider).map_err(|err| {
        warn!("Failed to build target URL: {}", err);
        StatusCode::BAD_REQUEST
    })?;
    info!("Provider: {} -> {}", provider.name(), target_url);
    request::log_headers(&parts.headers);

    let mut record = FlightRecord {
        provider: provider.name().to_string(),
        ..Default::default()
    };
    let start_time = Instant::now();

    // Read and optimize
    let bytes = request::read_body(body, state.config.max_request_body_bytes).await?;
    record.request_size = bytes.len();
    let mut json_body = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
    let params = RequestParams::from_json(json_body.as_ref());
    if let Some(model) = &params.model {
        record.model = model.clone();
    }
    let final_bytes = match json_body.as_mut() {
        Some(body_json) if optimizations_active(&state.config) => optimizer::optimize_payload(
            body_json,
            bytes.len(),
            format,
            &state.config,
            &mut record,
            &state.ccr_store,
            params.stream,
        ),
        _ => bytes.to_vec(),
    };

    // Exact response cache (never for streaming, opted-out, or non-deterministic requests)
    let cacheable = state.config.cache_enabled
        && !params.stream
        && !cache_policy::bypasses_cache(path_and_query, &req_headers, &params);
    let cache_key = cacheable.then(|| {
        let repo_hash = req_headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("x-retoken-repo-hash"))
            .map(|(_, v)| v.as_str());
        ResponseCache::cache_key_with_context(
            method.as_str(),
            path_and_query,
            provider.name(),
            Some(upstream_base),
            &final_bytes,
            repo_hash,
            ResponseCache::compute_context_fingerprint(&req_headers).as_deref(),
        )
    });
    if let Some(key) = &cache_key {
        let mut cache = state.response_cache.lock().await;
        if let Some(cached) = cache.get(key) {
            record.cache_hits = 1;
            state.telemetry.lock().await.record(&record);
            return build_cache_hit_response(
                cached.response_status,
                &cached.response_headers,
                cached.response_bytes.clone(),
                &provider,
                &mut record,
            );
        }
        record.cache_misses = 1;
    }

    // Forward
    let forward_headers = forwardable_request_headers(&parts.headers);
    let resp = upstream::send(
        &state.client,
        method,
        &target_url,
        forward_headers.clone(),
        final_bytes,
        state.config.allow_private_upstreams,
    )
    .await
    .map_err(|status| record_failure(&state, &record, status))?;
    record.network_duration_ms = start_time.elapsed().as_millis() as u64;

    // Proxy-side CCR recovery needs the whole Anthropic-format response.
    let resp_headers = upstream::header_pairs(&resp);
    let proxy_recovery = state.config.ccr_enabled
        && !params.stream
        && !upstream::is_event_stream(&resp_headers)
        && format == WireFormat::Anthropic
        && resp.status().is_success();
    let (status, resp_headers, body) = match json_body.as_ref().filter(|_| proxy_recovery) {
        Some(request_body) => {
            let buffered =
                ccr_followup::resolve(&state, resp, request_body, &target_url, &forward_headers)
                    .await
                    .map_err(|status| record_failure(&state, &record, status))?;
            (
                buffered.status,
                buffered.headers,
                UpstreamBody::Buffered(buffered.body),
            )
        }
        None => (resp.status(), resp_headers, UpstreamBody::Unread(resp)),
    };

    response::process_response(
        status,
        resp_headers,
        body,
        params.stream,
        state.config.max_response_body_bytes,
        &provider,
        &mut record,
        cache_key.map(|key| CacheStore {
            cache: &state.response_cache,
            key,
        }),
        &state.telemetry,
    )
    .await
}

/// Bypass mode: `x-retoken-bypass` is present and not explicitly false/off.
fn is_bypass(headers: &HeaderMap) -> bool {
    headers
        .get("x-retoken-bypass")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v != "false" && v != "0" && v != "off")
}

/// With every optimization disabled, client bytes are forwarded bit-for-bit.
fn optimizations_active(config: &AppConfig) -> bool {
    config.compression_enabled || config.terse_mode || config.prompt_caching_enabled
}

/// Records a failed request in telemetry and returns its status.
fn record_failure(state: &GatewayState, record: &FlightRecord, status: StatusCode) -> StatusCode {
    let mut rec = record.clone();
    rec.errors = 1;
    let telemetry = state.telemetry.clone();
    tokio::spawn(async move {
        telemetry.lock().await.record(&rec);
    });
    status
}
