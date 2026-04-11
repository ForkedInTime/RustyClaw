/// MCP resource tools — port of mcpResources in the TS codebase
/// ListMcpResourcesTool: calls resources/list on all connected MCP servers
/// ReadMcpResourceTool:  calls resources/read for a specific URI
use super::{Tool, ToolContext, ToolOutput, async_trait};
use crate::mcp::client::McpClient;
use anyhow::Result;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

// ── ListMcpResourcesTool ──────────────────────────────────────────────────────

pub struct ListMcpResourcesTool {
    pub clients: Vec<Arc<McpClient>>,
}

#[async_trait]
impl Tool for ListMcpResourcesTool {
    fn name(&self) -> &str {
        "ListMcpResources"
    }

    fn description(&self) -> &str {
        "List all resources exposed by connected MCP servers. \
        Resources are data sources (files, database tables, etc.) that MCP servers make available."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        if self.clients.is_empty() {
            return Ok(ToolOutput::success("No MCP servers connected."));
        }

        let mut out = String::new();
        let mut total = 0;

        for client in &self.clients {
            match client.list_resources().await {
                Ok(resources) => {
                    if resources.is_empty() {
                        out.push_str(&format!("[{}] No resources.\n", client.server_name));
                    } else {
                        out.push_str(&format!("[{}]\n", client.server_name));
                        for r in &resources {
                            let desc = if r.description.is_empty() {
                                String::new()
                            } else {
                                format!(" — {}", r.description)
                            };
                            out.push_str(&format!("  {} ({}){}\n", r.uri, r.name, desc));
                            total += 1;
                        }
                    }
                }
                Err(e) => {
                    out.push_str(&format!("[{}] Error: {}\n", client.server_name, e));
                }
            }
        }

        if total == 0 {
            out.push_str("\nNo resources found across any connected MCP servers.");
        }

        Ok(ToolOutput::success(out.trim().to_string()))
    }
}

// ── ReadMcpResourceTool ───────────────────────────────────────────────────────

pub struct ReadMcpResourceTool {
    pub clients: Vec<Arc<McpClient>>,
}

#[derive(Deserialize)]
struct ReadInput {
    uri: String,
    /// Optional: constrain to this server name (mcp__<server>)
    #[serde(default)]
    server: Option<String>,
}

#[async_trait]
impl Tool for ReadMcpResourceTool {
    fn name(&self) -> &str {
        "ReadMcpResource"
    }

    fn description(&self) -> &str {
        "Read the content of an MCP resource by its URI. \
        Use ListMcpResources first to discover available resource URIs."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "uri": {
                    "type": "string",
                    "description": "The resource URI (from ListMcpResources)"
                },
                "server": {
                    "type": "string",
                    "description": "Optional server name to target (if multiple servers expose the same URI)"
                }
            },
            "required": ["uri"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let input: ReadInput = serde_json::from_value(input)?;

        if self.clients.is_empty() {
            return Ok(ToolOutput::error("No MCP servers connected."));
        }

        // Find the right client
        let clients_to_try: Vec<&Arc<McpClient>> = if let Some(server) = &input.server {
            self.clients
                .iter()
                .filter(|c| &c.server_name == server)
                .collect()
        } else {
            self.clients.iter().collect()
        };

        if clients_to_try.is_empty() {
            return Ok(ToolOutput::error(format!(
                "No MCP server found matching '{}'.",
                input.server.as_deref().unwrap_or("")
            )));
        }

        // Try each client until one succeeds
        let mut last_err = String::new();
        for client in clients_to_try {
            match client.read_resource(&input.uri).await {
                Ok(text) => return Ok(ToolOutput::success(text)),
                Err(e) => {
                    last_err = e.to_string();
                }
            }
        }

        Ok(ToolOutput::error(format!(
            "Could not read resource '{}': {}",
            input.uri, last_err
        )))
    }
}
