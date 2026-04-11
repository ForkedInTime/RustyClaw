# Auto git commits + `/undo` + `/redo` — Design Spec

**Status:** approved
**Date:** 2026-04-10
**Phase:** Phase 2 robustness (feature 1 of 5)
**Branch:** `feature/auto-commit-undo`

## Goal

After every assistant turn that produced file changes, RustyClaw silently snapshots the full working tree as a git commit on a private shadow ref, giving the user a per-turn undo/redo history that's invisible to normal git tooling but recoverable via `/undo` and `/redo` slash commands.

## Motivation

The auto-fix loop we just shipped tells users "on retry-cap, use `/undo` to revert" — but `/undo` doesn't exist yet. This spec closes that gap and provides the missing safety net: every RustyClaw turn becomes a checkpoint the user can rewind to without `git reflog` archaeology.

Competitive landscape:
- **[redacted]** — auto-commits per turn but uses normal branch commits (pollutes history), has `/undo` but no `/redo`
- **Claude Code** — no auto-commit, no `/undo`
- **[redacted]** — no auto-commit, no `/undo`
- **[redacted]** — in-editor undo stack, doesn't cross sessions

RustyClaw's differentiator: **shadow refs** (zero history pollution) + **interactive picker** (turn snippets, timestamps, file counts) + **session-resume survival** (undo across restarts) + **`/redo`** (bidirectional navigation).

## Non-goals (v1)

- Bare repos, submodule recursion, LFS blob capture
- Divergent lineage browsing after `/undo`+new-work (redo stack is silently discarded on new work, matching every editor's muscle memory)
- Pushing shadow refs to origin (they are strictly local)
- CLI flag (`--auto-commit`) — settings only
- Manual checkpoint commands (`/checkpoint`) — deferred
- TUI integration tests for overlay wiring — manual QA

## Storage model

### Ref namespace

- All auto-commits live under `refs/rustyclaw/sessions/<session-id>` — one ref per session, pointing at the tip of that session's commit lineage
- Invisible to `git log`, `git branch`, `git status` — only visible via `git for-each-ref refs/rustyclaw/`
- Never pushed (no `refs/heads/` or `refs/tags/` — outside the default push refspec)
- Session ID matches the existing session ID from [src/session/mod.rs:20](src/session/mod.rs#L20) (`SessionMeta.id`)

### Commit shape

| Field | Value |
|---|---|
| Tree | Full working tree via `git write-tree` on a temp index built from `git add -A` |
| Parent | Previous turn's commit SHA, or user's `HEAD` SHA at session start for turn 1 |
| Author name | `rustyclaw` |
| Author email | `noreply@rustyclaw.local` |
| Committer | Same as author |
| Subject line | `<messagePrefix> turn <N>: <first 60 chars of user prompt>` (default prefix: `rustyclaw`) |
| Trailers | `RustyClaw-Session: <id>`, `RustyClaw-Turn: <N>`, `RustyClaw-Files: <count>` |

Example:
```
rustyclaw turn 3: add retry loop to auto-fix check

RustyClaw-Session: 2026-04-10T14-23-05-abc123
RustyClaw-Turn: 3
RustyClaw-Files: 5
```

### Lineage tracking in SessionMeta

`SessionMeta` in [src/session/mod.rs:19](src/session/mod.rs#L19) gains two new fields:

```rust
pub struct SessionMeta {
    pub id: String,
    // ... existing fields ...
    /// Auto-commit SHAs in chronological order (oldest → newest). Empty if
    /// auto-commit is disabled or the session is in a non-git directory.
    #[serde(default)]
    pub auto_commits: Vec<String>,
    /// User's current read-head in `auto_commits`. `0` means "at session base,
    /// before any auto-commits"; `auto_commits.len()` means "at latest turn".
    /// `/undo` decrements; `/redo` increments. Lateral new work truncates the
    /// vector at this position before appending.
    #[serde(default)]
    pub undo_position: usize,
}
```

Backward compatibility: both fields use `#[serde(default)]`, so existing `.meta` files on disk load cleanly with empty auto-commit history.

## Commit mechanics (plumbing, not porcelain)

We never invoke `git add` or `git commit` directly because those mutate the user's real index. Instead, we use git plumbing with `GIT_INDEX_FILE` pointed at a tempdir:

1. **Create temp index:** `let temp_index = tempdir.path().join("turn.index")`
2. **Stage everything:** `GIT_INDEX_FILE=<temp_index> git add -A` (respects `.gitignore`, honors user's `.git/info/exclude`)
3. **Write tree:** `GIT_INDEX_FILE=<temp_index> git write-tree` → `<tree-sha>`
4. **Build commit:** `git commit-tree <tree-sha> -p <parent-sha> -m <msg>` with `GIT_AUTHOR_NAME` / `GIT_AUTHOR_EMAIL` / `GIT_COMMITTER_NAME` / `GIT_COMMITTER_EMAIL` env overrides → `<commit-sha>`
5. **Update shadow ref:** `git update-ref refs/rustyclaw/sessions/<id> <commit-sha>`
6. **Persist SHA:** append to `SessionMeta.auto_commits`, increment `undo_position` to `auto_commits.len()`, save `.meta` file

Total: ~4 subprocess calls per turn, all <50ms on a warm repo. User's real `.git/index` is untouched.

### Empty-turn optimization

After step 3, compare the new tree SHA to the parent commit's tree SHA. If identical, the turn produced no file changes — skip commit, log `[autocommit] no changes to snapshot`. This is common for pure-read turns (grep, glob, read).

### Parent resolution

- **First turn of a session, git HEAD exists:** parent = `git rev-parse HEAD`
- **First turn of a session, unborn branch (no HEAD):** parent = none; use `git commit-tree <tree>` without `-p` (root commit)
- **Subsequent turns:** parent = `auto_commits.last().unwrap()`
- **After `/undo`:** parent = `auto_commits[undo_position - 1]` or none if `undo_position == 0`

## Restore mechanics

`/undo` and `/redo` restore the working tree to a target commit without touching the user's real index:

1. **Build temp index from target tree:** `GIT_INDEX_FILE=<temp_index> git read-tree <target-tree-sha>`
2. **Write files to working tree:** `GIT_INDEX_FILE=<temp_index> git checkout-index -a -f --prefix=<cwd>/`

The `-a` flag extracts all entries in the temp index; `-f` overwrites existing files; `--prefix` is required because `checkout-index` defaults to the index's own cwd.

**Untracked files are sacred.** We do NOT run `git clean` — if the user created a new file during a turn that we're undoing, it stays. Only files that were tracked in the target commit get overwritten. Files that were in the current working tree but NOT in the target tree also stay (orphaned from the user's perspective — they can manually `git rm` if desired).

**Edge case:** if the target commit deleted a file that exists in the current working tree, `checkout-index` alone won't remove it. v1 behavior: leave it alone with a `[undo] note: 1 file(s) exist in working tree but not in target — run 'git status' to review`. v2 could offer `/undo --clean` to force deletion.

## `/undo` command flow

### `/undo` (no args)

1. If `autoCommit.enabled == false` → emit `[undo] auto-commit is disabled in settings` and return
2. If not a git repo → emit `[undo] auto-commit disabled — not a git repo` and return
3. If `auto_commits.is_empty()` → emit `[undo] nothing to undo (session has no auto-commits)` and return
4. If `undo_position == 0` → emit `[undo] at session start, nothing more to undo` and return
5. Open Overlay picker titled `"undo"`, listing `auto_commits[0..undo_position]` in reverse order (newest first), plus a synthetic final row for the session base. Each row format:
   ```
   turn 5  ·  3 files  ·  14:23  ·  "add retry loop to auto-fix"  ← current
   turn 4  ·  1 file   ·  14:19  ·  "fix typo in README"
   turn 3  ·  7 files  ·  14:12  ·  "implement run_auto_fix_check"
   turn 2  ·  1 file   ·  14:05  ·  "rename struct field"
   turn 1  ·  2 files  ·  13:55  ·  "initial edit"
   session base (pre-RustyClaw)
   ```
   The `← current` marker sits on the row for turn `undo_position` (i.e., `auto_commits[undo_position - 1]`).
6. User picks target with arrows+Enter; Esc cancels (no-op)
7. Compute `new_position`: "pick turn N" → `new_position = N`; "pick session base" → `new_position = 0`. Picking the `← current` row is a no-op. Semantically, "pick turn N" means "land at the state produced by turn N".
8. If `new_position == 0`, restore working tree from the session base tree (parent commit of `auto_commits[0]`, or an empty tree on an unborn-branch session). Otherwise restore from `auto_commits[new_position - 1]`.
9. Save updated `.meta` with `undo_position = new_position`
10. Emit `SystemMessage: [undo] rewound to turn N (M files restored)` (or `[undo] rewound to session base (M files restored)` for `new_position == 0`)

### `/undo N`

Skip the picker. `new_position = undo_position.saturating_sub(N)`. If N was larger than available, append warning `[undo] reached session start (wanted N=5, only 3 turns available)`. Otherwise emit same message as above.

### `/undo 0` or `/undo` with invalid arg

Treat as `/undo` (no args) and show the picker.

## `/redo` command flow

### `/redo` (no args)

1. Same disabled/non-git checks as `/undo`
2. If `undo_position == auto_commits.len()` → emit `[redo] nothing to redo (at latest turn)` and return
3. Open Overlay picker titled `"redo"`, listing the current position first and then `auto_commits[undo_position..]` in forward order. If `undo_position == 0`, the "current" row is a synthetic `session base (pre-RustyClaw) ← current`; otherwise it's `auto_commits[undo_position - 1]`:
   ```
   turn 3  ·  7 files  ·  14:12  ·  "implement run_auto_fix_check"  ← current
   turn 4  ·  1 file   ·  14:19  ·  "fix typo in README"
   turn 5  ·  3 files  ·  14:23  ·  "add retry loop to auto-fix"
   ```
4. User picks; Esc cancels. Compute `new_position = picked_turn_N` (same "land at turn N" semantics as `/undo`). Picking the `← current` row is a no-op.
5. Restore working tree from `auto_commits[new_position - 1]`
6. Save `.meta` with `undo_position = new_position`
7. Emit `SystemMessage: [redo] advanced to turn N (M files restored)`

### `/redo N`

Skip picker. `new_position = (undo_position + N).min(auto_commits.len())`.

## Redo stack discard on new work

When an assistant turn fires its auto-commit and `undo_position < auto_commits.len()` (user is in an undone state):

1. Truncate `auto_commits` at `undo_position` (drops the orphaned future turns)
2. Append the new commit SHA
3. `undo_position = auto_commits.len()`
4. Update shadow ref to point at the new commit (it no longer has a path to the discarded SHAs — they become unreferenced and will be GC'd in a future `git gc` cycle)

No warning or prompt — matches universal editor behavior.

## `/autocommit status` command

New slash command showing the current auto-commit state:

```
Auto-commit status:
  Enabled:        yes
  Repo:           git detected (cwd)
  Session ID:     2026-04-10T14-23-05-abc123
  Turns recorded: 5
  Undo position:  5 / 5 (at latest)
  Retention:      keep 10 most recent sessions
  Message prefix: "rustyclaw"
  Shadow ref:     refs/rustyclaw/sessions/2026-04-10T14-23-05-abc123
  Shadow ref tip: abc1234 — "rustyclaw turn 5: add retry loop..."
```

Exits with a `SystemMessage`, no overlay.

## Configuration

New top-level key in `settings.json`:

```json
{
  "autoCommit": {
    "enabled": true,
    "keepSessions": 10,
    "messagePrefix": "rustyclaw"
  }
}
```

### Fields

| Field | Default | Range | Clamp behavior |
|---|---|---|---|
| `enabled` | `true` | `bool` | — |
| `keepSessions` | `10` | `0..=1000` (0 = unlimited) | Out of range → warn + clamp to default (10) |
| `messagePrefix` | `"rustyclaw"` | any string (empty allowed) | — |

### Rust types

```rust
// src/autocommit.rs
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
            keep_sessions: 10,
            message_prefix: "rustyclaw".to_string(),
        }
    }
}

// src/settings.rs
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AutoCommitSettings {
    pub enabled: Option<bool>,
    pub keep_sessions: Option<u32>,
    pub message_prefix: Option<String>,
}
```

Parsed in `Config::load` with clamping logic mirroring the `max_retries` pattern from `autofix`.

## Startup prune

After config loads, before TUI first draw:

1. If not a git repo or `autoCommit.enabled == false` or `keep_sessions == 0` → skip
2. `git for-each-ref --format='%(refname) %(committerdate:unix)' refs/rustyclaw/sessions/`
3. Parse output, sort descending by committer date
4. For each ref past `keep_sessions`: `git update-ref -d <refname>`
5. If any refs were deleted, spawn `git gc --auto` as a detached background process (no `.wait()`)

Non-fatal: any error → `tracing::warn!("autoCommit prune failed: {e}")`, continue startup. Prune runs <100ms for typical ref counts (<200); no async needed.

## Module structure

### New files

- **`src/autocommit.rs`** (~700 lines) — all git plumbing, snapshot creation, restore, prune. Mirrors `src/autofix.rs` structure.
  - `pub struct AutoCommitConfig`
  - `pub enum AutoCommitStatus { Committed { sha, files }, NoChanges, Disabled { reason }, Failed { reason } }`
  - `pub fn snapshot_turn(cwd, meta, config, user_prompt) -> AutoCommitStatus`
  - `pub fn restore_to(cwd, meta, target_position) -> Result<RestoreReport>`
  - `pub fn prune_old_refs(cwd, keep_sessions) -> Result<u32>`
  - `pub fn is_git_repo(cwd) -> bool`
  - Unit tests: ~12 tests against `tempdir()` fixtures with fresh `git init`

- **`tests/autocommit_integration.rs`** (~200 lines) — end-to-end flow with real git commands:
  - 3-turn snapshot sequence → verify lineage via `git log`
  - `/undo` flow: 3 turns → undo 1 → verify file contents match turn 2
  - `/redo` flow: undo 2 → redo 1 → verify file contents match turn 2
  - Redo stack discard: undo 1 → new turn → verify turn 3 no longer reachable
  - Prune: create 15 fake refs → prune(keep=10) → assert 10 remain
  - Session resume: save meta → reload meta → `/undo` still works
  - ~6 tests total

### Modified files

- **`src/session/mod.rs`** — add `auto_commits` and `undo_position` fields to `SessionMeta` with `#[serde(default)]`
- **`src/config.rs`** — parse `autoCommit` settings → `AutoCommitConfig`, clamp `keep_sessions`
- **`src/settings.rs`** — add `AutoCommitSettings` struct, `auto_commit: Option<AutoCommitSettings>` field on `Settings` with `#[serde(rename = "autoCommit")]`
- **`src/commands/mod.rs`** — register `/undo`, `/redo`, `/autocommit` slash commands; add `CommandAction::Undo { n }`, `CommandAction::Redo { n }`, `CommandAction::AutoCommitStatus` variants
- **`src/tui/run.rs`** —
  - Call `autocommit::prune_old_refs` after config load
  - Call `autocommit::snapshot_turn` at end of each assistant turn (after auto-fix loop completes, replacing `auto_fix_touched.clear()` with a snapshot-then-clear sequence)
  - Handle `CommandAction::Undo` / `Redo` by opening an overlay populated from `meta.auto_commits` with turn metadata
  - Handle `CommandAction::AutoCommitStatus` by emitting a formatted SystemMessage
- **`src/lib.rs`** — `pub mod autocommit;`
- **`README.md`** — new Highlights bullet
- **`CHANGELOG.md`** — Unreleased Added entry
- **`CLAUDE.md`** — move from NEXT UP to PHASE 2 SHIPPED

## Testing strategy

### Unit tests (in `src/autocommit.rs`, ~12 tests)

- `snapshot_turn_creates_commit_with_correct_parent` — write file → snapshot → assert parent matches HEAD
- `snapshot_turn_chains_commits` — 3 snapshots → assert each parent matches previous
- `snapshot_turn_skips_empty_tree` — snapshot with no changes → `AutoCommitStatus::NoChanges`
- `snapshot_turn_disabled_when_not_git_repo` — non-git tempdir → `Disabled { reason: "not a git repo" }`
- `snapshot_turn_disabled_when_config_disabled` — `enabled: false` → `Disabled { reason: "config.enabled = false" }`
- `snapshot_respects_gitignore` — gitignored file → not included in tree
- `restore_to_overwrites_modified_files` — snapshot file v1 → modify to v2 → restore → assert v1
- `restore_to_leaves_untracked_files` — create untracked file → restore to earlier commit → untracked file survives
- `restore_to_session_base_zero_position` — restore to `undo_position=0` → matches pre-session HEAD
- `prune_keeps_newest_n_refs` — 15 refs with staggered timestamps → prune(keep=10) → 10 newest remain
- `prune_noop_when_unlimited` — `keep_sessions=0` → nothing pruned
- `is_git_repo_detection` — cwd with `.git/` → true; without → false

### Integration tests (in `tests/autocommit_integration.rs`, ~6 tests)

- `full_turn_sequence_with_undo_redo` — 3 turns → `/undo` 1 → verify turn 2 files → `/redo` 1 → verify turn 3
- `undo_past_session_start_is_clamped` — 2 turns → `/undo 5` → lands at session base with warning
- `redo_stack_discarded_on_new_turn` — 3 turns → `/undo 1` → new 4th turn → verify `auto_commits.len() == 3` (old turn 3 gone)
- `session_resume_preserves_undo_history` — 3 turns → save meta → fresh load → `/undo 1` works
- `prune_integration_15_refs` — create 15 fake session refs → prune → 10 remain
- `disabled_config_no_commits` — `enabled: false` → 3 turns → `auto_commits` stays empty

### TUI overlay wiring: manual QA only

Same rationale as the auto-fix loop's Task 11: overlay picker UX is best verified by eye. The underlying `/undo`/`/redo` logic is fully covered by unit + integration tests against the public API in `src/autocommit.rs`. Manual QA checklist added to the plan's final verification section.

## Implementation risks

| Risk | Mitigation |
|---|---|
| `GIT_INDEX_FILE` env var leaks into subsequent subprocesses | Scope each git invocation to its own `Command` builder; never set it globally |
| User runs `git gc --aggressive` and loses our objects | Shadow refs pin objects; safe. Prune also triggers `git gc --auto` on our own to keep the store tidy |
| Concurrent RustyClaw sessions racing on the same ref | Session IDs are unique per process start (timestamp + random); refs don't collide |
| `/undo` during an in-flight assistant turn | Disallow: `/undo` is ignored while `app.generating == true`, emit `[undo] cannot undo during assistant turn` |
| Symlinks in the working tree | `git add -A` handles them correctly (stores link contents); `checkout-index` restores them |
| File permissions drift (chmod'd files) | Git tracks mode bits on tracked files; `checkout-index` restores mode; untracked mode changes are not captured (acceptable) |
| `messagePrefix` containing git-special characters | Pass prefix to `commit-tree` as `-m` arg (not shell-interpolated); git handles escaping |

## Out of scope (v2+)

- Shadow ref garbage collection granularity (`/autocommit gc`)
- Cross-session undo ("undo back to yesterday's session 5 turns deep")
- Divergent lineage browsing (post-undo branches in the picker)
- Push/share shadow refs between machines (would require opt-in `refs/rustyclaw/shared/`)
- Auto-commit on sandbox-escaped Bash changes (already covered — full tree snapshot)
- Multi-cwd / submodule recursion
- LFS blob snapshotting
- Conflict resolution when `/undo` encounters user's uncommitted manual changes mid-session

## Acceptance criteria

1. **Snapshot creation:** a turn that Writes/Edits 3 files produces exactly 1 commit on the session's shadow ref with the correct parent linkage
2. **Empty turn skip:** a read-only turn (grep/glob only) produces zero commits
3. **`/undo` basic:** after 3 turns, `/undo 1` restores file contents to turn 2 state
4. **`/undo` picker:** `/undo` with no args opens an overlay with turn snippets and timestamps
5. **`/redo` basic:** after `/undo 1`, `/redo 1` restores file contents to turn 3 state
6. **Redo stack discard:** after `/undo 1` then new turn 3', old turn 3 is unreachable (not in `auto_commits`)
7. **Session resume:** save → exit → restart → `/undo` still works with same history
8. **Prune on startup:** 15 old session refs + `keepSessions=10` → 5 oldest are deleted
9. **Non-git fallback:** `/undo` in a non-git directory prints a clear disabled message and doesn't crash
10. **Disabled config:** `enabled: false` → zero auto-commits created, `/undo` says disabled
11. **Settings round-trip:** `autoCommit` JSON parses correctly, clamps out-of-range values, defaults apply
12. **`/autocommit status`:** prints accurate state for enabled and disabled cases
13. **Full test suite:** all pre-existing tests still pass (284 → ~302 with new tests)
14. **Clippy baseline:** warnings stay ≤ 217

## Deliverables

- `src/autocommit.rs` (new, ~700 LOC)
- `src/session/mod.rs` (modified, +2 fields)
- `src/config.rs` (modified, auto_commit settings parsing + clamp)
- `src/settings.rs` (modified, `AutoCommitSettings` struct)
- `src/commands/mod.rs` (modified, 3 new commands + `CommandAction` variants)
- `src/tui/run.rs` (modified, snapshot wiring + overlay dispatch)
- `src/lib.rs` (modified, `pub mod autocommit`)
- `tests/autocommit_integration.rs` (new, ~6 tests)
- `README.md`, `CHANGELOG.md`, `CLAUDE.md` (docs refresh)

Total: ~1000 LOC added, ~18 new tests.
