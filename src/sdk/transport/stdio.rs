//! NDJSON stdin/stdout transport.
//!
//! Reads one JSON object per line from stdin.
//! Writes one JSON object per line to stdout.
//! Stderr is reserved for debug/log output.

use crate::sdk::protocol::{SdkNotification, SdkRequest, SdkResponse};
use crate::sdk::transport::Transport;
use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

/// Max line size: 4MB — generous limit for large tool outputs.
const MAX_LINE_SIZE: usize = 4 * 1024 * 1024;

pub struct StdioTransport {
    reader: Mutex<BufReader<tokio::io::Stdin>>,
    writer: Mutex<tokio::io::Stdout>,
}

impl Default for StdioTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl StdioTransport {
    pub fn new() -> Self {
        Self {
            reader: Mutex::new(BufReader::new(tokio::io::stdin())),
            writer: Mutex::new(tokio::io::stdout()),
        }
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn read_request(&mut self) -> Result<Option<SdkRequest>> {
        let mut line = String::new();
        let mut reader = self.reader.lock().await;
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .await
                .context("Failed to read from stdin")?;
            if n == 0 {
                return Ok(None); // EOF — host closed stdin
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue; // skip blank lines
            }
            if trimmed.len() > MAX_LINE_SIZE {
                eprintln!("[sdk] Warning: line exceeds 4MB, skipping");
                continue;
            }
            let req: SdkRequest = serde_json::from_str(trimmed).with_context(|| {
                format!(
                    "Invalid JSON request: {}",
                    &trimmed[..trimmed.len().min(200)]
                )
            })?;
            return Ok(Some(req));
        }
    }

    async fn send_response(&self, response: SdkResponse) -> Result<()> {
        let mut json = serde_json::to_string(&response)?;
        json.push('\n');
        let mut writer = self.writer.lock().await;
        writer.write_all(json.as_bytes()).await?;
        writer.flush().await?;
        Ok(())
    }

    async fn send_notification(&self, notification: SdkNotification) -> Result<()> {
        let mut json = serde_json::to_string(&notification)?;
        json.push('\n');
        let mut writer = self.writer.lock().await;
        writer.write_all(json.as_bytes()).await?;
        writer.flush().await?;
        Ok(())
    }
}
