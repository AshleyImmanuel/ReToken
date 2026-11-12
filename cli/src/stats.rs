use retoken_core::config::AppConfig;
use std::time::Duration;

pub async fn run_stats() -> anyhow::Result<()> {
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
    if !config.allowed_upstream_hosts.is_empty() {
        println!(
            "  Allowed Upstream Hosts: {:?}",
            config.allowed_upstream_hosts
        );
    }
    println!();

    // Query live gateway daemon for runtime telemetry
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()?;
    let stats_url = format!("http://127.0.0.1:{}/_retoken/stats", config.port);

    match client.get(&stats_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                let hits = json.get("total_hits").and_then(|v| v.as_u64()).unwrap_or(0);
                let misses = json
                    .get("total_misses")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let total_reqs = hits + misses;
                let hit_ratio = if total_reqs > 0 {
                    (hits as f64 / total_reqs as f64) * 100.0
                } else {
                    0.0
                };
                let cached = json
                    .get("cached_entries")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let max_cached = json
                    .get("max_cache_entries")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(config.max_cache_entries as u64);
                let uptime = json
                    .get("uptime_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                println!("Live Gateway Telemetry (http://127.0.0.1:{}):", config.port);
                println!("  Status: ACTIVE");
                println!("  Uptime: {}s", uptime);
                println!("  Cache Utilization: {} / {} entries", cached, max_cached);
                println!("  Cache Hits: {}", hits);
                println!("  Cache Misses: {}", misses);
                println!("  Cache Hit Ratio: {:.1}%", hit_ratio);
            }
        }
        _ => {
            println!(
                "Live Gateway Telemetry: OFFLINE (gateway not currently running on port {})",
                config.port
            );
            println!(
                "  Start the gateway with `retoken run` or wrap your tool with `retoken wrap`."
            );
        }
    }

    println!();
    println!("Use `retoken doctor` to diagnose network and upstream service reachability.");
    Ok(())
}
