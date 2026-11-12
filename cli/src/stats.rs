use retoken_core::config::AppConfig;

pub fn run_stats() -> anyhow::Result<()> {
    println!("ReToken Telemetry & Configuration Overview");
    println!("===========================================");

    let config = AppConfig::load();
    println!("Runtime Settings:");
    println!("  Loopback Port: {}", config.port);
    println!("  Cache Enabled: {}", config.cache_enabled);
    println!("  Cache TTL: {}s", config.cache_ttl_secs);
    println!("  Max Cache Entries: {}", config.max_cache_entries);
    println!("  Token Budget: {}", config.token_budget);
    println!("  Terse Mode: {}", config.terse_mode);
    println!(
        "  Max Request Body Size: {} MB",
        config.max_request_body_bytes / (1024 * 1024)
    );
    println!("  Upstream Timeout: {}s", config.upstream_timeout_secs);
    println!(
        "  Private Upstreams Allowed: {}",
        config.allow_private_upstreams
    );
    println!();
    println!("Telemetry metrics are collected per request via FlightRecord in the gateway.");
    println!("Use `retoken doctor` to diagnose network and service reachability.");
    Ok(())
}
