use axum::{body::Body, response::Response};
use futures_util::{stream, StreamExt};
use retoken_core::cache::ResponseCache;
use retoken_core::recorder::FlightRecord;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

use super::cache::build_cache_miss_response;
use crate::provider::{
    extract_token_usage, extract_token_usage_value, is_hop_by_hop_header, Provider, TokenUsage,
};

const MAX_SSE_LINE_BYTES: usize = 1024 * 1024;

/// Parses only complete SSE `data:` lines and retains only token usage.
///
/// The relay never rewrites or withholds data. This small bounded buffer exists
/// solely because an upstream SSE line may be split across network chunks.
#[derive(Debug)]
struct SseUsageExtractor {
    provider: Provider,
    pending: Vec<u8>,
    usage: TokenUsage,
}

impl SseUsageExtractor {
    fn new(provider: Provider) -> Self {
        Self {
            provider,
            pending: Vec::new(),
            usage: TokenUsage::default(),
        }
    }

    fn push(&mut self, chunk: &[u8]) {
        self.pending.extend_from_slice(chunk);

        while let Some(newline) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=newline).collect();
            self.consume_line(&line);
        }

        if self.pending.len() > MAX_SSE_LINE_BYTES {
            // Do not make a malformed upstream stream consume unbounded memory.
            // Discarding parser state cannot affect the byte-for-byte relay.
            warn!("SSE usage line exceeded parser limit; skipping telemetry for that line");
            self.pending.clear();
        }
    }

    fn finish(mut self) -> TokenUsage {
        if !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            self.consume_line(&line);
        }
        self.usage
    }

    fn consume_line(&mut self, line: &[u8]) {
        let Ok(line) = std::str::from_utf8(line) else {
            return;
        };
        let line = line.trim_end_matches(['\r', '\n']);
        let Some(data) = line.strip_prefix("data:") else {
            return;
        };
        let data = data.trim_start();
        if data.is_empty() || data == "[DONE]" {
            return;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
            return;
        };
        merge_usage(
            &mut self.usage,
            extract_token_usage_value(&self.provider, &value),
        );
    }
}

fn merge_usage(target: &mut TokenUsage, update: TokenUsage) {
    if update.input_tokens.is_some() {
        target.input_tokens = update.input_tokens;
    }
    if update.output_tokens.is_some() {
        target.output_tokens = update.output_tokens;
    }
    if update.cached_tokens.is_some() {
        target.cached_tokens = update.cached_tokens;
    }
    if update.reasoning_tokens.is_some() {
        target.reasoning_tokens = update.reasoning_tokens;
    }
}

fn apply_usage(record: &mut FlightRecord, usage: TokenUsage) {
    record.input_tokens = usage.input_tokens;
    record.output_tokens = usage.output_tokens;
    record.cached_tokens = usage.cached_tokens;
    record.reasoning_tokens = usage.reasoning_tokens;
}

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

    // Native streaming relay: preserve protocol, do not buffer/cache response
    // bodies, and inspect terminal usage events without modifying any bytes.
    if effective_stream {
        if let Some(resp) = resp {
            info!(
                "Streaming response natively (is_stream={}, is_sse={}, bypassing cache)",
                is_stream, is_sse
            );
            let provider = detected_provider.clone();
            let telemetry = telemetry.clone();
            let stream_record = record.clone();
            let final_success = status.is_success();
            let stream = stream::unfold(
                (
                    resp.bytes_stream(),
                    Some(SseUsageExtractor::new(provider)),
                    Some(stream_record),
                    telemetry,
                ),
                move |(mut upstream, mut extractor, mut record, telemetry)| async move {
                    let (Some(mut extractor), Some(mut flight)) = (extractor.take(), record.take())
                    else {
                        return None;
                    };

                    match upstream.next().await {
                        Some(Ok(chunk)) => Some((
                            Ok::<_, reqwest::Error>(chunk.clone()),
                            (
                                upstream,
                                Some({
                                    extractor.push(&chunk);
                                    extractor
                                }),
                                Some(flight),
                                telemetry,
                            ),
                        )),
                        Some(Err(error)) => {
                            apply_usage(&mut flight, extractor.finish());
                            flight.errors += 1;
                            flight.final_success = Some(false);
                            telemetry.lock().await.record(&flight);
                            Some((Err(error), (upstream, None, None, telemetry)))
                        }
                        None => {
                            apply_usage(&mut flight, extractor.finish());
                            flight.final_success = Some(final_success);
                            telemetry.lock().await.record(&flight);
                            None
                        }
                    }
                },
            );
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
    apply_usage(record, usage);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_openai_responses_usage_across_split_chunks() {
        let mut extractor = SseUsageExtractor::new(Provider::OpenAI);
        extractor.push(b"event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":120,\"output_tokens\":");
        extractor.push(b"45,\"input_tokens_details\":{\"cached_tokens\":30},\"output_tokens_details\":{\"reasoning_tokens\":11}}}}\n\n");

        assert_eq!(
            extractor.finish(),
            TokenUsage {
                input_tokens: Some(120),
                output_tokens: Some(45),
                cached_tokens: Some(30),
                reasoning_tokens: Some(11),
            }
        );
    }

    #[test]
    fn merges_anthropic_start_and_delta_usage() {
        let mut extractor = SseUsageExtractor::new(Provider::Anthropic);
        extractor.push(b"event: message_start\r\ndata: {\"usage\":{\"input_tokens\":300,\"cache_read_input_tokens\":40}}\r\n\r\n");
        extractor.push(b"event: message_delta\ndata: {\"usage\":{\"output_tokens\":25,\"thinking_tokens\":8}}\n\n");

        assert_eq!(
            extractor.finish(),
            TokenUsage {
                input_tokens: Some(300),
                output_tokens: Some(25),
                cached_tokens: Some(40),
                reasoning_tokens: Some(8),
            }
        );
    }

    #[test]
    fn ignores_invalid_events_and_done_markers() {
        let mut extractor = SseUsageExtractor::new(Provider::OpenAI);
        extractor.push(b"data: not-json\n\ndata: [DONE]\n\n");

        assert_eq!(extractor.finish(), TokenUsage::default());
    }
}
