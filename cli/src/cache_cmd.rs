use crate::CacheCommands;
use retoken_core::config::AppConfig;
use std::time::Duration;

pub async fn run_cache(cmd: Option<CacheCommands>) -> anyhow::Result<()> {
    let config = AppConfig::load()?;
    let action = cmd.unwrap_or(CacheCommands::Status);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()?;
    let admin_url = format!("http://127.0.0.1:{}", config.port);

    match action {
        CacheCommands::Status => {
            println!("ReToken Cache Status");
            println!("====================");
            println!("  Cache Enabled: {}", config.cache_enabled);
            println!("  TTL: {}s", config.cache_ttl_secs);
            println!(
                "  Max Capacity: {} entries ({} MB limit)",
                config.max_cache_entries,
                config.max_cache_bytes / (1024 * 1024)
            );
            println!(
                "  Isolation Policy: SHA-256 context fingerprint + upstream base + context hash"
            );
            println!();

            // Query live daemon if reachable
            match client
                .get(format!("{}/_retoken/stats", admin_url))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        let hits = json.get("total_hits").and_then(|v| v.as_u64()).unwrap_or(0);
                        let misses = json
                            .get("total_misses")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        let total = hits + misses;
                        let hit_pct = if total > 0 {
                            (hits as f64 / total as f64) * 100.0
                        } else {
                            0.0
                        };
                        let cached_entries = json["cached_entries"].as_u64().unwrap_or(0);
                        let current_bytes = json["current_cache_bytes"].as_u64().unwrap_or(0);
                        let max_bytes = json["max_cache_bytes"]
                            .as_u64()
                            .unwrap_or(config.max_cache_bytes as u64);

                        println!("Live Gateway Cache State:");
                        println!(
                            "  Cached Entries: {} (Memory: {:.2} MB / {:.2} MB)",
                            cached_entries,
                            current_bytes as f64 / (1024.0 * 1024.0),
                            max_bytes as f64 / (1024.0 * 1024.0)
                        );
                        println!("  Total Hits: {}", hits);
                        println!("  Total Misses: {}", misses);
                        println!("  Hit Ratio: {:.1}%", hit_pct);
                    }
                }
                _ => {
                    println!(
                        "Live Gateway Cache State: Gateway offline on port {}",
                        config.port
                    );
                }
            }

            // Fetch active cached entry list
            if let Ok(resp) = client
                .get(format!("{}/_retoken/cache", admin_url))
                .send()
                .await
            {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    if let Some(entries) = json.get("entries").and_then(|e| e.as_array()) {
                        if !entries.is_empty() {
                            println!();
                            println!("Active Cached Entries (top {}):", entries.len().min(10));
                            for (i, entry) in entries.iter().take(10).enumerate() {
                                let key = entry
                                    .get("key")
                                    .and_then(|k| k.as_str())
                                    .unwrap_or("unknown");
                                let short_key = if key.len() > 16 { &key[..16] } else { key };
                                let age =
                                    entry.get("age_secs").and_then(|a| a.as_u64()).unwrap_or(0);
                                let hits =
                                    entry.get("hit_count").and_then(|h| h.as_u64()).unwrap_or(0);
                                let size = entry
                                    .get("size_bytes")
                                    .and_then(|s| s.as_u64())
                                    .unwrap_or(0);
                                println!(
                                    "  [{}] {}... | Age: {}s | Hits: {} | Size: {} B",
                                    i + 1,
                                    short_key,
                                    age,
                                    hits,
                                    size
                                );
                            }
                        }
                    }
                }
            }

            // Fetch active cached entry list
            if let Ok(resp) = client
                .get(format!("{}/_retoken/cache", admin_url))
                .send()
                .await
            {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    if let Some(entries) = json.get("entries").and_then(|e| e.as_array()) {
                        if !entries.is_empty() {
                            println!();
                            println!("Active Cached Entries (top {}):", entries.len().min(10));
                            for (i, entry) in entries.iter().take(10).enumerate() {
                                let key = entry
                                    .get("key")
                                    .and_then(|k| k.as_str())
                                    .unwrap_or("unknown");
                                let short_key = if key.len() > 16 { &key[..16] } else { key };
                                let age =
                                    entry.get("age_secs").and_then(|a| a.as_u64()).unwrap_or(0);
                                let hits =
                                    entry.get("hit_count").and_then(|h| h.as_u64()).unwrap_or(0);
                                let size = entry
                                    .get("size_bytes")
                                    .and_then(|s| s.as_u64())
                                    .unwrap_or(0);
                                println!(
                                    "  [{}] {}... | Age: {}s | Hits: {} | Size: {} B",
                                    i + 1,
                                    short_key,
                                    age,
                                    hits,
                                    size
                                );
                            }
                        }
                    }
                }
            }
        }
        CacheCommands::Clear => {
            let clear_url = format!("{}/_retoken/cache/clear", admin_url);
            match client.post(&clear_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        let cleared = json
                            .get("cleared_entries")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        println!(
                            "[OK] Live gateway cache cleared (purged {} in-memory entries).",
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

            // Also clear persistent CCR storage if present
            let ccr_dir = compressors::ccr::paths::dirs_or_default().join("ccr");
            if ccr_dir.exists() {
                match std::fs::remove_dir_all(&ccr_dir) {
                    Ok(_) => {
                        let _ = std::fs::create_dir_all(&ccr_dir);
                        println!("[OK] Persistent CCR storage purged ({})", ccr_dir.display());
                    }
                    Err(e) => {
                        println!(
                            "[WARN] Failed to purge persistent CCR storage ({}): {}",
                            ccr_dir.display(),
                            e
                        );
                    }
                }
            }
        }
        CacheCommands::Invalidate { key } => {
            let inv_url = format!("{}/_retoken/cache?key={}", admin_url, key);
            match client.delete(&inv_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    println!("[OK] Cache entry '{}' successfully invalidated.", key);
                }
                Ok(resp) => {
                    println!("[WARN] Invalidation returned status HTTP {}", resp.status());
                }
                Err(e) => {
                    println!(
                        "[ERROR] Failed to connect to gateway at {}: {}",
                        admin_url, e
                    );
                }
            }
        }
    }

    Ok(())
}
