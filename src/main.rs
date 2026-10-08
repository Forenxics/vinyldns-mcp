use anyhow::Context;
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

use vinyldns_mcp::{client::VinylDnsClient, config::Config, server::VinylDnsServer};

const HELP: &str = "\
vinyldns-mcp — Model Context Protocol server for the VinylDNS API

USAGE:
    vinyldns-mcp            Run the server on stdio (started by an MCP client)
    vinyldns-mcp --help     Show this help
    vinyldns-mcp --version  Show the version

REQUIRED ENVIRONMENT:
    VINYLDNS_API_URL        Base URL of the VinylDNS API, e.g. https://vinyldns-api.example.com
    VINYLDNS_ACCESS_KEY     VinylDNS access key
    VINYLDNS_SECRET_KEY     VinylDNS secret key

OPTIONAL ENVIRONMENT:
    VINYLDNS_MCP_ENABLE_WRITES     true to enable the plan/confirm write tools (default false)
    VINYLDNS_MCP_ENABLE_ADMIN      true to also enable zone management and batch review tools
    VINYLDNS_MCP_CONFIRMATION      auto | elicit | token (default auto)
    VINYLDNS_MCP_PENDING_TTL_SECS  seconds a planned change stays confirmable (default 600)
    VINYLDNS_HTTP_TIMEOUT_SECS     API request timeout (default 30)
    VINYLDNS_MCP_DNS_NAMESERVERS   nameservers for the DNS cross-check (default: from NS records)
    VINYLDNS_MCP_DNS_TIMEOUT_SECS  timeout per DNS query (default 3)
    VINYLDNS_MCP_LOG               log filter, logs go to stderr (default info)

Documentation: docs/CONFIGURATION.md and docs/TOOLS.md in the source repository.";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Some(arg) = std::env::args().nth(1) {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("vinyldns-mcp {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                println!("{HELP}");
                return Ok(());
            }
            other => {
                eprintln!("error: unknown argument '{other}'; see vinyldns-mcp --help");
                std::process::exit(2);
            }
        }
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
        admin_enabled = config.enable_admin,
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
