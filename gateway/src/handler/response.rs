use axum::{body::Body, response::Response};
use futures_util::StreamExt;
use retoken_core::cache::ResponseCache;
use retoken_core::recorder::{FlightRecord, FlightTelemetry};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

use super::cache::build_cache_miss_response;
use super::upstream;
use crate::provider::{extract_token_usage, is_hop_by_hop_header, Provider};
use crate::stream_usage::SseUsageScanner;

/// Records a streamed request's telemetry, including token usage parsed from the SSE events,
/// once the stream is dropped: on normal completion, upstream error, or client disconnect.
struct StreamTelemetryGuard {
    scanner: SseUsageScanner,
    record: FlightRecord,
    telemetry: Arc<Mutex<FlightTelemetry>>,
    failed: bool,
}

impl Drop for StreamTelemetryGuard {
    fn drop(&mut self) {
        let usage = self.scanner.usage();
        let mut rec = std::mem::take(&mut self.record);
        rec.input_tokens = usage.input_tokens;
        rec.output_tokens = usage.output_tokens;
        rec.cached_tokens = usage.cached_tokens;
        rec.reasoning_tokens = usage.reasoning_tokens;
        if self.failed {
            rec.errors = 1;
        }
        let telemetry = self.telemetry.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    telemetry.lock().await.record(&rec);
                });
            }
            Err(_) => {
                if let Ok(mut tel) = telemetry.try_lock() {
                    tel.record(&rec);
                }
            }
        }
    }
}

/// The upstream response body: still unread, or already buffered (proxy-side CCR recovery).
pub enum UpstreamBody {
    Unread(reqwest::Response),
    Buffered(Vec<u8>),
}

/// Where a successful non-streaming response is stored, if it may be cached.
pub struct CacheStore<'a> {
    pub cache: &'a Arc<Mutex<ResponseCache>>,
    pub key: String,
}

/// Turns the upstream response into the client response: streams pass through untouched
/// (token usage is read on the fly); other bodies are buffered within `max_body_bytes`,
/// metered, and cached on success when `cache_store` is given.
#[allow(clippy::too_many_arguments)]
pub async fn process_response(
    status: reqwest::StatusCode,
    resp_headers: Vec<(String, String)>,
    body: UpstreamBody,
    is_stream: bool,
    max_body_bytes: usize,
    provider: &Provider,
    record: &mut FlightRecord,
    cache_store: Option<CacheStore<'_>>,
    telemetry: &Arc<Mutex<FlightTelemetry>>,
) -> Result<Response<Body>, axum::http::StatusCode> {
    // Streaming is decided by the request (`stream: true`) or the response (SSE content type).
    let is_sse = upstream::is_event_stream(&resp_headers);

    if is_stream || is_sse {
        let UpstreamBody::Unread(resp) = body else {
            return Err(axum::http::StatusCode::BAD_GATEWAY);
        };
        info!(
            "Streaming response natively (is_stream={}, is_sse={}, bypassing cache)",
            is_stream, is_sse
        );
        record.final_success = Some(status.is_success());
        let mut guard = StreamTelemetryGuard {
            scanner: SseUsageScanner::new(provider.clone()),
            record: record.clone(),
            telemetry: telemetry.clone(),
            failed: false,
        };
        let stream = resp.bytes_stream().map(move |chunk| {
            match &chunk {
                Ok(bytes) => guard.scanner.feed(bytes),
                Err(_) => guard.failed = true,
            }
            chunk
        });
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

    let resp_bytes = match body {
        UpstreamBody::Buffered(bytes) => bytes,
        UpstreamBody::Unread(resp) => match upstream::read_limited(resp, max_body_bytes).await {
            Ok(bytes) => bytes,
            Err(e) => {
                record.errors = 1;
                telemetry.lock().await.record(record);
                return Err(e);
            }
        },
    };

    let usage = extract_token_usage(provider, &resp_bytes);
    record.input_tokens = usage.input_tokens;
    record.output_tokens = usage.output_tokens;
    record.cached_tokens = usage.cached_tokens;
    record.reasoning_tokens = usage.reasoning_tokens;
    record.final_success = Some(status.is_success());

    info!("Flight Record: {:?}", record);
    telemetry.lock().await.record(record);

    if let Some(store) = cache_store.filter(|_| status.is_success()) {
        store.cache.lock().await.insert(
            store.key,
            resp_bytes.clone(),
            status.as_u16(),
            resp_headers.clone(),
        );
    }

    build_cache_miss_response(status, &resp_headers, resp_bytes)
}
