use retoken_core::config::AppConfig;
use std::net::TcpStream;
use std::time::Duration;

pub async fn run_doctor() -> anyhow::Result<()> {
    println!("ReToken Diagnostic Doctor");
    println!("==========================");

    let config = AppConfig::load();
    println!("Configuration:");
    println!("  Port: {}", config.port);
    println!("  Cache Enabled: {}", config.cache_enabled);
    println!("  Cache TTL: {}s", config.cache_ttl_secs);
    println!("  Max Cache Entries: {}", config.max_cache_entries);
    println!(
        "  Max Request Body: {} bytes",
        config.max_request_body_bytes
    );
    println!(
        "  Allow Private Upstreams: {}",
        config.allow_private_upstreams
    );
    println!("  Upstream Timeout: {}s", config.upstream_timeout_secs);
    if !config.allowed_upstream_hosts.is_empty() {
        println!("  Allowed Upstreams: {:?}", config.allowed_upstream_hosts);
    }
    println!();

    // 1. Check local gateway status
    let proxy_addr = format!("127.0.0.1:{}", config.port);
    match TcpStream::connect_timeout(&proxy_addr.parse().unwrap(), Duration::from_millis(500)) {
        Ok(_) => {
            println!(
                "[OK] Local Gateway: ACTIVE (listening on http://{})",
                proxy_addr
            );
        }
        Err(_) => {
            println!("[INFO] Local Gateway: NOT RUNNING (start with `retoken run` or use `retoken wrap`)");
        }
    }

    // 2. Upstream provider connectivity
    println!("Provider Connectivity Check:");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    match client.get("https://api.anthropic.com").send().await {
        Ok(resp) => {
            println!(
                "  [OK] Anthropic API (https://api.anthropic.com) - HTTP {}",
                resp.status()
            );
        }
        Err(e) => {
            println!("  [WARN] Anthropic API check failed: {}", e);
        }
    }

    match client.get("https://api.openai.com").send().await {
        Ok(resp) => {
            println!(
                "  [OK] OpenAI API (https://api.openai.com) - HTTP {}",
                resp.status()
            );
        }
        Err(e) => {
            println!("  [WARN] OpenAI API check failed: {}", e);
        }
    }

    // 3. Git workspace check
    let in_git = std::path::Path::new(".git").exists();
    if in_git {
        println!("[OK] Git Repository: DETECTED");
    } else {
        println!("[INFO] Git Repository: Not at root of git repository");
    }

    println!();
    println!("Doctor diagnosis complete.");
    Ok(())
}
