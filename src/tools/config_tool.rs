/// ConfigTool — port of config.ts (slash command) exposed as a tool
/// Reads the current configuration and returns it to Claude.
use super::{Tool, ToolContext, ToolOutput, async_trait};
use anyhow::Result;
use serde_json::json;

pub struct ConfigTool {
    pub config: crate::config::Config,
}

#[async_trait]
impl Tool for ConfigTool {
    fn name(&self) -> &str {
        "Config"
    }

    fn description(&self) -> &str {
        "Read the current assistant configuration (model, settings, enabled features). \
        Use this to check what model is active or whether features like prompt caching, \
        extended thinking, or plan mode are enabled."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let cfg = &self.config;

        let mut lines = vec![
            format!("model: {}", cfg.model),
            format!("max_tokens: {}", cfg.max_tokens),
            format!("prompt_cache: {}", cfg.prompt_cache),
            format!("plan_mode: {}", cfg.plan_mode),
        ];

        if let Some(budget) = cfg.thinking_budget_tokens {
            lines.push(format!("thinking_budget_tokens: {budget}"));
        } else {
            lines.push("thinking_budget_tokens: disabled".to_string());
        }

        if let Some(hooks) = &cfg.hooks {
            lines.push(format!(
                "hooks: pre_tool_use={}, post_tool_use={}",
                hooks.pre_tool_use.len(),
                hooks.post_tool_use.len()
            ));
        } else {
            lines.push("hooks: none".to_string());
        }

        Ok(ToolOutput::success(lines.join("\n")))
    }
}
