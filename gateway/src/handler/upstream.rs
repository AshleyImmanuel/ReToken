//! Sending requests to the upstream provider and reading bounded responses.

use axum::http::StatusCode;
use reqwest::header::HeaderMap;
use reqwest::{Client, Method};
use tracing::warn;

use crate::provider::{check_destination_host, is_ssrf_block};

/// Sends a request upstream. Enforces the SSRF policy for IP-literal hosts here; hostnames
/// are checked at connect time by the client's `SsrfResolver`. Blocked destinations map to
/// 403, other transport failures to 502.
pub async fn send(
    client: &Client,
    method: Method,
    url: &str,
    headers: HeaderMap,
    body: Vec<u8>,
    allow_private_upstreams: bool,
) -> Result<reqwest::Response, StatusCode> {
    let request = client
        .request(method, url)
        .headers(headers)
        .body(body)
        .build()
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    if !allow_private_upstreams {
        let host = request.url().host_str().ok_or(StatusCode::BAD_REQUEST)?;
        if let Err(e) = check_destination_host(host) {
            warn!("Outbound destination blocked by SSRF policy: {}", e);
            return Err(StatusCode::FORBIDDEN);
        }
    }

    client.execute(request).await.map_err(|e| {
        if is_ssrf_block(&e) {
            warn!("Outbound destination blocked by SSRF policy: {}", e);
            StatusCode::FORBIDDEN
        } else {
            warn!("Provider request failed: {}", e);
            StatusCode::BAD_GATEWAY
        }
    })
}

/// Response headers as `(name, value)` pairs, skipping values that are not valid UTF-8.
pub fn header_pairs(resp: &reqwest::Response) -> Vec<(String, String)> {
    resp.headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.to_string(), v.to_string()))
        })
        .collect()
}

/// True if the response is a Server-Sent Events stream.
pub fn is_event_stream(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("content-type")
            && v.to_ascii_lowercase().contains("text/event-stream")
    })
}

/// Buffers a response body, failing (502) if the declared or actual size exceeds `max_bytes`.
pub async fn read_limited(
    mut resp: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, StatusCode> {
    if resp
        .content_length()
        .is_some_and(|len| len as usize > max_bytes)
    {
        warn!(
            "Upstream Content-Length ({:?} bytes) exceeds maximum allowed response size ({} bytes)",
            resp.content_length(),
            max_bytes
        );
        return Err(StatusCode::BAD_GATEWAY);
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| {
        warn!("Upstream response read error: {}", e);
        StatusCode::BAD_GATEWAY
    })? {
        if body.len() + chunk.len() > max_bytes {
            warn!(
                "Upstream response body exceeded limit of {} bytes",
                max_bytes
            );
            return Err(StatusCode::BAD_GATEWAY);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
