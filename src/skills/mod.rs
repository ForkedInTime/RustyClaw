/// Skills system — port of skills/ and the /skill-name slash command mechanism.
///
/// Skills are markdown files that expand into prompts when invoked with /skill-name.
/// They live in ~/.claude/skills/ and in the bundled skills directory.
///
/// A skill file looks like:
///   # My Skill
///   Description of what it does.
///   ---
///   The prompt template. {{ARGS}} is replaced with the user's arguments.

use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs;

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub prompt_template: String,
}

impl Skill {
    /// Expand the skill with user arguments
    pub fn expand(&self, args: &str) -> String {
        self.prompt_template.replace("{{ARGS}}", args).replace("{{args}}", args)
    }
}

/// Load all skills from ~/.claude/skills/ and return a name→Skill map.
pub async fn load_skills() -> HashMap<String, Skill> {
    let mut skills = HashMap::new();

    // Load bundled skills first
    for skill in bundled_skills() {
        skills.insert(skill.name.clone(), skill);
    }

    // Load user skills from ~/.claude/skills/ (override bundled)
    let skills_dir = crate::config::Config::claude_dir().join("skills");
    if let Ok(mut entries) = fs::read_dir(&skills_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Ok(skill) = load_skill_file(&path).await {
                    skills.insert(skill.name.clone(), skill);
                }
            }
        }
    }

    skills
}

/// Parse a skill from a markdown file.
async fn load_skill_file(path: &PathBuf) -> Result<Skill> {
    let content = fs::read_to_string(path).await?;
    parse_skill_content(&content, path)
}

fn parse_skill_content(content: &str, path: &PathBuf) -> Result<Skill> {
    // Name is the filename without extension
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unnamed")
        .to_lowercase()
        .replace(' ', "-");

    // Try to find a --- separator between description and prompt template
    let (description, prompt_template) = if let Some(sep) = content.find("\n---\n") {
        let desc_part = content[..sep].trim();
        // Strip leading # heading if present
        let desc = desc_part
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        let prompt = content[sep + 5..].trim().to_string();
        (desc, prompt)
    } else {
        // No separator — whole file is the prompt template
        (name.clone(), content.trim().to_string())
    };

    Ok(Skill { name, description, prompt_template })
}

/// Check if an input string is a skill invocation (starts with /).
/// Returns (skill_name, args) if it matches.
pub fn parse_skill_invocation(input: &str) -> Option<(&str, &str)> {
    let input = input.trim();
    if !input.starts_with('/') {
        return None;
    }
    let rest = &input[1..];
    if let Some(space) = rest.find(' ') {
        Some((&rest[..space], rest[space + 1..].trim()))
    } else {
        Some((rest, ""))
    }
}

/// Built-in skills bundled with rustyclaw.
fn bundled_skills() -> Vec<Skill> {
    vec![
        Skill {
            name: "commit".to_string(),
            description: "Create a git commit with a well-formatted message".to_string(),
            prompt_template: "Please create a git commit for the current staged changes. \
                Follow conventional commit format. Run git diff --staged first to see the changes, \
                then write a commit message and run git commit. {{ARGS}}"
                .to_string(),
        },
        Skill {
            name: "review".to_string(),
            description: "Review code changes for quality and correctness".to_string(),
            prompt_template: "Please review the following code/changes for correctness, \
                quality, potential bugs, and style issues. Be specific about any problems found. \
                {{ARGS}}"
                .to_string(),
        },
        Skill {
            name: "explain".to_string(),
            description: "Explain how a piece of code works".to_string(),
            prompt_template: "Please explain how the following code works, including its \
                purpose, key logic, and any non-obvious design decisions. {{ARGS}}"
                .to_string(),
        },
        Skill {
            name: "fix".to_string(),
            description: "Find and fix a bug or error".to_string(),
            prompt_template: "Please investigate and fix the following issue. Read relevant \
                files first, diagnose the root cause, then make the minimal change to fix it. \
                {{ARGS}}"
                .to_string(),
        },
        Skill {
            name: "test".to_string(),
            description: "Write tests for a function or module".to_string(),
            prompt_template: "Please write comprehensive tests for the following. Include \
                unit tests for edge cases and happy paths. {{ARGS}}"
                .to_string(),
        },
    ]
}
