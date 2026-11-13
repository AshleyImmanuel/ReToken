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

use crate::block_reuse;
use crate::bypass::forward_raw;
use crate::optimizer;
use crate::provider::{
    build_target_url, detect_provider, is_hop_by_hop_header, is_internal_header,
    is_sensitive_header, Provider,
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

    // Bypass mode: active when x-retoken-bypass header is present and not explicitly false/off
    if req
        .headers()
        .get("x-retoken-bypass")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v != "false" && v != "0" && v != "off")
    {
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

    // Apply Optimizations: when all optimizations are disabled, preserve raw client bytes bit-for-bit
    // The Responses API carries structured tool and item payloads. Its upstream prompt
    // caching works without rewriting the body, so preserve it byte-for-byte here.
    let is_openai_responses =
        detected_provider == Provider::OpenAI && is_openai_responses_endpoint(path_and_query);
    let mut block_reuse_applied = false;
    if state.config.block_reuse_enabled {
        if let Some(ref mut body_json) = json_body {
            let stats = block_reuse::reuse_exact_tool_outputs(body_json);
            if stats.reused_blocks > 0 {
                block_reuse_applied = true;
                let reduced_size = original_size.saturating_sub(stats.bytes_saved);
                if original_size > 0 {
                    let ratio = 1.0 - (reduced_size as f64 / original_size as f64);
                    record.context_reduction = Some(ratio);
                    record.compression_ratio = Some(ratio);
                }
                record.block_reuse_blocks = stats.reused_blocks;
                record.block_reuse_bytes_saved = stats.bytes_saved;
                info!(
                    "Exact block reuse saved {} bytes across {} duplicate tool outputs",
                    stats.bytes_saved, stats.reused_blocks
                );
            }
        }
    }

    let optimizations_active = block_reuse_applied
        || (!is_openai_responses
            && (state.config.compression_enabled
                || state.config.terse_mode
                || ((detected_provider == Provider::Anthropic
                    || detected_provider == Provider::OpenAI)
                    && state.config.prompt_caching_enabled)));

    // Responses payloads remain byte-for-byte pass-through unless an exact
    // duplicate block was actually folded. In that case, serialize only the
    // already-validated block-reuse mutation; do not run unrelated prefix
    // canonicalisation over the Responses protocol.
    let final_bytes = if block_reuse_applied && is_openai_responses {
        json_body
            .as_ref()
            .and_then(|body_json| serde_json::to_vec(body_json).ok())
            .unwrap_or_else(|| bytes.to_vec())
    } else if optimizations_active {
        if let Some(ref mut body_json) = json_body {
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
        }
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

    // LLM generation endpoints (/messages, /chat/completions, /responses) require explicit opt-in to cache
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

    // Connect-time SSRF verification & IP pinning
    let client_to_use = if !state.config.allow_private_upstreams {
        let dest_url = reqwest_req.url();
        let host = dest_url
            .host_str()
            .ok_or(axum::http::StatusCode::BAD_REQUEST)?;
        let port = dest_url
            .port_or_known_default()
            .unwrap_or(if dest_url.scheme() == "https" {
                443
            } else {
                80
            });

        match crate::provider::resolve_and_verify_destination(
            host,
            port,
            state.config.allow_private_upstreams,
        ) {
            Ok(Some(verified_addr)) => {
                // Pin client to the verified address to prevent TOCTOU DNS rebinding
                reqwest::Client::builder()
                    .resolve(host, verified_addr)
                    .timeout(std::time::Duration::from_secs(
                        state.config.upstream_timeout_secs,
                    ))
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?
            }
            Ok(None) => state.client.clone(),
            Err(e) => {
                warn!(
                    "Outbound connection destination blocked by SSRF policy: {}",
                    e
                );
                record.errors = 1;
                state.telemetry.lock().await.record(&record);
                return Err(axum::http::StatusCode::FORBIDDEN);
            }
        }
    } else {
        state.client.clone()
    };

    let resp = client_to_use.execute(reqwest_req).await.map_err(|e| {
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

    let mut status = resp.status();
    let mut resp_headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.to_string(), v.to_string()))
        })
        .collect();

    let is_sse = resp_headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("content-type")
            && v.to_ascii_lowercase().contains("text/event-stream")
    });

    // Multi-Turn CCR Tool Recovery Loop:
    // If ccr_enabled is active, the request is non-streaming, Anthropic was detected,
    // and the provider responded successfully, check if the model invoked `retoken_retrieve`.
    // If so, do NOT rewrite the assistant's response to fake text. Instead, send a follow-up
    // request with the tool_result turn so the model actually reasons over the recovered context.
    let (resp_opt, prebuffered_body) = if state.config.ccr_enabled
        && !is_stream
        && !is_sse
        && detected_provider == Provider::Anthropic
        && status.is_success()
    {
        // Validate Content-Length if present before buffering
        if let Some((_, len_str)) = resp_headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        {
            if let Ok(content_len) = len_str.parse::<usize>() {
                if content_len > state.config.max_response_body_bytes {
                    warn!(
                        "Upstream CCR response Content-Length {} exceeds limit of {} bytes",
                        content_len, state.config.max_response_body_bytes
                    );
                    record.errors = 1;
                    let tel = state.telemetry.clone();
                    let rec = record.clone();
                    tokio::spawn(async move {
                        tel.lock().await.record(&rec);
                    });
                    return Err(axum::http::StatusCode::BAD_GATEWAY);
                }
            }
        }

        let mut initial_body = Vec::new();
        let mut resp_stream = resp;
        while let Some(chunk_res) = resp_stream.chunk().await.map_err(|e| {
            warn!("Upstream response read error during CCR processing: {}", e);
            record.errors = 1;
            let tel = state.telemetry.clone();
            let rec = record.clone();
            tokio::spawn(async move {
                tel.lock().await.record(&rec);
            });
            axum::http::StatusCode::BAD_GATEWAY
        })? {
            if initial_body.len() + chunk_res.len() > state.config.max_response_body_bytes {
                warn!(
                    "Upstream response body exceeded limit of {} bytes during CCR processing",
                    state.config.max_response_body_bytes
                );
                record.errors = 1;
                let tel = state.telemetry.clone();
                let rec = record.clone();
                tokio::spawn(async move {
                    tel.lock().await.record(&rec);
                });
                return Err(axum::http::StatusCode::BAD_GATEWAY);
            }
            initial_body.extend_from_slice(&chunk_res);
        }

        if let Ok(resp_json) = serde_json::from_slice::<serde_json::Value>(&initial_body) {
            let calls = crate::interceptor::find_ccr_tool_calls(&resp_json);
            if !calls.is_empty() && json_body.is_some() {
                let mut req_body_val = json_body.clone().unwrap();
                let (assistant_msg, tool_result_msg) =
                    crate::interceptor::build_ccr_tool_result_messages(
                        &resp_json["content"],
                        &calls,
                        &state.ccr_store,
                    );

                if let Some(messages) = req_body_val
                    .get_mut("messages")
                    .and_then(|m| m.as_array_mut())
                {
                    messages.push(assistant_msg);
                    messages.push(tool_result_msg);
                }

                let mut follow_up_builder = client_to_use.post(target_url.clone());
                for (name, val) in &req_headers {
                    let name_lower = name.to_lowercase();
                    if !is_hop_by_hop_header(&name_lower)
                        && name_lower != "content-length"
                        && name_lower != "host"
                        && !is_internal_header(&name_lower)
                    {
                        follow_up_builder = follow_up_builder.header(name, val);
                    }
                }
                let follow_up_bytes = serde_json::to_vec(&req_body_val).unwrap_or_default();
                follow_up_builder = follow_up_builder.body(follow_up_bytes);

                info!(
                    "CCR: Model requested {} handle(s). Executing multi-turn follow-up request to upstream provider...",
                    calls.len()
                );

                match follow_up_builder.send().await {
                    Ok(follow_up_resp) => {
                        status = follow_up_resp.status();
                        resp_headers = follow_up_resp
                            .headers()
                            .iter()
                            .filter_map(|(name, value)| {
                                value
                                    .to_str()
                                    .ok()
                                    .map(|v| (name.to_string(), v.to_string()))
                            })
                            .collect();

                        // Validate follow-up Content-Length if present
                        if let Some((_, len_str)) = resp_headers
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                        {
                            if let Ok(content_len) = len_str.parse::<usize>() {
                                if content_len > state.config.max_response_body_bytes {
                                    warn!(
                                        "CCR follow-up Content-Length {} exceeds limit of {} bytes",
                                        content_len, state.config.max_response_body_bytes
                                    );
                                    record.errors = 1;
                                    let tel = state.telemetry.clone();
                                    let rec = record.clone();
                                    tokio::spawn(async move {
                                        tel.lock().await.record(&rec);
                                    });
                                    return Err(axum::http::StatusCode::BAD_GATEWAY);
                                }
                            }
                        }

                        let mut follow_up_body = Vec::new();
                        let mut fu_stream = follow_up_resp;
                        while let Some(chunk) = fu_stream.chunk().await.map_err(|e| {
                            warn!("CCR follow-up response read error: {}", e);
                            record.errors = 1;
                            let tel = state.telemetry.clone();
                            let rec = record.clone();
                            tokio::spawn(async move {
                                tel.lock().await.record(&rec);
                            });
                            axum::http::StatusCode::BAD_GATEWAY
                        })? {
                            if follow_up_body.len() + chunk.len()
                                > state.config.max_response_body_bytes
                            {
                                warn!(
                                    "CCR follow-up response body exceeded limit of {} bytes",
                                    state.config.max_response_body_bytes
                                );
                                record.errors = 1;
                                let tel = state.telemetry.clone();
                                let rec = record.clone();
                                tokio::spawn(async move {
                                    tel.lock().await.record(&rec);
                                });
                                return Err(axum::http::StatusCode::BAD_GATEWAY);
                            }
                            follow_up_body.extend_from_slice(&chunk);
                        }
                        (None, Some(follow_up_body))
                    }
                    Err(e) => {
                        warn!("CCR follow-up request failed: {}", e);
                        record.errors = 1;
                        let tel = state.telemetry.clone();
                        let rec = record.clone();
                        tokio::spawn(async move {
                            tel.lock().await.record(&rec);
                        });
                        return Err(axum::http::StatusCode::BAD_GATEWAY);
                    }
                }
            } else {
                (None, Some(initial_body))
            }
        } else {
            (None, Some(initial_body))
        }
    } else {
        (Some(resp), None)
    };

    response::process_response(
        method.as_str(),
        path_and_query,
        status,
        resp_headers,
        resp_opt,
        prebuffered_body,
        is_stream,
        state.config.max_response_body_bytes,
        &detected_provider,
        Some(upstream_base),
        &mut record,
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
    let path = path.split('?').next().unwrap_or(path).trim_end_matches('/');
    matches!(
        path.to_ascii_lowercase().as_str(),
        "/v1/messages"
            | "/messages"
            | "/v1/chat/completions"
            | "/chat/completions"
            | "/v1/completions"
            | "/completions"
            | "/v1/responses"
            | "/responses"
    )
}

fn is_openai_responses_endpoint(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path).trim_end_matches('/');
    matches!(
        path.to_ascii_lowercase().as_str(),
        "/v1/responses" | "/responses"
    )
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
        assert!(is_generation_endpoint("/v1/responses"));
        assert!(is_generation_endpoint("/v1/responses?stream=true"));
        assert!(!is_generation_endpoint("/v1/models"));
        assert!(!is_generation_endpoint("/v1/responses/input_tokens"));
        assert!(!is_generation_endpoint("/v1/embeddings"));
        assert!(!is_generation_endpoint("/_retoken/stats"));
    }
}
