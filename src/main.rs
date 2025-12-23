use clap::Parser;
use dotenvy::dotenv;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod error;
mod gemini;
mod mcp;
mod tools;

#[derive(Parser, Debug)]
#[command(name = "gemini-mcp")]
#[command(about = "MCP server for Gemini integration (Rust)")]
#[command(version)]
struct Cli {
    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    /// Run in quiet mode
    #[arg(short, long)]
    quiet: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();

    let cli = Cli::parse();

    // Setup logging
    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else if cli.quiet {
        EnvFilter::new("error")
    } else {
        EnvFilter::new("info")
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    info!("Starting Gemini MCP Server (Rust) v{}", env!("CARGO_PKG_VERSION"));

    // Get API key
    let api_key = std::env::var("GEMINI_API_KEY").map_err(|_| {
        anyhow::anyhow!("GEMINI_API_KEY environment variable required")
    })?;

    // Create server
    let server = mcp::server::McpGeminiServer::new(api_key)?;

    // Test connection
    server.test_connection().await?;

    // Run server
    server.run().await?;

    Ok(())
}
