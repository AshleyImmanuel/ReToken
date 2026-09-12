use axum::{
    routing::any,
    Router,
};
use reqwest::Client;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;
use tokio::sync::Mutex;

use retoken_core::state::StateEngine;
use retoken_core::cache::ResponseCache;
use retoken_core::config::AppConfig;

pub mod provider;
pub mod optimizer;
pub mod bypass;
pub mod handler;

use handler::{handler as gateway_handler, GatewayState};

pub fn app(state_engine: Arc<Mutex<StateEngine>>, config: AppConfig) -> Router {
    let response_cache = Arc::new(Mutex::new(
        ResponseCache::new(config.cache_ttl_secs, config.max_cache_entries)
    ));

    let state = Arc::new(GatewayState {
        client: Client::new(),
        state_engine,
        response_cache,
        config,
    });

    Router::new()
        .route("/*path", any(gateway_handler))
        .route("/", any(gateway_handler))
        .with_state(state)
}

pub async fn start(config: AppConfig) -> anyhow::Result<()> {
    let addr = format!("127.0.0.1:{}", config.port);
    let state_engine = Arc::new(Mutex::new(StateEngine::new()));
    let app = app(state_engine, config);

    let listener = TcpListener::bind(&addr).await?;
    info!("ReToken Gateway listening on {}", addr);
    
    axum::serve(listener, app).await?;
    Ok(())
}
