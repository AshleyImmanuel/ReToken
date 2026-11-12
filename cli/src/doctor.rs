use retoken_core::config::AppConfig;
use std::net::{SocketAddr, TcpStream};
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
    let proxy_addr_str = format!("127.0.0.1:{}", config.port);
    match proxy_addr_str.parse::<SocketAddr>() {
        Ok(addr) => match TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
            Ok(_) => {
                println!(
                    "[OK] Local Gateway: ACTIVE (listening on http://{})",
                    proxy_addr_str
                );
            }
            Err(_) => {
                println!(
                    "[INFO] Local Gateway: NOT RUNNING (start with `retoken run` or use `retoken wrap`)"
                );
            }
        },
        Err(e) => {
            println!(
                "[ERROR] Invalid local gateway address '{}': {}",
                proxy_addr_str, e
            );
        }
    }

    // 2. Upstream provider connectivity
    println!("Provider Connectivity Check:");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let upstreams_to_check = vec![
        ("Anthropic API", config.anthropic_base_url.clone()),
        ("OpenAI API", config.openai_base_url.clone()),
    ];

    for (name, url) in upstreams_to_check {
        match client.get(&url).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() || status.is_redirection() {
                    println!("  [OK] {} ({}) - HTTP {} (Healthy)", name, url, status);
                } else if status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                    || status == reqwest::StatusCode::NOT_FOUND
                {
                    println!(
                        "  [OK] {} ({}) - HTTP {} (Reachable, provider auth/route responded)",
                        name, url, status
                    );
                } else if status.is_server_error() {
                    println!(
                        "  [WARN] {} ({}) - HTTP {} (Provider server error)",
                        name, url, status
                    );
                } else {
                    println!("  [OK] {} ({}) - HTTP {}", name, url, status);
                }
            }
            Err(e) => {
                println!("  [WARN] {} ({}) check failed: {}", name, url, e);
            }
        }
    }

    // Check custom configured upstreams if present in allowed_upstream_hosts
    for host in &config.allowed_upstream_hosts {
        if host.starts_with("*.") {
            continue;
        }
        let custom_url = format!("https://{}", host);
        if custom_url != config.anthropic_base_url && custom_url != config.openai_base_url {
            match client.get(&custom_url).send().await {
                Ok(resp) => {
                    println!(
                        "  [OK] Custom Upstream ({}) - HTTP {}",
                        custom_url,
                        resp.status()
                    );
                }
                Err(e) => {
                    println!("  [INFO] Custom Upstream ({}) probe: {}", custom_url, e);
                }
            }
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
