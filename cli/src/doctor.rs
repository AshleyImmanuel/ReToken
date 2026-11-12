use retoken_core::config::AppConfig;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

pub async fn run_doctor() -> anyhow::Result<()> {
    println!("ReToken Diagnostic Doctor");
    println!("==========================");

    // 1. Configuration Validation
    let config = match AppConfig::load() {
        Ok(cfg) => {
            println!("[OK] Configuration: VALID");
            println!("  Port: {}", cfg.port);
            println!("  Cache Enabled: {}", cfg.cache_enabled);
            println!("  Cache TTL: {}s", cfg.cache_ttl_secs);
            println!("  Max Cache Entries: {}", cfg.max_cache_entries);
            println!("  Max Request Body: {} bytes", cfg.max_request_body_bytes);
            println!("  Allow Private Upstreams: {}", cfg.allow_private_upstreams);
            println!("  Upstream Timeout: {}s", cfg.upstream_timeout_secs);
            if !cfg.allowed_upstream_hosts.is_empty() {
                println!("  Allowed Upstreams: {:?}", cfg.allowed_upstream_hosts);
            }
            cfg
        }
        Err(e) => {
            println!("[ERROR] Configuration: MALFORMED - {}", e);
            println!(
                "Fix the configuration error in retoken.json or remove the file to use defaults."
            );
            return Err(e);
        }
    };
    println!();

    // 2. Check local gateway status & ping latency
    let proxy_addr_str = format!("127.0.0.1:{}", config.port);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    match proxy_addr_str.parse::<SocketAddr>() {
        Ok(_) => {
            let start = Instant::now();
            let stats_url = format!("http://{}/_retoken/stats", proxy_addr_str);
            match client.get(&stats_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    let elapsed = start.elapsed();
                    println!(
                        "[OK] Local Gateway: ACTIVE (listening on http://{}, ping: {:.2}ms)",
                        proxy_addr_str,
                        elapsed.as_micros() as f64 / 1000.0
                    );
                }
                _ => {
                    println!(
                        "[INFO] Local Gateway: NOT RUNNING (start with `retoken run` or use `retoken wrap`)"
                    );
                }
            }
        }
        Err(e) => {
            println!(
                "[ERROR] Invalid local gateway address '{}': {}",
                proxy_addr_str, e
            );
        }
    }

    // 3. Upstream provider connectivity with latency timing
    println!("Provider Connectivity Check:");
    let upstreams_to_check = vec![
        ("Anthropic API", config.anthropic_base_url.clone()),
        ("OpenAI API", config.openai_base_url.clone()),
    ];

    for (name, url) in upstreams_to_check {
        let start = Instant::now();
        match client.get(&url).send().await {
            Ok(resp) => {
                let latency_ms = start.elapsed().as_millis();
                let status = resp.status();
                if status.is_success() || status.is_redirection() {
                    println!(
                        "  [OK] {} ({}) - HTTP {} ({}ms, Healthy)",
                        name, url, status, latency_ms
                    );
                } else if status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                    || status == reqwest::StatusCode::NOT_FOUND
                {
                    println!(
                        "  [OK] {} ({}) - HTTP {} ({}ms, Reachable, provider auth/route responded)",
                        name, url, status, latency_ms
                    );
                } else if status.is_server_error() {
                    println!(
                        "  [WARN] {} ({}) - HTTP {} ({}ms, Provider server error)",
                        name, url, status, latency_ms
                    );
                } else {
                    println!(
                        "  [OK] {} ({}) - HTTP {} ({}ms)",
                        name, url, status, latency_ms
                    );
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
            let start = Instant::now();
            match client.get(&custom_url).send().await {
                Ok(resp) => {
                    let latency_ms = start.elapsed().as_millis();
                    println!(
                        "  [OK] Custom Upstream ({}) - HTTP {} ({}ms)",
                        custom_url,
                        resp.status(),
                        latency_ms
                    );
                }
                Err(e) => {
                    println!("  [INFO] Custom Upstream ({}) probe: {}", custom_url, e);
                }
            }
        }
    }

    // 4. CCR persistent storage check
    let ccr_dir = compressors::ccr::paths::dirs_or_default().join("ccr");
    match std::fs::create_dir_all(&ccr_dir) {
        Ok(_) => {
            println!("[OK] CCR Store: ACCESSIBLE ({})", ccr_dir.display());
        }
        Err(e) => {
            println!(
                "[WARN] CCR Store directory unreachable ({}): {}",
                ccr_dir.display(),
                e
            );
        }
    }

    // 5. Git workspace check
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
