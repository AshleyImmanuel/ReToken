use retoken_core::telemetry;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use analyzers::scanner::Scanner;
use analyzers::symbols::SymbolExtractor;
use analyzers::graph::DependencyGraph;

#[derive(Parser)]
#[command(name = "retoken")]
#[command(about = "ReToken Agent Runtime", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the ReToken gateway proxy (default)
    Run,
    /// Analyze a repository for intelligence
    Analyze {
        #[arg(short, long, default_value = ".")]
        path: String,
    },
    /// Wrap an agent or CLI tool, automatically routing its LLM traffic through ReToken
    Wrap {
        /// Optional extra environment variables to override with the proxy URL (e.g. OLLAMA_API_BASE)
        #[arg(short, long)]
        inject: Vec<String>,

        /// The command to wrap (e.g. `claude`, `aider`, `python script.py`)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init_tracing();
    let cli = Cli::parse();

    match &cli.command.unwrap_or(Commands::Run) {
        Commands::Run => {
            tracing::info!("ReToken Agent Flight Recorder starting...");
            // For MVP, just use Default config, we will load this from a file later
            let config = retoken_core::config::AppConfig::default();
            gateway::start(config).await?;
        }
        Commands::Analyze { path } => {
            tracing::info!("Analyzing repository at {}", path);
            let scanner = Scanner::new(PathBuf::from(path));
            let files = scanner.scan();
            tracing::info!("Discovered {} source files", files.len());
            
            let extractor = SymbolExtractor::new();
            let graph = DependencyGraph::build(files, &extractor);
            
            tracing::info!("Graph built with {} nodes", graph.files.len());
            for (file, node) in graph.files.iter().take(5) {
                tracing::info!("File: {:?}", file);
                tracing::info!("  Functions: {:?}", node.functions);
                tracing::info!("  Classes: {:?}", node.classes);
                tracing::info!("  Imports: {:?}", node.imports);
                tracing::info!("  Exports: {:?}", node.exports);
            }
        }
        Commands::Wrap { inject, command } => {
            if command.is_empty() {
                eprintln!("Error: wrap requires a command to execute (e.g., `retoken wrap claude`)");
                std::process::exit(1);
            }

            let proxy_addr = "127.0.0.1:8888"; // Wait, gateway is running on 8888 in code!
            let proxy_url = "http://127.0.0.1:8888/v1";

            if std::net::TcpStream::connect(proxy_addr).is_err() {
                tracing::info!("Gateway not running on {}. Spawning background proxy...", proxy_addr);
                let current_exe = std::env::current_exe().unwrap();
                let _daemon = std::process::Command::new(current_exe)
                    .arg("run")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .expect("Failed to spawn background proxy");
                
                // Wait for proxy to bind
                for _ in 0..20 {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    if std::net::TcpStream::connect(proxy_addr).is_ok() {
                        break;
                    }
                }
            }

            tracing::info!("Wrapping command: {:?}", command);
            let mut cmd = tokio::process::Command::new(&command[0]);
            
            cmd.args(&command[1..])
                .env("ANTHROPIC_BASE_URL", proxy_url)
                .env("OPENAI_BASE_URL", proxy_url)
                .env("OPENAI_API_BASE", proxy_url)
                .env("GEMINI_BASE_URL", proxy_url)
                .env("GOOGLE_GEMINI_BASE_URL", proxy_url)
                .env("LITELLM_BASE_URL", proxy_url)
                .env("GROQ_BASE_URL", proxy_url)
                .env("MISTRAL_API_BASE", proxy_url)
                .env("_CLAUDE_CODE_ASSUME_FIRST_PARTY_BASE_URL", "1"); // Fix for Claude Code 1M token context

            // Inject custom user-provided variables
            for var in inject {
                tracing::info!("Injecting custom env var: {}={}", var, proxy_url);
                cmd.env(&var, proxy_url);
            }

            let mut child = cmd.spawn().expect("Failed to spawn wrapped command");

            let status = child.wait().await.expect("Failed to wait on child process");
            std::process::exit(status.code().unwrap_or(1));
        }
    }
    
    Ok(())
}
