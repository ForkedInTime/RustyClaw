/// SleepTool — port of sleep.ts
/// Pauses execution for a specified number of milliseconds (capped at 60 000).

use super::{async_trait, Tool, ToolContext, ToolOutput};
use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

pub struct SleepTool;

#[derive(Deserialize)]
struct Input {
    milliseconds: u64,
}

#[async_trait]
impl Tool for SleepTool {
    fn name(&self) -> &str { "Sleep" }

    fn description(&self) -> &str {
        "Pause execution for a number of milliseconds (max 60 000). \
        Useful between retries or when waiting for external processes."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "milliseconds": {
                    "type": "integer",
                    "description": "How many milliseconds to sleep (1–60 000)",
                    "minimum": 1,
                    "maximum": 60000
                }
            },
            "required": ["milliseconds"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let input: Input = serde_json::from_value(input)?;
        let ms = input.milliseconds.min(60_000);
        tokio::time::sleep(tokio::time::Duration::from_millis(ms)).await;
        Ok(ToolOutput::success(format!("Slept for {ms} ms.")))
    }
}
