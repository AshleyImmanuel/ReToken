use axum::{
    body::Body,
    extract::Request,
    response::Response,
    routing::{get, post},
    Router,
};
use http_body_util::BodyExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tower::ServiceExt;

use compressors::ccr::CcrStore;
use retoken_core::config::AppConfig;
use retoken_core::state::StateEngine;

/// Starts a mock upstream server on an ephemeral port.
async fn start_mock_upstream() -> (String, Arc<AtomicUsize>) {
    let call_count = Arc::new(AtomicUsize::new(0));
    let count_clone = call_count.clone();

    let app = Router::new()
        .route(
            "/v1/messages",
            post(move |req: Request<Body>| {
                let count = count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    let body_bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
                        .await
                        .unwrap();
                    let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

                    // Echo back inspection headers or validation
                    let resp_body = serde_json::json!({
                        "id": "msg_mock",
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "text", "text": "mock anthropic response"}],
                        "model": "claude-3-opus",
                        "stop_reason": "end_turn",
                        "usage": {
                            "input_tokens": 10,
                            "output_tokens": 5,
                            "cache_read_input_tokens": 2
                        },
                        "echoed_received_system": json.get("system"),
                        "echoed_messages": json.get("messages")
                    });

                    Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .header("x-upstream-custom", "preserved")
                        .body(Body::from(serde_json::to_vec(&resp_body).unwrap()))
                        .unwrap()
                }
            }),
        )
        .route(
            "/v1/chat/completions",
            post(|req: Request<Body>| async move {
                let body_bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

                let resp_body = serde_json::json!({
                    "id": "chatcmpl_mock",
                    "object": "chat.completion",
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "mock openai response"},
                        "finish_reason": "stop"
                    }],
                    "usage": {
                        "prompt_tokens": 12,
                        "completion_tokens": 6
                    },
                    "echoed_messages": json.get("messages"),
                    "echoed_tools": json.get("tools")
                });

                Response::builder()
                    .status(200)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&resp_body).unwrap()))
                    .unwrap()
            }),
        )
        .route(
            "/stream",
            post(|_req: Request<Body>| async move {
                Response::builder()
                    .status(200)
                    .header("content-type", "text/event-stream")
                    .body(Body::from("data: {\"chunk\": 1}\n\ndata: [DONE]\n\n"))
                    .unwrap()
            }),
        )
        .route(
            "/v1/models",
            get(|_req: Request<Body>| async move {
                Response::builder()
                    .status(200)
                    .header("content-type", "application/json")
                    .body(Body::from("{\"data\":[]}"))
                    .unwrap()
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let addr = format!("http://127.0.0.1:{}", port);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, call_count)
}

fn create_test_gateway(upstream_base: &str) -> Router {
    let state_engine = Arc::new(Mutex::new(StateEngine::new()));
    let ccr_store = Arc::new(CcrStore::open_in_memory().unwrap());
    let mut config = AppConfig::default();
    config.anthropic_base_url = upstream_base.to_string();
    config.openai_base_url = upstream_base.to_string();
    config.cache_enabled = true;
    config.terse_mode = true;

    gateway::app_with_client(reqwest::Client::new(), state_engine, config, ccr_store)
}

#[tokio::test]
async fn test_anthropic_routing_and_protocol_safety() {
    let (upstream_url, count) = start_mock_upstream().await;
    let app = create_test_gateway(&upstream_url);

    let req_body = serde_json::json!({
        "model": "claude-3-opus",
        "messages": [
            {"role": "user", "content": "Explain rust"}
        ]
    });

    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("x-api-key", "secret-key")
        .body(Body::from(serde_json::to_vec(&req_body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers().get("x-retoken-cache").unwrap(), "miss");
    assert_eq!(
        resp.headers().get("x-upstream-custom").unwrap(),
        "preserved"
    );

    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let resp_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    // Verify Anthropic protocol correctness:
    // 1. System instruction must be in top-level system, NOT in messages!
    assert!(resp_json.get("echoed_received_system").is_some());
    let echoed_msgs = resp_json["echoed_messages"].as_array().unwrap();
    assert_eq!(echoed_msgs.len(), 1);
    assert_eq!(
        echoed_msgs[0]["role"], "user",
        "messages array must NOT have role: system"
    );

    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_openai_routing_and_protocol_safety() {
    let (upstream_url, _) = start_mock_upstream().await;
    let app = create_test_gateway(&upstream_url);

    let req_body = serde_json::json!({
        "model": "gpt-4o",
        "messages": [
            {"role": "user", "content": "Explain rust"}
        ]
    });

    let req = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .header("authorization", "Bearer sk-test")
        .body(Body::from(serde_json::to_vec(&req_body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let resp_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    // Verify OpenAI protocol correctness:
    // System instruction injected into messages array for OpenAI
    let echoed_msgs = resp_json["echoed_messages"].as_array().unwrap();
    assert_eq!(echoed_msgs[0]["role"], "system");
    assert_eq!(echoed_msgs[1]["role"], "user");
}

#[tokio::test]
async fn test_cache_hit_on_duplicate_request() {
    let (upstream_url, count) = start_mock_upstream().await;
    let app = create_test_gateway(&upstream_url);

    let req_body = serde_json::json!({
        "model": "claude-3-opus",
        "messages": [
            {"role": "user", "content": "Cache test prompt"}
        ]
    });
    let payload = serde_json::to_vec(&req_body).unwrap();

    // First request -> Miss
    let req1 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(payload.clone()))
        .unwrap();

    let resp1 = app.clone().oneshot(req1).await.unwrap();
    assert_eq!(resp1.status(), 200);
    assert_eq!(resp1.headers().get("x-retoken-cache").unwrap(), "miss");
    assert_eq!(count.load(Ordering::SeqCst), 1);

    // Second request with identical payload -> Cache HIT
    let req2 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(payload))
        .unwrap();

    let resp2 = app.oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), 200);
    assert_eq!(resp2.headers().get("x-retoken-cache").unwrap(), "hit");
    // Upstream was NOT called a second time!
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_streaming_requests_bypass_cache() {
    let (upstream_url, _) = start_mock_upstream().await;
    let app = create_test_gateway(&upstream_url);

    let req_body = serde_json::json!({
        "model": "claude-3-opus",
        "stream": true,
        "messages": [
            {"role": "user", "content": "stream prompt"}
        ]
    });
    let payload = serde_json::to_vec(&req_body).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(payload))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    // Streaming responses must NOT have cache header and must not be cached
    assert!(resp.headers().get("x-retoken-cache").is_none());
}

#[tokio::test]
async fn test_untrusted_custom_upstream_url_validation() {
    let (upstream_url, _) = start_mock_upstream().await;
    let app = create_test_gateway(&upstream_url);

    // 1. Dangerous scheme rejected
    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("x-retoken-upstream-url", "file:///etc/shadow")
        .body(Body::from(b"{}".to_vec()))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);

    // 2. Malformed URL rejected
    let req2 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("x-retoken-upstream-url", "invalid-url")
        .body(Body::from(b"{}".to_vec()))
        .unwrap();

    let resp2 = app.clone().oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), 400);

    // 3. Valid HTTP URL accepted
    let valid_custom = format!("{}/custom", upstream_url);
    let req3 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("x-retoken-upstream-url", &valid_custom)
        .body(Body::from(b"{}".to_vec()))
        .unwrap();

    let resp3 = app.oneshot(req3).await.unwrap();
    assert_ne!(resp3.status(), 400);
}

#[tokio::test]
async fn test_bypass_mode_header_preservation() {
    let (upstream_url, count) = start_mock_upstream().await;
    let app = create_test_gateway(&upstream_url);

    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("x-retoken-bypass", "1")
        .header("content-type", "application/json")
        .header("authorization", "Bearer bypass-key")
        .body(Body::from(b"{\"hello\":\"world\"}".to_vec()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("x-upstream-custom").unwrap(),
        "preserved"
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
