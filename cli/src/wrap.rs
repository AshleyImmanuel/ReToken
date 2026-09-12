use retoken_core::config::AppConfig;

pub async fn run_wrap(inject: Vec<String>, command: Vec<String>) -> anyhow::Result<()> {
    if command.is_empty() {
        eprintln!("Error: wrap requires a command to execute (e.g., `retoken wrap claude`)");
        std::process::exit(1);
    }

    let config = AppConfig::default();
    let proxy_addr = format!("127.0.0.1:{}", config.port);
    let proxy_url = format!("http://127.0.0.1:{}/v1", config.port);

    if std::net::TcpStream::connect(&proxy_addr).is_err() {
        tracing::info!("Gateway not running on {}. Spawning background proxy...", proxy_addr);
        let current_exe = std::env::current_exe().unwrap();
        let mut daemon_cmd = std::process::Command::new(current_exe);
        daemon_cmd.arg("run")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            daemon_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        
        #[allow(clippy::zombie_processes)]
        let _ = daemon_cmd.spawn().expect("Failed to spawn background proxy");
        
        // Wait for proxy to bind
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if std::net::TcpStream::connect(&proxy_addr).is_ok() {
                break;
            }
        }
    }

    tracing::info!("Wrapping command: {:?}", command);
    let mut cmd = tokio::process::Command::new(&command[0]);
    
    cmd.args(&command[1..])
        .env("ANTHROPIC_BASE_URL", &proxy_url)
        .env("OPENAI_BASE_URL", &proxy_url)
        .env("OPENAI_API_BASE", &proxy_url)
        .env("GEMINI_BASE_URL", &proxy_url)
        .env("GOOGLE_GEMINI_BASE_URL", &proxy_url)
        .env("LITELLM_BASE_URL", &proxy_url)
        .env("GROQ_BASE_URL", &proxy_url)
        .env("MISTRAL_API_BASE", &proxy_url)
        .env("_CLAUDE_CODE_ASSUME_FIRST_PARTY_BASE_URL", "1"); // Fix for Claude Code 1M token context

    // Inject custom user-provided variables
    for var in inject {
        tracing::info!("Injecting custom env var: {}={}", var, proxy_url);
        cmd.env(var, &proxy_url);
    }

    let mut child = cmd.spawn().expect("Failed to spawn wrapped command");

    let status = child.wait().await.expect("Failed to wait on child process");
    std::process::exit(status.code().unwrap_or(1));
}
