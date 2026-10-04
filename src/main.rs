use anyhow::Context;
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

use vinyldns_mcp::{client::VinylDnsClient, config::Config, server::VinylDnsServer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("vinyldns-mcp {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // stdout carries the MCP protocol, so all logging goes to stderr.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_env("VINYLDNS_MCP_LOG").unwrap_or_else(|_| EnvFilter::new("info")))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let config = Config::from_env().context("invalid configuration (see docs/CONFIGURATION.md)")?;
    if config.is_insecure_remote() {
        tracing::warn!(url = %config.api_url, "VINYLDNS_API_URL uses plain http to a non-local host; requests are signed but not encrypted");
    }
    tracing::info!(
        url = %config.api_url,
        writes_enabled = config.enable_writes,
        confirmation = ?config.confirmation,
        "starting vinyldns-mcp {}",
        env!("CARGO_PKG_VERSION")
    );

    let client = VinylDnsClient::new(&config).context("failed to build HTTP client")?;
    let service = VinylDnsServer::new(&config, client)
        .serve(stdio())
        .await
        .context("MCP handshake failed")?;
    service.waiting().await?;
    Ok(())
}
