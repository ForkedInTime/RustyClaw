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
use std::process::{Command, Stdio};

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

// ── Snapshot pipeline ─────────────────────────────────────────────────────────

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
    Ok(s.trim().to_string())
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

/// Look up the tree SHA of a given commit SHA. Returns `None` if the lookup
/// fails, so the call site can distinguish "not found" from a valid tree SHA.
fn tree_of_commit(cwd: &Path, commit: &str) -> Option<String> {
    git_cmd(cwd)
        .args(["rev-parse", &format!("{commit}^{{tree}}")])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
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

/// Assumes a single writer per `session_id`; concurrent snapshots with the same session id race on the shadow ref and are not supported.
///
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
        if let Some(tree) = tree_of_commit(cwd, p) {
            let s = git_cmd(cwd)
                .env("GIT_INDEX_FILE", &temp_index)
                .args(["read-tree", &tree])
                .status()?;
            if !s.success() {
                anyhow::bail!("git read-tree {tree} failed");
            }
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
        if let Some(parent_tree) = tree_of_commit(cwd, p) {
            if parent_tree == tree_sha {
                return Ok(SnapshotOutcome::NoChanges);
            }
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
    fn snapshot_works_on_unborn_head() {
        // Fresh repo — no commits, HEAD is unborn.
        let td = init_test_repo();
        write_file(td.path(), "a.txt", "hello\n");

        let cfg = AutoCommitConfig::default();
        let mut commits = Vec::new();
        let mut pos = 0usize;
        let outcome = snapshot_turn(
            td.path(),
            &cfg,
            "s-new",
            "first turn",
            1,
            &mut commits,
            &mut pos,
        )
        .unwrap();

        let sha = match outcome {
            SnapshotOutcome::Committed { sha, files } => {
                assert_eq!(files, 1, "only a.txt should be in the tree");
                sha
            }
            other => panic!("expected Committed, got {other:?}"),
        };

        // The commit must have no parent line.
        let out = git_cmd(td.path())
            .args(["cat-file", "-p", &sha])
            .output()
            .unwrap();
        let info = String::from_utf8(out.stdout).unwrap();
        assert!(
            !info.contains("parent "),
            "unborn-HEAD commit should have no parent, got:\n{info}"
        );
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
