//! Reading and inspecting the incoming client request.

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use serde_json::Value;
use tracing::{info, warn};

use crate::provider::is_sensitive_header;

/// Request headers as `(name, value)` pairs; non-UTF-8 values become empty strings.
/// Used for detection and cache keys only. Forwarding uses the raw `HeaderMap`.
pub fn header_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_str().unwrap_or("").to_string()))
        .collect()
}

/// Logs request headers, never the values of credential headers.
pub fn log_headers(headers: &HeaderMap) {
    for (name, value) in headers {
        if !is_sensitive_header(name.as_str()) {
            info!(
                "  Header: {}: {}",
                name,
                value.to_str().unwrap_or("[binary]")
            );
        }
    }
}

/// Buffers the request body, rejecting bodies over `max_bytes` with 413.
pub async fn read_body(body: Body, max_bytes: usize) -> Result<axum::body::Bytes, StatusCode> {
    axum::body::to_bytes(body, max_bytes).await.map_err(|e| {
        warn!(
            "Request body exceeded max limit of {} bytes: {}",
            max_bytes, e
        );
        StatusCode::PAYLOAD_TOO_LARGE
    })
}

/// Generation parameters that affect streaming and caching decisions.
#[derive(Debug, Default)]
pub struct RequestParams {
    pub model: Option<String>,
    pub stream: bool,
    pub temperature: Option<f64>,
    /// `n > 1`: several sampled completions.
    pub multiple_completions: bool,
    pub has_seed: bool,
}

impl RequestParams {
    pub fn from_json(body: Option<&Value>) -> Self {
        let Some(body) = body else {
            return Self::default();
        };
        Self {
            model: body
                .get("model")
                .and_then(|m| m.as_str())
                .map(str::to_string),
            stream: body
                .get("stream")
                .and_then(|s| s.as_bool())
                .unwrap_or(false),
            temperature: body.get("temperature").and_then(|t| t.as_f64()),
            multiple_completions: body
                .get("n")
                .and_then(|n| n.as_u64())
                .is_some_and(|n| n > 1),
            has_seed: body.get("seed").is_some(),
        }
    }
}
