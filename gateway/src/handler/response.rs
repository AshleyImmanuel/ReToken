use axum::{body::Body, response::Response};
use compressors::ccr::CcrStore;
use retoken_core::cache::ResponseCache;
use retoken_core::recorder::FlightRecord;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

use super::cache::build_cache_miss_response;
use crate::interceptor::intercept_ccr_tool_calls;
use crate::provider::{extract_token_usage, is_hop_by_hop_header, Provider};

#[allow(clippy::too_many_arguments)]
pub async fn process_response(
    method: &str,
    path_and_query: &str,
    status: reqwest::StatusCode,
    resp_headers: Vec<(String, String)>,
    resp: reqwest::Response,
    is_stream: bool,
    detected_provider: &Provider,
    record: &mut FlightRecord,
    ccr_store: &Arc<CcrStore>,
    cache_enabled: bool,
    no_cache: bool,
    response_cache: &Arc<Mutex<ResponseCache>>,
    final_bytes: &[u8],
    repo_hash: Option<&str>,
    auth_fingerprint: Option<&str>,
) -> Result<Response<Body>, axum::http::StatusCode> {
    // Native Streaming Bypass: preserve protocol, do not buffer, do not cache
    if is_stream {
        info!("Streaming response natively (bypassing cache & token extraction)");
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
    }

    let resp_bytes = resp
        .bytes()
        .await
        .map_err(|_| axum::http::StatusCode::BAD_GATEWAY)?;

    // CCR Tool Call Interception (only transforms supported formats, leaves unsupported unchanged)
    let resp_bytes = intercept_ccr_tool_calls(&resp_bytes, ccr_store);

    // Extract Token Usage
    let usage = extract_token_usage(detected_provider, &resp_bytes);
    record.input_tokens = usage.input_tokens;
    record.output_tokens = usage.output_tokens;
    record.cached_tokens = usage.cached_tokens;
    record.final_success = Some(status.is_success());

    info!("Flight Record: {:?}", record);

    // Cache on Miss: Only cache non-streaming, successful 2xx responses when caching is not bypassed
    if cache_enabled && !no_cache && status.is_success() && !is_stream {
        let cache_key = ResponseCache::cache_key_with_context(
            method,
            path_and_query,
            detected_provider.name(),
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
