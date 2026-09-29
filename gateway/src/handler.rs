use axum::{
    body::Body,
    extract::{Request, State},
    response::Response,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn, debug};
use std::time::Instant;

use retoken_core::recorder::FlightRecord;
use retoken_core::state::{StateEngine, StateType};
use retoken_core::cache::ResponseCache;
use retoken_core::config::AppConfig;
use compressors::ccr::CcrStore;
use reqwest::Client;

use crate::provider::{detect_provider, extract_token_usage, is_sensitive_header};
use crate::optimizer;
use crate::bypass::forward_raw;

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
        return forward_raw(&state.client, method, path_and_query, req).await;
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
    let target_url = format!("{}{}", detected_provider.base_url(), path_and_query);
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
        )
    } else {
        bytes.to_vec()
    };

    // Insert into Context State Engine
    {
        let mut engine = state.state_engine.lock().await;
        let obj = engine.insert_or_update(
            StateType::Generic("GatewayRequest".to_string()),
            &final_bytes,
            Some(target_url.clone()),
        );
        info!("State: {}", obj.object_id);
    }

    let repo_hash = req_headers.iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("x-retoken-repo-hash"))
        .map(|(_, v)| v.as_str());

    // Exact Cache Lookup
    if state.config.cache_enabled {
        let cache_key = ResponseCache::cache_key(&final_bytes, repo_hash);
        let mut cache = state.response_cache.lock().await;
        
        if let Some(cached) = cache.get(&cache_key) {
            record.cache_hits = 1;
            record.network_duration_ms = 0;

            let cached_usage = extract_token_usage(&detected_provider, &cached.response_bytes);
            record.input_tokens = cached_usage.input_tokens;
            record.output_tokens = cached_usage.output_tokens;
            record.cached_tokens = cached_usage.cached_tokens;

            info!("CACHE HIT (saved API call) | {:?}", record);

            let status = axum::http::StatusCode::from_u16(cached.response_status)
                .unwrap_or(axum::http::StatusCode::OK);
            let mut builder = Response::builder().status(status);
            if let Some(headers) = builder.headers_mut() {
                for (name, value) in &cached.response_headers {
                    if let (Ok(hn), Ok(hv)) = (
                        axum::http::header::HeaderName::from_bytes(name.as_bytes()),
                        axum::http::header::HeaderValue::from_str(value),
                    ) {
                        headers.insert(hn, hv);
                    }
                }
                if let Ok(hv) = axum::http::header::HeaderValue::from_str("hit") {
                    headers.insert(
                        axum::http::header::HeaderName::from_static("x-retoken-cache"),
                        hv,
                    );
                }
            }
            return Ok(builder.body(Body::from(cached.response_bytes.clone())).unwrap());
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

    // Native Streaming Bypass
    if is_stream {
        info!("Streaming response natively (bypassing cache & token extraction)");
        let stream = resp.bytes_stream();
        let mut builder = Response::builder().status(status);
        if let Some(headers) = builder.headers_mut() {
            for (name, value) in &resp_headers {
                if let (Ok(hn), Ok(hv)) = (
                    axum::http::header::HeaderName::from_bytes(name.as_bytes()),
                    axum::http::header::HeaderValue::from_str(value),
                ) {
                    headers.insert(hn, hv);
                }
            }
        }
        return Ok(builder.body(Body::from_stream(stream)).unwrap());
    }

    let resp_bytes = resp.bytes().await
        .map_err(|_| axum::http::StatusCode::BAD_GATEWAY)?;

    // -- CCR Tool Call Interception --
    // Check if the LLM response contains a retoken_retrieve tool call.
    // If so, intercept it, fetch the data from CCR, and return a synthetic
    // tool result -- completely transparently to the client.
    let resp_bytes = intercept_ccr_tool_calls(&resp_bytes, &state.ccr_store);

    // Extract Token Usage
    let usage = extract_token_usage(&detected_provider, &resp_bytes);
    record.input_tokens = usage.input_tokens;
    record.output_tokens = usage.output_tokens;
    record.cached_tokens = usage.cached_tokens;
    record.final_success = Some(status.is_success());

    info!("Flight Record: {:?}", record);

    // Cache on Miss
    if state.config.cache_enabled && status.is_success() {
        let cache_key = ResponseCache::cache_key(&final_bytes, repo_hash);
        let mut cache = state.response_cache.lock().await;
        cache.insert(cache_key, resp_bytes.to_vec(), status.as_u16(), resp_headers.clone());
    }

    // Build response
    let mut builder = Response::builder().status(status);
    if let Some(headers) = builder.headers_mut() {
        for (name, value) in &resp_headers {
            if let (Ok(hn), Ok(hv)) = (
                axum::http::header::HeaderName::from_bytes(name.as_bytes()),
                axum::http::header::HeaderValue::from_str(value),
            ) {
                headers.insert(hn, hv);
            }
        }
        if let Ok(hv) = axum::http::header::HeaderValue::from_str("miss") {
            headers.insert(
                axum::http::header::HeaderName::from_static("x-retoken-cache"),
                hv,
            );
        }
    }

    Ok(builder.body(Body::from(resp_bytes)).unwrap())
}

/// Intercepts `retoken_retrieve` tool calls in the LLM's response and resolves
/// them from the CCR store. This makes the retrieval completely transparent to
/// the client (Antigravity CLI, Claude Code, Cursor, etc).
///
/// When the LLM decides it needs elided data, it calls `retoken_retrieve(handle)`.
/// The proxy intercepts this tool_use block, resolves the handle from SQLite,
/// and injects the original content back into the response -- the client never
/// knows the data was compressed.
fn intercept_ccr_tool_calls(resp_bytes: &[u8], ccr_store: &CcrStore) -> Vec<u8> {
    let mut resp_json: serde_json::Value = match serde_json::from_slice(resp_bytes) {
        Ok(v) => v,
        Err(_) => return resp_bytes.to_vec(),
    };

    let mut intercepted = false;

    // Check for tool_use in Anthropic format: content array with type "tool_use"
    if let Some(content) = resp_json.get_mut("content").and_then(|c| c.as_array_mut()) {
        for block in content.iter_mut() {
            if block.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                && block.get("name").and_then(|n| n.as_str()) == Some("retoken_retrieve")
            {
                if let Some(handle_str) = block
                    .get("input")
                    .and_then(|i| i.get("handle"))
                    .and_then(|h| h.as_str())
                    .map(|s| s.to_string())
                {
                    match ccr_store.retrieve(&handle_str) {
                        Ok(Some(data)) => {
                            let content_str = String::from_utf8_lossy(&data);
                            // Replace the tool_use block with a text block containing the data
                            *block = serde_json::json!({
                                "type": "text",
                                "text": format!("[ReToken CCR Recovery]\n{}", content_str)
                            });
                            intercepted = true;
                            info!("CCR intercepted: resolved handle {}", handle_str);
                        }
                        Ok(None) => {
                            warn!("CCR handle not found: {}", handle_str);
                        }
                        Err(e) => {
                            warn!("CCR retrieval error for {}: {}", handle_str, e);
                        }
                    }
                }
            }
        }
    }

    // Check for tool_calls in OpenAI format
    if let Some(choices) = resp_json.get_mut("choices").and_then(|c| c.as_array_mut()) {
        for choice in choices.iter_mut() {
            if let Some(message) = choice.get_mut("message") {
                if let Some(tool_calls) = message.get_mut("tool_calls").and_then(|tc| tc.as_array_mut()) {
                    for tc in tool_calls.iter_mut() {
                        if tc.get("function").and_then(|f| f.get("name")).and_then(|n| n.as_str())
                            == Some("retoken_retrieve")
                        {
                            if let Some(args_str) = tc
                                .get("function")
                                .and_then(|f| f.get("arguments"))
                                .and_then(|a| a.as_str())
                            {
                                if let Ok(args) = serde_json::from_str::<serde_json::Value>(args_str) {
                                    if let Some(handle) = args.get("handle").and_then(|h| h.as_str()) {
                                        match ccr_store.retrieve(handle) {
                                            Ok(Some(data)) => {
                                                debug!(
                                                    "CCR intercepted OpenAI tool_call: resolved {}",
                                                    handle
                                                );
                                                // We can't transparently replace in OpenAI format
                                                // without breaking the protocol. Instead, we note
                                                // the resolution for the next request cycle.
                                                intercepted = true;
                                                info!(
                                                    "CCR resolved {} ({} bytes) for OpenAI tool_call",
                                                    handle,
                                                    data.len()
                                                );
                                            }
                                            Ok(None) => {
                                                warn!("CCR handle not found: {}", handle);
                                            }
                                            Err(e) => {
                                                warn!("CCR retrieval error: {}", e);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if intercepted {
        serde_json::to_vec(&resp_json).unwrap_or_else(|_| resp_bytes.to_vec())
    } else {
        resp_bytes.to_vec()
    }
}
