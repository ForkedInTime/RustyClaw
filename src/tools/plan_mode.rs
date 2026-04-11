/// Plan mode tools — let Claude enter/exit read-only plan mode.
///
/// In plan mode, destructive tools (Bash, Write, Edit, MultiEdit, EnterWorktree)
/// are blocked.  Claude uses this to outline a plan before executing changes.
use crate::tools::{Tool, ToolContext, ToolOutput, async_trait};
use anyhow::Result;

pub struct EnterPlanModeTool;
pub struct ExitPlanModeTool;

#[async_trait]
impl Tool for EnterPlanModeTool {
    fn name(&self) -> &str {
        "EnterPlanMode"
    }

    fn description(&self) -> &str {
        "Enter plan mode. In plan mode you can only read files and think; all write/execute \
         tools are blocked. Use this when you want to outline a plan and get user approval \
         before making any changes."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _input: serde_json::Value, ctx: &ToolContext) -> Result<ToolOutput> {
        if let Some(tx) = &ctx.plan_mode_tx {
            let _ = tx.send(true);
        }
        Ok(ToolOutput::success(
            "Plan mode enabled. Destructive tools are now blocked. \
             Use ExitPlanMode when you are ready to execute the plan.",
        ))
    }
}

#[async_trait]
impl Tool for ExitPlanModeTool {
    fn name(&self) -> &str {
        "ExitPlanMode"
    }

    fn description(&self) -> &str {
        "Exit plan mode and return to normal mode where all tools are available."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _input: serde_json::Value, ctx: &ToolContext) -> Result<ToolOutput> {
        if let Some(tx) = &ctx.plan_mode_tx {
            let _ = tx.send(false);
        }
        Ok(ToolOutput::success(
            "Plan mode disabled. All tools are now available.",
        ))
    }
}
