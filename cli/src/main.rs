use retoken_core::telemetry;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init_tracing();
    tracing::info!("ReToken Agent Flight Recorder starting...");

    // Start gateway
    gateway::start().await?;
    
    Ok(())
}
