use axum::{routing::any, Router};
use compressors::ccr::CcrStore;
use reqwest::Client;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::info;

use retoken_core::cache::ResponseCache;
use retoken_core::config::AppConfig;
use retoken_core::state::StateEngine;

pub mod bypass;
pub mod handler;
pub mod interceptor;
pub mod optimizer;
pub mod provider;

use handler::{handler as gateway_handler, GatewayState};

pub fn app_with_client(
    client: Client,
    state_engine: Arc<Mutex<StateEngine>>,
    config: AppConfig,
    ccr_store: Arc<CcrStore>,
) -> Router {
    let response_cache = Arc::new(Mutex::new(ResponseCache::new(
        config.cache_ttl_secs,
        config.max_cache_entries,
    )));

    let state = Arc::new(GatewayState {
        client,
        state_engine,
        response_cache,
        config,
        ccr_store,
        start_time: std::time::Instant::now(),
    });

    Router::new()
        .route(
            "/_retoken/cache/clear",
            axum::routing::post(handler::admin::admin_cache_clear),
        )
        .route(
            "/_retoken/stats",
            axum::routing::get(handler::admin::admin_stats),
        )
        .route("/*path", any(gateway_handler))
        .route("/", any(gateway_handler))
        .with_state(state)
}

pub fn app(state_engine: Arc<Mutex<StateEngine>>, config: AppConfig) -> Router {
    let ccr_store = match CcrStore::open(None) {
        Ok(store) => Arc::new(store),
        Err(e) => {
            tracing::warn!(
                "Failed to open persistent CCR store: {}. Falling back to in-memory store.",
                e
            );
            Arc::new(CcrStore::open_in_memory().expect("in-memory CCR store must not fail"))
        }
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(config.upstream_timeout_secs))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    app_with_client(client, state_engine, config, ccr_store)
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
