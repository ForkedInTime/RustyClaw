/// SkillTool — port of skill.ts
/// Looks up a skill by name from the skills registry and executes it.
/// Skills are .md files in ~/.claude/skills/ or .claude/skills/ — each is a prompt template.

use super::{async_trait, Tool, ToolContext, ToolOutput};
use anyhow::Result;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;

pub struct SkillTool;

#[derive(Deserialize)]
struct Input {
    /// Skill name (filename without extension)
    skill: String,
    /// Optional arguments to append to the skill prompt
    #[serde(default)]
    args: Option<String>,
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str { "Skill" }

    fn description(&self) -> &str {
        "Execute a skill by name. Skills are markdown prompt templates stored in \
        ~/.claude/skills/ or .claude/skills/. Use DiscoverSkills to list available skills."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "Skill name (the filename without .md extension)"
                },
                "args": {
                    "type": "string",
                    "description": "Optional arguments or context to pass to the skill"
                }
            },
            "required": ["skill"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let input: Input = serde_json::from_value(input)?;

        let skill_content = find_skill(&ctx.cwd, &input.skill).await?;

        // Expand the skill content (strip frontmatter, optionally append args)
        let prompt = expand_skill(&skill_content, input.args.as_deref());

        // Return the expanded prompt — the caller (run_api_task) will send it
        // as a user message. We signal this with a special prefix.
        Ok(ToolOutput::success(format!("[SKILL_PROMPT]\n{prompt}")))
    }
}

async fn find_skill(cwd: &std::path::Path, name: &str) -> Result<String> {
    let dirs: Vec<PathBuf> = {
        let mut d = Vec::new();
        // Local project skills override global ones
        d.push(cwd.join(".claude").join("skills"));
        if let Some(home) = dirs::home_dir() {
            d.push(home.join(".claude").join("skills"));
        }
        d
    };

    for dir in &dirs {
        let path = dir.join(format!("{name}.md"));
        if path.exists() {
            return tokio::fs::read_to_string(&path).await
                .map_err(|e| anyhow::anyhow!("Cannot read skill {name}: {e}"));
        }
    }

    Err(anyhow::anyhow!(
        "Skill '{}' not found. Searched in .claude/skills/ and ~/.claude/skills/.\n\
        Use DiscoverSkills to see available skills.",
        name
    ))
}

/// Strip YAML frontmatter (between --- delimiters) and optionally append args.
fn expand_skill(content: &str, args: Option<&str>) -> String {
    let stripped = strip_frontmatter(content);
    if let Some(extra) = args {
        if extra.trim().is_empty() {
            stripped
        } else {
            format!("{stripped}\n\n{extra}")
        }
    } else {
        stripped
    }
}

fn strip_frontmatter(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.first().map(|l| l.trim()) == Some("---") {
        // Find closing ---
        if let Some(end) = lines[1..].iter().position(|l| l.trim() == "---") {
            return lines[end + 2..].join("\n").trim_start().to_string();
        }
    }
    content.trim_start().to_string()
}
