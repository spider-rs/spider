extern crate env_logger;

use clap::Parser;
use rmcp::ServiceExt;

mod server;
mod state;
mod tools;

use server::SpiderMcpServer;

#[derive(Parser)]
#[command(name = "spider-mcp", about = "MCP server for Spider web crawler")]
struct Cli {
    /// Log level (default: warn). Logs go to stderr.
    #[arg(long, default_value = "warn")]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    env_logger::Builder::new()
        .parse_filters(&cli.log_level)
        .target(env_logger::Target::Stderr)
        .init();

    let server = SpiderMcpServer::new();
    let transport = (tokio::io::stdin(), tokio::io::stdout());

    server.serve(transport).await?.waiting().await?;

    Ok(())
}
