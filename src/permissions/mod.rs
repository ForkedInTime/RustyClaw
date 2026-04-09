/// Permission system — port of utils/permissions/permissions.ts
///
/// Before executing sensitive tools (Bash, FileWrite, FileEdit, FileRead of
/// sensitive paths), a permission check is performed. The result is one of:
///   Allow   — proceed immediately
///   Deny    — block, return an error to Claude
///   Ask     — pause and prompt the user in the TUI
///
/// "Always allow" decisions are remembered for the session.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq)]
pub enum PermissionDecision {
    /// Allow this one time
    Allow,
    /// Allow all future calls to this tool for the rest of the session
    AlwaysAllow,
    /// Deny this call
    Deny,
}

/// Tools that require explicit permission before execution.
/// Mirrors the hasPermissionsToUseTool logic in permissions.ts.
pub const SENSITIVE_TOOLS: &[&str] = &["Bash", "Write", "Edit"];

/// Session-scoped permission state — shared between tool executor and TUI.
#[derive(Clone, Default)]
pub struct PermissionState {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    /// Tools the user has said "always allow" for this session
    always_allowed: HashSet<String>,
    /// Tools permanently denied by settings.json (permissions.deny)
    deny_list: HashSet<String>,
    /// Whether the user enabled --dangerously-skip-permissions
    bypass: bool,
}

impl PermissionState {
    /// Create a new state.
    ///
    /// `allow` pre-populates the always-allowed set (from settings.permissions.allow).
    /// `deny`  pre-populates the deny list      (from settings.permissions.deny).
    pub fn new(bypass: bool, allow: &[String], deny: &[String]) -> Self {
        let mut inner = Inner::default();
        inner.bypass = bypass;
        for t in allow { inner.always_allowed.insert(t.clone()); }
        for t in deny  { inner.deny_list.insert(t.clone()); }
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// Check with optional tool input for prefix-rule matching.
    pub fn check_with_input(&self, tool_name: &str, input: Option<&serde_json::Value>) -> CheckResult {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.bypass {
            return CheckResult::Allow;
        }

        // Check deny list — supports Bash(prefix:...) rules
        for rule in &inner.deny_list {
            if rule_matches(rule, tool_name, input) {
                return CheckResult::Deny;
            }
        }

        if !SENSITIVE_TOOLS.contains(&tool_name) {
            return CheckResult::Allow;
        }

        // Check always-allowed — also supports prefix rules
        for rule in &inner.always_allowed {
            if rule_matches(rule, tool_name, input) {
                return CheckResult::Allow;
            }
        }

        CheckResult::Ask
    }

    /// Record an "always allow" decision for a tool.
    pub fn record_always_allow(&self, tool_name: &str) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .always_allowed
            .insert(tool_name.to_string());
    }
}

/// Check whether a permission rule entry matches the given tool call.
///
/// Rule syntax:
///   - `"Bash"` — matches any Bash call
///   - `"Bash(git:*)"` or `"Bash(prefix:git )"` — matches Bash when command starts with `git `
///   - `"Edit"` — matches any Edit call
fn rule_matches(rule: &str, tool_name: &str, input: Option<&serde_json::Value>) -> bool {
    // Parse: ToolName or ToolName(prefix:...) or ToolName(command:*)
    if let Some(paren_start) = rule.find('(') {
        let rule_tool = &rule[..paren_start];
        if !rule_tool.eq_ignore_ascii_case(tool_name) {
            return false;
        }
        let inner = rule[paren_start + 1..].trim_end_matches(')');

        // Extract prefix: support both "prefix:git " and "git:*" shorthand
        let prefix = if let Some(rest) = inner.strip_prefix("prefix:") {
            rest.to_string()
        } else if inner.ends_with(":*") {
            // "git:*" → command must start with "git "
            format!("{} ", inner.trim_end_matches(":*"))
        } else {
            return false;
        };

        // Check the input's "command" field for Bash, or "file_path" for file tools
        if let Some(inp) = input {
            let value = match tool_name {
                "Bash" => inp["command"].as_str().unwrap_or(""),
                "Write" | "Edit" | "Read" => inp["file_path"].as_str().unwrap_or(""),
                _ => return false,
            };
            return value.starts_with(prefix.as_str());
        }
        false
    } else {
        // Simple name match
        rule.eq_ignore_ascii_case(tool_name)
    }
}

pub enum CheckResult {
    Allow,
    Ask,
    /// Tool is permanently denied (via settings.permissions.deny)
    Deny,
}

/// Split a compound bash command into individual sub-commands.
/// Handles `&&`, `||`, `;`, and `|` as separators.
/// Does NOT descend into subshells `$(...)` or backticks — just top-level splits.
pub fn split_compound_command(cmd: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let bytes = cmd.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;

    while i < len {
        let c = bytes[i];
        match c {
            b'\'' if !in_double => { in_single = !in_single; i += 1; }
            b'"'  if !in_single => { in_double = !in_double; i += 1; }
            b'\\' if !in_single => { i += 2; } // skip escaped char
            _ if in_single || in_double => { i += 1; }
            b'&' if i + 1 < len && bytes[i + 1] == b'&' => {
                let part = cmd[start..i].trim();
                if !part.is_empty() { parts.push(part); }
                i += 2;
                start = i;
            }
            b'|' if i + 1 < len && bytes[i + 1] == b'|' => {
                let part = cmd[start..i].trim();
                if !part.is_empty() { parts.push(part); }
                i += 2;
                start = i;
            }
            b';' => {
                let part = cmd[start..i].trim();
                if !part.is_empty() { parts.push(part); }
                i += 1;
                start = i;
            }
            b'|' => {
                let part = cmd[start..i].trim();
                if !part.is_empty() { parts.push(part); }
                i += 1;
                start = i;
            }
            _ => { i += 1; }
        }
    }
    let tail = cmd[start..].trim();
    if !tail.is_empty() { parts.push(tail); }
    parts
}

/// Check a compound bash command against permission rules.
/// Returns Deny if ANY sub-command matches a deny rule.
/// Returns Allow only if ALL sub-commands match an allow rule.
/// Otherwise returns Ask.
pub fn check_compound_bash(state: &PermissionState, full_command: &str) -> CheckResult {
    let subs = split_compound_command(full_command);
    if subs.is_empty() {
        return CheckResult::Ask;
    }

    let mut any_ask = false;
    for sub in &subs {
        let fake_input = serde_json::json!({ "command": *sub });
        let result = state.check_with_input("Bash", Some(&fake_input));
        match result {
            CheckResult::Deny => return CheckResult::Deny,
            CheckResult::Ask => any_ask = true,
            CheckResult::Allow => {}
        }
    }
    if any_ask { CheckResult::Ask } else { CheckResult::Allow }
}

/// Build a human-readable description of a tool call for the permission dialog.
pub fn describe_tool_call(tool_name: &str, input: &serde_json::Value) -> String {
    match tool_name {
        "Bash" => {
            let cmd = input["command"].as_str().unwrap_or("(unknown)");
            format!("Run shell command:\n  {cmd}")
        }
        "Write" => {
            let path = input["file_path"].as_str().unwrap_or("(unknown)");
            format!("Write/overwrite file:\n  {path}")
        }
        "Edit" => {
            let path = input["file_path"].as_str().unwrap_or("(unknown)");
            let old = input["old_string"].as_str().unwrap_or("");
            format!("Edit file:\n  {path}\n  Replace: {}", truncate(old, 60))
        }
        _ => format!("{tool_name}({})", truncate(&input.to_string(), 80)),
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        // Find a char boundary at or before max to avoid slicing mid-codepoint
        let end = s.char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i < max)
            .last()
            .unwrap_or(0);
        &s[..end]
    }
}
