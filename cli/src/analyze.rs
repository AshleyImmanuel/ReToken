use std::path::PathBuf;
use analyzers::scanner::Scanner;
use analyzers::symbols::SymbolExtractor;
use analyzers::graph::DependencyGraph;

pub fn run_analyze(path: String) -> anyhow::Result<()> {
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
    
    Ok(())
}
