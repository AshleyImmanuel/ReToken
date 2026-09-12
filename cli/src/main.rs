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
    }
    
    Ok(())
}
