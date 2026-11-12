use retoken_core::config::AppConfig;
use std::time::Duration;

pub async fn run_cache(action: Option<String>) -> anyhow::Result<()> {
    let config = AppConfig::load();
    let action = action.unwrap_or_else(|| "status".to_string());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()?;
    let admin_url = format!("http://127.0.0.1:{}", config.port);

    match action.as_str() {
        "status" => {
            println!("ReToken Cache Status");
            println!("====================");
            println!("  Cache Enabled: {}", config.cache_enabled);
            println!("  TTL: {}s", config.cache_ttl_secs);
            println!("  Max Capacity: {} entries", config.max_cache_entries);
            println!(
                "  Isolation Policy: SHA-256 context fingerprint + upstream base + context hash"
            );

            // Query live daemon if reachable
            match client
                .get(format!("{}/_retoken/stats", admin_url))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        println!("Live Gateway Cache State:");
                        println!("  Cached Entries: {}", json["cached_entries"]);
                        println!("  Total Hits: {}", json["total_hits"]);
                        println!("  Total Misses: {}", json["total_misses"]);
                    }
                }
                _ => {
                    println!(
                        "Live Gateway Cache State: Gateway offline on port {}",
                        config.port
                    );
                }
            }
        }
        "clear" => {
            let clear_url = format!("{}/_retoken/cache/clear", admin_url);
            match client.post(&clear_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        let cleared = json
                            .get("cleared_entries")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        println!(
                            "[OK] Live gateway cache cleared (purged {} entries).",
                            cleared
                        );
                    } else {
                        println!("[OK] Live gateway cache cleared.");
                    }
                }
                _ => {
                    println!(
                        "[INFO] Gateway daemon is not running on 127.0.0.1:{}. In-memory cache is currently empty.",
                        config.port
                    );
                }
            }
        }
        other => {
            println!(
                "Unknown cache action: {}. Supported actions: status, clear",
                other
            );
        }
    }

    Ok(())
}
