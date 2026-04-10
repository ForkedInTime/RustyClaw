//! Auto-rollback on test regression — core primitives.
//!
//! Watches Write/Edit tool calls. If resulting code breaks tests, the caller
//! uses these primitives to restore modified files via `git checkout --` and
//! feed the test error back to the model for a retry.
//!
//! This module provides only the building blocks:
//!   - `detect_test_command` — infer runner from project files
//!   - `should_trigger`      — apply trigger-mode rules
//!   - `git_stash_create`    — snapshot working tree without history pollution
//!   - `git_restore_files`   — restore specific files to HEAD
//!   - `run_tests`           — execute the configured test command
//!
//! Task 7 wires these into the Write/Edit tool handler; Task 8 runs
//! end-to-end integration tests. Until Task 7 lands these symbols are unused
//! by the binary target.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

// ── Config types ──────────────────────────────────────────────────────────────

/// Configuration for auto-rollback feature.
#[derive(Debug, Clone)]
pub struct RollbackConfig {
    pub enabled: bool,
    pub trigger: RollbackTrigger,
    pub test_command: Option<String>,
    /// Reserved for a future multi-turn retry loop. The current hook runs
    /// tests exactly once per turn and does not retry — setting this to
    /// anything other than the default has no runtime effect today, and
    /// `Config::load` emits a warning when the user overrides it.
    pub max_retries: u32,
    /// Maximum wall-clock seconds to let the test command run before killing
    /// it and returning `TestResult::Timeout`. `0` means no timeout.
    pub timeout_secs: u64,
}

/// Default test timeout if the user hasn't overridden it.
pub const DEFAULT_TEST_TIMEOUT_SECS: u64 = 60;

/// When should the rollback check run?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollbackTrigger {
    /// Only run when `/autonomy` mode is `auto-edit` or `full-auto`.
    Autonomous,
    /// Run after every edit regardless of autonomy.
    Always,
    /// Never run.
    Off,
}

/// Result of running the test command.
#[derive(Debug)]
pub enum TestResult {
    Pass,
    Fail { stderr: String },
    /// No command detected and none configured.
    NoTestRunner,
    /// Skipped due to trigger rules, git errors, etc.
    Skipped { reason: String },
    Timeout,
}

impl Default for RollbackConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger: RollbackTrigger::Autonomous,
            test_command: None,
            max_retries: 3,
            timeout_secs: DEFAULT_TEST_TIMEOUT_SECS,
        }
    }
}

// ── Detection ─────────────────────────────────────────────────────────────────

/// Detect a test command based on project files in `cwd`.
/// If `override_cmd` is `Some`, return it unchanged.
/// Returns `None` if no runner detected AND no override given.
///
/// Detection order (first hit wins):
///   1. Cargo.toml      → `cargo test`
///   2. package.json    → `npm test`
///   3. pyproject.toml  → `pytest`
///   4. setup.py        → `pytest`
///   5. go.mod          → `go test ./...`
pub fn detect_test_command(cwd: &Path, override_cmd: &Option<String>) -> Option<String> {
    if let Some(cmd) = override_cmd {
        return Some(cmd.clone());
    }

    if cwd.join("Cargo.toml").is_file() {
        return Some("cargo test".to_string());
    }
    if cwd.join("package.json").is_file() {
        return Some("npm test".to_string());
    }
    if cwd.join("pyproject.toml").is_file() || cwd.join("setup.py").is_file() {
        return Some("pytest".to_string());
    }
    if cwd.join("go.mod").is_file() {
        return Some("go test ./...".to_string());
    }
    None
}

// ── Trigger logic ─────────────────────────────────────────────────────────────

/// Check if rollback should trigger right now.
///
/// `autonomy_mode` is a &str matching the current autonomy level:
/// `"read-only"`, `"plan-only"`, `"auto-edit"`, or `"full-auto"`.
pub fn should_trigger(config: &RollbackConfig, autonomy_mode: &str) -> bool {
    if !config.enabled {
        return false;
    }
    match config.trigger {
        RollbackTrigger::Off => false,
        RollbackTrigger::Always => true,
        RollbackTrigger::Autonomous => {
            matches!(autonomy_mode, "auto-edit" | "full-auto")
        }
    }
}

// ── Git primitives ────────────────────────────────────────────────────────────

/// Create a git stash ref without pushing it to the stash list.
///
/// Runs `git stash create`, which returns a commit SHA representing the
/// current worktree state without touching `git stash list`. Returns
/// `Some(sha)` on success, `None` if there are no changes or a git error
/// occurs (e.g., not a git repo).
pub fn git_stash_create(cwd: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("stash")
        .arg("create")
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .ok()?;

    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

/// Restore specific files via `git checkout -- <files>`.
///
/// Returns `Ok(())` on success, `Err(stderr)` on failure. If `files` is
/// empty, returns `Ok(())` immediately (nothing to do).
pub fn git_restore_files(cwd: &Path, files: &[PathBuf]) -> Result<(), String> {
    if files.is_empty() {
        return Ok(());
    }

    let out = Command::new("git")
        .arg("checkout")
        .arg("--")
        .args(files)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("failed to spawn git: {e}"))?;

    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

// ── Test runner ───────────────────────────────────────────────────────────────

/// Run the test command and return a `TestResult`.
///
/// `test_cmd` is a shell-style command string like `"cargo test"` or
/// `"npm test"`. The first whitespace-separated token is the program,
/// the rest are argv. `timeout_secs` caps total wall-clock runtime; if the
/// process is still running past it we send SIGKILL and return
/// `TestResult::Timeout`. Pass `0` to wait indefinitely.
///
/// NOTE on implementation: the original spec called for `wait_timeout`, but
/// pulling in a new dep for ~20 lines isn't worth it. We use a poll+kill loop
/// via `try_wait`, which has the same behavior with no extra deps.
pub fn run_tests(cwd: &Path, test_cmd: &str, timeout_secs: u64) -> TestResult {
    let mut parts = test_cmd.split_whitespace();
    let Some(prog) = parts.next() else {
        return TestResult::Skipped {
            reason: "empty test command".to_string(),
        };
    };
    let args: Vec<&str> = parts.collect();

    let spawn = Command::new(prog)
        .args(&args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut child = match spawn {
        Ok(c) => c,
        Err(e) => {
            return TestResult::Skipped {
                reason: format!("failed to spawn `{test_cmd}`: {e}"),
            };
        }
    };

    // Poll until exit or timeout. `timeout_secs == 0` means "wait forever".
    let timeout = std::time::Duration::from_secs(timeout_secs);
    let has_timeout = timeout_secs > 0;
    let start = std::time::Instant::now();
    let poll = std::time::Duration::from_millis(100);

    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if has_timeout && start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return TestResult::Timeout;
                }
                std::thread::sleep(poll);
            }
            Err(e) => {
                return TestResult::Skipped {
                    reason: format!("failed to wait on child: {e}"),
                };
            }
        }
    }

    // Collect output.
    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            return TestResult::Skipped {
                reason: format!("failed to collect output: {e}"),
            };
        }
    };

    if output.status.success() {
        TestResult::Pass
    } else {
        let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();
        // Some runners (cargo test, go test) print failure details to stdout.
        if stderr.trim().is_empty() {
            stderr = String::from_utf8_lossy(&output.stdout).to_string();
        }
        TestResult::Fail { stderr }
    }
}
