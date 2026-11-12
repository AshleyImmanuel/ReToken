use clap::{Parser, Subcommand};
use retoken_core::telemetry;

pub mod analyze;
pub mod cache_cmd;
pub mod doctor;
pub mod init;
pub mod stats;
pub mod wrap;

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
    /// Initialize agent configurations (.claudecode, SKILL.md) in the current directory
    Init,
    /// Wrap an agent or CLI tool, automatically routing its LLM traffic through ReToken
    Wrap {
        /// Optional extra environment variables to override with the proxy URL (e.g. OLLAMA_API_BASE)
        #[arg(short, long)]
        inject: Vec<String>,

        /// The command to wrap (e.g. `claude`, `aider`, `python script.py`)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Run diagnostic checks on local gateway, providers, and environment
    Doctor,
    /// Display telemetry overview and runtime configuration
    Stats,
    /// Inspect or clear local cache
    Cache {
        /// Action to perform: status, clear
        action: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init_tracing();
    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => {
            tracing::info!("ReToken Agent Flight Recorder starting...");
            let config = retoken_core::config::AppConfig::load();
            gateway::start(config).await?;
        }
        Commands::Analyze { path } => {
            analyze::run_analyze(path)?;
        }
        Commands::Init => {
            init::run_init()?;
        }
        Commands::Wrap { inject, command } => {
            wrap::run_wrap(inject, command).await?;
        }
        Commands::Doctor => {
            doctor::run_doctor().await?;
        }
        Commands::Stats => {
            stats::run_stats().await?;
        }
        Commands::Cache { action } => {
            cache_cmd::run_cache(action).await?;
        }
    }

    Ok(())
}
