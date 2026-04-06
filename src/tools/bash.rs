/// BashTool — port of tools/BashTool/BashTool.ts

use super::{async_trait, Tool, ToolContext, ToolOutput};
use anyhow::Result;
use serde::Deserialize;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use std::process::Stdio;

const DEFAULT_TIMEOUT_MS: u64 = 120_000; // 2 minutes, same as TypeScript default
const MAX_OUTPUT_BYTES: usize = 1_000_000; // 1MB cap

/// Strip ANSI escape sequences and carriage returns from terminal output.
/// Prevents progress-bar output (e.g. from `ollama pull`) from corrupting the TUI.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                // Carriage return — skip (progress bars use \r to overwrite lines)
            }
            '\x1b' => {
                // ESC — consume the escape sequence
                if chars.peek() == Some(&'[') {
                    chars.next(); // consume '['
                    // consume until a letter (the command character)
                    for c in chars.by_ref() {
                        if c.is_ascii_alphabetic() { break; }
                    }
                } else {
                    // Other ESC sequences — consume next char
                    chars.next();
                }
            }
            _ => out.push(ch),
        }
    }
    out
}

pub struct BashTool;

#[derive(Deserialize)]
struct BashInput {
    command: String,
    #[serde(default)]
    timeout: Option<u64>,
    #[serde(default)]
    description: Option<String>,
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn description(&self) -> &str {
        "Execute a bash command in the shell. Use for running tests, git commands, \
        build commands, installing packages, and other shell operations. \
        Avoid interactive commands. For long-running operations, consider adding \
        a timeout."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout": {
                    "type": "number",
                    "description": "Timeout in milliseconds (default: 120000)"
                },
                "description": {
                    "type": "string",
                    "description": "Short description of what this command does"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let input: BashInput = serde_json::from_value(input)?;
        let timeout_ms = input.timeout.unwrap_or(DEFAULT_TIMEOUT_MS);

        // Apply sandbox if enabled
        let allow_net = ctx.sandbox_allow_network;
        let command = if let Some(ref mode) = ctx.sandbox_mode {
            match crate::sandbox::apply_sandbox(&input.command, mode, &ctx.cwd, allow_net) {
                Ok(cmd) => cmd,
                Err(reason) => return Ok(ToolOutput::error(reason)),
            }
        } else {
            input.command.clone()
        };

        let command_str = command.clone();
        let stream_tx = ctx.stream_tx.clone();
        let cwd = ctx.cwd.clone();
        // Resolve shell: ctx.default_shell → $SHELL env var → "bash"
        let shell = ctx.default_shell.clone()
            .unwrap_or_else(|| std::env::var("SHELL").unwrap_or_else(|_| "bash".into()));

        let fut = async move {
            let mut child = Command::new(&shell)
                .arg("-c")
                .arg(&command)
                .current_dir(&cwd)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;

            let stdout = child.stdout.take()
                .ok_or_else(|| anyhow::anyhow!("Failed to capture stdout"))?;
            let stderr = child.stderr.take()
                .ok_or_else(|| anyhow::anyhow!("Failed to capture stderr"))?;

            let mut combined = String::new();
            let mut stdout_reader = BufReader::new(stdout).lines();
            let mut stderr_reader = BufReader::new(stderr).lines();

            // Drain both stdout and stderr concurrently, streaming each line
            loop {
                tokio::select! {
                    line = stdout_reader.next_line() => {
                        match line? {
                            None => break,
                            Some(l) => {
                                let clean = strip_ansi(&l);
                                if !clean.is_empty() {
                                    if let Some(ref tx) = stream_tx {
                                        let _ = tx.send(clean.clone());
                                    }
                                    if combined.len() < MAX_OUTPUT_BYTES {
                                        combined.push_str(&clean);
                                        combined.push('\n');
                                    }
                                }
                            }
                        }
                    }
                    line = stderr_reader.next_line() => {
                        match line? {
                            None => {}
                            Some(l) => {
                                let clean = strip_ansi(&l);
                                if !clean.is_empty() {
                                    if let Some(ref tx) = stream_tx {
                                        let _ = tx.send(clean.clone());
                                    }
                                    if combined.len() < MAX_OUTPUT_BYTES {
                                        combined.push_str(&clean);
                                        combined.push('\n');
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Drain remaining stderr after stdout closes
            while let Some(l) = stderr_reader.next_line().await? {
                let clean = strip_ansi(&l);
                if !clean.is_empty() {
                    if let Some(ref tx) = stream_tx {
                        let _ = tx.send(clean.clone());
                    }
                    if combined.len() < MAX_OUTPUT_BYTES {
                        combined.push_str(&clean);
                        combined.push('\n');
                    }
                }
            }

            let status = child.wait().await?;

            if combined.len() >= MAX_OUTPUT_BYTES {
                combined.push_str("\n... (output truncated)");
            }

            if combined.is_empty() {
                combined = format!("(exit code: {})", status.code().unwrap_or(-1));
            }

            let is_error = !status.success();
            Ok::<ToolOutput, anyhow::Error>(if is_error {
                ToolOutput::error(combined)
            } else {
                ToolOutput::success(combined)
            })
        };

        match timeout(Duration::from_millis(timeout_ms), fut).await {
            Ok(result) => result,
            Err(_) => Ok(ToolOutput::error(format!(
                "Command timed out after {}ms: {}",
                timeout_ms, command_str
            ))),
        }
    }
}
