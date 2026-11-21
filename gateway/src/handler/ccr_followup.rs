//! Proxy-side CCR recovery for non-streaming Anthropic-format requests.
//!
//! If the model's response calls `retoken_retrieve`, the proxy resolves the handles from the
//! CCR store and sends one follow-up request carrying the `tool_result` turn, so the model
//! reasons over the recovered content. The client only ever sees the final response.

use axum::http::StatusCode;
use reqwest::header::HeaderMap;
use reqwest::Method;
use serde_json::Value;
use tracing::info;

use super::upstream;
use super::GatewayState;
use crate::interceptor::{build_ccr_tool_result_messages, find_ccr_tool_calls};

/// A fully buffered upstream response.
pub struct BufferedResponse {
    pub status: StatusCode,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Buffers `resp`; if it requests CCR retrievals, replaces it with the follow-up response.
pub async fn resolve(
    state: &GatewayState,
    resp: reqwest::Response,
    request_body: &Value,
    target_url: &str,
    forward_headers: &HeaderMap,
) -> Result<BufferedResponse, StatusCode> {
    let max_bytes = state.config.max_response_body_bytes;
    let status = resp.status();
    let headers = upstream::header_pairs(&resp);
    let body = upstream::read_limited(resp, max_bytes).await?;

    let Ok(resp_json) = serde_json::from_slice::<Value>(&body) else {
        return Ok(BufferedResponse {
            status,
            headers,
            body,
        });
    };
    let calls = find_ccr_tool_calls(&resp_json);
    if calls.is_empty() {
        return Ok(BufferedResponse {
            status,
            headers,
            body,
        });
    }

    let (assistant_msg, tool_result_msg) =
        build_ccr_tool_result_messages(&resp_json["content"], &calls, &state.ccr_store);
    let mut follow_up = request_body.clone();
    if let Some(messages) = follow_up.get_mut("messages").and_then(|m| m.as_array_mut()) {
        messages.push(assistant_msg);
        messages.push(tool_result_msg);
    }

    info!(
        "CCR: Model requested {} handle(s). Executing multi-turn follow-up request to upstream provider...",
        calls.len()
    );
    let resp = upstream::send(
        &state.client,
        Method::POST,
        target_url,
        forward_headers.clone(),
        serde_json::to_vec(&follow_up).unwrap_or_default(),
        state.config.allow_private_upstreams,
    )
    .await?;
    let status = resp.status();
    let headers = upstream::header_pairs(&resp);
    let body = upstream::read_limited(resp, max_bytes).await?;
    Ok(BufferedResponse {
        status,
        headers,
        body,
    })
}
