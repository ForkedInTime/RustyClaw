/// AskUserQuestion tool — pauses tool execution and asks the user a question.
///
/// Claude calls this when it needs clarification or input from the user during
/// a tool-use sequence.  The TUI shows a text-input dialog; the user's answer
/// is returned as the tool result.

use crate::tools::{async_trait, Tool, ToolContext, ToolOutput};
use anyhow::Result;
use tokio::sync::oneshot;

pub struct AskUserQuestionTool;

#[async_trait]
impl Tool for AskUserQuestionTool {
    fn name(&self) -> &str { "AskUserQuestion" }

    fn description(&self) -> &str {
        "Ask the user a question and wait for their response. Use this when you need \
         clarification or additional input from the user before proceeding."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user"
                }
            },
            "required": ["question"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let question = input
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("What would you like to do?")
            .to_string();

        let Some(tx) = &ctx.ask_user_tx else {
            return Ok(ToolOutput::error(
                "AskUserQuestion is not available in this execution context."
            ));
        };

        let (reply_tx, reply_rx) = oneshot::channel::<String>();
        if tx.send((question, reply_tx)).is_err() {
            return Ok(ToolOutput::error("Failed to send question to TUI."));
        }

        match reply_rx.await {
            Ok(answer) => Ok(ToolOutput::success(answer)),
            Err(_) => Ok(ToolOutput::error("User question was cancelled.")),
        }
    }
}
