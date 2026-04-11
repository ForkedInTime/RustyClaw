# Auto git commits + `/undo` + `/redo` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Per-turn auto-commit snapshots on private shadow refs (`refs/rustyclaw/sessions/<id>`) with `/undo` / `/redo` / `/autocommit status` slash commands.

**Architecture:** A new `src/autocommit.rs` module drives git plumbing (`write-tree`, `commit-tree`, `update-ref`, `read-tree`, `checkout-index`) via `std::process::Command` with `GIT_INDEX_FILE` isolation so the user's real index is untouched. `SessionMeta` gains `auto_commits: Vec<String>` + `undo_position: usize` for lineage tracking. `tui/run.rs` calls `snapshot_turn` at the end of each assistant turn and opens overlay pickers for `/undo`/`/redo`.

**Tech Stack:** Rust, `std::process::Command`, `tempfile::TempDir` (dev-dep, already present), `serde` / `serde_json` for `SessionMeta`, `tracing` for warnings, existing `ratatui` overlay system.

**Spec:** `docs/superpowers/specs/2026-04-10-auto-commit-undo-design.md`

**Branch:** `feature/auto-commit-undo` (already created, off `main`)

**Ground rules for every task:**
- **Branch hygiene:** Never `git checkout` another branch. Work only on `feature/auto-commit-undo`. Never merge, push, or force-push.
- **No dead code suppressions:** Do NOT add `#[allow(dead_code)]`, `#[allow(clippy::*)]`, or `// nolint` to make warnings go away. Fix the root cause.
- **Commit trailer:** Use `Co-Authored-By: Arch Linux <noreply@archlinux.org>`. NEVER mention Claude or Claude Code in commit messages.
- **Clippy baseline:** The repo sits at 217 warnings pre-plan. Every task must keep it at ≤ 217. Run `cargo clippy --all-targets -- -W clippy::all 2>&1 | rg '^warning' | wc -l` after your changes.
- **Test baseline:** 284 tests pass pre-plan. Every task must keep all tests green.
- **TDD:** Write the failing test first, confirm it fails for the right reason, then implement.

---

## File Structure

### New files

- **`src/autocommit.rs`** (~700 LOC) — all git plumbing, types, snapshot/restore/prune logic, unit tests. Mirrors `src/autofix.rs` structure.
- **`tests/autocommit_integration.rs`** (~200 LOC) — end-to-end tests against real `git init` tempdirs.

### Modified files

- **`src/lib.rs`** — `pub mod autocommit;`
- **`src/session/mod.rs`** — add 2 fields to `SessionMeta`
- **`src/settings.rs`** — add `AutoCommitSettings` struct + field on `Settings`
- **`src/config.rs`** — add `auto_commit: AutoCommitConfig` field + parsing + clamp logic
- **`src/commands/mod.rs`** — register `/undo`, `/redo`, `/autocommit` + `CommandAction` variants + help entry
- **`src/tui/run.rs`** — call `prune_old_refs` on startup, call `snapshot_turn` at end of each assistant turn, handle the three new `CommandAction` variants with overlay pickers
- **`README.md`** — new Highlights bullet
- **`CHANGELOG.md`** — Unreleased Added entry
- **`CLAUDE.md`** — move Phase 2 #1 from NEXT UP to SHIPPED

### Key interface (defined in Task 2 / 4 / 5 / 6 / 7, referenced throughout the plan)

```rust
// src/autocommit.rs — stable public API used by tui/run.rs and integration tests

pub const DEFAULT_KEEP_SESSIONS: u32 = 10;
pub const DEFAULT_MESSAGE_PREFIX: &str = "rustyclaw";

#[derive(Debug, Clone)]
pub struct AutoCommitConfig {
    pub enabled: bool,
    pub keep_sessions: u32,
    pub message_prefix: String,
}

impl Default for AutoCommitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            keep_sessions: DEFAULT_KEEP_SESSIONS,
            message_prefix: DEFAULT_MESSAGE_PREFIX.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotOutcome {
    Committed { sha: String, files: u32 },
    NoChanges,
    Disabled { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    /// Number of files extracted from the target tree to the working directory.
    pub files_restored: u32,
    /// Files that exist in the working tree but not in the target tree.
    /// These are left alone (not deleted); the caller may surface them to the user.
    pub orphaned_files: Vec<std::path::PathBuf>,
}

/// Detect whether `cwd` (or any ancestor) is a git work tree.
pub fn is_git_repo(cwd: &std::path::Path) -> bool;

/// Take a full-tree snapshot of `cwd` as a commit on
/// `refs/rustyclaw/sessions/<session_id>`.
///
/// Appends the new commit SHA to `auto_commits` and bumps `undo_position`
/// to `auto_commits.len()`. If `undo_position < auto_commits.len()` on
/// entry, the redo tail is truncated first (matches editor muscle memory).
///
/// No-ops (returning `NoChanges`) when the working tree matches the parent
/// commit's tree. No-ops (returning `Disabled`) when `config.enabled == false`
/// or `cwd` is not inside a git repo.
pub fn snapshot_turn(
    cwd: &std::path::Path,
    config: &AutoCommitConfig,
    session_id: &str,
    user_prompt: &str,
    turn_index: u32,
    auto_commits: &mut Vec<String>,
    undo_position: &mut usize,
) -> anyhow::Result<SnapshotOutcome>;

/// Restore the working tree at `cwd` to the tree of `auto_commits[target_position - 1]`
/// (or the session-base tree when `target_position == 0`).
///
/// Does NOT mutate `auto_commits` — the caller updates `undo_position` on success.
pub fn restore_to(
    cwd: &std::path::Path,
    auto_commits: &[String],
    target_position: usize,
) -> anyhow::Result<RestoreReport>;

/// Delete old `refs/rustyclaw/sessions/*` refs, keeping the `keep` newest.
/// `keep == 0` means "unlimited, prune nothing".
/// Returns the number of refs deleted.
pub fn prune_old_refs(cwd: &std::path::Path, keep: u32) -> anyhow::Result<u32>;
```

The signatures take `session_id: &str` and `auto_commits: &mut Vec<String>` rather than `&mut SessionMeta` so the module lives in `lib.rs` (which does not compile the `session` module — see [src/lib.rs](src/lib.rs)). The caller in `tui/run.rs` handles `SessionMeta` persistence.

---

## Task 1: Scaffold `src/autocommit.rs` module + `lib.rs` export

**Why first:** Establishes the module, constants, types, and public API surface so later tasks can reference them.

**Files:**
- Create: `src/autocommit.rs`
- Modify: `src/lib.rs` (add `pub mod autocommit;`)

- [ ] **Step 1: Create `src/autocommit.rs` with the type skeleton**

Write this exact content to `src/autocommit.rs`:

```rust
//! Auto-commit loop — per-turn working-tree snapshots on private shadow refs.
//!
//! Uses git plumbing (`write-tree`, `commit-tree`, `update-ref`, `read-tree`,
//! `checkout-index`) under `refs/rustyclaw/sessions/<session-id>`, driven via
//! `std::process::Command` with `GIT_INDEX_FILE` pointed at a temp index so the
//! user's real `.git/index` is never touched.
//!
//! Commits are invisible to normal git tooling (`log`, `status`, `branch`) —
//! only `git for-each-ref refs/rustyclaw/` sees them. They are strictly local
//! (never pushed) and serve as a per-turn undo stack the user can navigate
//! with `/undo` and `/redo`.

use std::path::{Path, PathBuf};

pub const DEFAULT_KEEP_SESSIONS: u32 = 10;
pub const DEFAULT_MESSAGE_PREFIX: &str = "rustyclaw";
pub const SHADOW_REF_PREFIX: &str = "refs/rustyclaw/sessions/";

/// Runtime config for the auto-commit loop. Built from `AutoCommitSettings`
/// in `Config::load` with out-of-range `keep_sessions` clamped to the default.
#[derive(Debug, Clone)]
pub struct AutoCommitConfig {
    pub enabled: bool,
    pub keep_sessions: u32,
    pub message_prefix: String,
}

impl Default for AutoCommitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            keep_sessions: DEFAULT_KEEP_SESSIONS,
            message_prefix: DEFAULT_MESSAGE_PREFIX.to_string(),
        }
    }
}

/// Outcome of a single `snapshot_turn` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotOutcome {
    /// A new commit was written and `auto_commits` updated.
    Committed { sha: String, files: u32 },
    /// Working tree matches parent tree — nothing to commit.
    NoChanges,
    /// Auto-commit is disabled (config, non-git dir, etc). Human-readable reason.
    Disabled { reason: String },
}

/// Report returned by `restore_to`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    /// Files extracted from the target tree back into the working directory.
    pub files_restored: u32,
    /// Files in the working tree that do NOT exist in the target tree. They
    /// are left untouched; the caller may mention them in a system message.
    pub orphaned_files: Vec<PathBuf>,
}
```

- [ ] **Step 2: Add the module to `src/lib.rs`**

Use Edit to insert `pub mod autocommit;` alphabetically — right before the existing `pub mod autofix;` line:

```
Find:    pub mod autofix;
Replace: pub mod autocommit;
         pub mod autofix;
```

- [ ] **Step 3: Build**

Run: `cargo build 2>&1 | tail -20`
Expected: clean build. Warnings about unused code are acceptable at this stage (types are used by later tasks within the same plan).

- [ ] **Step 4: Clippy**

Run: `cargo clippy --all-targets -- -W clippy::all 2>&1 | rg '^warning' | wc -l`
Expected: ≤ 217.

- [ ] **Step 5: Commit**

```bash
git add src/autocommit.rs src/lib.rs
git commit -m "$(cat <<'EOF'
feat(autocommit): scaffold module + public types

Adds src/autocommit.rs with AutoCommitConfig, SnapshotOutcome,
RestoreReport and the SHADOW_REF_PREFIX constant. Wired into lib.rs.
No behavior yet — implementation lands in the following tasks.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 2: `is_git_repo` detection + git command helper

**Files:**
- Modify: `src/autocommit.rs` (add functions + unit tests)

- [ ] **Step 1: Write the failing tests**

Append to `src/autocommit.rs`:

```rust
#[cfg(test)]
mod git_detection_tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    /// Helper: create a tempdir and `git init` inside it. Returns the tempdir handle.
    pub(super) fn init_test_repo() -> TempDir {
        let td = TempDir::new().unwrap();
        let status = Command::new("git")
            .args(["init", "--quiet", "--initial-branch=main"])
            .current_dir(td.path())
            .status()
            .expect("git init");
        assert!(status.success(), "git init failed");
        // Set required git config so later commits work.
        for (k, v) in [
            ("user.name", "rustyclaw-test"),
            ("user.email", "noreply@rustyclaw.local"),
            ("commit.gpgsign", "false"),
        ] {
            let s = Command::new("git")
                .args(["config", k, v])
                .current_dir(td.path())
                .status()
                .expect("git config");
            assert!(s.success());
        }
        td
    }

    #[test]
    fn detects_fresh_git_repo() {
        let td = init_test_repo();
        assert!(is_git_repo(td.path()));
    }

    #[test]
    fn detects_subdirectory_of_git_repo() {
        let td = init_test_repo();
        let sub = td.path().join("pkg").join("nested");
        std::fs::create_dir_all(&sub).unwrap();
        assert!(is_git_repo(&sub));
    }

    #[test]
    fn rejects_non_git_dir() {
        let td = TempDir::new().unwrap();
        assert!(!is_git_repo(td.path()));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib autocommit::git_detection_tests 2>&1 | tail -20`
Expected: FAIL — `is_git_repo` is not yet defined.

- [ ] **Step 3: Implement `is_git_repo` + `git_cmd` helper**

Append to `src/autocommit.rs` (above the `#[cfg(test)]` module):

```rust
// ── Git subprocess helpers ────────────────────────────────────────────────────

/// Build a `std::process::Command` for `git` rooted at `cwd` with a clean,
/// locale-agnostic environment and sourcing no user git config that might
/// break plumbing output parsing.
pub(crate) fn git_cmd(cwd: &Path) -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(cwd);
    // Deterministic output: no localised messages, no terminal prompting,
    // no optional-locks (git status would take a repo lock otherwise).
    cmd.env("LC_ALL", "C");
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GIT_OPTIONAL_LOCKS", "0");
    cmd
}

/// Return true if `cwd` is inside a git work tree. Uses
/// `git rev-parse --is-inside-work-tree`, which walks parent dirs.
pub fn is_git_repo(cwd: &Path) -> bool {
    git_cmd(cwd)
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .map(|o| o.status.success() && o.stdout.trim_ascii() == b"true")
        .unwrap_or(false)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib autocommit::git_detection_tests 2>&1 | tail -20`
Expected: 3 passed, 0 failed.

- [ ] **Step 5: Clippy check**

Run: `cargo clippy --all-targets -- -W clippy::all 2>&1 | rg '^warning' | wc -l`
Expected: ≤ 217.

- [ ] **Step 6: Commit**

```bash
git add src/autocommit.rs
git commit -m "$(cat <<'EOF'
feat(autocommit): add is_git_repo detection + git_cmd helper

git_cmd builds a subprocess with LC_ALL=C, no terminal prompting, and
no optional locks so plumbing output is deterministic. is_git_repo
uses rev-parse --is-inside-work-tree so it works from subdirectories.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 3: Settings + config parsing + clamp logic

**Files:**
- Modify: `src/settings.rs` (add `AutoCommitSettings` struct + field on `Settings` + merge + tests)
- Modify: `src/config.rs` (add `auto_commit` field on `Config`, parse in `load`, clamp, tests)

- [ ] **Step 1: Write failing settings tests**

Open `src/settings.rs` and append inside the existing `#[cfg(test)] mod auto_fix_key_tests` block's parent file — i.e., add a new `#[cfg(test)] mod auto_commit_key_tests` at the very end of the file:

```rust
#[cfg(test)]
mod auto_commit_key_tests {
    use super::*;

    #[test]
    fn parses_full_autocommit_block() {
        let json = r#"{
            "autoCommit": {
                "enabled": false,
                "keepSessions": 25,
                "messagePrefix": "claw"
            }
        }"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        let ac = s.auto_commit.expect("auto_commit should deserialise");
        assert_eq!(ac.enabled, Some(false));
        assert_eq!(ac.keep_sessions, Some(25));
        assert_eq!(ac.message_prefix.as_deref(), Some("claw"));
    }

    #[test]
    fn defaults_when_autocommit_missing() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert!(s.auto_commit.is_none());
    }

    #[test]
    fn serialises_with_camelcase_key() {
        let s = Settings {
            auto_commit: Some(AutoCommitSettings {
                enabled: Some(true),
                keep_sessions: Some(10),
                message_prefix: Some("rustyclaw".to_string()),
            }),
            ..Default::default()
        };
        let out = serde_json::to_string(&s).unwrap();
        assert!(out.contains("autoCommit"), "expected autoCommit key: {out}");
        assert!(out.contains("keepSessions"), "expected keepSessions key: {out}");
    }
}
```

- [ ] **Step 2: Run settings tests to verify they fail**

Run: `cargo test --lib settings::auto_commit_key_tests 2>&1 | tail -20`
Expected: FAIL — `auto_commit` field and `AutoCommitSettings` not yet defined.

- [ ] **Step 3: Add `AutoCommitSettings` struct**

Open `src/settings.rs`. Locate the existing `AutoFixSettings` struct (around line 261). Immediately AFTER its closing `}`, add:

```rust
/// Settings for the auto-commit loop (per-turn shadow-ref snapshots + /undo).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AutoCommitSettings {
    /// Enable the auto-commit loop (default: true).
    pub enabled: Option<bool>,
    /// How many session refs to keep on startup prune (default: 10, 0 = unlimited).
    pub keep_sessions: Option<u32>,
    /// Commit subject prefix (default: "rustyclaw").
    pub message_prefix: Option<String>,
}
```

- [ ] **Step 4: Add the field to `Settings`**

Locate the existing `auto_fix` field on `Settings` (around line 248). Immediately AFTER it, add:

```rust
    /// Auto-commit loop: per-turn working-tree snapshots on private shadow refs
    /// (`refs/rustyclaw/sessions/<id>`) navigable via `/undo` and `/redo`.
    #[serde(rename = "autoCommit")]
    pub auto_commit: Option<AutoCommitSettings>,
```

- [ ] **Step 5: Wire `auto_commit` into the `merge` function**

In `src/settings.rs`, locate `Self::merge` (around line 339). Find the line:

```rust
            auto_fix:             other.auto_fix.or(self.auto_fix),
```

Insert immediately AFTER it:

```rust
            auto_commit:          other.auto_commit.or(self.auto_commit),
```

- [ ] **Step 6: Run settings tests**

Run: `cargo test --lib settings::auto_commit_key_tests 2>&1 | tail -20`
Expected: 3 passed.

- [ ] **Step 7: Write failing config clamp tests**

Open `src/config.rs`. At the very end of the file, add a new test module:

```rust
#[cfg(test)]
mod auto_commit_clamp_tests {
    use crate::autocommit::{AutoCommitConfig, DEFAULT_KEEP_SESSIONS, DEFAULT_MESSAGE_PREFIX};
    use crate::settings::AutoCommitSettings;

    /// Mirrors the clamp logic in `Config::load` so we can unit-test it without
    /// touching disk. If this logic changes, `Config::load` must change in lockstep.
    fn apply(settings: &AutoCommitSettings) -> AutoCommitConfig {
        let mut cfg = AutoCommitConfig::default();
        if let Some(e) = settings.enabled { cfg.enabled = e; }
        if let Some(k) = settings.keep_sessions {
            if k <= 1000 {
                cfg.keep_sessions = k;
            } else {
                cfg.keep_sessions = DEFAULT_KEEP_SESSIONS;
            }
        }
        if let Some(p) = &settings.message_prefix {
            cfg.message_prefix = p.clone();
        }
        cfg
    }

    #[test]
    fn default_config_matches_spec() {
        let cfg = AutoCommitConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.keep_sessions, DEFAULT_KEEP_SESSIONS);
        assert_eq!(cfg.message_prefix, DEFAULT_MESSAGE_PREFIX);
    }

    #[test]
    fn keep_sessions_accepts_in_range() {
        let s = AutoCommitSettings {
            enabled: None,
            keep_sessions: Some(25),
            message_prefix: None,
        };
        assert_eq!(apply(&s).keep_sessions, 25);
    }

    #[test]
    fn keep_sessions_accepts_zero_unlimited() {
        let s = AutoCommitSettings {
            enabled: None,
            keep_sessions: Some(0),
            message_prefix: None,
        };
        assert_eq!(apply(&s).keep_sessions, 0);
    }

    #[test]
    fn keep_sessions_out_of_range_clamps_to_default() {
        let s = AutoCommitSettings {
            enabled: None,
            keep_sessions: Some(9999),
            message_prefix: None,
        };
        assert_eq!(apply(&s).keep_sessions, DEFAULT_KEEP_SESSIONS);
    }

    #[test]
    fn disabled_propagates() {
        let s = AutoCommitSettings {
            enabled: Some(false),
            keep_sessions: None,
            message_prefix: None,
        };
        assert!(!apply(&s).enabled);
    }

    #[test]
    fn message_prefix_override() {
        let s = AutoCommitSettings {
            enabled: None,
            keep_sessions: None,
            message_prefix: Some("claw".to_string()),
        };
        assert_eq!(apply(&s).message_prefix, "claw");
    }
}
```

- [ ] **Step 8: Run config tests — they should fail to compile**

Run: `cargo test --lib auto_commit_clamp_tests 2>&1 | tail -20`
Expected: FAIL — `AutoCommitConfig` is defined but not yet imported by `Config`, and the `Config::load` clamp logic doesn't exist yet.

- [ ] **Step 9: Add `auto_commit` field to `Config`**

Open `src/config.rs`. Locate the `auto_fix` field on `Config` (around line 346):

```rust
    #[serde(skip)]
    pub auto_fix: crate::autofix::AutoFixConfig,
```

Immediately AFTER it, add:

```rust
    /// Auto-commit loop: per-turn working-tree snapshots on private shadow
    /// refs navigable via `/undo` and `/redo`.
    #[serde(skip)]
    pub auto_commit: crate::autocommit::AutoCommitConfig,
```

- [ ] **Step 10: Add `auto_commit` to `Config::default`**

Find the `auto_fix: crate::autofix::AutoFixConfig::default(),` line inside `impl Default for Config` (around line 421). Immediately AFTER it, add:

```rust
            auto_commit: crate::autocommit::AutoCommitConfig::default(),
```

- [ ] **Step 11: Add parsing + clamp logic to `Config::load`**

Locate the end of the auto-fix settings block in `Config::load` (around line 534, the line `if let Some(t) = ar.timeout_secs { cfg.auto_fix.timeout_secs = t; }` followed by a closing `}`). Immediately AFTER that closing `}`, add:

```rust
        // Auto-commit settings → AutoCommitConfig
        if let Some(ac) = &settings.auto_commit {
            if let Some(e) = ac.enabled {
                cfg.auto_commit.enabled = e;
            }
            if let Some(k) = ac.keep_sessions {
                if k <= 1000 {
                    cfg.auto_commit.keep_sessions = k;
                } else {
                    tracing::warn!(
                        "autoCommit.keepSessions = {k} is out of bounds (0..=1000); \
                         clamping to default ({})",
                        crate::autocommit::DEFAULT_KEEP_SESSIONS
                    );
                    cfg.auto_commit.keep_sessions = crate::autocommit::DEFAULT_KEEP_SESSIONS;
                }
            }
            if let Some(p) = &ac.message_prefix {
                cfg.auto_commit.message_prefix = p.clone();
            }
        }
```

- [ ] **Step 12: Run all config/settings tests**

Run: `cargo test --lib auto_commit 2>&1 | tail -30`
Expected: 9 passed total (3 settings + 6 config clamp).

- [ ] **Step 13: Clippy**

Run: `cargo clippy --all-targets -- -W clippy::all 2>&1 | rg '^warning' | wc -l`
Expected: ≤ 217.

- [ ] **Step 14: Commit**

```bash
git add src/settings.rs src/config.rs
git commit -m "$(cat <<'EOF'
feat(autocommit): settings + Config::load clamp logic

Adds AutoCommitSettings struct and Config::auto_commit field.
keepSessions out-of-range (>1000) clamps to the default (10) with a
tracing warning, matching the pattern established for autoFixLoop
max_retries. 6 clamp tests + 3 serde round-trip tests.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 4: `snapshot_turn` — git plumbing for per-turn commits

**Files:**
- Modify: `src/autocommit.rs` (add `snapshot_turn` + helpers + unit tests)

- [ ] **Step 1: Write failing tests for `snapshot_turn`**

Append to `src/autocommit.rs`, AFTER the existing `git_detection_tests` module:

```rust
#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use super::git_detection_tests::init_test_repo;
    use std::fs;

    fn write_file(repo: &Path, rel: &str, body: &str) {
        let p = repo.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, body).unwrap();
    }

    fn initial_commit(repo: &Path) {
        write_file(repo, "README.md", "base\n");
        let s = git_cmd(repo).args(["add", "README.md"]).status().unwrap();
        assert!(s.success());
        let s = git_cmd(repo)
            .args(["commit", "-q", "-m", "base"])
            .status()
            .unwrap();
        assert!(s.success());
    }

    #[test]
    fn snapshot_creates_commit_with_head_parent() {
        let td = init_test_repo();
        initial_commit(td.path());
        write_file(td.path(), "src/lib.rs", "fn main() {}\n");

        let cfg = AutoCommitConfig::default();
        let mut commits = Vec::new();
        let mut pos = 0usize;
        let outcome = snapshot_turn(
            td.path(),
            &cfg,
            "test-session-1",
            "add src/lib.rs",
            1,
            &mut commits,
            &mut pos,
        )
        .unwrap();

        match outcome {
            SnapshotOutcome::Committed { sha, files } => {
                assert_eq!(sha.len(), 40, "sha should be full 40-char hex");
                assert_eq!(files, 2, "README.md + src/lib.rs should both be in the tree");
                assert_eq!(commits, vec![sha]);
                assert_eq!(pos, 1);
            }
            other => panic!("expected Committed, got {other:?}"),
        }

        // Shadow ref points at the new commit.
        let out = git_cmd(td.path())
            .args(["rev-parse", "refs/rustyclaw/sessions/test-session-1"])
            .output()
            .unwrap();
        assert!(out.status.success());
        let ref_sha = String::from_utf8(out.stdout).unwrap().trim().to_string();
        assert_eq!(ref_sha, commits[0]);
    }

    #[test]
    fn snapshot_chains_parents_across_turns() {
        let td = init_test_repo();
        initial_commit(td.path());

        let cfg = AutoCommitConfig::default();
        let mut commits = Vec::new();
        let mut pos = 0usize;

        write_file(td.path(), "a.txt", "1\n");
        snapshot_turn(td.path(), &cfg, "s1", "add a", 1, &mut commits, &mut pos).unwrap();

        write_file(td.path(), "b.txt", "2\n");
        snapshot_turn(td.path(), &cfg, "s1", "add b", 2, &mut commits, &mut pos).unwrap();

        write_file(td.path(), "c.txt", "3\n");
        snapshot_turn(td.path(), &cfg, "s1", "add c", 3, &mut commits, &mut pos).unwrap();

        assert_eq!(commits.len(), 3);
        assert_eq!(pos, 3);
        // Each commit's parent is the previous.
        for (i, sha) in commits.iter().enumerate().skip(1) {
            let out = git_cmd(td.path())
                .args(["rev-parse", &format!("{sha}^")])
                .output()
                .unwrap();
            let parent = String::from_utf8(out.stdout).unwrap().trim().to_string();
            assert_eq!(parent, commits[i - 1], "chain broken at index {i}");
        }
    }

    #[test]
    fn snapshot_no_changes_is_noop() {
        let td = init_test_repo();
        initial_commit(td.path());
        let cfg = AutoCommitConfig::default();
        let mut commits = Vec::new();
        let mut pos = 0usize;

        let outcome = snapshot_turn(
            td.path(),
            &cfg,
            "s-empty",
            "pure read turn",
            1,
            &mut commits,
            &mut pos,
        )
        .unwrap();
        assert_eq!(outcome, SnapshotOutcome::NoChanges);
        assert!(commits.is_empty());
        assert_eq!(pos, 0);
    }

    #[test]
    fn snapshot_disabled_when_config_disabled() {
        let td = init_test_repo();
        initial_commit(td.path());
        let cfg = AutoCommitConfig {
            enabled: false,
            ..AutoCommitConfig::default()
        };
        let mut commits = Vec::new();
        let mut pos = 0usize;
        let outcome = snapshot_turn(
            td.path(),
            &cfg,
            "s",
            "msg",
            1,
            &mut commits,
            &mut pos,
        )
        .unwrap();
        match outcome {
            SnapshotOutcome::Disabled { reason } => {
                assert!(reason.contains("disabled"), "reason: {reason}");
            }
            other => panic!("expected Disabled, got {other:?}"),
        }
        assert!(commits.is_empty());
    }

    #[test]
    fn snapshot_disabled_when_not_git_repo() {
        let td = tempfile::TempDir::new().unwrap();
        let cfg = AutoCommitConfig::default();
        let mut commits = Vec::new();
        let mut pos = 0usize;
        let outcome = snapshot_turn(
            td.path(),
            &cfg,
            "s",
            "msg",
            1,
            &mut commits,
            &mut pos,
        )
        .unwrap();
        assert!(matches!(outcome, SnapshotOutcome::Disabled { .. }));
    }

    #[test]
    fn snapshot_respects_gitignore() {
        let td = init_test_repo();
        initial_commit(td.path());
        write_file(td.path(), ".gitignore", "secret.txt\n");
        write_file(td.path(), "secret.txt", "password\n");
        write_file(td.path(), "ok.txt", "safe\n");

        let cfg = AutoCommitConfig::default();
        let mut commits = Vec::new();
        let mut pos = 0usize;
        snapshot_turn(
            td.path(),
            &cfg,
            "s",
            "ignored",
            1,
            &mut commits,
            &mut pos,
        )
        .unwrap();
        assert_eq!(commits.len(), 1);

        // ls-tree the commit and confirm secret.txt is NOT present.
        let out = git_cmd(td.path())
            .args(["ls-tree", "-r", "--name-only", &commits[0]])
            .output()
            .unwrap();
        let tree = String::from_utf8(out.stdout).unwrap();
        assert!(!tree.contains("secret.txt"), "secret.txt leaked into tree:\n{tree}");
        assert!(tree.contains("ok.txt"));
    }

    #[test]
    fn snapshot_discards_redo_tail_on_new_work() {
        let td = init_test_repo();
        initial_commit(td.path());
        let cfg = AutoCommitConfig::default();
        let mut commits = Vec::new();
        let mut pos = 0usize;

        write_file(td.path(), "a.txt", "1\n");
        snapshot_turn(td.path(), &cfg, "s", "a", 1, &mut commits, &mut pos).unwrap();
        write_file(td.path(), "b.txt", "2\n");
        snapshot_turn(td.path(), &cfg, "s", "b", 2, &mut commits, &mut pos).unwrap();
        write_file(td.path(), "c.txt", "3\n");
        snapshot_turn(td.path(), &cfg, "s", "c", 3, &mut commits, &mut pos).unwrap();
        assert_eq!(commits.len(), 3);
        assert_eq!(pos, 3);

        // Simulate /undo 2 → pos = 1, then new work — expect redo tail discarded.
        pos = 1;
        write_file(td.path(), "d.txt", "4\n");
        snapshot_turn(td.path(), &cfg, "s", "d", 2, &mut commits, &mut pos).unwrap();
        assert_eq!(commits.len(), 2, "turns 2 and 3 should be dropped, new turn 2 appended");
        assert_eq!(pos, 2);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib autocommit::snapshot_tests 2>&1 | tail -20`
Expected: FAIL — `snapshot_turn` is not yet defined.

- [ ] **Step 3: Implement `snapshot_turn`**

Append to `src/autocommit.rs` (above the `#[cfg(test)] mod git_detection_tests` block — place all non-test code above all test modules):

```rust
// ── Snapshot pipeline ─────────────────────────────────────────────────────────

use std::process::{Command, Stdio};

fn shadow_ref(session_id: &str) -> String {
    format!("{SHADOW_REF_PREFIX}{session_id}")
}

/// Run a git command, capturing stdout; return the trimmed stdout as a String
/// on success or an error carrying stderr on failure.
fn git_output(cmd: &mut Command) -> anyhow::Result<String> {
    let out = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("git command failed: {stderr}");
    }
    let s = String::from_utf8(out.stdout)?;
    Ok(s.trim_end_matches('\n').to_string())
}

/// Resolve HEAD to a SHA, returning None if HEAD is unborn (no commits yet).
fn resolve_head(cwd: &Path) -> Option<String> {
    let out = git_cmd(cwd)
        .args(["rev-parse", "--verify", "HEAD"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// Look up the tree SHA of a given commit SHA. Returns an empty string if the
/// lookup fails (treated as "tree-not-found" by the empty-turn check).
fn tree_of_commit(cwd: &Path, commit: &str) -> String {
    git_cmd(cwd)
        .args(["rev-parse", &format!("{commit}^{{tree}}")])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Count the number of file entries in a tree via `git ls-tree -r --name-only <tree>`.
fn count_tree_files(cwd: &Path, tree: &str) -> u32 {
    let out = git_cmd(cwd)
        .args(["ls-tree", "-r", "--name-only", tree])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .count() as u32,
        _ => 0,
    }
}

/// Trim a user prompt to a 60-char single-line subject fragment.
fn subject_from_prompt(prompt: &str) -> String {
    let first_line = prompt.lines().next().unwrap_or("").trim();
    if first_line.chars().count() <= 60 {
        first_line.to_string()
    } else {
        let truncated: String = first_line.chars().take(57).collect();
        format!("{truncated}...")
    }
}

/// Take a full-tree snapshot of `cwd` as a commit on the session's shadow ref.
pub fn snapshot_turn(
    cwd: &Path,
    config: &AutoCommitConfig,
    session_id: &str,
    user_prompt: &str,
    turn_index: u32,
    auto_commits: &mut Vec<String>,
    undo_position: &mut usize,
) -> anyhow::Result<SnapshotOutcome> {
    if !config.enabled {
        return Ok(SnapshotOutcome::Disabled {
            reason: "auto-commit disabled in settings".to_string(),
        });
    }
    if !is_git_repo(cwd) {
        return Ok(SnapshotOutcome::Disabled {
            reason: "not a git repo".to_string(),
        });
    }

    // 1. Temp index file, isolated via GIT_INDEX_FILE.
    let td = tempfile::TempDir::new()?;
    let temp_index = td.path().join("turn.index");

    // 2. Seed the temp index from the parent's tree (if we have one) so `add -A`
    //    only records diffs relative to the parent, which makes empty-turn
    //    detection accurate even when the user's real index has other staging.
    let parent = if *undo_position > 0 {
        auto_commits.get(*undo_position - 1).cloned()
    } else {
        resolve_head(cwd)
    };
    if let Some(p) = &parent {
        let tree = tree_of_commit(cwd, p);
        if !tree.is_empty() {
            git_cmd(cwd)
                .env("GIT_INDEX_FILE", &temp_index)
                .args(["read-tree", &tree])
                .status()?;
        }
    }

    // 3. Stage everything under cwd into the temp index.
    let add_status = git_cmd(cwd)
        .env("GIT_INDEX_FILE", &temp_index)
        .args(["add", "-A"])
        .status()?;
    if !add_status.success() {
        anyhow::bail!("git add -A failed");
    }

    // 4. Write the tree.
    let tree_sha = git_output(
        git_cmd(cwd)
            .env("GIT_INDEX_FILE", &temp_index)
            .args(["write-tree"]),
    )?;

    // 5. Empty-turn optimization: compare against parent tree.
    if let Some(p) = &parent {
        if tree_of_commit(cwd, p) == tree_sha {
            return Ok(SnapshotOutcome::NoChanges);
        }
    }

    // 6. Build the commit.
    let subject = format!(
        "{} turn {}: {}",
        config.message_prefix,
        turn_index,
        subject_from_prompt(user_prompt),
    );
    let files = count_tree_files(cwd, &tree_sha);
    let body = format!(
        "\nRustyClaw-Session: {session_id}\nRustyClaw-Turn: {turn_index}\nRustyClaw-Files: {files}\n"
    );
    let full_msg = format!("{subject}\n{body}");

    let mut commit_cmd = git_cmd(cwd);
    commit_cmd
        .env("GIT_AUTHOR_NAME", "rustyclaw")
        .env("GIT_AUTHOR_EMAIL", "noreply@rustyclaw.local")
        .env("GIT_COMMITTER_NAME", "rustyclaw")
        .env("GIT_COMMITTER_EMAIL", "noreply@rustyclaw.local")
        .args(["commit-tree", &tree_sha, "-m", &full_msg]);
    if let Some(p) = &parent {
        commit_cmd.args(["-p", p]);
    }
    let commit_sha = git_output(&mut commit_cmd)?;

    // 7. Update the shadow ref.
    let ref_name = shadow_ref(session_id);
    let update_status = git_cmd(cwd)
        .args(["update-ref", &ref_name, &commit_sha])
        .status()?;
    if !update_status.success() {
        anyhow::bail!("git update-ref {ref_name} failed");
    }

    // 8. Discard redo tail if user was in an undone state, then append.
    if *undo_position < auto_commits.len() {
        auto_commits.truncate(*undo_position);
    }
    auto_commits.push(commit_sha.clone());
    *undo_position = auto_commits.len();

    Ok(SnapshotOutcome::Committed { sha: commit_sha, files })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib autocommit::snapshot_tests 2>&1 | tail -30`
Expected: 7 passed, 0 failed.

- [ ] **Step 5: Clippy**

Run: `cargo clippy --all-targets -- -W clippy::all 2>&1 | rg '^warning' | wc -l`
Expected: ≤ 217.

- [ ] **Step 6: Commit**

```bash
git add src/autocommit.rs
git commit -m "$(cat <<'EOF'
feat(autocommit): snapshot_turn via git plumbing

Writes a full-tree snapshot of cwd to refs/rustyclaw/sessions/<id>
using GIT_INDEX_FILE-isolated temp index, commit-tree, update-ref.
Seeds temp index from parent tree so empty-turn detection ignores the
user's real staging state. Respects .gitignore. Discards redo tail on
new work from an undone position. 7 unit tests.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 5: `restore_to` — undo/redo working-tree restore

**Files:**
- Modify: `src/autocommit.rs` (add `restore_to` + unit tests)

- [ ] **Step 1: Write failing tests**

Append to `src/autocommit.rs`, AFTER the `snapshot_tests` module:

```rust
#[cfg(test)]
mod restore_tests {
    use super::*;
    use super::git_detection_tests::init_test_repo;
    use super::snapshot_tests::*;
    use std::fs;

    // Tiny duplication of helpers so this module compiles standalone if we
    // ever split the tests file; low cost, high clarity.
    fn write_file(repo: &Path, rel: &str, body: &str) {
        let p = repo.join(rel);
        if let Some(parent) = p.parent() { fs::create_dir_all(parent).unwrap(); }
        fs::write(p, body).unwrap();
    }

    #[test]
    fn restore_overwrites_modified_files() {
        let td = init_test_repo();
        // Initial HEAD commit with v1 content.
        write_file(td.path(), "app.txt", "v1\n");
        git_cmd(td.path()).args(["add", "app.txt"]).status().unwrap();
        git_cmd(td.path()).args(["commit", "-q", "-m", "v1"]).status().unwrap();

        let cfg = AutoCommitConfig::default();
        let mut commits = Vec::new();
        let mut pos = 0usize;

        // Turn 1: v2
        write_file(td.path(), "app.txt", "v2\n");
        snapshot_turn(td.path(), &cfg, "s", "v2", 1, &mut commits, &mut pos).unwrap();
        // Turn 2: v3
        write_file(td.path(), "app.txt", "v3\n");
        snapshot_turn(td.path(), &cfg, "s", "v3", 2, &mut commits, &mut pos).unwrap();

        // Restore to turn 1 (position 1 → commits[0])
        let report = restore_to(td.path(), &commits, 1).unwrap();
        assert!(report.files_restored >= 1);
        let contents = std::fs::read_to_string(td.path().join("app.txt")).unwrap();
        assert_eq!(contents, "v2\n");

        // Restore to turn 2
        let _ = restore_to(td.path(), &commits, 2).unwrap();
        let contents = std::fs::read_to_string(td.path().join("app.txt")).unwrap();
        assert_eq!(contents, "v3\n");
    }

    #[test]
    fn restore_leaves_untracked_files_alone() {
        let td = init_test_repo();
        write_file(td.path(), "tracked.txt", "x\n");
        git_cmd(td.path()).args(["add", "tracked.txt"]).status().unwrap();
        git_cmd(td.path()).args(["commit", "-q", "-m", "base"]).status().unwrap();

        let cfg = AutoCommitConfig::default();
        let mut commits = Vec::new();
        let mut pos = 0usize;

        write_file(td.path(), "tracked.txt", "y\n");
        snapshot_turn(td.path(), &cfg, "s", "mod", 1, &mut commits, &mut pos).unwrap();

        // Create an untracked file AFTER the snapshot — it's not in any shadow commit.
        write_file(td.path(), "untracked.log", "scratch\n");

        restore_to(td.path(), &commits, 0).unwrap(); // back to session base (HEAD tree)
        // tracked.txt restored to "x\n"
        assert_eq!(
            std::fs::read_to_string(td.path().join("tracked.txt")).unwrap(),
            "x\n"
        );
        // untracked.log still exists
        assert!(td.path().join("untracked.log").exists(), "untracked file was deleted!");
    }

    #[test]
    fn restore_to_session_base_zero_position() {
        let td = init_test_repo();
        write_file(td.path(), "app.txt", "base\n");
        git_cmd(td.path()).args(["add", "app.txt"]).status().unwrap();
        git_cmd(td.path()).args(["commit", "-q", "-m", "base"]).status().unwrap();

        let cfg = AutoCommitConfig::default();
        let mut commits = Vec::new();
        let mut pos = 0usize;

        write_file(td.path(), "app.txt", "modified\n");
        snapshot_turn(td.path(), &cfg, "s", "m", 1, &mut commits, &mut pos).unwrap();

        restore_to(td.path(), &commits, 0).unwrap();
        assert_eq!(
            std::fs::read_to_string(td.path().join("app.txt")).unwrap(),
            "base\n"
        );
    }

    #[test]
    fn restore_reports_orphaned_files() {
        let td = init_test_repo();
        write_file(td.path(), "old.txt", "kept\n");
        git_cmd(td.path()).args(["add", "old.txt"]).status().unwrap();
        git_cmd(td.path()).args(["commit", "-q", "-m", "base"]).status().unwrap();

        let cfg = AutoCommitConfig::default();
        let mut commits = Vec::new();
        let mut pos = 0usize;

        write_file(td.path(), "new.txt", "added\n");
        snapshot_turn(td.path(), &cfg, "s", "add new", 1, &mut commits, &mut pos).unwrap();

        // new.txt is tracked in turn 1. Restore to session base — new.txt becomes orphaned.
        let report = restore_to(td.path(), &commits, 0).unwrap();
        assert!(
            report.orphaned_files.iter().any(|p| p.ends_with("new.txt")),
            "expected new.txt to be flagged as orphaned: {:?}",
            report.orphaned_files
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib autocommit::restore_tests 2>&1 | tail -20`
Expected: FAIL — `restore_to` not yet defined.

- [ ] **Step 3: Implement `restore_to`**

Append to `src/autocommit.rs` (above the test modules):

```rust
// ── Restore pipeline ──────────────────────────────────────────────────────────

/// List files (recursive, names only) in a given tree SHA.
fn list_tree_files(cwd: &Path, tree: &str) -> Vec<String> {
    git_cmd(cwd)
        .args(["ls-tree", "-r", "--name-only", tree])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| {
            s.lines()
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// List currently-tracked files in the user's real index (`git ls-files`).
/// Used to compute the orphan diff for `restore_to`.
fn list_tracked_files(cwd: &Path) -> Vec<String> {
    git_cmd(cwd)
        .args(["ls-files"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| {
            s.lines()
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Restore the working tree at `cwd` to the tree of `auto_commits[target_position - 1]`
/// (or the parent of `auto_commits[0]` / HEAD when `target_position == 0`).
pub fn restore_to(
    cwd: &Path,
    auto_commits: &[String],
    target_position: usize,
) -> anyhow::Result<RestoreReport> {
    if !is_git_repo(cwd) {
        anyhow::bail!("not a git repo");
    }
    if target_position > auto_commits.len() {
        anyhow::bail!(
            "target_position {target_position} out of range (max {})",
            auto_commits.len()
        );
    }

    // Resolve the target tree SHA.
    let tree_sha = if target_position == 0 {
        // Session base = parent of auto_commits[0], else HEAD, else empty tree.
        if let Some(first) = auto_commits.first() {
            let out = git_cmd(cwd)
                .args(["rev-parse", &format!("{first}^^{{tree}}")])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output();
            let parent_tree = out
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            if !parent_tree.is_empty() {
                parent_tree
            } else if let Some(head) = resolve_head(cwd) {
                tree_of_commit(cwd, &head)
            } else {
                // Unborn branch and first auto-commit had no parent → empty tree.
                "4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string() // git's canonical empty tree
            }
        } else if let Some(head) = resolve_head(cwd) {
            tree_of_commit(cwd, &head)
        } else {
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string()
        }
    } else {
        let commit = &auto_commits[target_position - 1];
        tree_of_commit(cwd, commit)
    };

    if tree_sha.is_empty() {
        anyhow::bail!("could not resolve target tree");
    }

    // Compute orphan diff BEFORE overwriting the working tree.
    let target_files: std::collections::HashSet<String> =
        list_tree_files(cwd, &tree_sha).into_iter().collect();
    let current_files = list_tracked_files(cwd);
    let orphaned_files: Vec<PathBuf> = current_files
        .iter()
        .filter(|f| !target_files.contains(*f))
        .map(PathBuf::from)
        .collect();

    // Build a temp index holding only the target tree.
    let td = tempfile::TempDir::new()?;
    let temp_index = td.path().join("restore.index");
    let read_status = git_cmd(cwd)
        .env("GIT_INDEX_FILE", &temp_index)
        .args(["read-tree", &tree_sha])
        .status()?;
    if !read_status.success() {
        anyhow::bail!("git read-tree {tree_sha} failed");
    }

    // Extract all entries into cwd. The trailing slash on --prefix is mandatory.
    let prefix = format!("{}/", cwd.display());
    let checkout_status = git_cmd(cwd)
        .env("GIT_INDEX_FILE", &temp_index)
        .args(["checkout-index", "-a", "-f", "--prefix", &prefix])
        .status()?;
    if !checkout_status.success() {
        anyhow::bail!("git checkout-index --prefix={prefix} failed");
    }

    Ok(RestoreReport {
        files_restored: target_files.len() as u32,
        orphaned_files,
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib autocommit::restore_tests 2>&1 | tail -30`
Expected: 4 passed, 0 failed.

- [ ] **Step 5: Clippy**

Run: `cargo clippy --all-targets -- -W clippy::all 2>&1 | rg '^warning' | wc -l`
Expected: ≤ 217.

- [ ] **Step 6: Commit**

```bash
git add src/autocommit.rs
git commit -m "$(cat <<'EOF'
feat(autocommit): restore_to via read-tree + checkout-index

restore_to rebuilds a temp index from the target tree and extracts
files into cwd via checkout-index --prefix. Untracked files are left
alone (no git clean). Orphaned tracked files (present in working tree
but not in the target tree) are reported but not deleted; caller
surfaces them in the /undo SystemMessage. 4 unit tests.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 6: `prune_old_refs` — keep N newest session refs

**Files:**
- Modify: `src/autocommit.rs` (add `prune_old_refs` + unit tests)

- [ ] **Step 1: Write failing tests**

Append to `src/autocommit.rs`, AFTER the `restore_tests` module:

```rust
#[cfg(test)]
mod prune_tests {
    use super::*;
    use super::git_detection_tests::init_test_repo;

    /// Create a single empty commit so we can point shadow refs at it.
    fn make_empty_commit(cwd: &Path, subject: &str) -> String {
        let td = tempfile::TempDir::new().unwrap();
        let idx = td.path().join("idx");
        let tree = git_cmd(cwd)
            .env("GIT_INDEX_FILE", &idx)
            .args(["write-tree"])
            .output()
            .unwrap();
        assert!(tree.status.success());
        let tree_sha = String::from_utf8(tree.stdout).unwrap().trim().to_string();
        let out = git_cmd(cwd)
            .env("GIT_AUTHOR_NAME", "rustyclaw")
            .env("GIT_AUTHOR_EMAIL", "noreply@rustyclaw.local")
            .env("GIT_COMMITTER_NAME", "rustyclaw")
            .env("GIT_COMMITTER_EMAIL", "noreply@rustyclaw.local")
            .args(["commit-tree", &tree_sha, "-m", subject])
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn make_ref(cwd: &Path, name: &str, sha: &str) {
        let s = git_cmd(cwd)
            .args(["update-ref", name, sha])
            .status()
            .unwrap();
        assert!(s.success());
    }

    #[test]
    fn prune_keeps_newest_n() {
        let td = init_test_repo();
        // Seed with an initial commit so HEAD resolves and new commits have
        // a deterministic committer-date ordering via -c.
        std::fs::write(td.path().join("x"), "").unwrap();
        git_cmd(td.path()).args(["add", "x"]).status().unwrap();
        git_cmd(td.path()).args(["commit", "-q", "-m", "x"]).status().unwrap();

        // Create 5 session refs at distinct committer dates (1000000000 + i).
        for i in 0..5 {
            let sha = make_empty_commit(td.path(), &format!("session-{i}"));
            make_ref(td.path(), &format!("refs/rustyclaw/sessions/s{i}"), &sha);
        }

        let deleted = prune_old_refs(td.path(), 3).unwrap();
        assert_eq!(deleted, 2, "should delete the 2 oldest of 5 refs");

        // Count remaining session refs.
        let out = git_cmd(td.path())
            .args(["for-each-ref", "--format=%(refname)", "refs/rustyclaw/sessions/"])
            .output()
            .unwrap();
        let remaining = String::from_utf8(out.stdout).unwrap();
        assert_eq!(remaining.lines().count(), 3);
    }

    #[test]
    fn prune_noop_when_unlimited() {
        let td = init_test_repo();
        std::fs::write(td.path().join("x"), "").unwrap();
        git_cmd(td.path()).args(["add", "x"]).status().unwrap();
        git_cmd(td.path()).args(["commit", "-q", "-m", "x"]).status().unwrap();
        for i in 0..3 {
            let sha = make_empty_commit(td.path(), &format!("s{i}"));
            make_ref(td.path(), &format!("refs/rustyclaw/sessions/s{i}"), &sha);
        }
        let deleted = prune_old_refs(td.path(), 0).unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn prune_noop_when_below_threshold() {
        let td = init_test_repo();
        std::fs::write(td.path().join("x"), "").unwrap();
        git_cmd(td.path()).args(["add", "x"]).status().unwrap();
        git_cmd(td.path()).args(["commit", "-q", "-m", "x"]).status().unwrap();
        for i in 0..2 {
            let sha = make_empty_commit(td.path(), &format!("s{i}"));
            make_ref(td.path(), &format!("refs/rustyclaw/sessions/s{i}"), &sha);
        }
        let deleted = prune_old_refs(td.path(), 10).unwrap();
        assert_eq!(deleted, 0);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib autocommit::prune_tests 2>&1 | tail -20`
Expected: FAIL — `prune_old_refs` not yet defined.

- [ ] **Step 3: Implement `prune_old_refs`**

Append to `src/autocommit.rs` (above the test modules):

```rust
// ── Prune pipeline ────────────────────────────────────────────────────────────

/// Delete old `refs/rustyclaw/sessions/*` refs, keeping the `keep` newest by
/// committer date. `keep == 0` disables pruning. Non-fatal: any error is
/// logged via `tracing::warn!` and the function returns 0.
pub fn prune_old_refs(cwd: &Path, keep: u32) -> anyhow::Result<u32> {
    if keep == 0 || !is_git_repo(cwd) {
        return Ok(0);
    }

    // List refs with committer-date for sorting.
    let out = git_cmd(cwd)
        .args([
            "for-each-ref",
            "--format=%(committerdate:unix) %(refname)",
            SHADOW_REF_PREFIX,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()?;
    if !out.status.success() {
        return Ok(0);
    }
    let s = String::from_utf8(out.stdout)?;
    let mut rows: Vec<(i64, String)> = s
        .lines()
        .filter_map(|line| {
            let mut it = line.splitn(2, ' ');
            let ts = it.next()?.parse::<i64>().ok()?;
            let refname = it.next()?.to_string();
            Some((ts, refname))
        })
        .collect();

    if rows.len() <= keep as usize {
        return Ok(0);
    }

    // Sort descending (newest first).
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    let to_delete: Vec<String> = rows
        .into_iter()
        .skip(keep as usize)
        .map(|(_, r)| r)
        .collect();

    let mut deleted = 0u32;
    for r in &to_delete {
        let status = git_cmd(cwd).args(["update-ref", "-d", r]).status();
        match status {
            Ok(s) if s.success() => deleted += 1,
            Ok(_) => tracing::warn!("autoCommit prune: failed to delete {r}"),
            Err(e) => tracing::warn!("autoCommit prune: error deleting {r}: {e}"),
        }
    }
    Ok(deleted)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib autocommit::prune_tests 2>&1 | tail -20`
Expected: 3 passed, 0 failed.

- [ ] **Step 5: Run the full autocommit test suite**

Run: `cargo test --lib autocommit 2>&1 | tail -30`
Expected: 17 passed total (3 detection + 7 snapshot + 4 restore + 3 prune).

- [ ] **Step 6: Clippy**

Run: `cargo clippy --all-targets -- -W clippy::all 2>&1 | rg '^warning' | wc -l`
Expected: ≤ 217.

- [ ] **Step 7: Commit**

```bash
git add src/autocommit.rs
git commit -m "$(cat <<'EOF'
feat(autocommit): prune_old_refs keeps N newest by committer date

Lists refs/rustyclaw/sessions/* via for-each-ref, sorts by committer
date descending, and deletes everything past 'keep'. keep=0 disables
pruning entirely. Individual delete failures are logged and counted
toward non-deleted — overall function is non-fatal on errors. 3 tests.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 7: `SessionMeta` extension + serde backward compat test

**Files:**
- Modify: `src/session/mod.rs` (add `auto_commits` and `undo_position` fields)

- [ ] **Step 1: Write the failing test — add inline mod tests at EOF**

Open `src/session/mod.rs`. At the very end of the file, append:

```rust
#[cfg(test)]
mod meta_serde_tests {
    use super::SessionMeta;

    #[test]
    fn loads_legacy_meta_without_autocommit_fields() {
        // Legacy .meta files (pre-autocommit feature) do NOT contain
        // `auto_commits` or `undo_position`. Those must default to empty/0.
        let json = r#"{
            "id": "abc",
            "name": "Test",
            "created_at": 1700000000,
            "preview": "hello"
        }"#;
        let m: SessionMeta = serde_json::from_str(json).unwrap();
        assert_eq!(m.id, "abc");
        assert!(m.auto_commits.is_empty());
        assert_eq!(m.undo_position, 0);
    }

    #[test]
    fn roundtrips_with_autocommit_fields() {
        let json = r#"{
            "id": "xyz",
            "name": "Test",
            "created_at": 1700000000,
            "preview": "hi",
            "auto_commits": ["aaa111", "bbb222"],
            "undo_position": 2
        }"#;
        let m: SessionMeta = serde_json::from_str(json).unwrap();
        assert_eq!(m.auto_commits, vec!["aaa111", "bbb222"]);
        assert_eq!(m.undo_position, 2);

        // Reserialise and confirm both fields survive.
        let out = serde_json::to_string(&m).unwrap();
        assert!(out.contains("auto_commits"));
        assert!(out.contains("undo_position"));
    }
}
```

- [ ] **Step 2: Run tests — they should fail**

Run: `cargo test --bin rustyclaw meta_serde_tests 2>&1 | tail -20`
Expected: FAIL — `auto_commits` and `undo_position` fields don't exist on `SessionMeta`.

- [ ] **Step 3: Add the fields to `SessionMeta`**

Open `src/session/mod.rs`. Locate the `SessionMeta` struct (around line 18):

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct SessionMeta {
    pub id: String,
    pub name: String,
    pub created_at: u64, // unix seconds
    pub preview: String, // first user message (truncated)
    #[serde(default)]
    pub tags: Vec<String>,
}
```

Replace with:

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct SessionMeta {
    pub id: String,
    pub name: String,
    pub created_at: u64, // unix seconds
    pub preview: String, // first user message (truncated)
    #[serde(default)]
    pub tags: Vec<String>,
    /// Auto-commit SHAs on the session's shadow ref, chronological order
    /// (oldest → newest). Empty when auto-commit is disabled or cwd is
    /// outside a git work tree.
    #[serde(default)]
    pub auto_commits: Vec<String>,
    /// User's current read-head inside `auto_commits`. `0` means
    /// "at session base"; `auto_commits.len()` means "at latest turn".
    #[serde(default)]
    pub undo_position: usize,
}
```

- [ ] **Step 4: Update `Session::new` to set the new fields**

In `src/session/mod.rs`, locate `Session::new` (around line 60). Find the `SessionMeta { ... }` literal:

```rust
        let meta = SessionMeta {
            id: id.clone(),
            name: human_session_name(),
            created_at: unix_now(),
            preview: String::new(),
            tags: Vec::new(),
        };
```

Replace with:

```rust
        let meta = SessionMeta {
            id: id.clone(),
            name: human_session_name(),
            created_at: unix_now(),
            preview: String::new(),
            tags: Vec::new(),
            auto_commits: Vec::new(),
            undo_position: 0,
        };
```

- [ ] **Step 5: Run tests**

Run: `cargo test --bin rustyclaw meta_serde_tests 2>&1 | tail -20`
Expected: 2 passed, 0 failed.

- [ ] **Step 6: Full build**

Run: `cargo build 2>&1 | tail -15`
Expected: clean build.

- [ ] **Step 7: Clippy**

Run: `cargo clippy --all-targets -- -W clippy::all 2>&1 | rg '^warning' | wc -l`
Expected: ≤ 217.

- [ ] **Step 8: Commit**

```bash
git add src/session/mod.rs
git commit -m "$(cat <<'EOF'
feat(session): add auto_commits + undo_position to SessionMeta

Both fields use #[serde(default)] so legacy .meta files on disk load
cleanly with empty history. Session::new initialises them to Vec::new()
and 0. 2 serde round-trip tests cover the legacy-load and full-round-trip
cases.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 8: Slash commands — `/undo`, `/redo`, `/autocommit`

**Files:**
- Modify: `src/commands/mod.rs` (register commands + add `CommandAction` variants + cmd_ functions + help category)

- [ ] **Step 1: Locate the `CommandAction` enum and scan for the pattern**

Run: `rg -n 'pub enum CommandAction' src/commands/mod.rs`
Run: `rg -n 'NoOp|SystemMessage' src/commands/mod.rs | head -20`

This orients you to the existing dispatch surface before editing.

- [ ] **Step 2: Add new `CommandAction` variants**

Open `src/commands/mod.rs`. Locate `pub enum CommandAction` and add the three new variants. Use Grep to confirm the exact surrounding lines of the last variant so the Edit is unique, then append before the closing `}` of the enum:

```rust
    /// `/undo [N]` — restore working tree to an earlier auto-commit.
    /// `n == None` opens a picker; `n == Some(k)` rewinds k turns.
    Undo { n: Option<u32> },
    /// `/redo [N]` — restore working tree to a later auto-commit in the redo stack.
    /// `n == None` opens a picker; `n == Some(k)` advances k turns.
    Redo { n: Option<u32> },
    /// `/autocommit status` — print auto-commit state to the chat.
    AutoCommitStatus,
```

- [ ] **Step 3: Add `cmd_undo`, `cmd_redo`, `cmd_autocommit` functions**

At the very end of `src/commands/mod.rs` (append, respecting existing style):

```rust
fn cmd_undo(args: &str) -> CommandAction {
    let n = args.trim().parse::<u32>().ok().filter(|n| *n > 0);
    CommandAction::Undo { n }
}

fn cmd_redo(args: &str) -> CommandAction {
    let n = args.trim().parse::<u32>().ok().filter(|n| *n > 0);
    CommandAction::Redo { n }
}

fn cmd_autocommit(args: &str) -> CommandAction {
    // v1 only supports `status`. Any other arg (or none) shows status.
    let _ = args;
    CommandAction::AutoCommitStatus
}
```

- [ ] **Step 4: Register the commands in the dispatch table**

Locate the command dispatch table — look for a `match` on command name or a `HashMap` lookup. Run:

```
rg -n '"clear"|"help"|"model"' src/commands/mod.rs | head -20
```

This finds where existing commands like `/clear`, `/help`, `/model` are registered. Follow the same pattern. If it's a `match` inside something like `fn dispatch(name: &str, args: &str)`, add:

```rust
        "undo"       => cmd_undo(args),
        "redo"       => cmd_redo(args),
        "autocommit" => cmd_autocommit(args),
```

If it's a HashMap, insert equivalent entries. The exact key name format must match the existing commands — mirror them precisely.

- [ ] **Step 5: Add help entries**

Find the `HELP_CATEGORIES` constant (search: `rg -n 'HELP_CATEGORIES' src/commands/mod.rs`). Add the three commands under the most appropriate category (probably "Session" or "Workflow" — whichever hosts existing editing/state commands like `/compact` or `/clear`):

```rust
        HelpCommand {
            name: "/undo",
            description: "Rewind working tree to an earlier auto-commit turn ([N] or picker)",
        },
        HelpCommand {
            name: "/redo",
            description: "Advance working tree to a later auto-commit turn ([N] or picker)",
        },
        HelpCommand {
            name: "/autocommit",
            description: "Show auto-commit status (enabled, session ID, turns recorded)",
        },
```

- [ ] **Step 6: Build**

Run: `cargo build 2>&1 | tail -20`
Expected: may show `non-exhaustive match` errors in `run.rs` because the new `CommandAction` variants aren't handled yet. That's expected and fixed in Task 9.

**If the build fails** with errors OTHER than the `CommandAction` exhaustiveness issue, stop and fix them before committing.

- [ ] **Step 7: Commit (with a marker for the partially-broken state)**

Even if the build has the expected non-exhaustive match warnings/errors, commit the command layer so Task 9 can focus solely on the TUI wiring. If the build fails outright, add a stub handler in `run.rs` that emits `SystemMessage("not yet implemented")` for the three new variants so the build is green — but this is a last resort. Prefer to finish Task 9 in the same working copy if the errors cascade.

```bash
git add src/commands/mod.rs
git commit -m "$(cat <<'EOF'
feat(commands): register /undo, /redo, /autocommit commands

Adds CommandAction::{Undo, Redo, AutoCommitStatus} variants and the
cmd_ dispatch functions. Help entries slot under the session category.
TUI wiring lands in the next task.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 9: TUI wiring — overlay pickers, snapshot at turn end, startup prune

**Files:**
- Modify: `src/tui/run.rs` (~5 edits)
- Modify: `src/session/mod.rs` (optional: add a `save_meta` helper if one doesn't already exist)

This is the largest task. Break it into sub-steps and commit frequently (after each sub-step, if the build is green).

- [ ] **Step 1: Locate the startup block and add the prune call**

Find where `Config::load` is called near startup in `run.rs` and where the session is loaded/created. Run:

```
rg -n 'Config::load|Session::new|Session::resume' src/tui/run.rs | head -20
```

After the session is established (so `session.id` is available) and BEFORE the first TUI frame is drawn, add:

```rust
    // Prune old auto-commit shadow refs (keeps the 10 newest by committer date).
    if let Err(e) =
        rustyclaw::autocommit::prune_old_refs(&config.cwd, config.auto_commit.keep_sessions)
    {
        tracing::warn!("autoCommit startup prune failed: {e}");
    }
```

Note: if `rustyclaw::` is not the binary crate name, use `crate::autocommit::` instead. Check `Cargo.toml` `[package] name`.

- [ ] **Step 2: Locate the end-of-assistant-turn hook**

The auto-fix loop already runs at end of turn — find it:

```
rg -n 'auto_fix|auto_fix_touched' src/tui/run.rs | head -20
```

Immediately AFTER the auto-fix loop completes (after the match on `AutoFixAction`), add:

```rust
    // Auto-commit: snapshot the working tree for this turn.
    if app.config.auto_commit.enabled {
        let turn_index = (session.meta.auto_commits.len() as u32) + 1;
        // Use the latest user prompt for the subject; fall back to empty string.
        let prompt = app
            .chat
            .iter()
            .rev()
            .find_map(|e| match e {
                crate::tui::app::ChatEntry::User(t) => Some(t.clone()),
                _ => None,
            })
            .unwrap_or_default();
        match crate::autocommit::snapshot_turn(
            &app.config.cwd,
            &app.config.auto_commit,
            &session.id,
            &prompt,
            turn_index,
            &mut session.meta.auto_commits,
            &mut session.meta.undo_position,
        ) {
            Ok(crate::autocommit::SnapshotOutcome::Committed { sha, files }) => {
                tracing::info!(
                    "autoCommit: turn {turn_index} committed ({files} files, sha={})",
                    &sha[..7]
                );
                // Persist updated meta.
                if let Err(e) = session.meta.save().await {
                    tracing::warn!("autoCommit: failed to save meta after snapshot: {e}");
                }
            }
            Ok(crate::autocommit::SnapshotOutcome::NoChanges) => {
                tracing::debug!("autoCommit: turn {turn_index} had no file changes");
            }
            Ok(crate::autocommit::SnapshotOutcome::Disabled { reason }) => {
                tracing::debug!("autoCommit: disabled ({reason})");
            }
            Err(e) => {
                tracing::warn!("autoCommit: snapshot failed: {e}");
            }
        }
    }
```

**Note:** `session.meta.save()` is currently private to the `Session` impl block. If it isn't accessible here, either (a) expose it via a new `pub(crate) async fn save_meta(&self) -> Result<()>` wrapper on `Session`, or (b) call a public-facing save helper. Check with:

```
rg -n 'fn save|pub.*save' src/session/mod.rs
```

If needed, add to `src/session/mod.rs` inside `impl Session`:

```rust
    /// Persist the current `SessionMeta` to disk. Used by the auto-commit loop
    /// to checkpoint updated `auto_commits` / `undo_position` after each turn.
    pub async fn save_meta(&self) -> anyhow::Result<()> {
        self.meta.save().await
    }
```

And replace `session.meta.save().await` with `session.save_meta().await` in the snapshot block above.

- [ ] **Step 3: Handle `CommandAction::Undo` and `Redo`**

Locate the `CommandAction` match in `run.rs`:

```
rg -n 'CommandAction::' src/tui/run.rs | head -30
```

Add these three arms (group them together near the other editing commands like `CommandAction::Clear`):

```rust
            CommandAction::Undo { n } => {
                if app.generating {
                    app.chat.push(ChatEntry::system(
                        "[undo] cannot undo while an assistant turn is running",
                    ));
                } else if !app.config.auto_commit.enabled {
                    app.chat.push(ChatEntry::system(
                        "[undo] auto-commit is disabled in settings",
                    ));
                } else if !crate::autocommit::is_git_repo(&app.config.cwd) {
                    app.chat.push(ChatEntry::system(
                        "[undo] auto-commit disabled — not a git repo",
                    ));
                } else if session.meta.auto_commits.is_empty() {
                    app.chat.push(ChatEntry::system(
                        "[undo] nothing to undo (session has no auto-commits)",
                    ));
                } else if session.meta.undo_position == 0 {
                    app.chat.push(ChatEntry::system(
                        "[undo] at session start, nothing more to undo",
                    ));
                } else {
                    match n {
                        Some(k) => {
                            let new_pos = session
                                .meta
                                .undo_position
                                .saturating_sub(k as usize);
                            match crate::autocommit::restore_to(
                                &app.config.cwd,
                                &session.meta.auto_commits,
                                new_pos,
                            ) {
                                Ok(report) => {
                                    session.meta.undo_position = new_pos;
                                    let _ = session.save_meta().await;
                                    let label = if new_pos == 0 {
                                        "session base".to_string()
                                    } else {
                                        format!("turn {new_pos}")
                                    };
                                    app.chat.push(ChatEntry::system(format!(
                                        "[undo] rewound to {label} ({} files restored)",
                                        report.files_restored
                                    )));
                                }
                                Err(e) => {
                                    app.chat.push(ChatEntry::system(format!(
                                        "[undo] restore failed: {e}"
                                    )));
                                }
                            }
                        }
                        None => {
                            // Build picker items from auto_commits[0..undo_position]
                            // in reverse order, plus a "session base" sentinel.
                            let mut labels: Vec<String> = Vec::new();
                            let mut positions: Vec<usize> = Vec::new(); // target positions
                            let cur = session.meta.undo_position;
                            for i in (0..cur).rev() {
                                // Target position when the user picks this row.
                                let target_pos = i + 1;
                                let marker = if target_pos == cur { " ← current" } else { "" };
                                labels.push(format!(
                                    "turn {target_pos}  ·  {}{marker}",
                                    session.meta.auto_commits[i].chars().take(7).collect::<String>()
                                ));
                                positions.push(target_pos);
                            }
                            labels.push("session base (pre-RustyClaw)".to_string());
                            positions.push(0);
                            app.overlay = Some(crate::tui::app::Overlay::with_items(
                                "undo".to_string(),
                                String::new(),
                                labels,
                            ));
                            app.pending_undo_positions = Some(positions);
                        }
                    }
                }
            }
            CommandAction::Redo { n } => {
                if app.generating {
                    app.chat.push(ChatEntry::system(
                        "[redo] cannot redo while an assistant turn is running",
                    ));
                } else if !app.config.auto_commit.enabled {
                    app.chat.push(ChatEntry::system(
                        "[redo] auto-commit is disabled in settings",
                    ));
                } else if !crate::autocommit::is_git_repo(&app.config.cwd) {
                    app.chat.push(ChatEntry::system(
                        "[redo] auto-commit disabled — not a git repo",
                    ));
                } else if session.meta.undo_position == session.meta.auto_commits.len() {
                    app.chat.push(ChatEntry::system(
                        "[redo] nothing to redo (at latest turn)",
                    ));
                } else {
                    match n {
                        Some(k) => {
                            let new_pos = (session.meta.undo_position + k as usize)
                                .min(session.meta.auto_commits.len());
                            match crate::autocommit::restore_to(
                                &app.config.cwd,
                                &session.meta.auto_commits,
                                new_pos,
                            ) {
                                Ok(report) => {
                                    session.meta.undo_position = new_pos;
                                    let _ = session.save_meta().await;
                                    app.chat.push(ChatEntry::system(format!(
                                        "[redo] advanced to turn {new_pos} ({} files restored)",
                                        report.files_restored
                                    )));
                                }
                                Err(e) => {
                                    app.chat.push(ChatEntry::system(format!(
                                        "[redo] restore failed: {e}"
                                    )));
                                }
                            }
                        }
                        None => {
                            let mut labels: Vec<String> = Vec::new();
                            let mut positions: Vec<usize> = Vec::new();
                            let cur = session.meta.undo_position;
                            // Row 0: current position.
                            let cur_label = if cur == 0 {
                                "session base (pre-RustyClaw) ← current".to_string()
                            } else {
                                format!(
                                    "turn {cur}  ·  {} ← current",
                                    session.meta.auto_commits[cur - 1].chars().take(7).collect::<String>()
                                )
                            };
                            labels.push(cur_label);
                            positions.push(cur);
                            // Subsequent rows: forward turns in the redo stack.
                            for i in cur..session.meta.auto_commits.len() {
                                labels.push(format!(
                                    "turn {}  ·  {}",
                                    i + 1,
                                    session.meta.auto_commits[i].chars().take(7).collect::<String>()
                                ));
                                positions.push(i + 1);
                            }
                            app.overlay = Some(crate::tui::app::Overlay::with_items(
                                "redo".to_string(),
                                String::new(),
                                labels,
                            ));
                            app.pending_redo_positions = Some(positions);
                        }
                    }
                }
            }
            CommandAction::AutoCommitStatus => {
                let cwd_ok = crate::autocommit::is_git_repo(&app.config.cwd);
                let msg = format!(
                    "Auto-commit status:\n  \
                     Enabled:        {}\n  \
                     Repo:           {}\n  \
                     Session ID:     {}\n  \
                     Turns recorded: {}\n  \
                     Undo position:  {} / {}\n  \
                     Retention:      keep {} most recent sessions\n  \
                     Message prefix: \"{}\"",
                    if app.config.auto_commit.enabled { "yes" } else { "no" },
                    if cwd_ok { "git detected (cwd)" } else { "not a git repo" },
                    session.id,
                    session.meta.auto_commits.len(),
                    session.meta.undo_position,
                    session.meta.auto_commits.len(),
                    app.config.auto_commit.keep_sessions,
                    app.config.auto_commit.message_prefix,
                );
                app.chat.push(ChatEntry::system(msg));
            }
```

- [ ] **Step 4: Add the pending_* fields to `App`**

Open `src/tui/app.rs` (find it with `rg -n 'struct App' src/tui/app.rs`). Locate the existing `pending_*` fields (search for `pending_`) and add:

```rust
    /// When the /undo picker is open, maps overlay row index → target
    /// position in `session.meta.auto_commits`.
    pub pending_undo_positions: Option<Vec<usize>>,
    /// When the /redo picker is open, maps overlay row index → target
    /// position in `session.meta.auto_commits`.
    pub pending_redo_positions: Option<Vec<usize>>,
```

Also add them to `App::new` (or `Default::default` — whichever constructor exists) with `None` initial values. Search:

```
rg -n 'pending_help_category|pending_voice_model' src/tui/app.rs
```

and follow the same pattern.

- [ ] **Step 5: Handle overlay selection for undo/redo**

In `src/tui/run.rs`, find the overlay key handler (look for `"help"`, `"models"`, `"voices"` title-based dispatch):

```
rg -n 'pending_voice_model|pending_help_category' src/tui/run.rs
```

In the place where the overlay title is matched, add handling for `"undo"` and `"redo"`:

```rust
                    "undo" => {
                        if let Some(positions) = app.pending_undo_positions.take() {
                            if let Some(&target_pos) = positions.get(selected_index) {
                                // No-op if selecting the "← current" row.
                                if target_pos != session.meta.undo_position {
                                    match crate::autocommit::restore_to(
                                        &app.config.cwd,
                                        &session.meta.auto_commits,
                                        target_pos,
                                    ) {
                                        Ok(report) => {
                                            session.meta.undo_position = target_pos;
                                            let _ = session.save_meta().await;
                                            let label = if target_pos == 0 {
                                                "session base".to_string()
                                            } else {
                                                format!("turn {target_pos}")
                                            };
                                            app.chat.push(ChatEntry::system(format!(
                                                "[undo] rewound to {label} ({} files restored)",
                                                report.files_restored
                                            )));
                                        }
                                        Err(e) => {
                                            app.chat.push(ChatEntry::system(format!(
                                                "[undo] restore failed: {e}"
                                            )));
                                        }
                                    }
                                }
                            }
                        }
                        app.overlay = None;
                    }
                    "redo" => {
                        if let Some(positions) = app.pending_redo_positions.take() {
                            if let Some(&target_pos) = positions.get(selected_index) {
                                if target_pos != session.meta.undo_position {
                                    match crate::autocommit::restore_to(
                                        &app.config.cwd,
                                        &session.meta.auto_commits,
                                        target_pos,
                                    ) {
                                        Ok(report) => {
                                            session.meta.undo_position = target_pos;
                                            let _ = session.save_meta().await;
                                            app.chat.push(ChatEntry::system(format!(
                                                "[redo] advanced to turn {target_pos} ({} files restored)",
                                                report.files_restored
                                            )));
                                        }
                                        Err(e) => {
                                            app.chat.push(ChatEntry::system(format!(
                                                "[redo] restore failed: {e}"
                                            )));
                                        }
                                    }
                                }
                            }
                        }
                        app.overlay = None;
                    }
```

(The exact variable names `selected_index`, `app.overlay`, etc., must match the existing overlay handler — mirror them precisely.)

- [ ] **Step 6: Also clear pending_undo_positions / pending_redo_positions on Esc**

In the overlay Esc-cancel handler, add:

```rust
        app.pending_undo_positions = None;
        app.pending_redo_positions = None;
```

next to the other `pending_* = None` assignments.

- [ ] **Step 7: Build**

Run: `cargo build 2>&1 | tail -30`
Expected: clean build.

If you see errors, common causes:
- `session` is `&`, not `&mut` — propagate mutability up the call chain or use an `Arc<RwLock<Session>>` (check the existing pattern).
- `save_meta` access — add the wrapper from Step 2.
- `ChatEntry::system` — confirm the exact constructor name with `rg -n 'ChatEntry::' src/tui/app.rs`.

- [ ] **Step 8: Run all tests**

Run: `cargo test 2>&1 | tail -20`
Expected: all tests pass, baseline of 284 + new tests from Tasks 1–7.

- [ ] **Step 9: Clippy**

Run: `cargo clippy --all-targets -- -W clippy::all 2>&1 | rg '^warning' | wc -l`
Expected: ≤ 217.

- [ ] **Step 10: Commit**

```bash
git add src/tui/run.rs src/tui/app.rs src/session/mod.rs
git commit -m "$(cat <<'EOF'
feat(tui): wire autocommit snapshot + /undo /redo /autocommit

Startup prunes old session shadow refs. Every assistant turn calls
snapshot_turn after the auto-fix loop and persists updated meta.
/undo and /redo open an overlay picker (title-based dispatch) with
row-to-position maps in app.pending_{undo,redo}_positions, or rewind
N turns when given a numeric arg. /autocommit prints a status card.
All commands are blocked while an assistant turn is generating.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 10: Integration tests

**Files:**
- Create: `tests/autocommit_integration.rs`

- [ ] **Step 1: Write the full integration test file**

Create `tests/autocommit_integration.rs` with the following content:

```rust
//! End-to-end tests for the auto-commit loop against real `git init` tempdirs.

#![cfg(unix)]

use rustyclaw::autocommit::{
    is_git_repo, prune_old_refs, restore_to, snapshot_turn, AutoCommitConfig, SnapshotOutcome,
    SHADOW_REF_PREFIX,
};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn git_init(path: &Path) {
    let s = Command::new("git")
        .args(["init", "--quiet", "--initial-branch=main"])
        .current_dir(path)
        .status()
        .unwrap();
    assert!(s.success());
    for (k, v) in [
        ("user.name", "rustyclaw-test"),
        ("user.email", "noreply@rustyclaw.local"),
        ("commit.gpgsign", "false"),
    ] {
        Command::new("git")
            .args(["config", k, v])
            .current_dir(path)
            .status()
            .unwrap();
    }
}

fn write(path: &Path, rel: &str, body: &str) {
    let p = path.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, body).unwrap();
}

fn read(path: &Path, rel: &str) -> String {
    fs::read_to_string(path.join(rel)).unwrap()
}

fn commit_initial(path: &Path) {
    write(path, "README.md", "base\n");
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(path)
        .status()
        .unwrap();
    Command::new("git")
        .args(["commit", "-q", "-m", "base"])
        .current_dir(path)
        .status()
        .unwrap();
}

#[test]
fn full_3_turn_sequence_with_undo_redo() {
    let td = TempDir::new().unwrap();
    git_init(td.path());
    commit_initial(td.path());

    let cfg = AutoCommitConfig::default();
    let mut commits: Vec<String> = Vec::new();
    let mut pos = 0usize;

    write(td.path(), "app.txt", "v1\n");
    snapshot_turn(td.path(), &cfg, "s", "turn 1", 1, &mut commits, &mut pos).unwrap();
    write(td.path(), "app.txt", "v2\n");
    snapshot_turn(td.path(), &cfg, "s", "turn 2", 2, &mut commits, &mut pos).unwrap();
    write(td.path(), "app.txt", "v3\n");
    snapshot_turn(td.path(), &cfg, "s", "turn 3", 3, &mut commits, &mut pos).unwrap();

    assert_eq!(commits.len(), 3);
    assert_eq!(pos, 3);
    assert_eq!(read(td.path(), "app.txt"), "v3\n");

    // /undo 1 → turn 2 state
    restore_to(td.path(), &commits, 2).unwrap();
    pos = 2;
    assert_eq!(read(td.path(), "app.txt"), "v2\n");

    // /redo 1 → turn 3 state
    restore_to(td.path(), &commits, 3).unwrap();
    pos = 3;
    assert_eq!(read(td.path(), "app.txt"), "v3\n");
}

#[test]
fn undo_past_session_start_is_clamped() {
    let td = TempDir::new().unwrap();
    git_init(td.path());
    commit_initial(td.path());

    let cfg = AutoCommitConfig::default();
    let mut commits: Vec<String> = Vec::new();
    let mut pos = 0usize;

    write(td.path(), "a.txt", "1\n");
    snapshot_turn(td.path(), &cfg, "s", "a", 1, &mut commits, &mut pos).unwrap();
    write(td.path(), "a.txt", "2\n");
    snapshot_turn(td.path(), &cfg, "s", "b", 2, &mut commits, &mut pos).unwrap();

    // Simulate `/undo 5` via saturating_sub → target_position = 0 (session base)
    let new_pos = pos.saturating_sub(5);
    assert_eq!(new_pos, 0);
    restore_to(td.path(), &commits, new_pos).unwrap();
    // Session base had the initial "base\n" README with no a.txt.
    assert!(!td.path().join("a.txt").exists() || read(td.path(), "a.txt") != "2\n");
}

#[test]
fn redo_stack_discarded_on_new_turn() {
    let td = TempDir::new().unwrap();
    git_init(td.path());
    commit_initial(td.path());

    let cfg = AutoCommitConfig::default();
    let mut commits: Vec<String> = Vec::new();
    let mut pos = 0usize;

    write(td.path(), "a.txt", "1\n");
    snapshot_turn(td.path(), &cfg, "s", "a", 1, &mut commits, &mut pos).unwrap();
    write(td.path(), "a.txt", "2\n");
    snapshot_turn(td.path(), &cfg, "s", "b", 2, &mut commits, &mut pos).unwrap();
    write(td.path(), "a.txt", "3\n");
    snapshot_turn(td.path(), &cfg, "s", "c", 3, &mut commits, &mut pos).unwrap();
    assert_eq!(commits.len(), 3);

    // Simulate /undo 2 → pos = 1
    pos = 1;
    write(td.path(), "a.txt", "new2\n");
    snapshot_turn(td.path(), &cfg, "s", "new", 2, &mut commits, &mut pos).unwrap();
    assert_eq!(commits.len(), 2, "old turns 2 + 3 should be discarded");
    assert_eq!(pos, 2);
}

#[test]
fn disabled_config_emits_no_commits() {
    let td = TempDir::new().unwrap();
    git_init(td.path());
    commit_initial(td.path());

    let cfg = AutoCommitConfig {
        enabled: false,
        ..AutoCommitConfig::default()
    };
    let mut commits: Vec<String> = Vec::new();
    let mut pos = 0usize;

    write(td.path(), "x.txt", "1\n");
    let out = snapshot_turn(td.path(), &cfg, "s", "x", 1, &mut commits, &mut pos).unwrap();
    assert!(matches!(out, SnapshotOutcome::Disabled { .. }));
    assert!(commits.is_empty());
    assert_eq!(pos, 0);
}

#[test]
fn non_git_dir_is_disabled() {
    let td = TempDir::new().unwrap(); // no git init
    assert!(!is_git_repo(td.path()));
    let cfg = AutoCommitConfig::default();
    let mut commits: Vec<String> = Vec::new();
    let mut pos = 0usize;
    let out = snapshot_turn(td.path(), &cfg, "s", "x", 1, &mut commits, &mut pos).unwrap();
    assert!(matches!(out, SnapshotOutcome::Disabled { .. }));
}

#[test]
fn prune_integration_15_refs_keeps_10() {
    let td = TempDir::new().unwrap();
    git_init(td.path());
    commit_initial(td.path());

    let cfg = AutoCommitConfig::default();
    // Create 15 sessions with 1 turn each.
    for i in 0..15 {
        let mut commits: Vec<String> = Vec::new();
        let mut pos = 0usize;
        write(td.path(), "app.txt", &format!("v{i}\n"));
        snapshot_turn(
            td.path(),
            &cfg,
            &format!("s{i}"),
            "t",
            1,
            &mut commits,
            &mut pos,
        )
        .unwrap();
    }

    let deleted = prune_old_refs(td.path(), 10).unwrap();
    assert_eq!(deleted, 5);

    let out = Command::new("git")
        .args(["for-each-ref", "--format=%(refname)", SHADOW_REF_PREFIX])
        .current_dir(td.path())
        .output()
        .unwrap();
    let remaining = String::from_utf8(out.stdout).unwrap();
    assert_eq!(remaining.lines().count(), 10);
}
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test --test autocommit_integration 2>&1 | tail -30`
Expected: 6 passed, 0 failed.

- [ ] **Step 3: Run the full test suite**

Run: `cargo test 2>&1 | tail -20`
Expected: all tests pass.

- [ ] **Step 4: Clippy**

Run: `cargo clippy --all-targets -- -W clippy::all 2>&1 | rg '^warning' | wc -l`
Expected: ≤ 217.

- [ ] **Step 5: Commit**

```bash
git add tests/autocommit_integration.rs
git commit -m "$(cat <<'EOF'
test(autocommit): end-to-end integration tests

6 tests against real git init tempdirs: full 3-turn sequence with
undo/redo, clamp on /undo past session start, redo-stack discard on
new work, disabled config, non-git directory, and 15-ref prune.
Gated by #![cfg(unix)] because the spec targets POSIX git behavior.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Task 11: Docs — README, CHANGELOG, CLAUDE.md

**Files:**
- Modify: `README.md`
- Modify: `CHANGELOG.md`
- Modify: `CLAUDE.md`

- [ ] **Step 1: Add a README Highlights bullet**

Open `README.md`. Find the Highlights list (search `rg -n 'Highlights' README.md`). Add a new bullet alongside the auto-fix loop bullet:

```markdown
- **Auto-commit snapshots with `/undo` and `/redo`** — Every assistant turn silently snapshots the working tree to a private shadow ref (`refs/rustyclaw/sessions/<id>`). Navigate history with a picker (`/undo`, `/redo`) or skip straight to a turn (`/undo 3`). Invisible to normal git tooling; never pushed.
```

- [ ] **Step 2: Add a CHANGELOG entry**

Open `CHANGELOG.md`. Find the `[Unreleased]` → `Added` section (create both if missing) and insert:

```markdown
### Added
- **Auto-commit loop** (Phase 2 robustness #1): every assistant turn now takes a full-tree snapshot on a private shadow ref at `refs/rustyclaw/sessions/<id>`. New `/undo`, `/redo`, and `/autocommit` slash commands. `autoCommit.{enabled,keepSessions,messagePrefix}` settings with startup prune. Zero impact on the user's real git index.
```

- [ ] **Step 3: Update `CLAUDE.md`**

Open the project-level `CLAUDE.md` (the one at `/mnt/Storage/Projects/RustyClaw/CLAUDE.md`). Find the `### PHASE 2 (shipping now)` section and the `### NEXT UP` section. Move the auto-commit bullet from NEXT UP (item 7) into PHASE 2 as a shipped item:

In `### PHASE 2 (shipping now)`, append after the auto-fix loop bullet:

```markdown
- **Auto git commits + /undo + /redo (2026-04-10)** — Per-turn working-tree snapshots on private shadow refs (`refs/rustyclaw/sessions/<id>`). New `/undo`, `/redo`, `/autocommit` slash commands. Keeps 10 newest session refs with startup prune. [redacted] has `/undo` but pollutes history; RustyClaw's shadow refs are invisible to `git log`/`branch`/`status`. No competitor has `/redo`.
```

In `### NEXT UP`, remove "Auto git commits + /undo" from the list (it's now shipped).

- [ ] **Step 4: Build + full test sweep**

```bash
cargo build 2>&1 | tail -10 && cargo test 2>&1 | tail -10 && cargo clippy --all-targets -- -W clippy::all 2>&1 | rg '^warning' | wc -l
```

Expected: clean build, all tests pass, clippy ≤ 217.

- [ ] **Step 5: Commit**

```bash
git add README.md CHANGELOG.md CLAUDE.md
git commit -m "$(cat <<'EOF'
docs: ship auto-commit + /undo + /redo (Phase 2 #1)

README Highlights bullet, CHANGELOG Unreleased entry, and CLAUDE.md
moved from NEXT UP → PHASE 2 SHIPPED.

Co-Authored-By: Arch Linux <noreply@archlinux.org>
EOF
)"
```

---

## Manual QA checklist

After all tasks are complete, run the binary and verify by eye:

- [ ] `cargo run --release` launches normally in the RustyClaw repo
- [ ] Run a turn that modifies a file (e.g., ask the model to add a comment to a random file)
- [ ] Run `git for-each-ref refs/rustyclaw/` — should show one ref for the current session
- [ ] Run `/autocommit` — should print status with "Turns recorded: 1"
- [ ] Modify a file in a second turn
- [ ] Run `/undo` — picker opens with "turn 2", "turn 1", "session base" rows
- [ ] Select "turn 1" — chat shows `[undo] rewound to turn 1 (N files restored)` and the file is restored
- [ ] Run `/redo` — picker opens with "turn 1 ← current", "turn 2"
- [ ] Select "turn 2" — file returns to turn 2 state
- [ ] Run `/undo 2` — should rewind to session base
- [ ] Run `/undo` in a non-git directory — prints disabled message, doesn't crash
- [ ] Add `"autoCommit": {"enabled": false}` to `.claude/settings.json`, restart — `/undo` says disabled
- [ ] Confirm `git log` / `git branch` / `git status` in the repo show NO trace of auto-commits
