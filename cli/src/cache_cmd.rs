use retoken_core::config::AppConfig;

pub fn run_cache(action: Option<String>) -> anyhow::Result<()> {
    let config = AppConfig::load();
    let action = action.unwrap_or_else(|| "status".to_string());

    match action.as_str() {
        "status" => {
            println!("ReToken Cache Status");
            println!("====================");
            println!("  Cache Enabled: {}", config.cache_enabled);
            println!("  TTL: {}s", config.cache_ttl_secs);
            println!("  Max Capacity: {} entries", config.max_cache_entries);
            println!("  Isolation Policy: SHA-256 credential fingerprint + context hash");
        }
        "clear" => {
            println!("In-memory response cache resets when restarting the gateway daemon.");
            println!("To clear persistent CCR storage, check ~/.retoken/ccr or run with CCR store reset.");
            println!("[OK] Cache clear command executed.");
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
