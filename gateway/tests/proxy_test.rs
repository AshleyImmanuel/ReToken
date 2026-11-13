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
    config.allow_private_upstreams = true; // allow mock upstream on 127.0.0.1 in tests

    // Test upstreams are loopback-only. Do not inherit a machine-wide HTTP proxy
    // here: doing so can route 127.0.0.1 test traffic to an external proxy and
    // produce misleading 502 failures.
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    gateway::app_with_client(client, state_engine, config, ccr_store)
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
        "temperature": 0.0,
        "messages": [
            {"role": "user", "content": "Cache test prompt"}
        ]
    });
    let payload = serde_json::to_vec(&req_body).unwrap();

    // First request -> Miss (with explicit opt-in header for generation caching)
    let req1 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("x-retoken-cache", "true")
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
        .header("x-retoken-cache", "true")
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

#[tokio::test]
async fn test_ssrf_protection_blocks_private_and_metadata_ips() {
    let (upstream_url, _) = start_mock_upstream().await;
    // Create gateway with strict SSRF policy (allow_private_upstreams = false)
    let state_engine = Arc::new(Mutex::new(StateEngine::new()));
    let ccr_store = Arc::new(CcrStore::open_in_memory().unwrap());
    let mut config = AppConfig::default();
    config.anthropic_base_url = upstream_url.clone();
    config.allow_private_upstreams = false; // strict default

    let app = gateway::app_with_client(reqwest::Client::new(), state_engine, config, ccr_store);

    // 1. Loopback IP rejected
    let req1 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("x-retoken-upstream-url", "http://127.0.0.1:8080")
        .body(Body::from(b"{}".to_vec()))
        .unwrap();
    let resp1 = app.clone().oneshot(req1).await.unwrap();
    assert_eq!(resp1.status(), 400);

    // 2. Localhost host rejected
    let req2 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("x-retoken-upstream-url", "http://localhost:3000")
        .body(Body::from(b"{}".to_vec()))
        .unwrap();
    let resp2 = app.clone().oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), 400);

    // 3. Cloud metadata service link-local IP rejected
    let req3 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header(
            "x-retoken-upstream-url",
            "http://169.254.169.254/latest/meta-data/",
        )
        .body(Body::from(b"{}".to_vec()))
        .unwrap();
    let resp3 = app.oneshot(req3).await.unwrap();
    assert_eq!(resp3.status(), 400);
}

#[tokio::test]
async fn test_request_body_size_limit_rejection() {
    let (upstream_url, _) = start_mock_upstream().await;
    let state_engine = Arc::new(Mutex::new(StateEngine::new()));
    let ccr_store = Arc::new(CcrStore::open_in_memory().unwrap());
    let mut config = AppConfig::default();
    config.anthropic_base_url = upstream_url;
    config.max_request_body_bytes = 100; // tiny 100-byte limit

    let app = gateway::app_with_client(reqwest::Client::new(), state_engine, config, ccr_store);

    // Oversized body (500 bytes > 100 limit)
    let oversized = vec![b'a'; 500];
    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(oversized))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 413); // 413 Payload Too Large
}

#[tokio::test]
async fn test_cache_credential_isolation_and_no_cache_opt_out() {
    let (upstream_url, count) = start_mock_upstream().await;
    let app = create_test_gateway(&upstream_url);

    let req_body = serde_json::json!({
        "model": "claude-3-opus",
        "temperature": 0.0,
        "messages": [
            {"role": "user", "content": "Credential isolation test"}
        ]
    });
    let payload = serde_json::to_vec(&req_body).unwrap();

    // Request 1 from User A -> Cache Miss
    let req1 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("x-retoken-cache", "true")
        .header("x-api-key", "user-A-key")
        .body(Body::from(payload.clone()))
        .unwrap();
    let resp1 = app.clone().oneshot(req1).await.unwrap();
    assert_eq!(resp1.status(), 200);
    assert_eq!(resp1.headers().get("x-retoken-cache").unwrap(), "miss");
    assert_eq!(count.load(Ordering::SeqCst), 1);

    // Duplicate Request 2 from User A -> Cache Hit
    let req2 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("x-retoken-cache", "true")
        .header("x-api-key", "user-A-key")
        .body(Body::from(payload.clone()))
        .unwrap();
    let resp2 = app.clone().oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), 200);
    assert_eq!(resp2.headers().get("x-retoken-cache").unwrap(), "hit");
    assert_eq!(count.load(Ordering::SeqCst), 1);

    // Request 3 with identical body from User B -> Cache MISS (strictly isolated by credential)
    let req3 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("x-retoken-cache", "true")
        .header("x-api-key", "user-B-key")
        .body(Body::from(payload.clone()))
        .unwrap();
    let resp3 = app.clone().oneshot(req3).await.unwrap();
    assert_eq!(resp3.status(), 200);
    assert_eq!(resp3.headers().get("x-retoken-cache").unwrap(), "miss");
    assert_eq!(count.load(Ordering::SeqCst), 2);

    // Request 4 from User A with cache-control: no-cache -> Cache MISS (explicit opt-out)
    let req4 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("x-retoken-cache", "true")
        .header("x-api-key", "user-A-key")
        .header("cache-control", "no-cache")
        .body(Body::from(payload))
        .unwrap();
    let resp4 = app.oneshot(req4).await.unwrap();
    assert_eq!(resp4.status(), 200);
    assert_eq!(resp4.headers().get("x-retoken-cache").unwrap(), "miss");
    assert_eq!(count.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn test_upstream_allowlist_enforcement() {
    let (upstream_url, _) = start_mock_upstream().await;
    let state_engine = Arc::new(Mutex::new(StateEngine::new()));
    let ccr_store = Arc::new(CcrStore::open_in_memory().unwrap());
    let mut config = AppConfig::default();
    config.anthropic_base_url = upstream_url;
    config.allowed_upstream_hosts = vec!["api.anthropic.com".into(), "*.trusted.com".into()];

    let app = gateway::app_with_client(reqwest::Client::new(), state_engine, config, ccr_store);

    // Host not in allowlist rejected
    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("x-retoken-upstream-url", "https://api.untrusted.com/v1")
        .body(Body::from(b"{}".to_vec()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);

    // Host in wildcard allowlist accepted
    let req2 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("x-retoken-upstream-url", "https://llm.trusted.com/v1")
        .body(Body::from(b"{}".to_vec()))
        .unwrap();
    let resp2 = app.oneshot(req2).await.unwrap();
    assert_ne!(resp2.status(), 400);
}

#[tokio::test]
async fn test_stochastic_temperature_bypasses_cache() {
    let (upstream_url, count) = start_mock_upstream().await;
    let app = create_test_gateway(&upstream_url);

    let req_body = serde_json::json!({
        "model": "claude-3-opus",
        "temperature": 0.8,
        "messages": [
            {"role": "user", "content": "Tell me a random story"}
        ]
    });
    let payload = serde_json::to_vec(&req_body).unwrap();

    // 1st request with temperature 0.8
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

    // 2nd duplicate request with temperature 0.8 -> also MISS (stochastic generation bypasses cache)
    let req2 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(payload))
        .unwrap();
    let resp2 = app.oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), 200);
    assert_eq!(resp2.headers().get("x-retoken-cache").unwrap(), "miss");
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_generation_endpoints_bypass_cache_by_default() {
    let (upstream_url, count) = start_mock_upstream().await;
    let app = create_test_gateway(&upstream_url);

    let req_body = serde_json::json!({
        "model": "claude-3-opus",
        "messages": [
            {"role": "user", "content": "Default caching test"}
        ]
    });
    let payload = serde_json::to_vec(&req_body).unwrap();

    // 1st request with no temperature specified (defaults to model sampling)
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

    // 2nd duplicate request without x-retoken-cache: true -> also MISS (generation endpoints bypass cache by default)
    let req2 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(payload))
        .unwrap();
    let resp2 = app.oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), 200);
    assert_eq!(resp2.headers().get("x-retoken-cache").unwrap(), "miss");
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_admin_cache_clear_and_stats_endpoints() {
    let (upstream_url, _) = start_mock_upstream().await;
    let app = create_test_gateway(&upstream_url);

    // 1. Initial stats check
    let req_stats = Request::builder()
        .method("GET")
        .uri("/_retoken/stats")
        .body(Body::empty())
        .unwrap();
    let resp_stats = app.clone().oneshot(req_stats).await.unwrap();
    assert_eq!(resp_stats.status(), 200);
    let bytes = axum::body::to_bytes(resp_stats.into_body(), usize::MAX)
        .await
        .unwrap();
    let stats_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(stats_json["status"], "ok");
    assert_eq!(stats_json["cached_entries"], 0);

    // 2. Insert cached item via opted-in request
    let req_body = serde_json::json!({
        "model": "claude-3-opus",
        "temperature": 0.0,
        "messages": [{"role": "user", "content": "Admin cache clear test"}]
    });
    let payload = serde_json::to_vec(&req_body).unwrap();
    let req_prime = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("x-retoken-cache", "true")
        .body(Body::from(payload))
        .unwrap();
    let resp_prime = app.clone().oneshot(req_prime).await.unwrap();
    assert_eq!(resp_prime.status(), 200);

    // 3. Stats should now show 1 cached entry
    let req_stats2 = Request::builder()
        .method("GET")
        .uri("/_retoken/stats")
        .body(Body::empty())
        .unwrap();
    let resp_stats2 = app.clone().oneshot(req_stats2).await.unwrap();
    let bytes2 = axum::body::to_bytes(resp_stats2.into_body(), usize::MAX)
        .await
        .unwrap();
    let stats_json2: serde_json::Value = serde_json::from_slice(&bytes2).unwrap();
    assert_eq!(stats_json2["cached_entries"], 1);

    // 4. Call cache clear endpoint
    let req_clear = Request::builder()
        .method("POST")
        .uri("/_retoken/cache/clear")
        .body(Body::empty())
        .unwrap();
    let resp_clear = app.clone().oneshot(req_clear).await.unwrap();
    assert_eq!(resp_clear.status(), 200);
    let bytes_clear = axum::body::to_bytes(resp_clear.into_body(), usize::MAX)
        .await
        .unwrap();
    let clear_json: serde_json::Value = serde_json::from_slice(&bytes_clear).unwrap();
    assert_eq!(clear_json["status"], "ok");
    assert_eq!(clear_json["cleared_entries"], 1);

    // 5. Stats after clear should show 0 cached entries
    let req_stats3 = Request::builder()
        .method("GET")
        .uri("/_retoken/stats")
        .body(Body::empty())
        .unwrap();
    let resp_stats3 = app.oneshot(req_stats3).await.unwrap();
    let bytes3 = axum::body::to_bytes(resp_stats3.into_body(), usize::MAX)
        .await
        .unwrap();
    let stats_json3: serde_json::Value = serde_json::from_slice(&bytes3).unwrap();
    assert_eq!(stats_json3["cached_entries"], 0);
}

#[tokio::test]
async fn test_multiple_completions_bypass_cache() {
    let (upstream_url, count) = start_mock_upstream().await;
    let app = create_test_gateway(&upstream_url);

    // Request with n=2 (multiple completions)
    let req_body = serde_json::json!({
        "model": "claude-3-opus",
        "temperature": 0.0,
        "n": 2,
        "messages": [{"role": "user", "content": "Multiple completions prompt"}]
    });
    let payload = serde_json::to_vec(&req_body).unwrap();

    let req1 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("x-retoken-cache", "true")
        .body(Body::from(payload.clone()))
        .unwrap();
    let resp1 = app.clone().oneshot(req1).await.unwrap();
    assert_eq!(resp1.status(), 200);
    assert_eq!(resp1.headers().get("x-retoken-cache").unwrap(), "miss");
    assert_eq!(count.load(Ordering::SeqCst), 1);

    // 2nd request with n=2 must also bypass cache
    let req2 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("x-retoken-cache", "true")
        .body(Body::from(payload))
        .unwrap();
    let resp2 = app.oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), 200);
    assert_eq!(resp2.headers().get("x-retoken-cache").unwrap(), "miss");
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_admin_cache_list_and_single_key_invalidation() {
    let (upstream_url, _) = start_mock_upstream().await;
    let app = create_test_gateway(&upstream_url);

    // 1. Prime cache with 1 deterministic item
    let req_body = serde_json::json!({
        "model": "claude-3-opus",
        "temperature": 0.0,
        "messages": [{"role": "user", "content": "List and invalidate test"}]
    });
    let payload = serde_json::to_vec(&req_body).unwrap();
    let req_prime = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("x-retoken-cache", "true")
        .body(Body::from(payload))
        .unwrap();
    let resp_prime = app.clone().oneshot(req_prime).await.unwrap();
    assert_eq!(resp_prime.status(), 200);

    // 2. Query GET /_retoken/cache to list entries
    let req_list = Request::builder()
        .method("GET")
        .uri("/_retoken/cache")
        .body(Body::empty())
        .unwrap();
    let resp_list = app.clone().oneshot(req_list).await.unwrap();
    assert_eq!(resp_list.status(), 200);
    let bytes = axum::body::to_bytes(resp_list.into_body(), usize::MAX)
        .await
        .unwrap();
    let list_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(list_json["count"], 1);
    let entry_key = list_json["entries"][0]["key"].as_str().unwrap().to_string();

    // 3. Delete single key via DELETE /_retoken/cache?key=<entry_key>
    let req_delete = Request::builder()
        .method("DELETE")
        .uri(format!("/_retoken/cache?key={}", entry_key))
        .body(Body::empty())
        .unwrap();
    let resp_delete = app.clone().oneshot(req_delete).await.unwrap();
    assert_eq!(resp_delete.status(), 200);

    // 4. Verify cache is now empty
    let req_list2 = Request::builder()
        .method("GET")
        .uri("/_retoken/cache")
        .body(Body::empty())
        .unwrap();
    let resp_list2 = app.oneshot(req_list2).await.unwrap();
    let bytes2 = axum::body::to_bytes(resp_list2.into_body(), usize::MAX)
        .await
        .unwrap();
    let list_json2: serde_json::Value = serde_json::from_slice(&bytes2).unwrap();
    assert_eq!(list_json2["count"], 0);
}

#[tokio::test]
async fn test_response_body_size_limit_rejection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream_url = format!("http://{}", addr);

    let app_mock = Router::new().route(
        "/v1/messages",
        post(|| async {
            let big_body = vec![b'x'; 500]; // 500 bytes response
            Response::builder()
                .status(200)
                .header("content-type", "application/octet-stream")
                .body(Body::from(big_body))
                .unwrap()
        }),
    );

    tokio::spawn(async move {
        axum::serve(listener, app_mock).await.unwrap();
    });

    let state_engine = Arc::new(Mutex::new(StateEngine::new()));
    let ccr_store = Arc::new(CcrStore::open_in_memory().unwrap());
    let mut config = AppConfig::default();
    config.anthropic_base_url = upstream_url;
    config.allow_private_upstreams = true;
    config.max_response_body_bytes = 100; // 100-byte max limit

    let app = gateway::app_with_client(reqwest::Client::new(), state_engine, config, ccr_store);

    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .body(Body::from(b"{}".to_vec()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 502); // 502 Bad Gateway due to response body size violation
}

#[tokio::test]
async fn test_response_side_stream_detection_bypasses_cache() {
    let call_count = Arc::new(AtomicUsize::new(0));
    let count_clone = call_count.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream_url = format!("http://{}", addr);

    let app_mock = Router::new().route(
        "/v1/messages",
        post(move || {
            let count = count_clone.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                Response::builder()
                    .status(200)
                    .header("content-type", "text/event-stream; charset=utf-8")
                    .body(Body::from(
                        "event: message_start\ndata: {\"usage\":{\"input_tokens\":7,\"cache_read_input_tokens\":1}}\n\nevent: message_delta\ndata: {\"usage\":{\"output_tokens\":2}}\n\n",
                    ))
                    .unwrap()
            }
        }),
    );

    tokio::spawn(async move {
        axum::serve(listener, app_mock).await.unwrap();
    });

    let state_engine = Arc::new(Mutex::new(StateEngine::new()));
    let ccr_store = Arc::new(CcrStore::open_in_memory().unwrap());
    let mut config = AppConfig::default();
    config.anthropic_base_url = upstream_url;
    config.allow_private_upstreams = true;

    let app = gateway::app_with_client(reqwest::Client::new(), state_engine, config, ccr_store);

    let payload = serde_json::json!({
        "model": "claude-3-opus",
        "temperature": 0.0,
        "messages": [{"role": "user", "content": "stream detection test"}]
    });
    let body_bytes = serde_json::to_vec(&payload).unwrap();

    // Request 1: client did NOT set stream: true, but server returned text/event-stream
    let req1 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("x-retoken-cache", "true")
        .body(Body::from(body_bytes.clone()))
        .unwrap();

    let resp1 = app.clone().oneshot(req1).await.unwrap();
    assert_eq!(resp1.status(), 200);
    let ct = resp1
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("text/event-stream"));
    let streamed_bytes = resp1.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        streamed_bytes.as_ref(),
        b"event: message_start\ndata: {\"usage\":{\"input_tokens\":7,\"cache_read_input_tokens\":1}}\n\nevent: message_delta\ndata: {\"usage\":{\"output_tokens\":2}}\n\n",
        "the telemetry tee must relay the upstream stream byte-for-byte"
    );
    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    let stats_req = Request::builder()
        .uri("/_retoken/stats")
        .body(Body::empty())
        .unwrap();
    let stats = app.clone().oneshot(stats_req).await.unwrap();
    let stats_bytes = stats.into_body().collect().await.unwrap().to_bytes();
    let stats: serde_json::Value = serde_json::from_slice(&stats_bytes).unwrap();
    assert_eq!(stats["total_requests"], 1);
    assert_eq!(stats["total_input_tokens"], 7);
    assert_eq!(stats["total_output_tokens"], 2);
    assert_eq!(stats["total_cached_tokens"], 1);

    // Request 2: identical request must NOT hit cache because response was an event stream
    let req2 = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("x-retoken-cache", "true")
        .body(Body::from(body_bytes))
        .unwrap();

    let resp2 = app.oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), 200);
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        2,
        "SSE responses must never be cached even if client did not explicitly set stream: true"
    );
}

#[tokio::test]
async fn test_openai_responses_reuses_only_exact_duplicate_tool_output_when_enabled() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let captured_request = captured.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let upstream = Router::new().route(
        "/v1/responses",
        post(move |req: Request<Body>| {
            let captured = captured_request.clone();
            async move {
                *captured.lock().await = axum::body::to_bytes(req.into_body(), usize::MAX)
                    .await
                    .unwrap()
                    .to_vec();
                Response::builder()
                    .status(200)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"id":"resp_test","usage":{"input_tokens":1,"output_tokens":1}}"#,
                    ))
                    .unwrap()
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
    });

    let state_engine = Arc::new(Mutex::new(StateEngine::new()));
    let ccr_store = Arc::new(CcrStore::open_in_memory().unwrap());
    let mut config = AppConfig::default();
    config.openai_base_url = format!("http://{}", addr);
    config.allow_private_upstreams = true;
    config.block_reuse_enabled = true;
    let app = gateway::app_with_client(reqwest::Client::new(), state_engine, config, ccr_store);

    let output = "exact successful command output\n".repeat(120);
    let request = serde_json::json!({
        "model": "gpt-5-codex",
        "input": [
            {"type": "function_call_output", "call_id": "first", "output": output},
            {"type": "function_call_output", "call_id": "second", "output": "exact successful command output\n".repeat(120)}
        ]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&request).unwrap()))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), 200);

    let forwarded: serde_json::Value = serde_json::from_slice(&captured.lock().await).unwrap();
    assert_eq!(forwarded["input"][0]["output"], output);
    assert!(forwarded["input"][1]["output"]
        .as_str()
        .unwrap()
        .contains("complete identical bytes appear earlier"));
    assert!(forwarded.get("instructions").is_none());
    assert!(forwarded.get("tools").is_none());
}

#[test]
fn test_connect_time_ssrf_destination_pinning() {
    use gateway::provider::resolve_and_verify_destination;

    // Strict policy: loopback, private, and metadata IPs must be rejected
    assert!(resolve_and_verify_destination("127.0.0.1", 80, false).is_err());
    assert!(resolve_and_verify_destination("localhost", 80, false).is_err());
    assert!(resolve_and_verify_destination("169.254.169.254", 80, false).is_err());
    assert!(resolve_and_verify_destination("10.0.0.1", 443, false).is_err());
    assert!(resolve_and_verify_destination("192.168.1.1", 443, false).is_err());
    assert!(resolve_and_verify_destination("internal.corp", 443, false).is_err());

    // Public IP literal passes and returns pinned SocketAddr
    let res = resolve_and_verify_destination("1.1.1.1", 443, false);
    assert!(res.is_ok());
    let addr = res.unwrap().expect("IP literal must resolve to SocketAddr");
    assert_eq!(addr.ip().to_string(), "1.1.1.1");
    assert_eq!(addr.port(), 443);

    // With allow_private = true (e.g. testing mode)
    assert!(resolve_and_verify_destination("127.0.0.1", 80, true).is_ok());
}

#[tokio::test]
async fn test_default_preserves_comments_and_prose_losslessly() {
    let captured_body = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let cap_clone = captured_body.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream_url = format!("http://{}", addr);

    let app_mock = Router::new().route(
        "/v1/messages",
        post(move |req: Request<Body>| {
            let cap = cap_clone.clone();
            async move {
                let bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
                    .await
                    .unwrap();
                *cap.lock().await = bytes.to_vec();
                Response::builder()
                    .status(200)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"id":"m1","content":[{"type":"text","text":"ok"}]}"#,
                    ))
                    .unwrap()
            }
        }),
    );

    tokio::spawn(async move {
        axum::serve(listener, app_mock).await.unwrap();
    });

    let state_engine = Arc::new(Mutex::new(StateEngine::new()));
    let ccr_store = Arc::new(CcrStore::open_in_memory().unwrap());
    let mut config = AppConfig::default();
    config.anthropic_base_url = upstream_url;
    config.allow_private_upstreams = true;
    // Default config has terse_mode = false, compression_enabled = false, ccr_enabled = false

    let app = gateway::app_with_client(reqwest::Client::new(), state_engine, config, ccr_store);

    let code_with_comments = r#"```rust
// Security constraint: must validate caller permissions
fn process_payment(user: &User) {
    // Audit log before state transition
    log_audit(user);
    // Line 1
    // Line 2
    // Line 3
    // Line 4
    // Line 5
    // Line 6
    // Line 7
    // Line 8
    // Line 9
    // Line 10
    // Line 11
    // Line 12
    // Line 13
    // Line 14
    // Line 15
    execute();
}
```"#;

    let payload = serde_json::json!({
        "model": "claude-3-opus",
        "messages": [
            {"role": "user", "content": code_with_comments}
        ]
    });

    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let forwarded_bytes = captured_body.lock().await.clone();
    let forwarded_str = String::from_utf8_lossy(&forwarded_bytes);

    // Comments must NOT be stripped
    assert!(forwarded_str.contains("Security constraint: must validate caller permissions"));
    assert!(forwarded_str.contains("Audit log before state transition"));
    // Middle lines must NOT be elided
    assert!(forwarded_str.contains("// Line 7"));
    // Terse persona must NOT be injected by default
    assert!(!forwarded_str.contains("ReToken Terse Mode"));
    assert!(!forwarded_str.contains("Caveman"));
    // Literal pass-through: prompt caching breakpoint must NOT be injected by default
    assert!(!forwarded_str.contains("cache_control"));
    // Forwarded bytes must match the exact original request bytes bit-for-bit
    let original_bytes = serde_json::to_vec(&payload).unwrap();
    assert_eq!(forwarded_bytes, original_bytes);
}

#[tokio::test]
async fn test_opt_in_prompt_caching_injects_breakpoints() {
    let captured_body = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let cap_clone = captured_body.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream_url = format!("http://{}", addr);

    let app_mock = Router::new().route(
        "/v1/messages",
        post(move |req: Request<Body>| {
            let cap = cap_clone.clone();
            async move {
                let bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
                    .await
                    .unwrap();
                *cap.lock().await = bytes.to_vec();
                Response::builder()
                    .status(200)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"id":"m2","content":[{"type":"text","text":"ok"}]}"#,
                    ))
                    .unwrap()
            }
        }),
    );

    tokio::spawn(async move {
        axum::serve(listener, app_mock).await.unwrap();
    });

    let state_engine = Arc::new(Mutex::new(StateEngine::new()));
    let ccr_store = Arc::new(CcrStore::open_in_memory().unwrap());
    let mut config = AppConfig::default();
    config.anthropic_base_url = upstream_url;
    config.allow_private_upstreams = true;
    config.prompt_caching_enabled = true; // Opt-in to prompt caching

    let app = gateway::app_with_client(reqwest::Client::new(), state_engine, config, ccr_store);

    let payload = serde_json::json!({
        "model": "claude-3-opus",
        "system": "You are a helpful coding assistant.",
        "messages": [
            {"role": "user", "content": "Explain rust ownership."}
        ]
    });

    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let forwarded_bytes = captured_body.lock().await.clone();
    let forwarded_str = String::from_utf8_lossy(&forwarded_bytes);

    // Prompt cache breakpoint MUST be injected when opted in
    assert!(forwarded_str.contains("cache_control"));
    assert!(forwarded_str.contains("ephemeral"));
}

#[tokio::test]
async fn test_live_ccr_multi_turn_flow() {
    let turn_counter = Arc::new(tokio::sync::Mutex::new(0));
    let tc_clone = turn_counter.clone();

    let ccr_store = Arc::new(CcrStore::open_in_memory().unwrap());
    let handle = ccr_store
        .store(b"CRITICAL_DATABASE_CRASH_LOG", "crash_log")
        .unwrap();
    let handle_clone = handle.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream_url = format!("http://{}", addr);

    let app_mock = Router::new().route(
        "/v1/messages",
        post(move |req: Request<Body>| {
            let tc = tc_clone.clone();
            let h = handle_clone.clone();
            async move {
                let bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let req_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let mut count = tc.lock().await;
                *count += 1;

                if *count == 1 {
                    // Turn 1: Upstream model invokes retoken_retrieve tool
                    Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .body(Body::from(serde_json::to_vec(&serde_json::json!({
                            "id": "msg_turn1",
                            "type": "message",
                            "role": "assistant",
                            "content": [
                                {
                                    "type": "tool_use",
                                    "id": "toolu_recover_01",
                                    "name": "retoken_retrieve",
                                    "input": {"handle": h}
                                }
                            ],
                            "stop_reason": "tool_use"
                        })).unwrap()))
                        .unwrap()
                } else {
                    // Turn 2: Verify model received the tool_result follow-up turn
                    let messages = req_json["messages"].as_array().unwrap();
                    assert_eq!(messages.len(), 3);
                    assert_eq!(messages[1]["role"], "assistant");
                    assert_eq!(messages[2]["role"], "user");
                    let tool_result_content = messages[2]["content"][0]["content"].as_str().unwrap();
                    assert!(tool_result_content.contains("CRITICAL_DATABASE_CRASH_LOG"));

                    // Upstream model reasons over recovered data and delivers final answer
                    Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .body(Body::from(serde_json::to_vec(&serde_json::json!({
                            "id": "msg_turn2",
                            "type": "message",
                            "role": "assistant",
                            "content": [
                                {
                                    "type": "text",
                                    "text": "The crash was caused by connection timeout in database pool."
                                }
                            ],
                            "stop_reason": "end_turn"
                        })).unwrap()))
                        .unwrap()
                }
            }
        }),
    );

    tokio::spawn(async move {
        axum::serve(listener, app_mock).await.unwrap();
    });

    let state_engine = Arc::new(Mutex::new(StateEngine::new()));
    let mut config = AppConfig::default();
    config.anthropic_base_url = upstream_url;
    config.allow_private_upstreams = true;
    config.ccr_enabled = true; // Enable live CCR multi-turn resolution

    let app = gateway::app_with_client(reqwest::Client::new(), state_engine, config, ccr_store);

    let payload = serde_json::json!({
        "model": "claude-3-opus",
        "messages": [
            {"role": "user", "content": "Analyze crash error log."}
        ]
    });

    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let resp_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp_json: serde_json::Value = serde_json::from_slice(&resp_bytes).unwrap();

    // Verify client received the model's authentic reasoned answer with end_turn
    assert_eq!(resp_json["stop_reason"], "end_turn");
    let content_text = resp_json["content"][0]["text"].as_str().unwrap();
    assert_eq!(
        content_text,
        "The crash was caused by connection timeout in database pool."
    );

    // Verify mock upstream executed 2 turns (turn 1 tool_use + turn 2 tool_result)
    assert_eq!(*turn_counter.lock().await, 2);
}

#[tokio::test]
async fn test_compression_enabled_preserves_comments_in_tool_output() {
    let captured_body = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let cap_clone = captured_body.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream_url = format!("http://{}", addr);

    let app_mock = Router::new().route(
        "/v1/messages",
        post(move |req: Request<Body>| {
            let cap = cap_clone.clone();
            async move {
                let bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
                    .await
                    .unwrap();
                *cap.lock().await = bytes.to_vec();
                Response::builder()
                    .status(200)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"id":"m3","content":[{"type":"text","text":"ok"}]}"#,
                    ))
                    .unwrap()
            }
        }),
    );

    tokio::spawn(async move {
        axum::serve(listener, app_mock).await.unwrap();
    });

    let state_engine = Arc::new(Mutex::new(StateEngine::new()));
    let ccr_store = Arc::new(CcrStore::open_in_memory().unwrap());
    let mut config = AppConfig::default();
    config.anthropic_base_url = upstream_url;
    config.allow_private_upstreams = true;
    config.compression_enabled = true; // Explicitly enable compression

    let app = gateway::app_with_client(reqwest::Client::new(), state_engine, config, ccr_store);

    let code_with_comments = r#"```rust
// CRITICAL_LOGIC_NOTE: must check caller permissions before withdrawal
fn withdraw(amount: u64) {
    // SECURITY_INVARIANT: balance cannot underflow
    let balance = 1000;
    // Padding line 1 to make block >= 200 bytes
    // Padding line 2 to make block >= 200 bytes
    // Padding line 3 to make block >= 200 bytes
    // Padding line 4 to make block >= 200 bytes
    // Padding line 5 to make block >= 200 bytes
    // Padding line 6 to make block >= 200 bytes
    execute();
}
```"#;

    let payload = serde_json::json!({
        "model": "claude-3-opus",
        "messages": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "toolu_01",
                        "content": code_with_comments
                    }
                ]
            }
        ]
    });

    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let forwarded_bytes = captured_body.lock().await.clone();
    let forwarded_str = String::from_utf8_lossy(&forwarded_bytes);

    // Comments MUST NOT be stripped even when compression is enabled
    assert!(forwarded_str
        .contains("CRITICAL_LOGIC_NOTE: must check caller permissions before withdrawal"));
    assert!(forwarded_str.contains("SECURITY_INVARIANT: balance cannot underflow"));
}

#[tokio::test]
async fn test_openai_codex_prefix_caching_canonicalization() {
    let captured_body = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let cap_clone = captured_body.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream_url = format!("http://{}", addr);

    let app_mock = Router::new().route(
        "/v1/chat/completions",
        post(move |req: Request<Body>| {
            let cap = cap_clone.clone();
            async move {
                let bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
                    .await
                    .unwrap();
                *cap.lock().await = bytes.to_vec();
                Response::builder()
                    .status(200)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"id":"chatcmpl-1","choices":[{"message":{"role":"assistant","content":"Done."}}]}"#,
                    ))
                    .unwrap()
            }
        }),
    );

    tokio::spawn(async move {
        axum::serve(listener, app_mock).await.unwrap();
    });

    let state_engine = Arc::new(Mutex::new(StateEngine::new()));
    let ccr_store = Arc::new(CcrStore::open_in_memory().unwrap());
    let mut config = AppConfig::default();
    config.openai_base_url = upstream_url;
    config.allow_private_upstreams = true;
    config.prompt_caching_enabled = true; // Enable prefix optimization

    let app = gateway::app_with_client(reqwest::Client::new(), state_engine, config, ccr_store);

    let payload = serde_json::json!({
        "model": "gpt-4o",
        "messages": [
            {"role": "user", "content": "Refactor this function."},
            {"role": "system", "content": "You are Codex, an expert coding assistant."}
        ],
        "tools": [
            {
                "type": "function",
                "function": {"name": "run_tests", "description": "Run unit tests"}
            },
            {
                "type": "function",
                "function": {"name": "apply_patch", "description": "Apply code patch"}
            },
            {
                "type": "function",
                "function": {"name": "analyze_ast", "description": "Analyze AST"}
            }
        ]
    });

    let req = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .header("authorization", "Bearer sk-test")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let forwarded_bytes = captured_body.lock().await.clone();
    let forwarded_json: serde_json::Value = serde_json::from_slice(&forwarded_bytes).unwrap();

    // 1. Verify system message moved to index 0 for stable prefix
    let messages = forwarded_json["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(
        messages[0]["content"],
        "You are Codex, an expert coding assistant."
    );
    assert_eq!(messages[1]["role"], "user");
    assert_eq!(messages[1]["content"], "Refactor this function.");

    // 2. Verify tools deterministically sorted by name to guarantee automatic server-side prefix cache hit
    let tools = forwarded_json["tools"].as_array().unwrap();
    assert_eq!(tools[0]["function"]["name"], "analyze_ast");
    assert_eq!(tools[1]["function"]["name"], "apply_patch");
    assert_eq!(tools[2]["function"]["name"], "run_tests");
}

#[tokio::test]
async fn test_ccr_follow_up_strips_internal_retoken_headers() {
    let turn_counter = Arc::new(tokio::sync::Mutex::new(0));
    let tc_clone = turn_counter.clone();
    let captured_turn2_headers = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let headers_clone = captured_turn2_headers.clone();

    let ccr_store = Arc::new(CcrStore::open_in_memory().unwrap());
    let handle = ccr_store
        .store(b"CRITICAL_AUTH_TRACE", "auth_trace")
        .unwrap();
    let handle_clone = handle.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream_url = format!("http://{}", addr);

    let app_mock = Router::new().route(
        "/v1/messages",
        post(move |req: Request<Body>| {
            let tc = tc_clone.clone();
            let h = handle_clone.clone();
            let ch = headers_clone.clone();
            async move {
                let mut count = tc.lock().await;
                *count += 1;

                if *count == 1 {
                    Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::to_vec(&serde_json::json!({
                                "id": "msg_turn1",
                                "type": "message",
                                "role": "assistant",
                                "content": [
                                    {
                                        "type": "tool_use",
                                        "id": "toolu_01",
                                        "name": "retoken_retrieve",
                                        "input": {"handle": h}
                                    }
                                ],
                                "stop_reason": "tool_use"
                            }))
                            .unwrap(),
                        ))
                        .unwrap()
                } else {
                    let mut turn2_headers = Vec::new();
                    for (k, v) in req.headers() {
                        if let Ok(v_str) = v.to_str() {
                            turn2_headers.push((k.to_string(), v_str.to_string()));
                        }
                    }
                    *ch.lock().await = turn2_headers;

                    Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::to_vec(&serde_json::json!({
                                "id": "msg_turn2",
                                "type": "message",
                                "role": "assistant",
                                "content": [{"type": "text", "text": "Resolved."}],
                                "stop_reason": "end_turn"
                            }))
                            .unwrap(),
                        ))
                        .unwrap()
                }
            }
        }),
    );

    tokio::spawn(async move {
        axum::serve(listener, app_mock).await.unwrap();
    });

    let state_engine = Arc::new(Mutex::new(StateEngine::new()));
    let mut config = AppConfig::default();
    config.anthropic_base_url = upstream_url;
    config.allow_private_upstreams = true;
    config.ccr_enabled = true;

    let app = gateway::app_with_client(reqwest::Client::new(), state_engine, config, ccr_store);

    let payload = serde_json::json!({
        "model": "claude-3-5-sonnet",
        "messages": [{"role": "user", "content": "Retrieve trace."}]
    });

    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("x-api-key", "secret-anthropic-key")
        .header("anthropic-version", "2023-06-01")
        .header("x-retoken-bypass", "false")
        .header("x-retoken-cache", "true")
        .header("x-retoken-repo-hash", "abc123hash")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let turn2_headers = captured_turn2_headers.lock().await;
    // Real provider headers must be preserved
    assert!(turn2_headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("x-api-key") && v == "secret-anthropic-key"));
    assert!(turn2_headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("anthropic-version") && v == "2023-06-01"));

    // Internal proxy-control headers must NOT be forwarded to upstream
    assert!(!turn2_headers
        .iter()
        .any(|(k, _)| k.to_lowercase().starts_with("x-retoken-")));
}

#[tokio::test]
async fn test_ccr_initial_response_body_limit_rejection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream_url = format!("http://{}", addr);

    let app_mock = Router::new().route(
        "/v1/messages",
        post(|| async {
            // Returns oversized payload in turn 1
            let large_padding = "x".repeat(500);
            Response::builder()
                .status(200)
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    "{{\"id\":\"msg\",\"type\":\"message\",\"content\":\"{}\"}}",
                    large_padding
                )))
                .unwrap()
        }),
    );

    tokio::spawn(async move {
        axum::serve(listener, app_mock).await.unwrap();
    });

    let state_engine = Arc::new(Mutex::new(StateEngine::new()));
    let ccr_store = Arc::new(CcrStore::open_in_memory().unwrap());
    let mut config = AppConfig::default();
    config.anthropic_base_url = upstream_url;
    config.allow_private_upstreams = true;
    config.ccr_enabled = true;
    config.max_response_body_bytes = 200; // 200 bytes max limit

    let app = gateway::app_with_client(reqwest::Client::new(), state_engine, config, ccr_store);

    let payload = serde_json::json!({
        "model": "claude-3-opus",
        "messages": [{"role": "user", "content": "Test limit"}]
    });

    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // Exceeding response size limit in CCR must return 502 Bad Gateway and NEVER return partial response
    assert_eq!(resp.status(), 502);
}

#[tokio::test]
async fn test_ccr_follow_up_body_limit_rejection() {
    let turn_counter = Arc::new(tokio::sync::Mutex::new(0));
    let tc_clone = turn_counter.clone();

    let ccr_store = Arc::new(CcrStore::open_in_memory().unwrap());
    let handle = ccr_store.store(b"SOME_DATA", "data").unwrap();
    let handle_clone = handle.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream_url = format!("http://{}", addr);

    let app_mock = Router::new().route(
        "/v1/messages",
        post(move || {
            let tc = tc_clone.clone();
            let h = handle_clone.clone();
            async move {
                let mut count = tc.lock().await;
                *count += 1;
                if *count == 1 {
                    // Turn 1 is small and invokes tool
                    Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::to_vec(&serde_json::json!({
                                "id": "msg_turn1",
                                "type": "message",
                                "role": "assistant",
                                "content": [
                                    {
                                        "type": "tool_use",
                                        "id": "tu_01",
                                        "name": "retoken_retrieve",
                                        "input": {"handle": h}
                                    }
                                ],
                                "stop_reason": "tool_use"
                            }))
                            .unwrap(),
                        ))
                        .unwrap()
                } else {
                    // Turn 2 returns oversized response (> 300 bytes)
                    let huge_content = "y".repeat(800);
                    Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .body(Body::from(format!(
                            "{{\"id\":\"msg2\",\"content\":\"{}\"}}",
                            huge_content
                        )))
                        .unwrap()
                }
            }
        }),
    );

    tokio::spawn(async move {
        axum::serve(listener, app_mock).await.unwrap();
    });

    let state_engine = Arc::new(Mutex::new(StateEngine::new()));
    let mut config = AppConfig::default();
    config.anthropic_base_url = upstream_url;
    config.allow_private_upstreams = true;
    config.ccr_enabled = true;
    config.max_response_body_bytes = 300; // 300 bytes max limit

    let app = gateway::app_with_client(reqwest::Client::new(), state_engine, config, ccr_store);

    let payload = serde_json::json!({
        "model": "claude-3-opus",
        "messages": [{"role": "user", "content": "Trigger multi turn"}]
    });

    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // Exceeding response size in follow-up must return 502 Bad Gateway
    assert_eq!(resp.status(), 502);
}
