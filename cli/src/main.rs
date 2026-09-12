use core::telemetry;

fn main() -> anyhow::Result<()> {
    telemetry::init_tracing();
    tracing::info!("ReToken Agent Flight Recorder starting...");

    // Start gateway
    gateway::start();
    
    Ok(())
}
