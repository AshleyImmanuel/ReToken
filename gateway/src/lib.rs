use axum::{
    body::Body,
    extract::{Request, State},
    response::Response,
    routing::any,
    Router,
};
use reqwest::Client;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{info, warn};
use retoken_core::recorder::FlightRecord;
use std::time::Instant;

struct GatewayState {
    client: Client,
}

pub async fn start() -> anyhow::Result<()> {
    let state = Arc::new(GatewayState {
        client: Client::new(),
    });

    let app = Router::new()
        .route("/*path", any(handler))
        .route("/", any(handler))
        .with_state(state);

    let addr = "127.0.0.1:8888";
    let listener = TcpListener::bind(addr).await?;
    info!("ReToken Gateway listening on {}", addr);
    
    axum::serve(listener, app).await?;
    
    Ok(())
}

async fn handler(State(state): State<Arc<GatewayState>>, req: Request<Body>) -> Result<Response<Body>, axum::http::StatusCode> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    
    info!("Intercepted request: {} {}", method, path_and_query);

    // Phase 0: Forward the request to a mock destination, or if headers specify it, to the real provider.
    // For now, we'll assume Anthropic API for testing purposes if not specified.
    let target_url = format!("https://api.anthropic.com{}", path_and_query);
    info!("Forwarding to {}", target_url);

    let mut record = FlightRecord::default();
    record.provider = "anthropic".into();
    let start_time = Instant::now();

    // Reconstruct the request to forward
    let (parts, body) = req.into_parts();
    let mut req_builder = state.client.request(method, &target_url);
    
    // Copy headers (ignoring host so it resolves correctly)
    for (header_name, header_value) in parts.headers.iter() {
        if header_name != axum::http::header::HOST {
            req_builder = req_builder.header(header_name, header_value);
        }
    }

    // Convert axum Body to bytes, then to reqwest Body
    let bytes = axum::body::to_bytes(body, usize::MAX).await.map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;
    let reqwest_body = reqwest::Body::from(bytes);
    let reqwest_req = req_builder.body(reqwest_body).build().map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;

    // Send the request
    let resp = state.client.execute(reqwest_req).await.map_err(|e| {
        warn!("Proxy request failed: {}", e);
        axum::http::StatusCode::BAD_GATEWAY
    })?;

    record.network_duration_ms = start_time.elapsed().as_millis() as u64;
    info!("Flight Record: {:?}", record);

    // Build the axum response
    let status = resp.status();
    let mut builder = Response::builder().status(status);
    
    if let Some(headers) = builder.headers_mut() {
        for (name, value) in resp.headers().iter() {
            headers.insert(name.clone(), value.clone());
        }
    }

    let resp_body = Body::from_stream(resp.bytes_stream());
    Ok(builder.body(resp_body).unwrap())
}
