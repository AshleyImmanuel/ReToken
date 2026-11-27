//! End-to-end tests for Gemini API (`generateContent`) traffic through the gateway.

use axum::{body::Body, extract::Request, response::Response, routing::post, Router};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tower::ServiceExt;

use compressors::ccr::CcrStore;
use retoken_core::config::AppConfig;

/// Mock Gemini API. Echoes what it received (path, query, API key header, body) so tests can
/// check what the gateway forwarded. `:streamGenerateContent` answers as an SSE stream.
async fn start_mock_gemini() -> (String, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let app = Router::new().route(
        "/v1beta/models/*method",
        post(move |req: Request<Body>| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                let path = req.uri().path().to_string();
                let query = req.uri().query().unwrap_or("").to_string();
                let api_key = req
                    .headers()
                    .get("x-goog-api-key")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                let bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let received: Value = serde_json::from_slice(&bytes).unwrap();
                let usage = json!({
                    "promptTokenCount": 120,
                    "candidatesTokenCount": 8,
                    "cachedContentTokenCount": 100,
                    "thoughtsTokenCount": 4
                });
                if path.ends_with(":streamGenerateContent") {
                    let chunk = json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "ok"}]}}]});
                    let last = json!({"candidates": [], "usageMetadata": usage});
                    let sse = format!("data: {chunk}\r\n\r\ndata: {last}\r\n\r\n");
                    return Response::builder()
                        .status(200)
                        .header("content-type", "text/event-stream")
                        .body(Body::from(sse))
                        .unwrap();
                }
                let resp = json!({
                    "candidates": [{"content": {"role": "model", "parts": [{"text": "ok"}]}}],
                    "usageMetadata": usage,
                    "echo": {"path": path, "query": query, "api_key": api_key, "body": received}
                });
                Response::builder()
                    .status(200)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&resp).unwrap()))
                    .unwrap()
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, calls)
}

fn gateway(upstream: &str, terse: bool) -> Router {
    let config = AppConfig {
        gemini_base_url: upstream.to_string(),
        cache_enabled: true,
        terse_mode: terse,
        allow_private_upstreams: true, // mock upstream on 127.0.0.1
        ..Default::default()
    };
    gateway::app(config, Arc::new(CcrStore::open_in_memory().unwrap())).unwrap()
}

fn build_log() -> String {
    (0..300)
        .map(|i| match i {
            150 => "error[E0425]: cannot find value `x` in this scope\n".to_string(),
            _ => format!("   Compiling dep_{i} v0.1.0\n"),
        })
        .collect()
}

/// A Gemini CLI-style turn: the model ran a shell command and the result is a long build log.
fn shell_request(log: &str, declare_retrieve: bool) -> Value {
    let mut declarations =
        vec![json!({"name": "run_shell_command", "parameters": {"type": "object"}})];
    if declare_retrieve {
        declarations.push(json!({"name": "retoken_retrieve", "parameters": {"type": "object"}}));
    }
    json!({
        "systemInstruction": {"parts": [{"text": "You are a coding agent."}]},
        "tools": [{"functionDeclarations": declarations}],
        "contents": [
            {"role": "user", "parts": [{"text": "Build the project"}]},
            {"role": "model", "parts": [{"functionCall": {"name": "run_shell_command", "args": {"command": "cargo build"}}}]},
            {"role": "user", "parts": [{"functionResponse": {"name": "run_shell_command", "response": {"output": log}}}]}
        ]
    })
}

async fn send(app: &Router, uri: &str, api_key: Option<&str>, body: &Value) -> (u16, Value) {
    let mut req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(key) = api_key {
        req = req.header("x-goog-api-key", key);
    }
    let req = req
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status().as_u16();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn routes_to_gemini_and_forwards_credentials_unchanged() {
    let (upstream, _) = start_mock_gemini().await;
    let app = gateway(&upstream, false);
    let body = json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]});

    let (status, resp) = send(
        &app,
        "/v1beta/models/gemini-4-hydrogen:generateContent",
        Some("AIza-header-key"),
        &body,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        resp["echo"]["path"],
        "/v1beta/models/gemini-4-hydrogen:generateContent"
    );
    assert_eq!(resp["echo"]["api_key"], "AIza-header-key");
    assert_eq!(
        resp["echo"]["body"], body,
        "nothing to compress: body unchanged"
    );

    let (status, resp) = send(
        &app,
        "/v1beta/models/gemini-4-neon:generateContent?key=AIza-query-key",
        None,
        &body,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(resp["echo"]["query"], "key=AIza-query-key");
}

#[tokio::test]
async fn compresses_shell_output_when_retrieval_is_available() {
    let (upstream, _) = start_mock_gemini().await;
    let app = gateway(&upstream, true);
    let log = build_log();

    let (status, resp) = send(
        &app,
        "/v1beta/models/gemini-4-hydrogen:generateContent",
        Some("k"),
        &shell_request(&log, true),
    )
    .await;
    assert_eq!(status, 200);
    let forwarded = &resp["echo"]["body"];
    let output = forwarded["contents"][2]["parts"][0]["functionResponse"]["response"]["output"]
        .as_str()
        .unwrap();
    assert!(output.len() < log.len() / 2, "log was shortened");
    assert!(output.contains("error[E0425]"), "errors are kept");
    assert!(output.contains("call `retoken_retrieve` with handle"));
    assert_eq!(
        forwarded["contents"][0]["parts"][0]["text"],
        "Build the project"
    );
    let system = forwarded["systemInstruction"]["parts"].as_array().unwrap();
    assert_eq!(system[0]["text"], "You are a coding agent.");
    assert!(
        system[1]["text"]
            .as_str()
            .unwrap()
            .contains("ReToken terse mode"),
        "terse persona appended as a separate part"
    );
}

#[tokio::test]
async fn without_retrieval_tool_output_is_not_elided() {
    let (upstream, _) = start_mock_gemini().await;
    let app = gateway(&upstream, false);
    let log = build_log();
    let request = shell_request(&log, false);

    let (_, resp) = send(
        &app,
        "/v1beta/models/gemini-4-hydrogen:generateContent",
        Some("k"),
        &request,
    )
    .await;
    assert_eq!(resp["echo"]["body"], request);
}

#[tokio::test]
async fn generations_are_not_cached_without_opt_in() {
    let (upstream, calls) = start_mock_gemini().await;
    let app = gateway(&upstream, false);
    let body = json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}],
                      "generationConfig": {"temperature": 0}});
    for _ in 0..2 {
        send(&app, "/v1beta/models/m:generateContent", Some("k"), &body).await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn streaming_passes_through_and_is_never_cached() {
    let (upstream, calls) = start_mock_gemini().await;
    let app = gateway(&upstream, false);
    let body = json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]});
    for _ in 0..2 {
        let req = Request::builder()
            .method("POST")
            .uri("/v1beta/models/m:streamGenerateContent?alt=sse")
            .header("content-type", "application/json")
            .header("x-goog-api-key", "k")
            .header("x-retoken-cache-stochastic", "true")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/event-stream"
        );
        let text = String::from_utf8(
            resp.into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(text.contains("usageMetadata"));
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "streams are never served from cache"
    );
}

#[tokio::test]
async fn code_assist_sign_in_traffic_is_rejected() {
    let (upstream, calls) = start_mock_gemini().await;
    let app = gateway(&upstream, false);
    let req = Request::builder()
        .method("POST")
        .uri("/v1internal:generateContent")
        .header("authorization", "Bearer ya29.token")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
