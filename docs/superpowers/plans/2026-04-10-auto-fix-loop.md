# Auto Lint/Test Loop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an Aider-style post-edit retry loop that runs lint + tests after the model edits code, feeds failures back as a synthetic user turn, and iterates up to `max_retries` rounds — replacing the existing single-shot `git checkout --` revert.

**Architecture:** Rename `src/rollback.rs` → `src/autofix.rs`, expand it with a lint-command detector and a combined `run_checks` helper returning a `CheckOutcome` enum. Replace the revert block in `src/tui/run.rs` (around line 4053) with a retry state machine that extracts the decision logic into a testable pure helper `run_auto_fix_check(...) -> AutoFixAction`. The TUI turn loop either continues on pass, re-enters the agentic loop with an injected `Role::User` feedback message on fail-under-cap, or emits a final `SystemMessage` + `Done` on cap-reached.

**Tech Stack:** Rust 2021, tokio async, std::process::Command for subprocess exec (no new deps), serde for settings, tempfile for integration test scaffolding.

**Reference:** Design spec at [docs/superpowers/specs/2026-04-10-auto-fix-loop-design.md](../specs/2026-04-10-auto-fix-loop-design.md).

---

## File Map

### Created
- `src/autofix.rs` — moved from `src/rollback.rs` via `git mv`, expanded with lint detection, `run_checks`, `format_feedback_message`, and the `run_auto_fix_check` public helper.
- `tests/autofix_loop_integration.rs` — integration tests for `run_auto_fix_check` against real tmpdir fixtures.

### Deleted
- `src/rollback.rs` — becomes `src/autofix.rs` via `git mv` (history preserved). The `git_restore_files` and `git_stash_create` functions are dropped during the move since the retry loop never reverts.

### Modified
- `src/lib.rs:20` — `pub mod rollback;` → `pub mod autofix;`
- `src/main.rs:17` — `mod rollback;` → `mod autofix;`
- `src/settings.rs:246` — rename `AutoRollbackSettings` → `AutoFixSettings`, add `lint_command: Option<String>`, add `#[serde(alias = "autoRollback")]`, rename field `auto_rollback` → `auto_fix`.
- `src/config.rs:340-346` — rename `auto_rollback: RollbackConfig` → `auto_fix: AutoFixConfig`; update the corresponding `Default::default` at line ~421 and the settings-merge block at lines 506-534.
- `src/tui/run.rs:3859` — rename `rollback_touched` → `auto_fix_touched`; add `auto_fix_retries: u32 = 0` above the outer loop; replace the revert block at lines 4053-4125 with a call to `crate::autofix::run_auto_fix_check(...)` and a match on its return.

---

## Task 1: Rename module via git mv + update mod declarations

**Files:**
- Rename: `src/rollback.rs` → `src/autofix.rs`
- Modify: `src/lib.rs:20`, `src/main.rs:17`

This task is a pure rename — no type renames yet, no behaviour change. Subsequent tasks rename the types inside. Keeping the type-rename separate keeps the commit small and the history bisect-clean.

- [ ] **Step 1: Git-move the file**

```bash
cd /mnt/Storage/Projects/RustyClaw
git mv src/rollback.rs src/autofix.rs
```

- [ ] **Step 2: Update the module declaration in `src/lib.rs`**

Find line 20, change `pub mod rollback;` to `pub mod autofix;`:

```rust
pub mod autofix;
```

- [ ] **Step 3: Update the module declaration in `src/main.rs`**

Find line 17, change `mod rollback;` to `mod autofix;`:

```rust
mod autofix;
```

- [ ] **Step 4: Update all `crate::rollback::` references to `crate::autofix::`**

```bash
grep -rn "crate::rollback" src/ tests/
```

Expected hits: `src/config.rs` (3 lines around 346, 421, 511-513), `src/tui/run.rs` (5 lines around 4062-4123).

Edit each with search-and-replace:

```bash
# Verify count before replacing
grep -rln "crate::rollback" src/ tests/
# Replace in each file using an editor; example one-liner that avoids sed footguns:
#   open the file, replace all `crate::rollback` with `crate::autofix`
```

- [ ] **Step 5: Verify the build passes**

Run: `cargo build 2>&1 | tail -20`
Expected: `Finished \`dev\` profile` with no errors.

- [ ] **Step 6: Commit**

```bash
git add src/autofix.rs src/lib.rs src/main.rs src/config.rs src/tui/run.rs
git commit -m "$(cat <<'EOF'
refactor(autofix): rename rollback module to autofix

Pure rename via git mv, plus import-path updates. No behaviour change.
Prepares the module for the retry-loop rewrite in the next commits.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 2: Rename types inside autofix.rs

**Files:**
- Modify: `src/autofix.rs` (all type declarations)
- Modify: `src/config.rs:340-534` (field type references)
- Modify: `src/settings.rs:244-273` (settings struct + field name)
- Modify: `src/tui/run.rs:3859, 4062-4123` (field reference sites)

Still no behaviour change. This task renames:
- `RollbackConfig` → `AutoFixConfig`
- `RollbackTrigger` → `AutoFixTrigger`
- `AutoRollbackSettings` → `AutoFixSettings`
- `Config.auto_rollback` → `Config.auto_fix`
- `Settings.auto_rollback` → `Settings.auto_fix`
- The serde JSON key stays `"autoRollback"` for this task; the key is renamed in Task 5.

- [ ] **Step 1: Rename types in `src/autofix.rs`**

Open `src/autofix.rs` and apply these replacements globally inside the file:

- `pub struct RollbackConfig` → `pub struct AutoFixConfig`
- `pub enum RollbackTrigger` → `pub enum AutoFixTrigger`
- `impl Default for RollbackConfig` → `impl Default for AutoFixConfig`
- All other `RollbackConfig` references → `AutoFixConfig`
- All other `RollbackTrigger::` references → `AutoFixTrigger::`

- [ ] **Step 2: Rename the field and type in `src/config.rs`**

Find around line 345-346:

```rust
/// Auto-rollback on test regression: run tests after Write/Edit and
/// `git checkout --` the modified files if tests fail.
#[serde(skip)]
pub auto_rollback: crate::rollback::RollbackConfig,
```

Replace with:

```rust
/// Auto-fix loop: after Write/Edit, run lint + tests and re-prompt the
/// model with the failure output up to `max_retries` times.
#[serde(skip)]
pub auto_fix: crate::autofix::AutoFixConfig,
```

Find around line 421:

```rust
auto_rollback: crate::rollback::RollbackConfig::default(),
```

Replace with:

```rust
auto_fix: crate::autofix::AutoFixConfig::default(),
```

Find the block around lines 506-534 and replace every `cfg.auto_rollback.` with `cfg.auto_fix.`, every `crate::rollback::RollbackTrigger::` with `crate::autofix::AutoFixTrigger::`, and every `settings.auto_rollback` with `settings.auto_fix`.

- [ ] **Step 3: Rename in `src/settings.rs`**

Find around line 244-246:

```rust
/// Auto-rollback on test regression — run tests after Write/Edit and revert on failure.
#[serde(rename = "autoRollback")]
pub auto_rollback: Option<AutoRollbackSettings>,
```

Replace with:

```rust
/// Auto-fix loop: run lint + tests after Write/Edit and re-prompt on failure.
#[serde(rename = "autoRollback")]
pub auto_fix: Option<AutoFixSettings>,
```

Find around line 258-273:

```rust
/// Settings for auto-rollback on test regression.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AutoRollbackSettings {
```

Replace `pub struct AutoRollbackSettings` with `pub struct AutoFixSettings`. Update the doc comment to:

```rust
/// Settings for the auto-fix loop (lint + tests + retry feedback).
```

Find the `Settings::merge` block around line 390:

```rust
auto_rollback:        other.auto_rollback.or(self.auto_rollback),
```

Replace with:

```rust
auto_fix:             other.auto_fix.or(self.auto_fix),
```

- [ ] **Step 4: Rename in `src/tui/run.rs`**

Find line 3859:

```rust
let mut rollback_touched: Vec<std::path::PathBuf> = Vec::new();
```

Replace with:

```rust
let mut auto_fix_touched: Vec<std::path::PathBuf> = Vec::new();
```

Replace every `rollback_touched` in the file with `auto_fix_touched` (there are 5 occurrences around lines 4040-4097).

Replace every `config.auto_rollback` with `config.auto_fix` and every `crate::rollback::` with `crate::autofix::` in this file.

- [ ] **Step 5: Build and verify**

Run: `cargo build 2>&1 | tail -20`
Expected: `Finished \`dev\` profile` with no errors.

Run: `cargo test --lib 2>&1 | tail -20`
Expected: all existing tests pass (should be ~181 total, matching the pre-rename count).

- [ ] **Step 6: Commit**

```bash
git add src/autofix.rs src/config.rs src/settings.rs src/tui/run.rs
git commit -m "$(cat <<'EOF'
refactor(autofix): rename Rollback* types to AutoFix*

Pure rename: RollbackConfig -> AutoFixConfig, RollbackTrigger ->
AutoFixTrigger, AutoRollbackSettings -> AutoFixSettings, and the
`auto_rollback` field on Config/Settings -> `auto_fix`. The serde
JSON key stays "autoRollback" for this commit; the user-facing key
moves to "autoFixLoop" (with alias) in a follow-up commit so the
diff stays reviewable.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 3: Remove the revert path (git_restore_files) and update the TUI block to a single-shot check

**Files:**
- Modify: `src/autofix.rs` (delete `git_restore_files`, `git_stash_create`)
- Modify: `src/tui/run.rs:4053-4125` (replace revert block with "emit failure SystemMessage only")

This task **removes** the file-revert behaviour but does **not yet** add the retry loop. After this commit, on test failure the TUI prints the lint/test output as a `SystemMessage` and continues the turn normally — the model sees the failure via the SystemMessage on the next turn if it asks, but there is no injected retry. This gives us a clean intermediate state where the scary `git checkout --` code is gone before we layer retry logic on top.

- [ ] **Step 1: Delete `git_restore_files` and `git_stash_create` from `src/autofix.rs`**

Open `src/autofix.rs`. Remove the entire `// ── Git primitives ─` section (roughly lines 130-183 in the current file), including both functions. Also remove the `use std::path::{Path, PathBuf};` line if `PathBuf` is no longer used after deletion (check: `detect_test_command` uses `&Path`, so keep `Path`; drop `PathBuf`).

Leave `run_tests`, `detect_test_command`, `should_trigger`, `TestResult`, `AutoFixConfig`, `AutoFixTrigger`, and `DEFAULT_TEST_TIMEOUT_SECS` in place.

- [ ] **Step 2: Remove the `#![allow(dead_code)]` if the file is now fully used**

Check whether the file still needs `#![allow(dead_code)]`. If `run_tests`, `detect_test_command`, and `should_trigger` are all called from `src/tui/run.rs` (they are), delete the `#![allow(dead_code)]` line at the top of the file.

- [ ] **Step 3: Replace the revert block in `src/tui/run.rs`**

Find the block starting at line 4053 (`// ── Auto-rollback check ───`) through line 4125 (the closing `}` of the outer `if !auto_fix_touched.is_empty()` block). Replace the entire block with:

```rust
                // ── Auto-fix check (pre-retry-loop intermediate) ─────────────
                // Run the detected test command once per turn after all tool
                // calls complete. On failure, surface the output as a
                // SystemMessage. The retry loop lands in a follow-up commit.
                if !auto_fix_touched.is_empty()
                    && crate::autofix::should_trigger(
                        &config.auto_fix,
                        &config.autonomy,
                    )
                {
                    let test_cmd = crate::autofix::detect_test_command(
                        &config.cwd,
                        &config.auto_fix.test_command,
                    );
                    if let Some(cmd) = test_cmd {
                        let _ = tx.send(AppEvent::SystemMessage(
                            format!("[auto-fix] running tests: {cmd}"),
                        ));
                        match crate::autofix::run_tests(
                            &config.cwd,
                            &cmd,
                            config.auto_fix.timeout_secs,
                        ) {
                            crate::autofix::TestResult::Pass => {
                                let _ = tx.send(AppEvent::SystemMessage(
                                    "[auto-fix] tests pass".into(),
                                ));
                            }
                            crate::autofix::TestResult::Fail { stderr } => {
                                let trimmed: String =
                                    stderr.chars().take(2000).collect();
                                let _ = tx.send(AppEvent::SystemMessage(
                                    format!(
                                        "[auto-fix] tests failed:\n{trimmed}"
                                    ),
                                ));
                            }
                            crate::autofix::TestResult::Timeout => {
                                let _ = tx.send(AppEvent::SystemMessage(
                                    format!(
                                        "[auto-fix] test run timed out after {}s, skipping",
                                        config.auto_fix.timeout_secs,
                                    ),
                                ));
                            }
                            crate::autofix::TestResult::NoTestRunner => {
                                // Silent skip.
                            }
                            crate::autofix::TestResult::Skipped { reason } => {
                                let _ = tx.send(AppEvent::SystemMessage(
                                    format!("[auto-fix] skipped: {reason}"),
                                ));
                            }
                        }
                    }
                    auto_fix_touched.clear();
                }
```

- [ ] **Step 4: Build and verify**

Run: `cargo build 2>&1 | tail -20`
Expected: `Finished \`dev\` profile` with no errors (a `dead_code` warning on `auto_fix_touched.clear()` is acceptable — it will become load-bearing in Task 7).

Run: `cargo test --lib 2>&1 | tail -20`
Expected: all existing tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/autofix.rs src/tui/run.rs
git commit -m "$(cat <<'EOF'
refactor(autofix): drop the git-revert path

Delete git_restore_files and git_stash_create — the working tree is
no longer reverted on test failure. The TUI now surfaces failing
test output as a SystemMessage and leaves the files alone. The retry
loop lands in a follow-up commit; this intermediate step makes the
scary revert code go away first so the retry rewrite is easier to
review.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 4: Rename TestResult → CommandResult, run_tests → run_command, inside autofix.rs

**Files:**
- Modify: `src/autofix.rs` (rename type + function)
- Modify: `src/tui/run.rs:4053-4125` (update call sites)

Pure rename so the helper's name matches its new dual use (lint + tests). Enables Task 5 to add `run_checks` that calls `run_command` twice.

- [ ] **Step 1: Rename in `src/autofix.rs`**

Inside `src/autofix.rs`:

- `pub enum TestResult` → `pub enum CommandResult`
- `/// Result of running the test command.` → `/// Result of running a single lint or test command.`
- `pub fn run_tests(` → `pub fn run_command(`
- `/// Run the test command and return a \`TestResult\`.` → `/// Run a single command and return a \`CommandResult\`.`
- Every `TestResult::` inside this file → `CommandResult::`

The parameter name `test_cmd` on `run_command` becomes `cmd`:

```rust
pub fn run_command(cwd: &Path, cmd: &str, timeout_secs: u64) -> CommandResult {
    // ... body unchanged, with `test_cmd` references replaced by `cmd`
}
```

- [ ] **Step 2: Update the call sites in `src/tui/run.rs`**

Inside the auto-fix block you rewrote in Task 3, replace:

- `crate::autofix::run_tests(` → `crate::autofix::run_command(`
- `crate::autofix::TestResult::` → `crate::autofix::CommandResult::`

- [ ] **Step 3: Build and verify**

Run: `cargo build 2>&1 | tail -20`
Expected: `Finished \`dev\` profile` with no errors.

Run: `cargo test --lib 2>&1 | tail -20`
Expected: all existing tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/autofix.rs src/tui/run.rs
git commit -m "$(cat <<'EOF'
refactor(autofix): rename TestResult -> CommandResult, run_tests -> run_command

The helper now runs both lint and test commands, so the test-specific
naming no longer fits. Pure rename, no behaviour change.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 5: Add lint detection + CheckOutcome enum + run_checks helper (TDD)

**Files:**
- Modify: `src/autofix.rs` — new `detect_lint_command` function, new `CheckOutcome` enum, new `run_checks` function, new `#[cfg(test)] mod tests` module

This task is pure-Rust additive — no TUI changes yet. All new functions are covered by unit tests written red-first.

- [ ] **Step 1: Write the failing test for `detect_lint_command` (Cargo)**

Add the following `#[cfg(test)]` block at the bottom of `src/autofix.rs` if it doesn't already have one. If it does, append to the existing one:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn detect_lint_command_cargo() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\nversion = \"0.1.0\"\n").unwrap();
        assert_eq!(
            detect_lint_command(dir.path(), &None),
            Some("cargo clippy --all-targets -- -D warnings".to_string())
        );
    }
}
```

You will need `tempfile` as a dev-dependency. Check if it's already present:

```bash
grep -A2 dev-dependencies Cargo.toml | grep tempfile
```

If missing, add it to `[dev-dependencies]` in `Cargo.toml`:

```toml
tempfile = "3"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib autofix::tests::detect_lint_command_cargo 2>&1 | tail -20`
Expected: FAIL with `cannot find function \`detect_lint_command\` in this scope`.

- [ ] **Step 3: Implement `detect_lint_command`**

Add this function above the `#[cfg(test)]` block in `src/autofix.rs`, right after `detect_test_command`:

```rust
/// Detect a lint command based on project files in `cwd`.
/// If `override_cmd` is `Some`, return it unchanged.
/// Returns `None` if no linter detected AND no override given.
///
/// Detection order (first hit wins, mirrors `detect_test_command`):
///   1. Cargo.toml      → `cargo clippy --all-targets -- -D warnings`
///   2. package.json    → `npx --no-install eslint .`
///   3. pyproject.toml  → `ruff check .`
///   4. setup.py        → `ruff check .`
///   5. go.mod          → `go vet ./...`
pub fn detect_lint_command(cwd: &Path, override_cmd: &Option<String>) -> Option<String> {
    if let Some(cmd) = override_cmd {
        return Some(cmd.clone());
    }

    if cwd.join("Cargo.toml").is_file() {
        return Some("cargo clippy --all-targets -- -D warnings".to_string());
    }
    if cwd.join("package.json").is_file() {
        return Some("npx --no-install eslint .".to_string());
    }
    if cwd.join("pyproject.toml").is_file() || cwd.join("setup.py").is_file() {
        return Some("ruff check .".to_string());
    }
    if cwd.join("go.mod").is_file() {
        return Some("go vet ./...".to_string());
    }
    None
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib autofix::tests::detect_lint_command_cargo 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Add the remaining detect_lint_command tests**

Append to `mod tests`:

```rust
    #[test]
    fn detect_lint_command_npm() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("package.json"), "{}").unwrap();
        assert_eq!(
            detect_lint_command(dir.path(), &None),
            Some("npx --no-install eslint .".to_string())
        );
    }

    #[test]
    fn detect_lint_command_python_pyproject() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("pyproject.toml"), "[project]\nname = \"x\"\n").unwrap();
        assert_eq!(
            detect_lint_command(dir.path(), &None),
            Some("ruff check .".to_string())
        );
    }

    #[test]
    fn detect_lint_command_python_setuppy() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("setup.py"), "from setuptools import setup; setup()").unwrap();
        assert_eq!(
            detect_lint_command(dir.path(), &None),
            Some("ruff check .".to_string())
        );
    }

    #[test]
    fn detect_lint_command_go() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("go.mod"), "module x\n").unwrap();
        assert_eq!(
            detect_lint_command(dir.path(), &None),
            Some("go vet ./...".to_string())
        );
    }

    #[test]
    fn detect_lint_command_override_wins() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        assert_eq!(
            detect_lint_command(dir.path(), &Some("my-linter".to_string())),
            Some("my-linter".to_string())
        );
    }

    #[test]
    fn detect_lint_command_no_runner() {
        let dir = tempdir().unwrap();
        assert_eq!(detect_lint_command(dir.path(), &None), None);
    }
```

Run: `cargo test --lib autofix::tests::detect_lint_command 2>&1 | tail -20`
Expected: 7 passed.

- [ ] **Step 6: Write the failing test for `CheckOutcome` + `run_checks` — both pass path**

Add to `mod tests`:

```rust
    #[test]
    fn run_checks_both_pass() {
        let dir = tempdir().unwrap();
        let outcome = run_checks(
            dir.path(),
            Some("true"),  // lint: unix `true` exits 0
            Some("true"),  // tests: same
            5,
        );
        assert!(matches!(outcome, CheckOutcome::Pass), "got {outcome:?}");
    }
```

Run: `cargo test --lib autofix::tests::run_checks_both_pass 2>&1 | tail -10`
Expected: FAIL with `cannot find type \`CheckOutcome\``.

- [ ] **Step 7: Implement `CheckOutcome` and `run_checks`**

Add above the `#[cfg(test)]` block (right after the `CommandResult` enum in `src/autofix.rs`):

```rust
/// Aggregate outcome of `run_checks`: lint + tests combined.
#[derive(Debug)]
pub enum CheckOutcome {
    /// Both commands passed (or were not configured and had no effect).
    Pass,
    /// At least one command failed. `test_stderr` is `None` when lint
    /// failed and tests were skipped (fast-fail).
    Fail {
        lint_stderr: Option<String>,
        test_stderr: Option<String>,
    },
    /// Neither lint nor tests are configured/detected. Caller should
    /// treat this as a silent pass.
    NoRunners,
    /// Something prevented the checks from running (bad command string,
    /// spawn error, etc.). Caller should log and continue.
    Skipped { reason: String },
}

/// Run the lint command (if any), then the test command (if any), and
/// return a combined `CheckOutcome`. If lint fails, tests are skipped
/// (fast-fail) — `test_stderr` will be `None` in that case.
pub fn run_checks(
    cwd: &Path,
    lint_cmd: Option<&str>,
    test_cmd: Option<&str>,
    timeout_secs: u64,
) -> CheckOutcome {
    if lint_cmd.is_none() && test_cmd.is_none() {
        return CheckOutcome::NoRunners;
    }

    let mut lint_stderr: Option<String> = None;
    let mut lint_failed = false;

    if let Some(cmd) = lint_cmd {
        match run_command(cwd, cmd, timeout_secs) {
            CommandResult::Pass => {}
            CommandResult::Fail { stderr } => {
                lint_failed = true;
                lint_stderr = Some(stderr);
            }
            CommandResult::Timeout => {
                lint_failed = true;
                lint_stderr = Some(format!(
                    "(lint timed out after {timeout_secs}s)"
                ));
            }
            CommandResult::Skipped { reason } => {
                return CheckOutcome::Skipped { reason };
            }
            CommandResult::NoTestRunner => {
                // Shouldn't happen for a concrete command string, but
                // handle defensively.
            }
        }
    }

    // Fast-fail: skip tests if lint failed.
    if lint_failed {
        return CheckOutcome::Fail {
            lint_stderr,
            test_stderr: None,
        };
    }

    let mut test_stderr: Option<String> = None;
    let mut test_failed = false;

    if let Some(cmd) = test_cmd {
        match run_command(cwd, cmd, timeout_secs) {
            CommandResult::Pass => {}
            CommandResult::Fail { stderr } => {
                test_failed = true;
                test_stderr = Some(stderr);
            }
            CommandResult::Timeout => {
                test_failed = true;
                test_stderr = Some(format!(
                    "(tests timed out after {timeout_secs}s)"
                ));
            }
            CommandResult::Skipped { reason } => {
                return CheckOutcome::Skipped { reason };
            }
            CommandResult::NoTestRunner => {
                // Defensive; same as above.
            }
        }
    }

    if test_failed {
        CheckOutcome::Fail {
            lint_stderr,
            test_stderr,
        }
    } else {
        CheckOutcome::Pass
    }
}
```

Run: `cargo test --lib autofix::tests::run_checks_both_pass 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 8: Add the remaining run_checks tests**

Append to `mod tests`:

```rust
    #[test]
    fn run_checks_lint_fail_skips_tests() {
        let dir = tempdir().unwrap();
        // lint `false` exits 1; tests command writes a sentinel file if run.
        let sentinel = dir.path().join("tests_ran");
        let sentinel_str = sentinel.display().to_string();
        let test_cmd = format!("sh -c 'touch {sentinel_str}'");
        let outcome = run_checks(
            dir.path(),
            Some("false"),
            Some(&test_cmd),
            5,
        );
        match outcome {
            CheckOutcome::Fail { lint_stderr: _, test_stderr } => {
                assert!(test_stderr.is_none(), "tests must be skipped when lint fails");
            }
            other => panic!("expected Fail, got {other:?}"),
        }
        assert!(!sentinel.exists(), "sentinel file should not exist — tests must not have run");
    }

    #[test]
    fn run_checks_lint_pass_tests_fail() {
        let dir = tempdir().unwrap();
        let outcome = run_checks(dir.path(), Some("true"), Some("false"), 5);
        match outcome {
            CheckOutcome::Fail { lint_stderr, test_stderr } => {
                assert!(lint_stderr.is_none());
                assert!(test_stderr.is_some());
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn run_checks_no_runners() {
        let dir = tempdir().unwrap();
        let outcome = run_checks(dir.path(), None, None, 5);
        assert!(matches!(outcome, CheckOutcome::NoRunners), "got {outcome:?}");
    }

    #[test]
    fn run_checks_lint_only_pass() {
        let dir = tempdir().unwrap();
        let outcome = run_checks(dir.path(), Some("true"), None, 5);
        assert!(matches!(outcome, CheckOutcome::Pass), "got {outcome:?}");
    }

    #[test]
    fn run_checks_tests_only_fail() {
        let dir = tempdir().unwrap();
        let outcome = run_checks(dir.path(), None, Some("false"), 5);
        match outcome {
            CheckOutcome::Fail { lint_stderr, test_stderr } => {
                assert!(lint_stderr.is_none());
                assert!(test_stderr.is_some());
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }
```

Run: `cargo test --lib autofix::tests::run_checks 2>&1 | tail -20`
Expected: 5 run_checks tests pass.

- [ ] **Step 9: Commit**

```bash
git add src/autofix.rs Cargo.toml
git commit -m "$(cat <<'EOF'
feat(autofix): add lint detection, CheckOutcome, and run_checks helper

detect_lint_command mirrors detect_test_command across Rust, Node,
Python, and Go. run_checks runs lint first and fast-fails on lint
failure so tests only run once the code compiles cleanly.

Adds 12 unit tests covering detection and the aggregate outcome.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 6: Add format_feedback_message + lint_command to AutoFixConfig/AutoFixSettings

**Files:**
- Modify: `src/autofix.rs` (new function + tests)
- Modify: `src/autofix.rs` (`AutoFixConfig` struct: add `lint_command`)
- Modify: `src/settings.rs` (`AutoFixSettings` struct: add `lint_command`)
- Modify: `src/config.rs:506-534` (merge `lint_command` into `AutoFixConfig`)

- [ ] **Step 1: Write the failing test for `format_feedback_message` — both failed**

Add to `src/autofix.rs`'s `mod tests`:

```rust
    #[test]
    fn format_feedback_message_both_failed() {
        let msg = format_feedback_message(
            Some("cargo clippy"),
            Some("cargo test"),
            Some("warning: unused variable `x`"),
            Some("test foo ... FAILED"),
        );
        assert!(msg.contains("Your last edits failed"));
        assert!(msg.contains("## Lint (cargo clippy)"));
        assert!(msg.contains("warning: unused variable"));
        assert!(msg.contains("## Tests (cargo test)"));
        assert!(msg.contains("test foo ... FAILED"));
        assert!(msg.contains("Do not disable lints"));
        assert!(msg.contains("#[allow"));
    }
```

Run: `cargo test --lib autofix::tests::format_feedback_message_both_failed 2>&1 | tail -10`
Expected: FAIL with `cannot find function \`format_feedback_message\``.

- [ ] **Step 2: Implement `format_feedback_message`**

Add above the `#[cfg(test)]` block in `src/autofix.rs`:

```rust
/// Maximum stderr bytes to include per section in the retry feedback message.
/// Keeps context cost predictable across retries.
pub const MAX_FEEDBACK_SECTION_BYTES: usize = 2048;

/// Build the synthetic user-text message sent to the model after a
/// failed lint/test round. Trims each stderr section to
/// `MAX_FEEDBACK_SECTION_BYTES` and appends an explicit anti-cheat
/// clause so the model does not converge on `#[allow(...)]` /
/// `eslint-disable` / etc.
pub fn format_feedback_message(
    lint_cmd: Option<&str>,
    test_cmd: Option<&str>,
    lint_stderr: Option<&str>,
    test_stderr: Option<&str>,
) -> String {
    let lint_cmd_str = lint_cmd.unwrap_or("(none)");
    let test_cmd_str = test_cmd.unwrap_or("(none)");

    let lint_body = match lint_stderr {
        Some(s) => trim_section(s),
        None => "(no output)".to_string(),
    };
    let test_body = match test_stderr {
        Some(s) => trim_section(s),
        None if lint_stderr.is_some() => "(skipped: lint failed)".to_string(),
        None => "(no output)".to_string(),
    };

    format!(
        "Your last edits failed automated checks. Fix the issues below.\n\
         \n\
         ## Lint ({lint_cmd_str})\n\
         {lint_body}\n\
         \n\
         ## Tests ({test_cmd_str})\n\
         {test_body}\n\
         \n\
         Make the minimum edits required to make both pass. Do not disable \
         lints, skip tests, or add `#[allow(...)]` / `# type: ignore` / \
         `eslint-disable` / `//nolint` unless the original code had them. \
         If a test assertion is genuinely wrong, explain why before changing it."
    )
}

fn trim_section(s: &str) -> String {
    if s.len() <= MAX_FEEDBACK_SECTION_BYTES {
        return s.to_string();
    }
    // Trim from the start so the tail of the stderr (usually the most
    // actionable part) is preserved.
    let start = s.len() - MAX_FEEDBACK_SECTION_BYTES;
    // Back up to the nearest char boundary so we don't slice inside a codepoint.
    let mut start = start;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    format!("... (output trimmed)\n{}", &s[start..])
}
```

Run: `cargo test --lib autofix::tests::format_feedback_message_both_failed 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 3: Add the remaining format_feedback_message tests**

Append to `mod tests`:

```rust
    #[test]
    fn format_feedback_message_lint_only() {
        let msg = format_feedback_message(
            Some("cargo clippy"),
            Some("cargo test"),
            Some("warning: dead code"),
            None,
        );
        assert!(msg.contains("## Lint (cargo clippy)"));
        assert!(msg.contains("warning: dead code"));
        assert!(msg.contains("(skipped: lint failed)"));
    }

    #[test]
    fn format_feedback_message_tests_only() {
        let msg = format_feedback_message(
            Some("cargo clippy"),
            Some("cargo test"),
            None,
            Some("assertion failed: x == 1"),
        );
        assert!(msg.contains("(no output)"));
        assert!(msg.contains("assertion failed"));
    }

    #[test]
    fn format_feedback_message_truncates_lint() {
        let big = "x".repeat(5000);
        let msg = format_feedback_message(
            Some("cargo clippy"),
            Some("cargo test"),
            Some(&big),
            None,
        );
        // Should contain the truncation marker
        assert!(msg.contains("(output trimmed)"));
        // Should not contain all 5000 x's
        let x_count = msg.matches('x').count();
        assert!(x_count <= MAX_FEEDBACK_SECTION_BYTES + 10, "x_count = {x_count}");
    }

    #[test]
    fn format_feedback_message_truncates_tests() {
        let big = "y".repeat(5000);
        let msg = format_feedback_message(
            Some("cargo clippy"),
            Some("cargo test"),
            Some("(lint passed)"),
            Some(&big),
        );
        assert!(msg.contains("(output trimmed)"));
        let y_count = msg.matches('y').count();
        assert!(y_count <= MAX_FEEDBACK_SECTION_BYTES + 10);
    }

    #[test]
    fn trim_section_respects_utf8_boundaries() {
        // 2049 bytes where the byte at index 1 starts a 3-byte codepoint ('€' = E2 82 AC)
        let s = format!("€{}", "a".repeat(MAX_FEEDBACK_SECTION_BYTES));
        // trim_section should not panic and should return valid UTF-8
        let out = trim_section(&s);
        assert!(out.is_char_boundary(0));
    }
```

Run: `cargo test --lib autofix::tests::format_feedback_message 2>&1 | tail -20`
Expected: all format_feedback_message tests pass.

Run: `cargo test --lib autofix::tests::trim_section_respects_utf8_boundaries 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 4: Add `lint_command` field to `AutoFixConfig`**

In `src/autofix.rs`, find:

```rust
pub struct AutoFixConfig {
    pub enabled: bool,
    pub trigger: AutoFixTrigger,
    pub test_command: Option<String>,
```

Insert `lint_command` above `test_command`:

```rust
pub struct AutoFixConfig {
    pub enabled: bool,
    pub trigger: AutoFixTrigger,
    pub lint_command: Option<String>,
    pub test_command: Option<String>,
```

Update `impl Default for AutoFixConfig`:

```rust
impl Default for AutoFixConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger: AutoFixTrigger::Autonomous,
            lint_command: None,
            test_command: None,
            max_retries: 3,
            timeout_secs: DEFAULT_TEST_TIMEOUT_SECS,
        }
    }
}
```

- [ ] **Step 5: Add `lint_command` field to `AutoFixSettings`**

In `src/settings.rs`, find the `AutoFixSettings` struct:

```rust
pub struct AutoFixSettings {
    pub enabled: Option<bool>,
    pub trigger: Option<String>,
    pub test_command: Option<String>,
```

Insert above `test_command`:

```rust
pub struct AutoFixSettings {
    pub enabled: Option<bool>,
    pub trigger: Option<String>,
    pub lint_command: Option<String>,
    pub test_command: Option<String>,
```

- [ ] **Step 6: Merge `lint_command` in `src/config.rs`**

Find the block around line 516 in `src/config.rs`:

```rust
            if ar.test_command.is_some() {
                cfg.auto_fix.test_command = ar.test_command.clone();
            }
```

Insert above:

```rust
            if ar.lint_command.is_some() {
                cfg.auto_fix.lint_command = ar.lint_command.clone();
            }
```

- [ ] **Step 7: Build and verify**

Run: `cargo build 2>&1 | tail -20`
Expected: `Finished \`dev\` profile` with no errors.

Run: `cargo test --lib 2>&1 | tail -20`
Expected: all tests pass (previous count + 5 new format tests + 1 utf8 test).

- [ ] **Step 8: Commit**

```bash
git add src/autofix.rs src/settings.rs src/config.rs
git commit -m "$(cat <<'EOF'
feat(autofix): lint_command config + format_feedback_message

AutoFixConfig and AutoFixSettings gain a lint_command override. The
new format_feedback_message builds the synthetic user turn that gets
injected back into the conversation when lint or tests fail, including
the anti-cheat clause that stops the model from converging on
#[allow(dead_code)] workarounds.

Section bodies are trimmed to MAX_FEEDBACK_SECTION_BYTES (2048) from
the tail, on a char boundary so multi-byte UTF-8 survives.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 7: Add run_auto_fix_check helper + AutoFixAction enum (TDD, pure function)

**Files:**
- Modify: `src/autofix.rs` (new public enum + function)

The testable decision helper lives in `autofix.rs` so the TUI call site becomes a tiny match. Pure function, no tokio, fully testable.

- [ ] **Step 1: Write the failing test for `run_auto_fix_check` — pass path**

Add to `mod tests`:

```rust
    #[test]
    fn run_auto_fix_check_pass() {
        let dir = tempdir().unwrap();
        let cfg = AutoFixConfig {
            enabled: true,
            trigger: AutoFixTrigger::Always,
            lint_command: Some("true".to_string()),
            test_command: Some("true".to_string()),
            max_retries: 3,
            timeout_secs: 5,
        };
        let action = run_auto_fix_check(dir.path(), &cfg, "auto-edit", 0);
        assert!(matches!(action, AutoFixAction::Continue { .. }), "got {action:?}");
    }
```

Run: `cargo test --lib autofix::tests::run_auto_fix_check_pass 2>&1 | tail -10`
Expected: FAIL with `cannot find function \`run_auto_fix_check\``.

- [ ] **Step 2: Implement `AutoFixAction` and `run_auto_fix_check`**

Add above the `#[cfg(test)]` block:

```rust
/// The decision returned by `run_auto_fix_check` — tells the TUI turn
/// loop what to do next.
#[derive(Debug)]
pub enum AutoFixAction {
    /// Trigger rules said skip, no runners detected, or checks passed.
    /// `status` is a short human-readable description for the
    /// SystemMessage stream (or `None` for silent skip).
    Continue { status: Option<String> },
    /// Checks failed and we have retries left. Caller should append
    /// `Message { role: User, content: [Text { feedback }] }` to the
    /// conversation and re-enter the agentic loop. `status` is the
    /// SystemMessage to emit before the retry.
    Retry { feedback: String, status: String },
    /// Checks failed and we're at the retry cap. Caller should emit
    /// `status` as a SystemMessage and end the turn with a `Done`
    /// event preserving the partial work.
    GiveUp { status: String },
}

/// Run lint + tests and decide what the TUI turn loop should do next.
///
/// `autonomy_mode` is the current autonomy string (`"read-only"` /
/// `"plan-only"` / `"auto-edit"` / `"full-auto"`).
/// `retries_used` is the number of retries *already consumed* by this
/// user-prompt turn (so the first call passes `0`).
pub fn run_auto_fix_check(
    cwd: &Path,
    config: &AutoFixConfig,
    autonomy_mode: &str,
    retries_used: u32,
) -> AutoFixAction {
    if !should_trigger(config, autonomy_mode) {
        return AutoFixAction::Continue { status: None };
    }

    let lint_cmd = detect_lint_command(cwd, &config.lint_command);
    let test_cmd = detect_test_command(cwd, &config.test_command);

    if lint_cmd.is_none() && test_cmd.is_none() {
        return AutoFixAction::Continue { status: None };
    }

    let outcome = run_checks(
        cwd,
        lint_cmd.as_deref(),
        test_cmd.as_deref(),
        config.timeout_secs,
    );

    match outcome {
        CheckOutcome::Pass => AutoFixAction::Continue {
            status: Some("[auto-fix] checks passed".to_string()),
        },
        CheckOutcome::NoRunners => AutoFixAction::Continue { status: None },
        CheckOutcome::Skipped { reason } => AutoFixAction::Continue {
            status: Some(format!("[auto-fix] skipped: {reason}")),
        },
        CheckOutcome::Fail { lint_stderr, test_stderr } => {
            if retries_used >= config.max_retries {
                let lint_tail = lint_stderr
                    .as_deref()
                    .map(trim_section)
                    .unwrap_or_else(|| "(no output)".to_string());
                let test_tail = test_stderr
                    .as_deref()
                    .map(trim_section)
                    .unwrap_or_else(|| "(skipped: lint failed)".to_string());
                AutoFixAction::GiveUp {
                    status: format!(
                        "[auto-fix] cap reached ({0}/{0}) — giving up, \
                         working tree left as-is\n\
                         Final lint output:\n{1}\n\
                         Final test output:\n{2}",
                        config.max_retries, lint_tail, test_tail,
                    ),
                }
            } else {
                let feedback = format_feedback_message(
                    lint_cmd.as_deref(),
                    test_cmd.as_deref(),
                    lint_stderr.as_deref(),
                    test_stderr.as_deref(),
                );
                AutoFixAction::Retry {
                    feedback,
                    status: format!(
                        "[auto-fix] checks failed — retry {}/{}",
                        retries_used + 1,
                        config.max_retries,
                    ),
                }
            }
        }
    }
}
```

Run: `cargo test --lib autofix::tests::run_auto_fix_check_pass 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 3: Add the remaining run_auto_fix_check tests**

Append to `mod tests`:

```rust
    #[test]
    fn run_auto_fix_check_lint_fail_under_cap() {
        let dir = tempdir().unwrap();
        let cfg = AutoFixConfig {
            enabled: true,
            trigger: AutoFixTrigger::Always,
            lint_command: Some("false".to_string()),
            test_command: Some("true".to_string()),
            max_retries: 3,
            timeout_secs: 5,
        };
        let action = run_auto_fix_check(dir.path(), &cfg, "auto-edit", 0);
        match action {
            AutoFixAction::Retry { feedback, status } => {
                assert!(feedback.contains("Your last edits failed"));
                assert!(feedback.contains("Do not disable lints"));
                assert!(feedback.contains("(skipped: lint failed)"));
                assert!(status.contains("1/3"));
            }
            other => panic!("expected Retry, got {other:?}"),
        }
    }

    #[test]
    fn run_auto_fix_check_tests_fail_under_cap() {
        let dir = tempdir().unwrap();
        let cfg = AutoFixConfig {
            enabled: true,
            trigger: AutoFixTrigger::Always,
            lint_command: Some("true".to_string()),
            test_command: Some("false".to_string()),
            max_retries: 3,
            timeout_secs: 5,
        };
        let action = run_auto_fix_check(dir.path(), &cfg, "auto-edit", 1);
        match action {
            AutoFixAction::Retry { feedback, status } => {
                assert!(feedback.contains("## Tests"));
                assert!(status.contains("2/3"));
            }
            other => panic!("expected Retry, got {other:?}"),
        }
    }

    #[test]
    fn run_auto_fix_check_cap_reached() {
        let dir = tempdir().unwrap();
        let cfg = AutoFixConfig {
            enabled: true,
            trigger: AutoFixTrigger::Always,
            lint_command: Some("false".to_string()),
            test_command: Some("true".to_string()),
            max_retries: 3,
            timeout_secs: 5,
        };
        let action = run_auto_fix_check(dir.path(), &cfg, "auto-edit", 3);
        match action {
            AutoFixAction::GiveUp { status } => {
                assert!(status.contains("cap reached"));
                assert!(status.contains("3/3"));
                assert!(status.contains("working tree left as-is"));
            }
            other => panic!("expected GiveUp, got {other:?}"),
        }
    }

    #[test]
    fn run_auto_fix_check_trigger_off() {
        let dir = tempdir().unwrap();
        let cfg = AutoFixConfig {
            enabled: true,
            trigger: AutoFixTrigger::Off,
            lint_command: Some("false".to_string()),
            test_command: Some("false".to_string()),
            max_retries: 3,
            timeout_secs: 5,
        };
        let action = run_auto_fix_check(dir.path(), &cfg, "auto-edit", 0);
        match action {
            AutoFixAction::Continue { status } => {
                assert!(status.is_none(), "trigger off should be silent");
            }
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    #[test]
    fn run_auto_fix_check_autonomous_skips_in_suggest_mode() {
        let dir = tempdir().unwrap();
        let cfg = AutoFixConfig {
            enabled: true,
            trigger: AutoFixTrigger::Autonomous,
            lint_command: Some("false".to_string()),
            test_command: Some("false".to_string()),
            max_retries: 3,
            timeout_secs: 5,
        };
        let action = run_auto_fix_check(dir.path(), &cfg, "suggest", 0);
        assert!(matches!(
            action,
            AutoFixAction::Continue { status: None }
        ));
    }

    #[test]
    fn run_auto_fix_check_no_runners() {
        let dir = tempdir().unwrap();
        let cfg = AutoFixConfig {
            enabled: true,
            trigger: AutoFixTrigger::Always,
            lint_command: None,
            test_command: None,
            max_retries: 3,
            timeout_secs: 5,
        };
        let action = run_auto_fix_check(dir.path(), &cfg, "auto-edit", 0);
        assert!(matches!(
            action,
            AutoFixAction::Continue { status: None }
        ));
    }
```

Run: `cargo test --lib autofix::tests::run_auto_fix_check 2>&1 | tail -20`
Expected: 7 run_auto_fix_check tests pass (pass + 5 new + 1 base).

- [ ] **Step 4: Commit**

```bash
git add src/autofix.rs
git commit -m "$(cat <<'EOF'
feat(autofix): run_auto_fix_check decision helper + AutoFixAction

Pure function that runs lint + tests via run_checks and returns one
of Continue / Retry / GiveUp. The TUI turn loop calls this once per
turn after edits and matches on the result.

Keeping the decision logic out of the TUI means the behaviour is
actually testable: 7 unit tests cover pass, lint-fail-under-cap,
tests-fail-under-cap, cap-reached, trigger-off, suggest-mode skip,
and no-runners.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 8: Wire run_auto_fix_check into the TUI turn loop

**Files:**
- Modify: `src/tui/run.rs:3616` (add `auto_fix_retries: u32` outside the loop)
- Modify: `src/tui/run.rs:4053-4125` (replace the Task-3 single-shot block with the retry state machine)

The retry loop reuses the existing outer `loop` in `run_api_task`: when retry is needed, we append a User message and `continue` the loop, which sends a fresh API request with the feedback in the history.

- [ ] **Step 1: Add the `auto_fix_retries` counter**

Find line 3615 in `src/tui/run.rs`:

```rust
    let mut iterations: u32 = 0;
    loop {
```

Change to:

```rust
    let mut iterations: u32 = 0;
    // Retries consumed by the auto-fix loop within the current user turn.
    // Reset to 0 on every user prompt; the retry helper enforces the cap.
    let mut auto_fix_retries: u32 = 0;
    loop {
```

- [ ] **Step 2: Replace the single-shot block with the retry state machine**

Find the block starting at the `// ── Auto-fix check (pre-retry-loop intermediate) ─` comment (inserted in Task 3). Replace the **entire block** — from `if !auto_fix_touched.is_empty()` through the final `auto_fix_touched.clear();` — with:

```rust
                // ── Auto-fix check with retry loop ───────────────────────────
                // After all tool calls complete, run lint + tests. On failure
                // and under cap, append a synthetic user message with the
                // failure output and re-enter the agentic loop. On cap reached,
                // emit a SystemMessage and end the turn preserving partial work.
                if !auto_fix_touched.is_empty() {
                    let action = crate::autofix::run_auto_fix_check(
                        &config.cwd,
                        &config.auto_fix,
                        &config.autonomy,
                        auto_fix_retries,
                    );
                    auto_fix_touched.clear();

                    match action {
                        crate::autofix::AutoFixAction::Continue { status } => {
                            if let Some(msg) = status {
                                let _ = tx.send(AppEvent::SystemMessage(msg));
                            }
                        }
                        crate::autofix::AutoFixAction::Retry { feedback, status } => {
                            let _ = tx.send(AppEvent::SystemMessage(status));
                            // Append the synthetic user turn so the model sees
                            // the lint/test failure on the next API round.
                            messages.push(Message {
                                role: Role::User,
                                content: vec![ContentBlock::Text { text: feedback }],
                            });
                            auto_fix_retries += 1;
                            // Re-enter the outer loop to call the model again
                            // with the injected feedback in history.
                            continue;
                        }
                        crate::autofix::AutoFixAction::GiveUp { status } => {
                            let _ = tx.send(AppEvent::SystemMessage(status));
                            let _ = tx.send(AppEvent::Done {
                                tokens_in: response.usage.input_tokens,
                                tokens_out: response.usage.output_tokens,
                                cache_read: response.usage.cache_read_input_tokens,
                                cache_write: response.usage.cache_creation_input_tokens,
                                messages: messages.clone(),
                                model_used: config.model.clone(),
                            });
                            return;
                        }
                    }
                }
```

**Critical note on the `continue`:** this `continue` must re-enter the outer `loop` in `run_api_task` (the one at line 3616). It is inside the `Some(StopReason::ToolUse) =>` arm, which is inside the outer `match` inside the outer `loop`. Rust's `continue` defaults to the innermost loop — which is the outer `loop` here, because `match` arms don't create loops. Verify by scanning for any nested `loop`/`while`/`for` statements that might capture the `continue`:

```bash
awk 'NR>=3800 && NR<=4130' src/tui/run.rs | grep -n "loop\|while\|for"
```

If a nested loop is found, change the `continue` to a labelled `continue 'outer;` after adding the `'outer:` label to the loop at line 3616.

- [ ] **Step 3: Build and verify**

Run: `cargo build 2>&1 | tail -20`
Expected: `Finished \`dev\` profile` with no errors.

Run: `cargo test --lib 2>&1 | tail -20`
Expected: all tests pass.

Run: `cargo clippy --all-targets -- -D warnings 2>&1 | grep -E "tui/run\.rs:(3616|361[5-9]|40[5-9][0-9]|41[0-2][0-9])" | head -10`
Expected: no matches (no new clippy issues in the edit region).

- [ ] **Step 4: Commit**

```bash
git add src/tui/run.rs
git commit -m "$(cat <<'EOF'
feat(autofix): wire retry loop into the TUI turn state machine

After the model's tool uses complete, run_auto_fix_check runs lint +
tests and returns Continue / Retry / GiveUp. On Retry we append a
synthetic User text turn containing the failure output and re-enter
the agentic loop with `continue`; the model sees the feedback on the
next API round and fixes its own work. On GiveUp the turn ends with a
SystemMessage and a normal Done event, preserving partial progress.

This is Phase 2's flagship feature: Aider-style post-edit lint+test
feedback, no silent reverts, anti-cheat clause in the prompt.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 9: Clamp max_retries to 1..=10 + remove "reserved" warning

**Files:**
- Modify: `src/config.rs:506-534` (clamp logic)
- Modify: `src/config.rs` (new `#[cfg(test)]` tests for clamping)

- [ ] **Step 1: Write the failing test for clamping low**

Find the `#[cfg(test)] mod data_dir_tests` (or any existing tests module) in `src/config.rs`. Add a new sibling test module at the bottom of the file, above any existing `#[cfg(test)]`:

```rust
#[cfg(test)]
mod auto_fix_clamp_tests {
    use crate::autofix::{AutoFixConfig, DEFAULT_TEST_TIMEOUT_SECS};
    use crate::settings::AutoFixSettings;

    // Helper: mirror the clamp logic Config::load applies, so we can test it
    // without spinning up a full Config::load from disk.
    fn clamp_max_retries(n: u32) -> u32 {
        if (1..=10).contains(&n) { n } else { 3 }
    }

    #[test]
    fn clamps_zero_to_default() {
        assert_eq!(clamp_max_retries(0), 3);
    }

    #[test]
    fn clamps_too_high_to_default() {
        assert_eq!(clamp_max_retries(50), 3);
    }

    #[test]
    fn keeps_valid_retry_count() {
        assert_eq!(clamp_max_retries(5), 5);
    }

    #[test]
    fn keeps_lower_bound() {
        assert_eq!(clamp_max_retries(1), 1);
    }

    #[test]
    fn keeps_upper_bound() {
        assert_eq!(clamp_max_retries(10), 10);
    }

    // Smoke test: AutoFixConfig default has a valid max_retries
    #[test]
    fn default_config_has_valid_max_retries() {
        let cfg = AutoFixConfig::default();
        assert!((1..=10).contains(&cfg.max_retries));
        assert_eq!(cfg.timeout_secs, DEFAULT_TEST_TIMEOUT_SECS);
    }

    // Smoke test: AutoFixSettings round-trips through serde with camelCase
    #[test]
    fn settings_camelcase_roundtrip() {
        let s = AutoFixSettings {
            enabled: Some(true),
            trigger: Some("always".to_string()),
            lint_command: Some("cargo clippy".to_string()),
            test_command: Some("cargo test".to_string()),
            max_retries: Some(5),
            timeout_secs: Some(30),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("lintCommand"));
        assert!(json.contains("testCommand"));
        assert!(json.contains("maxRetries"));
        assert!(json.contains("timeoutSecs"));
    }
}
```

Run: `cargo test --lib auto_fix_clamp_tests 2>&1 | tail -20`
Expected: all 7 tests pass (the helper function is pure, so they should pass immediately without any source change; this is confirmation the tests are wired up correctly).

- [ ] **Step 2: Update the clamp logic in `Config::load`**

Find the block around line 519-532 in `src/config.rs`:

```rust
            if let Some(m) = ar.max_retries {
                cfg.auto_fix.max_retries = m;
                // max_retries is reserved for a future multi-turn retry loop.
                // The current hook runs tests exactly once per turn, reverts on
                // failure, and emits a system message. Warn the user so they
                // don't expect retries to take effect yet.
                if m != 3 {
                    tracing::warn!(
                        "autoRollback.maxRetries = {m} is reserved — the current \
                         hook runs tests exactly once per turn and does not retry. \
                         This field will become active in a future multi-turn loop."
                    );
                }
            }
```

Replace with:

```rust
            if let Some(m) = ar.max_retries {
                if (1..=10).contains(&m) {
                    cfg.auto_fix.max_retries = m;
                } else {
                    tracing::warn!(
                        "autoFixLoop.maxRetries = {m} is out of bounds (1..=10); \
                         clamping to default (3)."
                    );
                    cfg.auto_fix.max_retries = 3;
                }
            }
```

- [ ] **Step 3: Build and verify**

Run: `cargo build 2>&1 | tail -20`
Expected: `Finished \`dev\` profile` with no errors.

Run: `cargo test --lib 2>&1 | tail -10`
Expected: all tests pass including the 7 new clamp tests.

- [ ] **Step 4: Commit**

```bash
git add src/config.rs
git commit -m "$(cat <<'EOF'
feat(autofix): clamp max_retries to 1..=10 and drop reserved warning

max_retries is now load-bearing (used by the retry loop in run_auto_fix_check).
Clamp out-of-range values to the default of 3 and warn the user, instead of
warning on every non-default value like the old "reserved" code did.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 10: Add the "autoFixLoop" settings key with serde rename

**Files:**
- Modify: `src/settings.rs:244-247` (rename the serde key and add alias)

- [ ] **Step 1: Add the forward-compat rename**

Find in `src/settings.rs`:

```rust
    /// Auto-fix loop: run lint + tests after Write/Edit and re-prompt on failure.
    #[serde(rename = "autoRollback")]
    pub auto_fix: Option<AutoFixSettings>,
```

Replace with:

```rust
    /// Auto-fix loop: run lint + tests after Write/Edit and re-prompt on failure.
    /// The JSON key `autoFixLoop` is preferred; `autoRollback` remains as a
    /// silent alias so existing user configs keep working.
    #[serde(rename = "autoFixLoop", alias = "autoRollback")]
    pub auto_fix: Option<AutoFixSettings>,
```

- [ ] **Step 2: Write the failing round-trip test**

Add to `src/settings.rs` in an existing `#[cfg(test)]` module (or create one at the bottom):

```rust
#[cfg(test)]
mod auto_fix_key_tests {
    use super::*;

    #[test]
    fn parses_new_autofixloop_key() {
        let json = r#"{
            "autoFixLoop": {
                "enabled": true,
                "trigger": "always",
                "lintCommand": "cargo clippy",
                "testCommand": "cargo test",
                "maxRetries": 5,
                "timeoutSecs": 30
            }
        }"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        let af = s.auto_fix.expect("auto_fix should deserialise");
        assert_eq!(af.enabled, Some(true));
        assert_eq!(af.trigger.as_deref(), Some("always"));
        assert_eq!(af.lint_command.as_deref(), Some("cargo clippy"));
        assert_eq!(af.max_retries, Some(5));
    }

    #[test]
    fn parses_legacy_autorollback_key() {
        let json = r#"{
            "autoRollback": {
                "enabled": true,
                "testCommand": "pytest"
            }
        }"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        let af = s.auto_fix.expect("legacy autoRollback should deserialise into auto_fix");
        assert_eq!(af.test_command.as_deref(), Some("pytest"));
    }

    #[test]
    fn serialises_with_new_key() {
        let mut s = Settings::default();
        s.auto_fix = Some(AutoFixSettings {
            enabled: Some(true),
            trigger: None,
            lint_command: None,
            test_command: Some("cargo test".to_string()),
            max_retries: Some(3),
            timeout_secs: None,
        });
        let out = serde_json::to_string(&s).unwrap();
        assert!(out.contains("autoFixLoop"), "serialised form should use the new key: {out}");
        assert!(!out.contains("autoRollback"), "serialised form should not use the legacy key: {out}");
    }
}
```

Run: `cargo test --lib auto_fix_key_tests 2>&1 | tail -20`
Expected: 3 passed.

- [ ] **Step 3: Commit**

```bash
git add src/settings.rs
git commit -m "$(cat <<'EOF'
feat(autofix): settings key "autoFixLoop" with "autoRollback" alias

New user-facing JSON key is "autoFixLoop". Existing configs using
"autoRollback" continue to work via serde alias. Serialising always
uses the new key.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 11: Integration test against a real tmpdir fixture

**Files:**
- Create: `tests/autofix_loop_integration.rs`

- [ ] **Step 1: Write the integration test file**

Create `tests/autofix_loop_integration.rs`:

```rust
//! Integration tests for `run_auto_fix_check` against real tmpdir fixtures.
//!
//! The TUI wiring itself is exercised by manual QA; this test pins the
//! public helper's behaviour against synthetic project layouts that
//! exercise each decision branch.

#![cfg(unix)]

use rustyclaw::autofix::{
    run_auto_fix_check, AutoFixAction, AutoFixConfig, AutoFixTrigger,
};
use std::fs;
use tempfile::tempdir;

fn base_cfg() -> AutoFixConfig {
    AutoFixConfig {
        enabled: true,
        trigger: AutoFixTrigger::Always,
        lint_command: None,
        test_command: None,
        max_retries: 3,
        timeout_secs: 10,
    }
}

#[test]
fn empty_dir_no_runners_continues_silently() {
    let dir = tempdir().unwrap();
    let cfg = base_cfg();
    let action = run_auto_fix_check(dir.path(), &cfg, "auto-edit", 0);
    assert!(matches!(
        action,
        AutoFixAction::Continue { status: None }
    ));
}

#[test]
fn cargo_project_detects_and_passes_with_overrides() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\nversion=\"0.1.0\"\n").unwrap();

    let mut cfg = base_cfg();
    cfg.lint_command = Some("true".to_string());
    cfg.test_command = Some("true".to_string());

    let action = run_auto_fix_check(dir.path(), &cfg, "auto-edit", 0);
    match action {
        AutoFixAction::Continue { status } => {
            assert!(status.is_some());
            assert!(status.unwrap().contains("checks passed"));
        }
        other => panic!("expected Continue, got {other:?}"),
    }
}

#[test]
fn lint_fail_under_cap_returns_retry_with_anticheat() {
    let dir = tempdir().unwrap();
    let mut cfg = base_cfg();
    cfg.lint_command = Some("sh -c 'echo \"warning: unused variable\" >&2; exit 1'".to_string());
    cfg.test_command = Some("true".to_string());

    let action = run_auto_fix_check(dir.path(), &cfg, "auto-edit", 0);
    match action {
        AutoFixAction::Retry { feedback, status } => {
            assert!(feedback.contains("Your last edits failed"));
            assert!(feedback.contains("Do not disable lints"));
            assert!(feedback.contains("#[allow"));
            assert!(feedback.contains("(skipped: lint failed)"));
            assert!(status.contains("1/3"));
        }
        other => panic!("expected Retry, got {other:?}"),
    }
}

#[test]
fn cap_reached_returns_giveup_with_working_tree_note() {
    let dir = tempdir().unwrap();
    let mut cfg = base_cfg();
    cfg.lint_command = Some("false".to_string());
    cfg.test_command = Some("true".to_string());

    let action = run_auto_fix_check(dir.path(), &cfg, "auto-edit", 3);
    match action {
        AutoFixAction::GiveUp { status } => {
            assert!(status.contains("cap reached"));
            assert!(status.contains("working tree left as-is"));
        }
        other => panic!("expected GiveUp, got {other:?}"),
    }
}

#[test]
fn trigger_autonomous_skips_in_read_only_mode() {
    let dir = tempdir().unwrap();
    let mut cfg = base_cfg();
    cfg.trigger = AutoFixTrigger::Autonomous;
    cfg.lint_command = Some("false".to_string());

    let action = run_auto_fix_check(dir.path(), &cfg, "read-only", 0);
    assert!(matches!(
        action,
        AutoFixAction::Continue { status: None }
    ));
}

#[test]
fn trigger_off_short_circuits() {
    let dir = tempdir().unwrap();
    let mut cfg = base_cfg();
    cfg.trigger = AutoFixTrigger::Off;
    cfg.lint_command = Some("false".to_string());
    cfg.test_command = Some("false".to_string());

    let action = run_auto_fix_check(dir.path(), &cfg, "full-auto", 0);
    assert!(matches!(
        action,
        AutoFixAction::Continue { status: None }
    ));
}
```

- [ ] **Step 2: Verify `rustyclaw` crate re-exports `autofix` publicly**

Check `src/lib.rs` for `pub mod autofix;` (added in Task 1). If the module is not `pub`, integration tests can't see it.

```bash
grep "pub mod autofix" src/lib.rs
```

Expected: `pub mod autofix;` on line 20.

- [ ] **Step 3: Run the integration tests**

Run: `cargo test --test autofix_loop_integration 2>&1 | tail -20`
Expected: `6 passed; 0 failed`.

- [ ] **Step 4: Run the entire test suite to confirm no regression**

Run: `cargo test 2>&1 | tail -20`
Expected: all tests pass (181 from before + unit tests from Tasks 5–7 + integration tests from this task).

- [ ] **Step 5: Commit**

```bash
git add tests/autofix_loop_integration.rs
git commit -m "$(cat <<'EOF'
test(autofix): integration tests for run_auto_fix_check

Six end-to-end tests hitting the public helper against real tmpdir
fixtures, covering the happy path, lint-fail retry with anti-cheat
verification, cap-reached giveup, autonomous-mode skip, and
trigger-off short-circuit.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 12: README + CHANGELOG + CLAUDE.md refresh

**Files:**
- Modify: `README.md` (add the feature to the competitive advantages section)
- Modify: `CHANGELOG.md` (add an entry under the next unreleased version)
- Modify: `CLAUDE.md` (strike "auto lint/test loop" from "NEXT UP", add to "PHASE 2 SHIPPED")

- [ ] **Step 1: README — add to the advantages section**

Find the "Our Advantages Over Claurst" or equivalent section in `README.md`. Add a new bullet:

```markdown
- **Auto-fix loop** — after every Write/Edit, lint + tests run automatically; failures feed back to the model for up to 3 retries. Aider's killer feature, now in Rust. Claurst: nothing.
```

- [ ] **Step 2: CHANGELOG entry**

Find `CHANGELOG.md`. If an "Unreleased" section exists, append under it. Otherwise create one at the top:

```markdown
## Unreleased

### Added
- **Auto-fix loop** (Phase 2): After the model edits code, RustyClaw runs
  project-appropriate lint + test commands and, on failure, injects the
  output back as a synthetic user turn for up to `maxRetries` rounds
  (default 3, cap 10). Supports Rust (`cargo clippy` + `cargo test`),
  Node (`npx eslint` + `npm test`), Python (`ruff check` + `pytest`),
  and Go (`go vet` + `go test`). Anti-cheat clause in the feedback
  prompt blocks `#[allow(dead_code)]`-style escapes.
  Configure via `autoFixLoop` in `settings.json`; `autoRollback` still
  works as an alias.

### Changed
- The `auto_rollback` module has been renamed to `autofix` and no longer
  reverts files on failure. On retry-cap, the working tree is left
  as-is; use `/undo` or `git checkout` to revert manually.
```

- [ ] **Step 3: CLAUDE.md — move auto lint/test loop from NEXT UP to SHIPPED**

Find `CLAUDE.md`. Under "NEXT UP":

```markdown
7. **Phase 2 robustness** — Auto git commits + /undo, auto lint/test loop, diff review, loop detection, self-update, shell completions.
```

Change `auto lint/test loop, ` to nothing (delete it from this line).

Above "NEXT UP" (under a section heading like "PHASE 2 SHIPPED" if one exists, or add it):

```markdown
### PHASE 2 (shipping now)
- **Auto lint/test loop (2026-04-10)** — Post-edit lint + tests + feedback-driven retries replace the old rollback revert. Aider-style, anti-cheat protected.
```

- [ ] **Step 4: Build and smoke-test the binary**

Run: `cargo build --release 2>&1 | tail -10`
Expected: `Finished \`release\` profile`.

Run: `./target/release/rustyclaw --version 2>&1 | head -5`
Expected: version string prints.

- [ ] **Step 5: Commit**

```bash
git add README.md CHANGELOG.md CLAUDE.md
git commit -m "$(cat <<'EOF'
docs(autofix): README + CHANGELOG + CLAUDE.md updates for Phase 2 auto-fix loop

Promote the auto-fix loop from "NEXT UP" to shipped. Covers the new
settings key, the backward-compat alias, and the behaviour change
(retry instead of revert).

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Final Verification Checklist

After all 12 tasks land, run this sanity sweep:

- [ ] `cargo build --release` — clean release build
- [ ] `cargo test` — full suite green (expect ~200+ tests: old 181 + lint detection 7 + run_checks 5 + format_feedback 5 + utf8 1 + run_auto_fix_check 7 + clamp 7 + settings key 3 + integration 6)
- [ ] `cargo clippy --all-targets 2>&1 | grep -E "(autofix\.rs|tui/run\.rs:(3615|361[6-9]|40[5-9][0-9]|41[0-2][0-9]))"` — no clippy regressions in the new code
- [ ] `grep -rn "rollback" src/ tests/ docs/superpowers/` — should return hits only in the spec/plan docs and commit messages, not in live source (other than possibly historical references in comments)
- [ ] `grep -rn "git_restore_files\|git_stash_create" src/` — no hits; the revert primitives are fully deleted
- [ ] Manual QA: create a scratch Rust project with a deliberate clippy warning, run `rustyclaw` in auto-edit mode, ask it to add a function, observe the auto-fix loop kick in
