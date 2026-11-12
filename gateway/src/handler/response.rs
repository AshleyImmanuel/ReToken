use axum::{body::Body, response::Response};
use retoken_core::cache::ResponseCache;
use retoken_core::recorder::FlightRecord;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

use super::cache::build_cache_miss_response;
use crate::provider::{extract_token_usage, is_hop_by_hop_header, Provider};

#[allow(clippy::too_many_arguments)]
pub async fn process_response(
    method: &str,
    path_and_query: &str,
    status: reqwest::StatusCode,
    resp_headers: Vec<(String, String)>,
    resp: Option<reqwest::Response>,
    prebuffered_bytes: Option<Vec<u8>>,
    is_stream: bool,
    max_response_body_bytes: usize,
    detected_provider: &Provider,
    upstream_base: Option<&str>,
    record: &mut FlightRecord,
    cache_enabled: bool,
    no_cache: bool,
    response_cache: &Arc<Mutex<ResponseCache>>,
    final_bytes: &[u8],
    repo_hash: Option<&str>,
    auth_fingerprint: Option<&str>,
    telemetry: &Arc<Mutex<retoken_core::FlightTelemetry>>,
) -> Result<Response<Body>, axum::http::StatusCode> {
    // Response-Side Streaming Detection: check if request requested streaming OR if response returned SSE
    let is_sse = resp_headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("content-type")
            && v.to_ascii_lowercase().contains("text/event-stream")
    });
    let effective_stream = is_stream || is_sse;

    // Native Streaming Bypass: preserve protocol, do not buffer, do not cache
    if effective_stream {
        if let Some(resp) = resp {
            info!(
                "Streaming response natively (is_stream={}, is_sse={}, bypassing cache & token extraction)",
                is_stream, is_sse
            );
            telemetry.lock().await.record(record);
            let stream = resp.bytes_stream();
            let mut builder = Response::builder().status(status);
            if let Some(headers) = builder.headers_mut() {
                for (name, value) in &resp_headers {
                    if is_hop_by_hop_header(name) || name.eq_ignore_ascii_case("content-length") {
                        continue;
                    }
                    if let (Ok(hn), Ok(hv)) = (
                        axum::http::header::HeaderName::from_bytes(name.as_bytes()),
                        axum::http::header::HeaderValue::from_str(value),
                    ) {
                        headers.insert(hn, hv);
                    }
                }
            }
            return builder
                .body(Body::from_stream(stream))
                .map_err(|_| axum::http::StatusCode::BAD_GATEWAY);
        } else {
            return Err(axum::http::StatusCode::BAD_GATEWAY);
        }
    }

    // Check upstream Content-Length header against max_response_body_bytes
    if let Some((_, cl_str)) = resp_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
    {
        if let Ok(cl) = cl_str.parse::<usize>() {
            if cl > max_response_body_bytes {
                tracing::warn!(
                    "Upstream Content-Length ({} bytes) exceeds maximum allowed response size ({} bytes)",
                    cl,
                    max_response_body_bytes
                );
                record.errors = 1;
                telemetry.lock().await.record(record);
                return Err(axum::http::StatusCode::BAD_GATEWAY);
            }
        }
    }

    // Buffer response body safely with bounded limit (or use prebuffered bytes from multi-turn resolution)
    let resp_bytes = if let Some(bytes) = prebuffered_bytes {
        bytes
    } else if let Some(mut resp) = resp {
        let mut bytes = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|_| axum::http::StatusCode::BAD_GATEWAY)?
        {
            if bytes.len() + chunk.len() > max_response_body_bytes {
                tracing::warn!(
                    "Upstream response body exceeded limit of {} bytes",
                    max_response_body_bytes
                );
                record.errors = 1;
                telemetry.lock().await.record(record);
                return Err(axum::http::StatusCode::BAD_GATEWAY);
            }
            bytes.extend_from_slice(&chunk);
        }
        bytes
    } else {
        return Err(axum::http::StatusCode::BAD_GATEWAY);
    };

    // Extract Token Usage
    let usage = extract_token_usage(detected_provider, &resp_bytes);
    record.input_tokens = usage.input_tokens;
    record.output_tokens = usage.output_tokens;
    record.cached_tokens = usage.cached_tokens;
    record.reasoning_tokens = usage.reasoning_tokens;
    record.final_success = Some(status.is_success());

    info!("Flight Record: {:?}", record);
    telemetry.lock().await.record(record);

    // Cache on Miss: Only cache non-streaming, successful 2xx responses when caching is not bypassed
    if cache_enabled && !no_cache && status.is_success() && !effective_stream {
        let cache_key = ResponseCache::cache_key_with_context(
            method,
            path_and_query,
            detected_provider.name(),
            upstream_base,
            final_bytes,
            repo_hash,
            auth_fingerprint,
        );
        let mut cache = response_cache.lock().await;
        cache.insert(
            cache_key,
            resp_bytes.to_vec(),
            status.as_u16(),
            resp_headers.clone(),
        );
    }

    build_cache_miss_response(status, &resp_headers, resp_bytes.to_vec())
}
