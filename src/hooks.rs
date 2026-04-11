/// Hooks execution engine — port of utils/hooks.ts
///
/// Hooks are user-defined shell commands that run at lifecycle events:
///   PreToolUse      — before a tool executes (can block)
///   PostToolUse     — after a tool completes
///   UserPromptSubmit — when the user sends a message
///   Notification    — when Claude sends a text chunk
///   Stop            — when the session ends
///   SessionStart    — when the session begins
///   PreCompact      — before a compact/summarize cycle
///   PostCompact     — after a compact/summarize cycle
///
/// Hook JSON output (parsed from stdout):
///   { "continue": false, "stopReason": "...", "decision": "approve"|"block",
///     "systemMessage": "...", "reason": "..." }
///
/// Exit codes:
///   0   — success (allow, continue)
///   2   — blocking error (block tool/continue, show stopReason or stdout)
///   other — non-blocking error (logged, execution continues)

use crate::settings::{HookEntry, HooksConfig};
use serde::Deserialize;

/// Result returned by a hook execution.
#[derive(Debug, Default)]
pub struct HookResult {
    /// If false, block the tool call / stop the turn (from `continue: false` or exit 2).
    pub should_continue: bool,
    /// Human-readable reason shown when blocked.
    pub stop_reason: Option<String>,
    /// System-level message to inject into the conversation.
    pub system_message: Option<String>,
    /// Permission decision returned by PreToolUse hooks.
    pub decision: Option<HookDecision>,
    /// Additional context to inject into the tool's environment.
    pub additional_context: Option<String>,
}

impl HookResult {
    pub fn allow() -> Self {
        Self { should_continue: true, ..Default::default() }
    }
}

#[derive(Debug, PartialEq)]
pub enum HookDecision {
    Approve,
    Block,
}

/// Minimal JSON output from a hook script.
#[derive(Debug, Deserialize, Default)]
struct HookOutput {
    #[serde(rename = "continue", default = "default_true")]
    continue_: bool,
    #[serde(rename = "stopReason")]
    stop_reason: Option<String>,
    #[serde(rename = "systemMessage")]
    system_message: Option<String>,
    decision: Option<String>,
    reason: Option<String>,
    #[serde(rename = "additionalContext")]
    additional_context: Option<String>,
}

fn default_true() -> bool { true }

/// Run all matching PreToolUse hooks. Returns a HookResult — if `should_continue` is false,
/// the caller must block the tool call.
pub async fn run_pre_tool_hooks(
    hooks: &HooksConfig,
    tool_name: &str,
    tool_input: &str,
    session_id: &str,
    cwd: &std::path::Path,
) -> HookResult {
    let mut result = HookResult::allow();
    for hook in &hooks.pre_tool_use {
        if !hook.matches(tool_name) { continue; }
        let r = execute_hook(hook, HookEnvVars {
            event: "PreToolUse",
            tool_name: Some(tool_name),
            tool_input: Some(tool_input),
            tool_result: None,
            prompt: None,
            session_id,
            cwd,
        }).await;
        if !r.should_continue {
            return r;
        }
        // Merge system messages / decisions
        if r.system_message.is_some() { result.system_message = r.system_message; }
        if r.decision.is_some() { result.decision = r.decision; }
        if r.additional_context.is_some() { result.additional_context = r.additional_context; }
    }
    result
}

/// Run all matching PostToolUse hooks. Fire-and-forget (result is not blocking).
pub async fn run_post_tool_hooks(
    hooks: &HooksConfig,
    tool_name: &str,
    tool_result: &str,
    session_id: &str,
    cwd: &std::path::Path,
) {
    for hook in &hooks.post_tool_use {
        if !hook.matches(tool_name) { continue; }
        execute_hook(hook, HookEnvVars {
            event: "PostToolUse",
            tool_name: Some(tool_name),
            tool_input: None,
            tool_result: Some(tool_result),
            prompt: None,
            session_id,
            cwd,
        }).await;
    }
}

/// Run all UserPromptSubmit hooks. Returns any additional_context to prepend.
pub async fn run_user_prompt_hooks(
    hooks: &HooksConfig,
    prompt: &str,
    session_id: &str,
    cwd: &std::path::Path,
) -> Option<String> {
    let mut additional: Vec<String> = Vec::new();
    for hook in &hooks.user_prompt_submit {
        let r = execute_hook(hook, HookEnvVars {
            event: "UserPromptSubmit",
            tool_name: None,
            tool_input: None,
            tool_result: None,
            prompt: Some(prompt),
            session_id,
            cwd,
        }).await;
        if let Some(ctx) = r.additional_context {
            additional.push(ctx);
        }
    }
    if additional.is_empty() { None } else { Some(additional.join("\n")) }
}

/// Run Stop hooks when the session ends.
pub async fn run_stop_hooks(
    hooks: &HooksConfig,
    session_id: &str,
    cwd: &std::path::Path,
) {
    for hook in &hooks.stop {
        execute_hook(hook, HookEnvVars {
            event: "Stop",
            tool_name: None,
            tool_input: None,
            tool_result: None,
            prompt: None,
            session_id,
            cwd,
        }).await;
    }
}

/// Run SessionStart hooks.
pub async fn run_session_start_hooks(
    hooks: &HooksConfig,
    session_id: &str,
    cwd: &std::path::Path,
) {
    for hook in &hooks.session_start {
        execute_hook(hook, HookEnvVars {
            event: "SessionStart",
            tool_name: None,
            tool_input: None,
            tool_result: None,
            prompt: None,
            session_id,
            cwd,
        }).await;
    }
}

/// Run PreCompact hooks.
pub async fn run_pre_compact_hooks(
    hooks: &HooksConfig,
    session_id: &str,
    cwd: &std::path::Path,
) {
    for hook in &hooks.pre_compact {
        execute_hook(hook, HookEnvVars {
            event: "PreCompact",
            tool_name: None,
            tool_input: None,
            tool_result: None,
            prompt: None,
            session_id,
            cwd,
        }).await;
    }
}

/// Run PostCompact hooks.
pub async fn run_post_compact_hooks(
    hooks: &HooksConfig,
    session_id: &str,
    cwd: &std::path::Path,
) {
    for hook in &hooks.post_compact {
        execute_hook(hook, HookEnvVars {
            event: "PostCompact",
            tool_name: None,
            tool_input: None,
            tool_result: None,
            prompt: None,
            session_id,
            cwd,
        }).await;
    }
}

// ── Internal ──────────────────────────────────────────────────────────────��───

struct HookEnvVars<'a> {
    event: &'a str,
    tool_name: Option<&'a str>,
    tool_input: Option<&'a str>,
    tool_result: Option<&'a str>,
    prompt: Option<&'a str>,
    session_id: &'a str,
    cwd: &'a std::path::Path,
}

async fn execute_hook(hook: &HookEntry, env: HookEnvVars<'_>) -> HookResult {
    use tokio::process::Command;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());

    let mut cmd = Command::new(&shell);
    cmd.arg("-c").arg(&hook.command);
    cmd.current_dir(env.cwd);

    // Standard env vars
    cmd.env("CLAUDE_HOOK_EVENT", env.event);
    cmd.env("CLAUDE_SESSION_ID", env.session_id);
    cmd.env("CLAUDE_CWD", env.cwd.to_string_lossy().as_ref());

    if let Some(name) = env.tool_name   { cmd.env("TOOL_NAME", name); }
    if let Some(inp)  = env.tool_input  { cmd.env("TOOL_INPUT", inp); }
    if let Some(res)  = env.tool_result { cmd.env("TOOL_RESULT", res); }
    if let Some(msg)  = env.prompt      { cmd.env("CLAUDE_MESSAGE", msg); }

    // Capture stdout and stderr
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let output = match cmd.output().await {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("Hook command failed to start: {}", e);
            return HookResult::allow();
        }
    };

    let exit_code = output.status.code().unwrap_or(0);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !stderr.is_empty() {
        tracing::debug!("Hook stderr: {stderr}");
    }

    // Exit code 2 = blocking error — always blocks regardless of stdout content.
    // If stdout is JSON, extract a human-readable reason from it instead of dumping raw JSON.
    if exit_code == 2 {
        let trimmed = stdout.trim();
        let stop_reason = if trimmed.starts_with('{') {
            if let Ok(hook_out) = serde_json::from_str::<HookOutput>(trimmed) {
                hook_out.stop_reason
                    .or(hook_out.reason)
                    .unwrap_or_else(|| format!("Hook '{}' blocked execution (exit 2)", hook.command))
            } else {
                // Malformed JSON — show raw so the hook author can debug
                trimmed.to_string()
            }
        } else if trimmed.is_empty() {
            format!("Hook '{}' blocked execution (exit 2)", hook.command)
        } else {
            trimmed.to_string()
        };
        return HookResult {
            should_continue: false,
            stop_reason: Some(stop_reason),
            ..Default::default()
        };
    }

    // Non-zero (not 2) = non-blocking error, log and continue
    if exit_code != 0 {
        tracing::warn!(
            "Hook '{}' exited with code {} (non-blocking)",
            hook.command, exit_code
        );
        return HookResult::allow();
    }

    // Parse JSON output from stdout if present
    let trimmed = stdout.trim();
    if trimmed.starts_with('{')
        && let Ok(hook_out) = serde_json::from_str::<HookOutput>(trimmed) {
            let decision = match hook_out.decision.as_deref() {
                Some("approve") => Some(HookDecision::Approve),
                Some("block")   => Some(HookDecision::Block),
                _ => None,
            };

            if !hook_out.continue_ {
                return HookResult {
                    should_continue: false,
                    stop_reason: hook_out.stop_reason
                        .or(hook_out.reason)
                        .or_else(|| Some(format!("Hook '{}' requested stop", hook.command))),
                    system_message: hook_out.system_message,
                    decision,
                    additional_context: hook_out.additional_context,
                };
            }

            return HookResult {
                should_continue: true,
                stop_reason: None,
                system_message: hook_out.system_message,
                decision,
                additional_context: hook_out.additional_context,
            };
        }

    HookResult::allow()
}
