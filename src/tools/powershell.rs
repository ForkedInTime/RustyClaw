/// PowerShellTool — port of powershell.ts
/// Runs a PowerShell command via `pwsh` and returns stdout + stderr.
use super::{Tool, ToolContext, ToolOutput, async_trait};
use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

pub struct PowerShellTool;

#[derive(Deserialize)]
struct Input {
    command: String,
    #[serde(default = "default_timeout")]
    timeout_ms: u64,
}

fn default_timeout() -> u64 {
    30_000
}

#[async_trait]
impl Tool for PowerShellTool {
    fn name(&self) -> &str {
        "PowerShell"
    }

    fn description(&self) -> &str {
        "Execute a PowerShell command using `pwsh`. Returns combined stdout and stderr. \
        Only available when PowerShell Core (pwsh) is installed."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The PowerShell command or script to execute"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default 30 000, max 120 000)",
                    "default": 30000,
                    "minimum": 1000,
                    "maximum": 120000
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let input: Input = serde_json::from_value(input)?;
        let timeout_ms = input.timeout_ms.min(120_000);

        use tokio::process::Command;
        use tokio::time::{Duration, timeout};

        let fut = Command::new("pwsh")
            .args(["-NoProfile", "-NonInteractive", "-Command", &input.command])
            .current_dir(&ctx.cwd)
            .output();

        let result = timeout(Duration::from_millis(timeout_ms), fut).await;

        match result {
            Err(_) => Ok(ToolOutput::error(format!(
                "PowerShell command timed out after {timeout_ms} ms."
            ))),
            Ok(Err(e)) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Ok(ToolOutput::error(
                        "pwsh not found. Install PowerShell Core to use this tool.",
                    ))
                } else {
                    Ok(ToolOutput::error(format!("Failed to run pwsh: {e}")))
                }
            }
            Ok(Ok(output)) => {
                let mut out = String::new();
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                if !stdout.is_empty() {
                    out.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str("[stderr]\n");
                    out.push_str(&stderr);
                }
                if out.is_empty() {
                    out = format!("(exit code {})", output.status.code().unwrap_or(-1));
                }

                let is_error = !output.status.success();
                if is_error {
                    Ok(ToolOutput::error(out))
                } else {
                    Ok(ToolOutput::success(out))
                }
            }
        }
    }
}
