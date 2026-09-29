use axum::response::Response;
use axum::body::Body;
use retoken_core::recorder::FlightRecord;
use crate::provider::{extract_token_usage, Provider};

/// Construct an axum Response for a cache hit.
pub fn build_cache_hit_response(
    cached_status: u16,
    cached_headers: &[(String, String)],
    cached_bytes: Vec<u8>,
    provider: &Provider,
    record: &mut FlightRecord,
) -> Result<Response<Body>, axum::http::StatusCode> {
    record.cache_hits = 1;
    record.network_duration_ms = 0;

    let cached_usage = extract_token_usage(provider, &cached_bytes);
    record.input_tokens = cached_usage.input_tokens;
    record.output_tokens = cached_usage.output_tokens;
    record.cached_tokens = cached_usage.cached_tokens;

    tracing::info!("CACHE HIT (saved API call) | {:?}", record);

    let status = axum::http::StatusCode::from_u16(cached_status)
        .unwrap_or(axum::http::StatusCode::OK);
    
    let mut builder = Response::builder().status(status);
    if let Some(headers) = builder.headers_mut() {
        for (name, value) in cached_headers {
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
    Ok(builder.body(Body::from(cached_bytes)).unwrap())
}

/// Construct an axum Response for a cache miss.
pub fn build_cache_miss_response(
    status: axum::http::StatusCode,
    resp_headers: &[(String, String)],
    resp_bytes: Vec<u8>,
) -> Result<Response<Body>, axum::http::StatusCode> {
    let mut builder = Response::builder().status(status);
    if let Some(headers) = builder.headers_mut() {
        for (name, value) in resp_headers {
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
