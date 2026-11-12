use axum::{body::Body, extract::Request, response::Response};
use reqwest::Client;
use retoken_core::config::AppConfig;
use tracing::warn;

use crate::provider::{
    build_target_url, detect_provider, is_hop_by_hop_header, is_internal_header,
};

/// Raw passthrough for bypass mode, streaming request and response preserving upstream headers.
pub async fn forward_raw(
    client: &Client,
    method: axum::http::Method,
    path_and_query: &str,
    req: Request<Body>,
    config: &AppConfig,
) -> Result<Response<Body>, axum::http::StatusCode> {
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .map(|(n, v)| (n.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    let provider = detect_provider(path_and_query, &headers)
        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;

    let target_url = build_target_url(provider.base_url(config), path_and_query, &provider)
        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;

    let (parts, body) = req.into_parts();
    let mut req_builder = client.request(method, &target_url);

    for (header_name, header_value) in parts.headers.iter() {
        let name_str = header_name.as_str();
        if header_name != axum::http::header::HOST
            && header_name != axum::http::header::CONTENT_LENGTH
            && !is_hop_by_hop_header(name_str)
            && !is_internal_header(name_str)
        {
            req_builder = req_builder.header(header_name, header_value);
        }
    }

    // Stream request body
    let reqwest_req = req_builder
        .body(reqwest::Body::wrap_stream(body.into_data_stream()))
        .build()
        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;

    let resp = client.execute(reqwest_req).await.map_err(|e| {
        warn!("Bypass proxy failed: {}", e);
        axum::http::StatusCode::BAD_GATEWAY
    })?;

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

    // Stream response body preserving upstream response headers
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

    builder
        .body(Body::from_stream(stream))
        .map_err(|_| axum::http::StatusCode::BAD_GATEWAY)
}
