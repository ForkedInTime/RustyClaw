/// McpManager — connects to all MCP servers from settings.json at startup.
///
/// Each server runs independently; failures are logged and skipped so a
/// broken MCP server never prevents rustyclaw from starting.

use crate::mcp::client::McpClient;
use crate::mcp::types::{McpServerConfig, McpServerStatus};
use crate::settings::Settings;
use std::sync::Arc;

pub struct McpManager {
    pub clients: Vec<Arc<McpClient>>,
}

impl McpManager {
    /// Start all MCP servers listed in settings + any injected via CLI --mcp-config.
    /// Errors per-server are logged; the manager is always returned.
    pub async fn start_with_extra(
        settings: &Settings,
        extra: &std::collections::HashMap<String, McpServerConfig>,
    ) -> Self {
        let mut all: std::collections::HashMap<&String, &McpServerConfig> =
            settings.mcp_servers.iter().collect();
        // extra (CLI --mcp-config) wins over settings on name conflicts
        for (k, v) in extra { all.insert(k, v); }

        let mut clients = Vec::new();
        for (name, cfg) in &all {
            match Self::connect_one((*name).clone(), cfg).await {
                Ok(client) => {
                    tracing::info!(
                        "MCP '{}': connected ({} tools via {})",
                        name,
                        client.tools.len(),
                        client.transport_kind
                    );
                    clients.push(Arc::new(client));
                }
                Err(e) => {
                    tracing::warn!("MCP '{}': failed to connect — {}", name, e);
                }
            }
        }
        Self { clients }
    }

    async fn connect_one(name: String, cfg: &McpServerConfig) -> anyhow::Result<McpClient> {
        match cfg {
            McpServerConfig::Stdio(s) => {
                McpClient::connect_stdio(name, &s.command, &s.args, &s.env).await
            }
            McpServerConfig::Http(h) => {
                McpClient::connect_http(name, &h.url, &h.headers).await
            }
        }
    }

    /// Return a snapshot of server statuses for the /mcp command.
    pub fn statuses(&self) -> Vec<McpServerStatus> {
        self.clients
            .iter()
            .map(|c| McpServerStatus {
                name: c.server_name.clone(),
                transport: c.transport_kind,
                tool_count: c.tools.len(),
            })
            .collect()
    }

}
