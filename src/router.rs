/// Smart Model Router — auto-detect task complexity and route to the cheapest capable model.
///
/// Simple tasks (explain, rename, format, typos) → cheap model (Haiku / Ollama)
/// Complex tasks (refactor, architect, debug race conditions) → expensive model (Opus / Sonnet)
///
/// Other tools exist solely to provide this functionality.
/// We do it natively, in the binary.

use tracing::debug;

// ─── Complexity levels ──────────────────────────────────────────────────────

/// Task complexity determines which model tier handles the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Complexity {
    /// Trivial: one-liner answers, typo fixes, simple explanations
    Low,
    /// Moderate: small code changes, test writing, focused debugging
    Medium,
    /// Hard: refactoring, architecture, multi-file changes, complex debugging
    High,
    /// Massive: full codebase analysis, huge multi-file refactors, needs 1M context
    SuperHigh,
}

impl std::fmt::Display for Complexity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Complexity::Low => write!(f, "low"),
            Complexity::Medium => write!(f, "medium"),
            Complexity::High => write!(f, "high"),
            Complexity::SuperHigh => write!(f, "super-high"),
        }
    }
}

// ─── Router configuration ───────────────────────────────────────────────────

/// Model assignments per complexity tier.
#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// Model for low-complexity tasks (default: claude-haiku-4-5)
    pub low_model: String,
    /// Model for medium-complexity tasks (default: claude-sonnet-4-6)
    pub medium_model: String,
    /// Model for high-complexity tasks (default: whatever the user configured)
    pub high_model: String,
    /// Model for super-high tasks needing 1M context (default: claude-opus-4-6)
    pub super_high_model: String,
    /// Whether the router is enabled
    pub enabled: bool,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            low_model: "claude-haiku-4-5".into(),
            medium_model: "claude-sonnet-4-6".into(),
            high_model: String::new(), // filled from config.model at runtime
            super_high_model: "claude-opus-4-6".into(),
            enabled: false,
        }
    }
}

impl RouterConfig {
    /// Create a new router config, inheriting the user's configured model as the high tier.
    pub fn new(user_model: &str) -> Self {
        Self {
            high_model: user_model.to_string(),
            ..Default::default()
        }
    }

    /// Select the model for a given complexity level.
    pub fn model_for(&self, complexity: Complexity) -> &str {
        match complexity {
            Complexity::Low => &self.low_model,
            Complexity::Medium => &self.medium_model,
            Complexity::High => &self.high_model,
            Complexity::SuperHigh => &self.super_high_model,
        }
    }
}

// ─── Complexity detection ───────────────────────────────────────────────────

/// Keywords / patterns that signal super-high complexity (needs 1M context / Opus).
const SUPER_HIGH_SIGNALS: &[&str] = &[
    "entire codebase", "all files", "every file", "whole project", "whole repo",
    "full codebase", "full project", "analyze everything", "review everything",
    "complete rewrite", "full rewrite", "full audit", "codebase-wide",
    "massive refactor", "large-scale", "cross-cutting",
];

/// Keywords / patterns that signal high complexity.
const HIGH_SIGNALS: &[&str] = &[
    "refactor", "rewrite", "architect", "redesign", "migrate",
    "race condition", "deadlock", "concurrency", "parallel",
    "security", "vulnerability", "exploit",
    "optimize", "performance", "benchmark",
    "implement", "build", "create a", "design",
    "multi-file", "across files",
    "debug", "investigate", "diagnose", "root cause",
    "review", "audit", "analyze",
];

/// Keywords / patterns that signal low complexity.
/// NOTE: multi-word phrases only to avoid false positives from substring matching.
/// Single-word signals are checked with word-boundary matching below.
const LOW_SIGNALS: &[&str] = &[
    "explain", "what is", "what does", "what's", "how does",
    "rename", "typo", "spelling", "format", "lint",
    "add a comment", "add comment", "docstring",
    "show me",
    "go ahead", "do it", "thank you",
];

/// Single-word low-complexity signals (matched as whole words).
const LOW_WORDS: &[&str] = &[
    "yes", "no", "ok", "sure", "thanks", "hello", "hi", "hey",
    "help", "version", "list", "print", "proceed", "continue",
];

/// Analyze a user message and determine its complexity.
pub fn detect_complexity(input: &str) -> Complexity {
    let lower = input.to_lowercase();
    let word_count = input.split_whitespace().count();

    // Very short messages are usually low complexity (confirmations, simple questions)
    if word_count <= 4 {
        // Unless they contain a high signal word
        for signal in HIGH_SIGNALS {
            if lower.contains(signal) {
                return Complexity::Medium;
            }
        }
        return Complexity::Low;
    }

    // Score-based detection
    let mut score: i32 = 0;

    // High-complexity signals (strong)
    for signal in HIGH_SIGNALS {
        if lower.contains(signal) {
            score += 4;
        }
    }

    // Low-complexity signals (moderate pull-down)
    for signal in LOW_SIGNALS {
        if lower.contains(signal) {
            score -= 2;
        }
    }
    // Single-word low signals (whole-word match to avoid "no" matching inside "tokens")
    let words: Vec<&str> = lower.split_whitespace().collect();
    for lw in LOW_WORDS {
        if words.iter().any(|w| w == lw) {
            score -= 2;
        }
    }

    // Length heuristic: longer messages tend to be more complex
    if word_count > 50 {
        score += 3;
    } else if word_count > 20 {
        score += 1;
    }

    // Code blocks suggest implementation work
    if input.contains("```") {
        score += 2;
    }

    // Multiple questions suggest complexity
    let question_marks = input.chars().filter(|c| *c == '?').count();
    if question_marks >= 2 {
        score += 1;
    }

    // File paths suggest code work
    if input.contains(".rs") || input.contains(".ts") || input.contains(".py")
        || input.contains(".go") || input.contains(".js")
        || input.contains("src/") || input.contains("./")
    {
        score += 1;
    }

    // Action verbs that suggest implementation (not just reading)
    for verb in &["add", "write", "fix", "change", "update", "modify", "remove", "delete", "test"] {
        if lower.starts_with(verb) || lower.contains(&format!(" {verb} ")) {
            score += 1;
            break;
        }
    }

    // Super-high signals: massive scope tasks needing 1M context
    let mut super_high = false;
    for signal in SUPER_HIGH_SIGNALS {
        if lower.contains(signal) {
            super_high = true;
            score += 6;
        }
    }

    debug!("Complexity score for input ({word_count} words): {score}");

    if super_high {
        Complexity::SuperHigh
    } else if score >= 4 {
        Complexity::High
    } else if score >= 1 {
        Complexity::Medium
    } else {
        Complexity::Low
    }
}

// ─── Router state (shared across turns) ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_confirmations_are_low() {
        assert_eq!(detect_complexity("yes"), Complexity::Low);
        assert_eq!(detect_complexity("ok"), Complexity::Low);
        assert_eq!(detect_complexity("do it"), Complexity::Low);
    }

    #[test]
    fn test_simple_questions_are_low() {
        assert_eq!(detect_complexity("what does this function do?"), Complexity::Low);
        assert_eq!(detect_complexity("explain the config module"), Complexity::Low);
    }

    #[test]
    fn test_refactor_is_high() {
        assert_eq!(
            detect_complexity("refactor the authentication module to use JWT tokens instead of sessions"),
            Complexity::High
        );
    }

    #[test]
    fn test_complex_debug_is_high() {
        assert_eq!(
            detect_complexity("there's a race condition in the connection pool when multiple threads try to acquire a connection simultaneously, can you debug and fix it?"),
            Complexity::High
        );
    }

    #[test]
    fn test_medium_tasks() {
        assert_eq!(
            detect_complexity("add a test for the parse_config function"),
            Complexity::Medium
        );
    }

    #[test]
    fn test_super_high_entire_codebase() {
        assert_eq!(
            detect_complexity("analyze the entire codebase and refactor all error handling to use a consistent pattern"),
            Complexity::SuperHigh
        );
    }

    #[test]
    fn test_super_high_full_audit() {
        assert_eq!(
            detect_complexity("do a full audit of every file in the project for security vulnerabilities"),
            Complexity::SuperHigh
        );
    }
}
