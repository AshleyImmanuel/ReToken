use anyhow::Context;
use retoken_core::config::AppConfig;

/// Base-URL variables pointed at the proxy's `/v1` root for the wrapped command: SDKs for the
/// Anthropic and OpenAI wire formats.
const PROXY_BASE_URL_VARS: &[&str] = &[
    "ANTHROPIC_BASE_URL",
    "OPENAI_BASE_URL",
    "OPENAI_API_BASE",
    "LITELLM_BASE_URL",
    "GROQ_BASE_URL",
    "MISTRAL_API_BASE",
];

/// Gemini base-URL variable (read by the Gemini SDKs and the Antigravity CLI). Gemini clients
/// append `/v1beta/...` themselves, so it gets the proxy root without `/v1`.
const GEMINI_BASE_URL_VAR: &str = "GOOGLE_GEMINI_BASE_URL";

/// Gemini clients are redirected only when they use an API key. Google sign-in (Code Assist)
/// traffic is a different API that the proxy rejects, and Google's terms reportedly forbid
/// routing that sign-in through third-party tools.
fn redirect_gemini(gemini_api_key: Option<&str>) -> bool {
    gemini_api_key.is_some_and(|k| !k.trim().is_empty())
}

pub async fn run_wrap(inject: Vec<String>, command: Vec<String>) -> anyhow::Result<()> {
    if command.is_empty() {
        anyhow::bail!("wrap requires a command to execute (e.g. `retoken wrap claude`)");
    }

    let config = AppConfig::load()?;
    let proxy_addr = format!("127.0.0.1:{}", config.port);
    let proxy_url = format!("http://127.0.0.1:{}/v1", config.port);

    if std::net::TcpStream::connect(&proxy_addr).is_err() {
        tracing::info!(
            "Gateway not running on {}. Spawning background proxy...",
            proxy_addr
        );
        let current_exe =
            std::env::current_exe().context("Failed to determine current executable path")?;
        let mut daemon_cmd = std::process::Command::new(current_exe);
        daemon_cmd
            .arg("run")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            daemon_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        #[allow(clippy::zombie_processes)]
        let _ = daemon_cmd.spawn().with_context(|| {
            format!("Failed to spawn background proxy daemon on {}", proxy_addr)
        })?;

        // Wait for proxy to bind and verify readiness
        let mut ready = false;
        for _ in 0..30 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if std::net::TcpStream::connect(&proxy_addr).is_ok() {
                ready = true;
                break;
            }
        }

        if !ready {
            anyhow::bail!(
                "Background proxy failed to start and bind on {} within 3 seconds. Check logs or start the gateway manually with `retoken run`.",
                proxy_addr
            );
        }
    }

    tracing::info!("Wrapping command: {:?}", command);
    let mut cmd = tokio::process::Command::new(&command[0]);

    cmd.args(&command[1..]);
    for var in PROXY_BASE_URL_VARS {
        cmd.env(var, &proxy_url);
    }
    // Keeps Claude Code's first-party features (prompt caching, tool search, long context)
    // when its base URL is local.
    cmd.env("_CLAUDE_CODE_ASSUME_FIRST_PARTY_BASE_URL", "1");
    if redirect_gemini(std::env::var("GEMINI_API_KEY").ok().as_deref()) {
        cmd.env(
            GEMINI_BASE_URL_VAR,
            format!("http://127.0.0.1:{}", config.port),
        );
    }

    // Inject custom user-provided variables
    for var in inject {
        tracing::info!("Injecting custom env var: {}={}", var, proxy_url);
        cmd.env(var, &proxy_url);
    }

    let mut child = cmd.spawn().with_context(|| {
        format!(
            "Failed to spawn wrapped command '{}'. Verify that the command is installed and available in your PATH.",
            command[0]
        )
    })?;

    let status = child
        .wait()
        .await
        .context("Failed while waiting on wrapped child process")?;
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use super::{redirect_gemini, PROXY_BASE_URL_VARS};

    #[test]
    fn gemini_is_not_in_the_v1_variables() {
        // Gemini clients need the proxy root, not `/v1`, so they are handled separately.
        assert!(PROXY_BASE_URL_VARS.iter().all(|v| !v.contains("GEMINI")));
        assert!(PROXY_BASE_URL_VARS.contains(&"ANTHROPIC_BASE_URL"));
        assert!(PROXY_BASE_URL_VARS.contains(&"OPENAI_BASE_URL"));
    }

    #[test]
    fn gemini_is_redirected_only_with_an_api_key() {
        assert!(redirect_gemini(Some("AIza-test")));
        assert!(!redirect_gemini(Some("  ")));
        assert!(!redirect_gemini(None));
    }
}
