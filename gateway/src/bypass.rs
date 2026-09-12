use axum::{
    body::Body,
    extract::Request,
    response::Response,
};
use reqwest::Client;
use tracing::warn;
use crate::provider::detect_provider;

/// Raw passthrough for bypass mode.
pub async fn forward_raw(
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

    // Zero-copy stream request body
    let reqwest_req = req_builder.body(reqwest::Body::wrap_stream(body.into_data_stream())).build()
        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;

    let resp = client.execute(reqwest_req).await.map_err(|e| {
        warn!("Bypass proxy failed: {}", e);
        axum::http::StatusCode::BAD_GATEWAY
    })?;

    let status = resp.status();
    
    // Zero-copy stream response body
    let stream = resp.bytes_stream();
    let builder = Response::builder().status(status);
    Ok(builder.body(Body::from_stream(stream)).unwrap())
}
