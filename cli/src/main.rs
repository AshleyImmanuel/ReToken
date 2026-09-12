use retoken_core::telemetry;
use clap::{Parser, Subcommand};

pub mod wrap;
pub mod analyze;

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

    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => {
            tracing::info!("ReToken Agent Flight Recorder starting...");
            // For MVP, just use Default config, we will load this from a file later
            let config = retoken_core::config::AppConfig::default();
            gateway::start(config).await?;
        }
        Commands::Analyze { path } => {
            analyze::run_analyze(path)?;
        }
        Commands::Wrap { inject, command } => {
            wrap::run_wrap(inject, command).await?;
        }
    }
    
    Ok(())
}
