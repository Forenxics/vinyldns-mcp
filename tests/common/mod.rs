//! Shared helpers for the end-to-end tests: an in-process MCP client talking
//! to the server, which talks to a wiremock VinylDNS.
#![allow(dead_code)]

use std::time::Duration;

use rmcp::{
    ClientHandler, ErrorData as McpError, RoleClient, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, ElicitRequestParams, ElicitResult,
        ElicitationAction, Implementation,
    },
    service::{RequestContext, RunningService},
};
use serde_json::{Value, json};
use url::Url;
use vinyldns_mcp::{
    client::VinylDnsClient,
    config::{Config, ConfirmationMode},
    server::VinylDnsServer,
};
use wiremock::{MockServer, Request};

/// A test MCP client. `elicit_answer` is `None` for a client without
/// elicitation support; otherwise it is the user's yes/no answer.
#[derive(Clone)]
pub struct TestClient {
    pub elicit_answer: Option<bool>,
}

impl ClientHandler for TestClient {
    async fn create_elicitation(
        &self,
        _request: ElicitRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, McpError> {
        Ok(match self.elicit_answer {
            Some(answer) => ElicitResult::new(ElicitationAction::Accept).with_content(json!({ "confirm": answer })),
            None => ElicitResult::new(ElicitationAction::Decline),
        })
    }

    fn get_info(&self) -> ClientConfig {
        let caps = if self.elicit_answer.is_some() {
            ClientCapabilities::builder().enable_elicitation().build()
        } else {
            ClientCapabilities::default()
        };
        ClientConfig::new(caps, Implementation::new("test-client", "0.0.0"))
    }
}

pub fn config(api: &MockServer, enable_writes: bool, confirmation: ConfirmationMode) -> Config {
    Config {
        api_url: Url::parse(&api.uri()).unwrap(),
        access_key: "testAccessKey".into(),
        secret_key: "testSecretKey".into(),
        signing_region: "us-east-1".into(),
        signing_service: "VinylDNS".into(),
        http_timeout: Duration::from_secs(5),
        enable_writes,
        enable_admin: false,
        confirmation,
        pending_ttl: Duration::from_secs(60),
        dns_nameservers: Vec::new(),
        dns_timeout: Duration::from_secs(2),
    }
}

pub async fn connect(cfg: Config, client: TestClient) -> RunningService<RoleClient, TestClient> {
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server = VinylDnsServer::new(&cfg, VinylDnsClient::new(&cfg).unwrap());
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    client.serve(client_io).await.expect("MCP handshake")
}

pub async fn call(client: &RunningService<RoleClient, TestClient>, tool: &'static str, args: Value) -> CallToolResult {
    let Value::Object(args) = args else {
        panic!("args must be an object")
    };
    client
        .call_tool(CallToolRequestParams::new(tool).with_arguments(args))
        .await
        .expect("tool call")
}

pub fn text(result: &CallToolResult) -> String {
    result.content[0].as_text().expect("text content").text.clone()
}

pub fn json_of(result: &CallToolResult) -> Value {
    assert_ne!(result.is_error, Some(true), "unexpected tool error: {}", text(result));
    serde_json::from_str(&text(result)).expect("JSON tool output")
}

pub fn signed(req: &Request) -> bool {
    req.headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("AWS4-HMAC-SHA256 Credential=testAccessKey/") && v.contains("Signature="))
}

/// Config with writes and admin tools enabled, confirmed by token only.
pub fn admin_config(api: &MockServer) -> Config {
    Config {
        enable_admin: true,
        ..config(api, true, ConfirmationMode::Token)
    }
}

/// Connects a client without elicitation support to an admin-enabled server.
pub async fn admin_client(api: &MockServer) -> RunningService<RoleClient, TestClient> {
    connect(admin_config(api), TestClient { elicit_answer: None }).await
}
