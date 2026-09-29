use std::sync::Arc;
use tokio::sync::Mutex;
use retoken_core::state::{StateEngine, StateType};
use tracing::info;

/// Inserts the payload into the context state engine asynchronously.
pub async fn insert_into_state_engine(
    state_engine: &Arc<Mutex<StateEngine>>,
    final_bytes: &[u8],
    target_url: String,
) {
    let mut engine = state_engine.lock().await;
    let obj = engine.insert_or_update(
        StateType::Generic("GatewayRequest".to_string()),
        final_bytes,
        Some(target_url),
    );
    info!("State: {}", obj.object_id);
}
