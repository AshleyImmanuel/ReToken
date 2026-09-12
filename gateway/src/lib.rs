use axum::{
    body::Body,
    extract::{Request, State},
    response::Response,
    routing::any,
    Router,
};
use reqwest::Client;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{info, warn};
use retoken_core::recorder::FlightRecord;
use std::time::Instant;

mod provider;
mod optimizer;

use tokio::sync::Mutex;
use retoken_core::state::{StateEngine, StateType};
use retoken_core::cache::ResponseCache;
use retoken_core::config::AppConfig;

use crate::provider::{detect_provider, extract_token_usage, is_sensitive_header};

struct GatewayState {
    client: Client,
    state_engine: Arc<Mutex<StateEngine>>,
    response_cache: Arc<Mutex<ResponseCache>>,
    config: AppConfig,
}

pub fn app(state_engine: Arc<Mutex<StateEngine>>, config: AppConfig) -> Router {
    let response_cache = Arc::new(Mutex::new(
        ResponseCache::new(config.cache_ttl_secs, config.max_cache_entries)
    ));

    let state = Arc::new(GatewayState {
        client: Client::new(),
        state_engine,
        response_cache,
        config,
    });

    Router::new()
        .route("/*path", any(handler))
        .route("/", any(handler))
        .with_state(state)
}

pub async fn start(config: AppConfig) -> anyhow::Result<()> {
    let state_engine = Arc::new(Mutex::new(StateEngine::new()));
    let app = app(state_engine, config);

    let addr = "127.0.0.1:8888";
    let listener = TcpListener::bind(addr).await?;
    info!("ReToken Gateway listening on {}", addr);
    
    axum::serve(listener, app).await?;
    Ok(())
}

async fn handler(State(state): State<Arc<GatewayState>>, req: Request<Body>) -> Result<Response<Body>, axum::http::StatusCode> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");

    // ── FIX #6: BYPASS MODE ──────────────────────────────────
    // If X-ReToken-Bypass is set, skip all optimization and forward raw.
    if req.headers().get("x-retoken-bypass").is_some() {
        info!("Bypass mode: forwarding raw request");
        return forward_raw(&state.client, method, path_and_query, req).await;
    }

    // ── FIX #7: SECRET REDACTION IN LOGS ─────────────────────
    // Log the request method/path but never log sensitive headers.
    info!("Intercepted: {} {}", method, path_and_query);

    let (parts, body) = req.into_parts();
    
    // Collect headers for provider detection, redacting secrets
    let req_headers: Vec<(String, String)> = parts.headers.iter()
        .map(|(name, value)| {
            (name.to_string(), value.to_str().unwrap_or("").to_string())
        })
        .collect();

    // ── FIX #2: PROVIDER DETECTION ───────────────────────────
    let detected_provider = detect_provider(path_and_query, &req_headers);
    let target_url = format!("{}{}", detected_provider.base_url(), path_and_query);
    info!("Provider: {} -> {}", detected_provider.name(), target_url);

    let mut record = FlightRecord {
        provider: detected_provider.name().to_string(),
        ..Default::default()
    };
    let start_time = Instant::now();

    // Build the outbound request, copying headers (skip Host, redact secrets from logs)
    let mut req_builder = state.client.request(method.clone(), &target_url);
    for (header_name, header_value) in parts.headers.iter() {
        if header_name != axum::http::header::HOST {
            req_builder = req_builder.header(header_name, header_value);
        }
        // FIX #7: Only log non-sensitive headers
        if !is_sensitive_header(header_name.as_str()) {
            info!("  Header: {}: {}", header_name, header_value.to_str().unwrap_or("[binary]"));
        }
    }

    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;

    // Inspect request body
    record.request_size = bytes.len();
    let original_size = bytes.len();

    let mut json_body = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
    
    if let Some(ref jb) = json_body {
        if let Some(model) = jb.get("model").and_then(|m| m.as_str()) {
            record.model = model.to_string();
        }
    }

    // ── FIX #1 + #8 + #10: APPLY OPTIMIZATIONS ──────────────
    // Only mutate the body if we successfully parsed JSON.
    let final_bytes = if let Some(ref mut body_json) = json_body {
        optimizer::optimize_payload(body_json, original_size, &detected_provider, &state.config, &mut record)
    } else {
        bytes.to_vec()
    };

    // Insert into Context State Engine for versioned tracking
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

    // ── EXACT CACHE LOOKUP (PRD Section 21, 23) ──────────────
    if state.config.cache_enabled {
        let cache_key = ResponseCache::cache_key(&final_bytes, repo_hash);
        let mut cache = state.response_cache.lock().await;
        
        if let Some(cached) = cache.get(&cache_key) {
            record.cache_hits = 1;
            record.network_duration_ms = 0;

            // FIX #3: Extract tokens from cached response too
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
                // Mark that this was served from ReToken cache
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

    // ── FORWARD TO PROVIDER ──────────────────────────────────
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

    let resp_bytes = resp.bytes().await
        .map_err(|_| axum::http::StatusCode::BAD_GATEWAY)?;

    // ── FIX #3: EXTRACT TOKEN USAGE FROM RESPONSE ────────────
    let usage = extract_token_usage(&detected_provider, &resp_bytes);
    record.input_tokens = usage.input_tokens;
    record.output_tokens = usage.output_tokens;
    record.cached_tokens = usage.cached_tokens;
    record.final_success = Some(status.is_success());

    info!("Flight Record: {:?}", record);

    // ── CACHE ON MISS ────────────────────────────────────────
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

/// Raw passthrough for bypass mode (FIX #6).
async fn forward_raw(
    client: &Client,
    method: axum::http::Method,
    path_and_query: &str,
    req: Request<Body>,
) -> Result<Response<Body>, axum::http::StatusCode> {
    // Detect provider even in bypass mode so we route correctly
    let headers: Vec<(String, String)> = req.headers().iter()
        .map(|(n, v)| (n.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let provider = detect_provider(path_and_query, &headers);
    let target_url = format!("{}{}", provider.base_url(), path_and_query);

    let (parts, body) = req.into_parts();
    let mut req_builder = client.request(method, &target_url);
    for (header_name, header_value) in parts.headers.iter() {
        if header_name != axum::http::header::HOST {
            req_builder = req_builder.header(header_name, header_value);
        }
    }

    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;
    let reqwest_req = req_builder.body(reqwest::Body::from(bytes)).build()
        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;

    let resp = client.execute(reqwest_req).await.map_err(|e| {
        warn!("Bypass proxy failed: {}", e);
        axum::http::StatusCode::BAD_GATEWAY
    })?;

    let status = resp.status();
    let resp_bytes = resp.bytes().await.map_err(|_| axum::http::StatusCode::BAD_GATEWAY)?;

    let builder = Response::builder().status(status);
    Ok(builder.body(Body::from(resp_bytes)).unwrap())
}
