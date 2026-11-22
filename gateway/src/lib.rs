use axum::{routing::any, Router};
use compressors::ccr::CcrStore;
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::info;

use retoken_core::cache::ResponseCache;
use retoken_core::config::AppConfig;

pub mod bypass;
pub mod handler;
mod host_guard;
pub mod interceptor;
pub mod optimizer;
pub mod provider;
pub mod stream_usage;

use handler::{handler as gateway_handler, GatewayState};

/// Builds the single upstream HTTP client shared by all requests, so connections are pooled.
///
/// `upstream_timeout_secs` bounds connecting and each idle gap between body reads, not the
/// whole request: a total timeout would cut off long-running streamed generations.
/// Unless private upstreams are allowed, DNS resolution goes through [`provider::SsrfResolver`],
/// which enforces the SSRF policy on the exact addresses that get dialed.
pub fn build_upstream_client(config: &AppConfig) -> reqwest::Result<Client> {
    let timeout = Duration::from_secs(config.upstream_timeout_secs);
    let mut builder = Client::builder()
        .connect_timeout(timeout)
        .read_timeout(timeout)
        .redirect(reqwest::redirect::Policy::none());
    if !config.allow_private_upstreams {
        builder = builder.dns_resolver(Arc::new(provider::SsrfResolver));
    }
    builder.build()
}

/// Builds the gateway router for the given configuration and CCR store.
pub fn app(config: AppConfig, ccr_store: Arc<CcrStore>) -> anyhow::Result<Router> {
    let client = build_upstream_client(&config)?;
    let response_cache = Arc::new(Mutex::new(ResponseCache::new(
        config.cache_ttl_secs,
        config.max_cache_entries,
        config.max_cache_bytes,
    )));
    let telemetry = Arc::new(Mutex::new(retoken_core::FlightTelemetry::new()));

    let state = Arc::new(GatewayState {
        client,
        response_cache,
        config,
        ccr_store,
        start_time: std::time::Instant::now(),
        telemetry,
    });

    Ok(Router::new()
        .route(
            "/_retoken/cache/clear",
            axum::routing::post(handler::admin::admin_cache_clear),
        )
        .route(
            "/_retoken/cache",
            axum::routing::get(handler::admin::admin_cache_list)
                .delete(handler::admin::admin_cache_invalidate),
        )
        .route(
            "/_retoken/stats",
            axum::routing::get(handler::admin::admin_stats),
        )
        .route("/*path", any(gateway_handler))
        .route("/", any(gateway_handler))
        .layer(axum::middleware::from_fn(host_guard::require_loopback_host))
        .with_state(state))
}

/// CCR originals no conversation has re-stored for this long are purged. Every compression
/// re-stores (and so refreshes) the originals a live conversation still references.
const CCR_MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60;

/// How often the CCR store is garbage-collected. The daemon runs for weeks from logon, so
/// collecting only at startup would let the store grow without bound.
const CCR_GC_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Collects stale CCR entries now and then every [`CCR_GC_INTERVAL`], off the async runtime.
fn spawn_ccr_gc(store: Arc<CcrStore>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(CCR_GC_INTERVAL);
        loop {
            tick.tick().await;
            let store = store.clone();
            if let Err(e) = tokio::task::spawn_blocking(move || store.gc(CCR_MAX_AGE_SECS)).await {
                tracing::warn!("CCR gc task failed: {}", e);
            }
        }
    });
}

pub async fn start(config: AppConfig) -> anyhow::Result<()> {
    let addr = format!("127.0.0.1:{}", config.port);
    let ccr_store = match CcrStore::open(None) {
        Ok(store) => {
            let store = Arc::new(store);
            spawn_ccr_gc(store.clone());
            store
        }
        Err(e) => {
            tracing::warn!(
                "Failed to open persistent CCR store: {}. Falling back to in-memory store.",
                e
            );
            Arc::new(CcrStore::open_in_memory().map_err(anyhow::Error::msg)?)
        }
    };
    let app = app(config, ccr_store)?;

    let listener = TcpListener::bind(&addr).await?;
    info!("ReToken Gateway listening on {}", addr);

    axum::serve(listener, app).await?;
    Ok(())
}
