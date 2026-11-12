use clap::{Parser, Subcommand};
use retoken_core::telemetry;

pub mod analyze;
pub mod cache_cmd;
pub mod doctor;
pub mod init;
pub mod stats;
pub mod wrap;

#[derive(Parser, Debug, PartialEq)]
#[command(name = "retoken")]
#[command(about = "ReToken Agent Runtime", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum Commands {
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
    /// Inspect, clear, or invalidate local cache
    Cache {
        #[command(subcommand)]
        command: Option<CacheCommands>,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum CacheCommands {
    /// Show cache status, hit ratios, and active entries (default)
    Status,
    /// Clear all cached responses from memory and persistent store
    Clear,
    /// Invalidate a specific cached entry by its key or hash
    Invalidate {
        /// The cache key to invalidate
        key: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init_tracing();
    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Run) {
        Commands::Run => {
            tracing::info!("ReToken Agent Flight Recorder starting...");
            let config = retoken_core::config::AppConfig::load()?;
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
        Commands::Cache { command } => {
            cache_cmd::run_cache(command).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_cache_subcommands_parsing() {
        let cli_default = Cli::try_parse_from(["retoken", "cache"]).unwrap();
        assert_eq!(cli_default.command, Some(Commands::Cache { command: None }));

        let cli_status = Cli::try_parse_from(["retoken", "cache", "status"]).unwrap();
        assert_eq!(
            cli_status.command,
            Some(Commands::Cache {
                command: Some(CacheCommands::Status)
            })
        );

        let cli_clear = Cli::try_parse_from(["retoken", "cache", "clear"]).unwrap();
        assert_eq!(
            cli_clear.command,
            Some(Commands::Cache {
                command: Some(CacheCommands::Clear)
            })
        );

        let cli_inv =
            Cli::try_parse_from(["retoken", "cache", "invalidate", "0123456789abcdef"]).unwrap();
        assert_eq!(
            cli_inv.command,
            Some(Commands::Cache {
                command: Some(CacheCommands::Invalidate {
                    key: "0123456789abcdef".to_string()
                })
            })
        );
    }

    #[test]
    fn test_cli_wrap_parsing() {
        let cli_wrap = Cli::try_parse_from(["retoken", "wrap", "claude", "--print"]).unwrap();
        match cli_wrap.command.unwrap() {
            Commands::Wrap { inject, command } => {
                assert!(inject.is_empty());
                assert_eq!(command, vec!["claude", "--print"]);
            }
            _ => panic!("Expected Wrap command"),
        }
    }
}
