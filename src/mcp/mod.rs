/// MCP (Model Context Protocol) integration.
///
/// Provides:
///   - McpManager  — connects to all servers from settings.json at startup
///   - McpDynamicTool — wraps an MCP tool as a built-in Tool (Arc<dyn Tool>)
///
/// Tool names use the format `mcp__<server>__<tool>` to avoid conflicts with
/// built-in tools and to let Claude identify which server provides each tool.

pub mod client;
pub mod manager;
pub mod types;

pub use client::McpClient;
pub use manager::McpManager;
pub use types::McpServerStatus;

use crate::api::types::ToolDefinition;
use crate::tools::{async_trait, DynTool, Tool, ToolContext, ToolOutput};
use anyhow::Result;
use std::sync::Arc;

// ── McpDynamicTool ────────────────────────────────────────────────────────────

/// Wraps an MCP tool definition + client so it looks identical to a built-in
/// Tool.  The tool name sent to the API is `mcp__<server>__<original_name>`.
pub struct McpDynamicTool {
    /// Composed name: `mcp__<server>__<original>`
    pub composed_name: String,
    /// Original tool name as defined by the MCP server (used in tools/call)
    pub original_name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub client: Arc<McpClient>,
}

#[async_trait]
impl Tool for McpDynamicTool {
    fn name(&self) -> &str {
        &self.composed_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> serde_json::Value {
        self.input_schema.clone()
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        match self.client.call_tool(&self.original_name, input).await {
            Ok(text) => Ok(ToolOutput::success(text)),
            Err(e) => Ok(ToolOutput::error(e.to_string())),
        }
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.composed_name.clone(),
            description: self.description.clone(),
            input_schema: self.input_schema.clone(),
            cache_control: None,
        }
    }
}

// ── Helper: build DynTool list from an McpManager ────────────────────────────

/// Convert all connected MCP servers' tools into `Arc<dyn Tool>` entries.
pub fn mcp_dyn_tools(manager: &McpManager) -> Vec<DynTool> {
    let mut tools: Vec<DynTool> = Vec::new();

    for client in &manager.clients {
        let server = sanitize_name(&client.server_name);

        for tool_def in &client.tools {
            let original = tool_def.name.clone();
            let composed = format!("mcp__{}__{}", server, sanitize_name(&original));

            let description = if tool_def.description.is_empty() {
                format!("[MCP: {}] {}", client.server_name, original)
            } else {
                format!("[MCP: {}] {}", client.server_name, tool_def.description)
            };

            tools.push(Arc::new(McpDynamicTool {
                composed_name: composed,
                original_name: original,
                description,
                input_schema: tool_def.input_schema.clone(),
                client: Arc::clone(client),
            }));
        }
    }

    tools
}

/// Replace non-alphanumeric characters with underscores for safe tool names.
fn sanitize_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}
