/// MCP client — connects to a single MCP server via stdio or HTTP.
///
/// Implements the Model Context Protocol (MCP) JSON-RPC 2.0 protocol.
/// Stdio transport: spawns the server process and communicates via stdin/stdout.
/// HTTP transport:  POSTs JSON-RPC requests to a URL (streamable HTTP).

use crate::mcp::types::{JsonRpcRequest, JsonRpcResponse, McpCallResult, McpResource, McpToolDef};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tokio::sync::{oneshot, Mutex};
use tokio::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

// ── Transport trait ───────────────────────────────────────────────────────────

#[async_trait]
pub(crate) trait McpTransport: Send + Sync {
    /// Send a request and await its response.
    async fn call(&self, id: u64, method: &str, params: Value) -> Result<Value>;

    /// Send a notification (fire-and-forget, no response expected).
    async fn notify(&self, _method: &str) {}
}

// ── Stdio transport ───────────────────────────────────────────────────────────

pub(crate) struct StdioTransport {
    stdin_tx: tokio::sync::mpsc::UnboundedSender<String>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>>,
}

impl StdioTransport {
    pub async fn connect(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Self> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::process::Command;

        let mut child = Command::new(command)
            .args(args)
            .envs(env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn MCP server '{}': {}", command, e))?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("MCP server '{}': could not open stdin", command))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("MCP server '{}': could not open stdout", command))?;

        let (stdin_tx, mut stdin_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Writer task: drain channel → child stdin
        tokio::spawn(async move {
            while let Some(line) = stdin_rx.recv().await {
                let data = format!("{}\n", line);
                if stdin.write_all(data.as_bytes()).await.is_err() {
                    break;
                }
            }
        });

        // Reap task: prevent zombie process
        tokio::spawn(async move {
            let _ = child.wait().await;
        });

        // Reader task: child stdout → pending oneshots
        let pending_clone = Arc::clone(&pending);
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<JsonRpcResponse>(trimmed) {
                    Ok(resp) => {
                        let id = match resp.id.as_ref().and_then(|v| v.as_u64()) {
                            Some(id) => id,
                            None => continue, // notification — ignore
                        };
                        let result = if let Some(err) = resp.error {
                            Err(anyhow!("MCP error {}: {}", err.code, err.message))
                        } else {
                            Ok(resp.result.unwrap_or(Value::Null))
                        };
                        let mut pending = pending_clone.lock().await;
                        if let Some(tx) = pending.remove(&id) {
                            let _ = tx.send(result);
                        }
                    }
                    Err(_) => {} // malformed / partial line — skip
                }
            }
            // Process exited — drain any remaining pending requests with an error
            let mut pending = pending_clone.lock().await;
            for (_, tx) in pending.drain() {
                let _ = tx.send(Err(anyhow!("MCP server process exited")));
            }
        });

        Ok(Self { stdin_tx, pending })
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn call(&self, id: u64, method: &str, params: Value) -> Result<Value> {
        let req = JsonRpcRequest::new(id, method, params);
        let json = serde_json::to_string(&req)?;

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        self.stdin_tx
            .send(json)
            .map_err(|_| anyhow!("MCP server stdin closed"))?;

        tokio::time::timeout(REQUEST_TIMEOUT, rx)
            .await
            .map_err(|_| anyhow!("MCP request timed out ({})", method))?
            .map_err(|_| anyhow!("MCP server disconnected"))?
    }

    async fn notify(&self, method: &str) {
        let req = JsonRpcRequest::notification(method);
        if let Ok(json) = serde_json::to_string(&req) {
            let _ = self.stdin_tx.send(json);
        }
    }
}

// ── HTTP transport ────────────────────────────────────────────────────────────

pub(crate) struct HttpTransport {
    url: String,
    client: reqwest::Client,
}

impl HttpTransport {
    pub fn new(url: &str, headers: &HashMap<String, String>) -> Result<Self> {
        let mut builder = reqwest::Client::builder();

        if !headers.is_empty() {
            let mut header_map = reqwest::header::HeaderMap::new();
            for (k, v) in headers {
                let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
                    .map_err(|e| anyhow!("Invalid MCP header name '{}': {}", k, e))?;
                let value = reqwest::header::HeaderValue::from_str(v)
                    .map_err(|e| anyhow!("Invalid MCP header value: {}", e))?;
                header_map.insert(name, value);
            }
            builder = builder.default_headers(header_map);
        }

        Ok(Self {
            url: url.to_string(),
            client: builder.build()?,
        })
    }
}

#[async_trait]
impl McpTransport for HttpTransport {
    async fn call(&self, id: u64, method: &str, params: Value) -> Result<Value> {
        let req = JsonRpcRequest::new(id, method, params);

        let resp = tokio::time::timeout(
            REQUEST_TIMEOUT,
            self.client
                .post(&self.url)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .json(&req)
                .send(),
        )
        .await
        .map_err(|_| anyhow!("HTTP MCP request timed out ({})", method))??;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("HTTP MCP {} failed: {} — {}", method, status, body));
        }

        let rpc_resp: JsonRpcResponse = resp.json().await?;

        if let Some(err) = rpc_resp.error {
            return Err(anyhow!("MCP error {}: {}", err.code, err.message));
        }

        Ok(rpc_resp.result.unwrap_or(Value::Null))
    }
}

// ── McpClient ─────────────────────────────────────────────────────────────────

pub struct McpClient {
    pub server_name: String,
    pub tools: Vec<McpToolDef>,
    pub transport_kind: &'static str, // "stdio" | "http"
    transport: Box<dyn McpTransport>,
    next_id: AtomicU64,
}

// Box<dyn McpTransport + Send + Sync> is Send+Sync; AtomicU64 is Send+Sync.
unsafe impl Send for McpClient {}
unsafe impl Sync for McpClient {}

impl McpClient {
    // ── Internal helpers ──────────────────────────────────────────────────────

    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.transport.call(id, method, params).await
    }

    /// Run the MCP initialize handshake and populate self.tools.
    async fn init(&mut self) -> Result<()> {
        // 1. initialize
        let params = json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": { "roots": { "listChanged": false }, "sampling": {} },
            "clientInfo": {
                "name": "rustyclaw",
                "version": env!("CARGO_PKG_VERSION")
            }
        });
        self.request("initialize", params)
            .await
            .map_err(|e| anyhow!("MCP initialize failed for '{}': {}", self.server_name, e))?;

        // 2. Notify server that client is ready (fire-and-forget)
        self.transport.notify("notifications/initialized").await;

        // 3. Fetch tool list
        let result = self.request("tools/list", json!({})).await.unwrap_or(Value::Null);
        self.tools = result
            .get("tools")
            .and_then(|t| serde_json::from_value(t.clone()).ok())
            .unwrap_or_default();

        Ok(())
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Connect to a stdio MCP server.
    pub async fn connect_stdio(
        server_name: String,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Self> {
        let transport = StdioTransport::connect(command, args, env).await?;
        let mut client = Self {
            server_name,
            tools: Vec::new(),
            transport_kind: "stdio",
            transport: Box::new(transport),
            next_id: AtomicU64::new(1),
        };
        client.init().await?;
        Ok(client)
    }

    /// Connect to an HTTP MCP server (streamable HTTP transport).
    pub async fn connect_http(
        server_name: String,
        url: &str,
        headers: &HashMap<String, String>,
    ) -> Result<Self> {
        let transport = HttpTransport::new(url, headers)?;
        let mut client = Self {
            server_name,
            tools: Vec::new(),
            transport_kind: "http",
            transport: Box::new(transport),
            next_id: AtomicU64::new(1),
        };
        client.init().await?;
        Ok(client)
    }

    /// List MCP resources exposed by this server.
    pub async fn list_resources(&self) -> Result<Vec<McpResource>> {
        let result = self.request("resources/list", json!({})).await?;
        let resources: Vec<McpResource> = result
            .get("resources")
            .and_then(|r| serde_json::from_value(r.clone()).ok())
            .unwrap_or_default();
        Ok(resources)
    }

    /// Read an MCP resource by URI.
    pub async fn read_resource(&self, uri: &str) -> Result<String> {
        let result = self
            .request("resources/read", json!({ "uri": uri }))
            .await?;

        // MCP resources/read returns { contents: [{ uri, text?, blob? }] }
        let text = result
            .get("contents")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_else(|| serde_json::to_string_pretty(&result).unwrap_or_default());

        Ok(text)
    }

    /// Execute an MCP tool call and return the text output.
    pub async fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<String> {
        let result = self
            .request(
                "tools/call",
                json!({ "name": tool_name, "arguments": arguments }),
            )
            .await?;

        // Deserialize the result
        let call_result: McpCallResult = serde_json::from_value(result.clone())
            .unwrap_or(McpCallResult {
                content: vec![],
                is_error: false,
            });

        let text = call_result
            .content
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        // Fall back to raw JSON if content array was empty
        let mut output = if text.is_empty() {
            serde_json::to_string_pretty(&result).unwrap_or_default()
        } else {
            text
        };

        // Respect _meta["anthropic/maxResultSizeChars"] from the tool definition.
        // This allows servers to declare a larger limit (up to 500K), otherwise we
        // cap at 25K to avoid flooding the context window.
        let max_chars = self.tools.iter()
            .find(|t| t.name == tool_name)
            .map(|t| t.max_result_chars())
            .unwrap_or(25_000);

        if output.len() > max_chars {
            // Truncate at a char boundary
            let truncated: String = output.chars().take(max_chars).collect();
            output = format!(
                "{}\n\n[Result truncated: {} chars total, limit {}]",
                truncated, output.len(), max_chars
            );
        }

        if call_result.is_error {
            Err(anyhow!("MCP tool '{}' error: {}", tool_name, output))
        } else {
            Ok(output)
        }
    }
}
